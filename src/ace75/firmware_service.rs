use super::bootloader::BootloaderRejected;
use super::firmware_catalog::{
    KeyboardFirmwareInfo, MAX_KEYBOARD_IMAGE_BYTES, verify_official_keyboard,
};
use super::{Ace75, Ace75Bootloader, BOOTLOADER_PRODUCT_ID, KeyboardFirmwarePlan, PRODUCT_ID};
use crate::transport::{self, DeviceInfo, DeviceKind, HidTransport};
use anyhow::{Context, Result, bail, ensure};
use std::{
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

pub struct PreparedKeyboardFirmware {
    info: KeyboardFirmwareInfo,
    plan: KeyboardFirmwarePlan,
}

impl PreparedKeyboardFirmware {
    pub fn verify_file(path: &Path) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .with_context(|| format!("ACE75: не удалось открыть прошивку {}", path.display()))?
            .take(MAX_KEYBOARD_IMAGE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .context("ACE75: не удалось прочитать файл прошивки")?;
        ensure!(
            bytes.len() <= MAX_KEYBOARD_IMAGE_BYTES,
            "ACE75: образ прошивки слишком большой"
        );
        let info = verify_official_keyboard(&bytes)?;
        let plan = KeyboardFirmwarePlan::new(
            bytes,
            info.sha256
                .as_deref()
                .context("ACE75: отсутствует официальный SHA-256")?,
        )?;
        Ok(Self { info, plan })
    }

    pub fn info(&self) -> &KeyboardFirmwareInfo {
        &self.info
    }
}

fn is_keyboard(info: &DeviceInfo) -> bool {
    info.vendor_id == transport::VENDOR_ID
        && match info.kind {
            DeviceKind::Keyboard => {
                info.product_id == PRODUCT_ID
                    && info.usage_page == 1
                    && info.usage == 0
                    && info.name == super::USB_PRODUCT
            }
            DeviceKind::KeyboardBootloader => {
                info.product_id == BOOTLOADER_PRODUCT_ID
                    && info.usage_page == 0xff00
                    && info.usage == 1
            }
            _ => false,
        }
}

fn unique_keyboard(devices: &[DeviceInfo]) -> Result<Option<DeviceInfo>> {
    let selected: Vec<_> = devices.iter().filter(|info| is_keyboard(info)).collect();
    ensure!(
        selected.len() <= 1,
        "ACE75: подключено несколько клавиатур или загрузчиков; оставьте одно устройство для прошивки"
    );
    Ok(selected.first().map(|info| (*info).clone()))
}

fn wait_for(kind: DeviceKind, timeout: Duration) -> Result<DeviceInfo> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(info) = unique_keyboard(&transport::enumerate()?)? {
            if info.kind == kind {
                return Ok(info);
            }
        }
        ensure!(
            Instant::now() < deadline,
            "ACE75: устройство не появилось после переключения USB-режима"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The caller must stop host lighting, close its transport and save a full backup first.
pub fn flash_keyboard(
    device: &DeviceInfo,
    prepared: &PreparedKeyboardFirmware,
    mut progress: impl FnMut(usize, usize),
) -> Result<DeviceInfo> {
    ensure!(
        is_keyboard(device),
        "ACE75: выбрано неподходящее устройство для прошивки"
    );
    let current =
        unique_keyboard(&transport::enumerate()?)?.context("ACE75: клавиатура отключена")?;
    ensure!(
        current.path == device.path && current.kind == device.kind,
        "ACE75: USB-устройство изменилось; повторите выбор клавиатуры"
    );
    let boot = if current.kind == DeviceKind::Keyboard {
        let mut driver = Ace75::new(Box::new(HidTransport::open(&current)?));
        let enter_result = driver.enter_bootloader();
        drop(driver);
        if enter_result
            .as_ref()
            .err()
            .is_some_and(|error| error.downcast_ref::<BootloaderRejected>().is_some())
        {
            return Err(enter_result.unwrap_err()).context("ACE75: переход в загрузчик отклонён");
        }
        match wait_for(DeviceKind::KeyboardBootloader, Duration::from_secs(15)) {
            Ok(info) => info,
            Err(error) => {
                return match enter_result {
                    Ok(()) => Err(error),
                    Err(enter_error) => {
                        Err(error.context(format!("ACE75: переход в загрузчик: {enter_error:#}")))
                    }
                };
            }
        }
    } else {
        current
    };
    let mut loader = Ace75Bootloader::new(Box::new(HidTransport::open_keyboard_bootloader(&boot)?));
    let mut transfer_verified = false;
    let flash_result = loader.flash_firmware(&prepared.plan, |done, total| {
        // All write and compare packets have strict ACKs. Only the final two
        // restart flags may race a real USB disconnect.
        transfer_verified = done >= total.saturating_sub(2);
        progress(done, total);
    });
    drop(loader);
    if let Err(error) = &flash_result {
        if !transfer_verified || error.downcast_ref::<BootloaderRejected>().is_some() {
            bail!("ACE75: запись или проверка прошивки не завершена: {error:#}");
        }
    }
    let returned = match wait_for(DeviceKind::Keyboard, Duration::from_secs(20)) {
        Ok(info) => info,
        Err(error) => {
            return match flash_result {
                Ok(()) => Err(error),
                Err(flash_error) => {
                    Err(error.context(format!("ACE75: завершение прошивки: {flash_error:#}")))
                }
            };
        }
    };
    let mut driver = Ace75::new(Box::new(HidTransport::open(&returned)?));
    let actual = driver
        .read_identity()
        .context("ACE75: не удалось проверить клавиатуру после прошивки")?;
    ensure!(
        actual.firmware_version == prepared.info.version,
        "ACE75: после прошивки получена версия {}, ожидалась {}",
        actual.firmware_version,
        prepared.info.version
    );
    Ok(returned)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device(kind: DeviceKind, pid: u16, page: u16, usage: u16) -> DeviceInfo {
        DeviceInfo {
            path: "test".into(),
            name: "Ace 75".into(),
            vendor_id: 0x3837,
            product_id: pid,
            interface: 1,
            usage_page: page,
            usage,
            kind,
        }
    }
    #[test]
    fn recovery_accepts_only_the_exact_model_and_one_usb_role() {
        let normal = device(DeviceKind::Keyboard, 0x303c, 1, 0);
        let boot = device(DeviceKind::KeyboardBootloader, 0x303d, 0xff00, 1);
        assert!(unique_keyboard(&[normal.clone()]).unwrap().is_some());
        assert!(unique_keyboard(&[boot.clone()]).unwrap().is_some());
        assert!(unique_keyboard(&[normal.clone(), boot]).is_err());
        assert!(
            unique_keyboard(&[device(DeviceKind::Keyboard, 0x301d, 1, 0)])
                .unwrap()
                .is_none()
        );
        assert!(
            unique_keyboard(&[device(DeviceKind::Keyboard, 0x303c, 1, 6)])
                .unwrap()
                .is_none()
        );
        let mut wrong_vendor = normal;
        wrong_vendor.vendor_id = 0x1234;
        assert!(!is_keyboard(&wrong_vendor));
    }
}
