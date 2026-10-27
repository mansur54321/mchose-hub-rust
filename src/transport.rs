use anyhow::{Context, Result, bail};
use hidapi::{HidApi, HidDevice};
use serde::{Deserialize, Serialize};
use std::ffi::CString;

pub const VENDOR_ID: u16 = 0x3837;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Dock,
    Keyboard,
    KeyboardBootloader,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub path: String,
    pub name: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub kind: DeviceKind,
}

pub fn enumerate() -> Result<Vec<DeviceInfo>> {
    let api = HidApi::new().context("Не удалось прочитать список HID-устройств")?;
    let mut devices = Vec::new();
    for d in api.device_list().filter(|d| d.vendor_id() == VENDOR_ID) {
        let name = d.product_string().unwrap_or("");
        let Some(kind) = classify_device(
            d.vendor_id(),
            d.product_id(),
            name,
            d.usage_page(),
            d.usage(),
        ) else {
            continue;
        };
        let path = d.path().to_string_lossy().into_owned();
        if devices.iter().any(|entry: &DeviceInfo| entry.path == path) {
            continue;
        }
        devices.push(DeviceInfo {
            path,
            name: name.to_owned(),
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            interface: d.interface_number(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            kind,
        });
    }
    Ok(devices)
}

fn classify_device(
    vendor: u16,
    product: u16,
    name: &str,
    usage_page: u16,
    usage: u16,
) -> Option<DeviceKind> {
    if vendor != VENDOR_ID {
        return None;
    }
    match product {
        0x1012 if usage_page == 0xff00 && name == "MCHOSE MagDock" => Some(DeviceKind::Dock),
        0x4033 | 0x1014 | 0x1018 if usage_page == 0xff01 && name == crate::firmware::MODEL => {
            Some(DeviceKind::Mouse)
        }
        crate::ace75::PRODUCT_ID
            if usage_page == 1 && usage == 0 && name == crate::ace75::USB_PRODUCT =>
        {
            Some(DeviceKind::Keyboard)
        }
        crate::ace75::BOOTLOADER_PRODUCT_ID if usage_page == 0xff00 && usage == 1 => {
            Some(DeviceKind::KeyboardBootloader)
        }
        _ => None,
    }
}

/// Report IDs are separate from payloads. Input reports retain their ID byte.
pub trait HidIo: Send {
    fn send_feature(&self, report_id: u8, payload: &[u8]) -> Result<()>;
    fn get_feature(&self, report_id: u8, payload_len: usize) -> Result<Vec<u8>>;
    fn write_output(&self, report_id: u8, payload: &[u8]) -> Result<()>;
    fn read_input(&self, timeout_ms: i32) -> Result<Vec<u8>>;
}

pub struct HidTransport {
    device: HidDevice,
    output_report_len: Option<usize>,
}

impl HidTransport {
    pub fn open(info: &DeviceInfo) -> Result<Self> {
        if info.kind == DeviceKind::KeyboardBootloader {
            bail!("Ace 75 находится в режиме загрузчика; обычное управление недоступно");
        }
        Self::open_selected(info)
    }

    pub fn open_keyboard_bootloader(info: &DeviceInfo) -> Result<Self> {
        anyhow::ensure!(
            info.kind == DeviceKind::KeyboardBootloader
                && classify_device(
                    info.vendor_id,
                    info.product_id,
                    &info.name,
                    info.usage_page,
                    info.usage
                ) == Some(DeviceKind::KeyboardBootloader),
            "ACE75: выбрано неподходящее устройство загрузчика"
        );
        Self::open_selected(info)
    }

    fn open_selected(info: &DeviceInfo) -> Result<Self> {
        let api = HidApi::new()?;
        let path = CString::new(info.path.as_str())?;
        let device = api.open_path(&path).with_context(|| {
            #[cfg(windows)]
            let advice = "Проверьте подключение и закройте другие программы управления MCHOSE";
            #[cfg(not(windows))]
            let advice = "Проверьте подключение и права hidraw";
            format!("Нет доступа к {} ({}). {advice}", info.name, info.path)
        })?;
        #[cfg(windows)]
        let output_report_len = match info.kind {
            DeviceKind::Dock | DeviceKind::Keyboard => Some(65),
            DeviceKind::Mouse => Some(64),
            DeviceKind::KeyboardBootloader => Some(65),
        };
        #[cfg(not(windows))]
        let output_report_len = None;
        Ok(Self {
            device,
            output_report_len,
        })
    }
}

impl HidIo for HidTransport {
    fn send_feature(&self, report_id: u8, payload: &[u8]) -> Result<()> {
        let mut report = Vec::with_capacity(payload.len() + 1);
        report.push(report_id);
        report.extend_from_slice(payload);
        self.device
            .send_feature_report(&report)
            .context("Ошибка отправки HID Feature Report")?;
        Ok(())
    }

    fn get_feature(&self, report_id: u8, payload_len: usize) -> Result<Vec<u8>> {
        let mut report = vec![0; payload_len + 1];
        report[0] = report_id;
        let count = self
            .device
            .get_feature_report(&mut report)
            .context("Ошибка чтения HID Feature Report")?;
        if count == 0 || report[0] != report_id {
            bail!("Устройство вернуло неверный ID Feature Report");
        }
        Ok(report[1..count].to_vec())
    }

    fn write_output(&self, report_id: u8, payload: &[u8]) -> Result<()> {
        let report = output_report(report_id, payload, self.output_report_len)?;
        let count = self
            .device
            .write(&report)
            .context("Ошибка отправки HID Output Report")?;
        if !complete_output(count, report.len(), cfg!(windows)) {
            bail!("Устройство приняло только {count} из {} байт", report.len());
        }
        Ok(())
    }

    fn read_input(&self, timeout_ms: i32) -> Result<Vec<u8>> {
        let mut report = vec![0; 1024];
        let count = self
            .device
            .read_timeout(&mut report, timeout_ms)
            .context("Ошибка чтения HID Input Report")?;
        report.truncate(count);
        Ok(report)
    }
}

fn output_report(report_id: u8, payload: &[u8], windows_length: Option<usize>) -> Result<Vec<u8>> {
    let mut report = Vec::with_capacity(payload.len() + 1);
    report.push(report_id);
    report.extend_from_slice(payload);
    if let Some(length) = windows_length {
        anyhow::ensure!(
            report.len() <= length,
            "HID report exceeds the device descriptor"
        );
        report.resize(length, 0);
    }
    Ok(report)
}

fn complete_output(count: usize, expected: usize, windows_native: bool) -> bool {
    // hidapi 2.6.7 windows-native returns 0 for synchronous WriteFile success.
    count == expected || windows_native && count == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ace75_selects_only_the_captured_configuration_collection() {
        assert_eq!(
            classify_device(0x3837, 0x303c, "Ace 75", 1, 0),
            Some(DeviceKind::Keyboard)
        );
        assert_eq!(classify_device(0x3837, 0x303c, "Ace 75", 1, 6), None);
        assert_eq!(classify_device(0x3837, 0x303c, "Ace 75", 0x0c, 1), None);
        assert_eq!(classify_device(0x3837, 0x301d, "Ace 75", 1, 0), None);
        assert_eq!(
            classify_device(0x3837, 0x303c, "Other keyboard", 1, 0),
            None
        );
        assert_eq!(classify_device(0x1234, 0x303c, "Ace 75", 1, 0), None);
        assert_eq!(
            classify_device(0x3837, 0x303d, "Ace 75 Upgrader", 0xff00, 1),
            Some(DeviceKind::KeyboardBootloader)
        );
        assert_eq!(
            classify_device(0x3837, 0x303d, "Ace 75 Upgrader", 1, 0),
            None
        );
    }

    #[test]
    fn ace75_unnumbered_report_keeps_all_64_payload_bytes() {
        let payload = std::array::from_fn::<u8, 64, _>(|index| index as u8);
        let windows = output_report(0, &payload, Some(65)).unwrap();
        let linux = output_report(0, &payload, None).unwrap();
        assert_eq!(windows.len(), 65);
        assert_eq!(windows[0], 0);
        assert_eq!(&windows[1..], &payload);
        assert_eq!(windows, linux);
    }

    #[test]
    fn windows_dock_reports_include_zero_id_and_descriptor_padding() {
        let packet = crate::dock::encode_packet(7, &[]).unwrap();
        let report = output_report(0, &packet, Some(65)).unwrap();
        assert_eq!(report.len(), 65);
        assert_eq!(&report[..3], &[0, 0xaa, 7]);
        assert_eq!(&report[1..64], &packet);
        assert_eq!(report[64], 0);
        assert_eq!(output_report(0, &packet, None).unwrap().len(), 64);
        assert!(output_report(0, &[0; 65], Some(65)).is_err());
    }

    #[test]
    fn windows_mouse_reports_retain_numbered_header_and_reject_short_writes() {
        let report = output_report(0x4d, &[0; 63], Some(64)).unwrap();
        assert_eq!(report.len(), 64);
        assert_eq!(report[0], 0x4d);
        assert!(complete_output(65, 65, true));
        assert!(complete_output(0, 65, true));
        assert!(!complete_output(64, 65, true));
        assert!(!complete_output(0, 64, false));
        assert!(!complete_output(65, 64, false));
        assert!(complete_output(64, 64, false));
    }
}
