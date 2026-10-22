use anyhow::{Context, Result, bail, ensure};
use mchose_native::{
    controller::{data_dir, save_new_json},
    dock::{Dock, DockSettings},
    mouse::{DpiSettings, Mouse, MouseSnapshot},
    transport::{self, DeviceInfo, DeviceKind, HidTransport},
};
use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Serialize)]
struct Identity {
    device: DeviceInfo,
    hid_sysfs_path: PathBuf,
    usb_sysfs_path: PathBuf,
    usb_serial: Option<String>,
    usb_bus: String,
    usb_address: String,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    unix_nanos: u128,
    mouse_identity: Identity,
    dock_identity: Identity,
    mouse: MouseSnapshot,
    dock: DockSettings,
}

#[derive(Serialize)]
struct Step<T: Serialize> {
    write_attempted: bool,
    expected_test_value: T,
    after_test: Option<T>,
    test_verified: bool,
    test_error: Option<String>,
    restore_attempted: bool,
    after_restore: Option<T>,
    restore_verified: bool,
    restore_write_error: Option<String>,
    restore_read_error: Option<String>,
}

impl<T: Serialize> Step<T> {
    fn new(expected: T) -> Self {
        Self {
            write_attempted: false,
            expected_test_value: expected,
            after_test: None,
            test_verified: false,
            test_error: None,
            restore_attempted: false,
            after_restore: None,
            restore_verified: false,
            restore_write_error: None,
            restore_read_error: None,
        }
    }
}

fn timestamp() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}

fn identity(info: &DeviceInfo) -> Result<Identity> {
    let node = std::path::Path::new(&info.path)
        .file_name()
        .context("Устройство не имеет имени hidraw")?;
    let hid = PathBuf::from("/sys/class/hidraw")
        .join(node)
        .join("device")
        .canonicalize()
        .context("Не удалось прочитать физический адрес HID")?;
    let usb = hid
        .ancestors()
        .find(|path| path.join("idVendor").exists() && path.join("idProduct").exists())
        .context("HID не связан с USB-устройством")?
        .to_path_buf();
    let read = |name: &str| -> Result<String> {
        Ok(std::fs::read_to_string(usb.join(name))?.trim().to_owned())
    };
    ensure!(
        u16::from_str_radix(&read("idVendor")?, 16)? == info.vendor_id
            && u16::from_str_radix(&read("idProduct")?, 16)? == info.product_id,
        "Устройство изменилось во время обнаружения"
    );
    Ok(Identity {
        device: info.clone(),
        hid_sysfs_path: hid.clone(),
        usb_sysfs_path: usb.clone(),
        usb_serial: std::fs::read_to_string(usb.join("serial"))
            .ok()
            .map(|value| value.trim().to_owned()),
        usb_bus: read("busnum")?,
        usb_address: read("devnum")?,
    })
}

fn ensure_fresh(expected: &Identity) -> Result<()> {
    let actual = transport::enumerate()?
        .into_iter()
        .find(|info| {
            let old = &expected.device;
            info.path == old.path
                && info.name == old.name
                && info.vendor_id == old.vendor_id
                && info.product_id == old.product_id
                && info.interface == old.interface
                && info.usage_page == old.usage_page
                && info.usage == old.usage
                && info.kind == old.kind
        })
        .context("Исходное HID-устройство отключилось или изменилось")?;
    let actual = identity(&actual)?;
    ensure!(
        actual.hid_sysfs_path == expected.hid_sysfs_path
            && actual.usb_sysfs_path == expected.usb_sysfs_path
            && actual.usb_serial == expected.usb_serial
            && actual.usb_bus == expected.usb_bus
            && actual.usb_address == expected.usb_address,
        "Физическое USB-устройство переподключено или заменено; запись в неизвестную цель запрещена"
    );
    Ok(())
}

fn same_dpi(actual: &DpiSettings, expected: &DpiSettings) -> Result<()> {
    ensure!(
        actual.encode(0)? == expected.encode(0)?,
        "Прочитанные DPI X отличаются от ожидаемых"
    );
    if expected.separate_y {
        ensure!(
            actual.encode(1)? == expected.encode(1)?,
            "Прочитанные DPI Y отличаются от ожидаемых"
        );
    }
    Ok(())
}

fn changed_dpi(before: &DpiSettings) -> Result<DpiSettings> {
    before.encode(0)?;
    if before.separate_y {
        before.encode(1)?;
    }
    let mut changed = before.clone();
    let stage = usize::from(before.selected);
    let old = before.x[stage];
    changed.x[stage] = if old <= 49_950 { old + 50 } else { old - 50 };
    if !before.separate_y {
        changed.y = changed.x;
    }
    changed.encode(0)?;
    Ok(changed)
}

fn verify_mouse(mouse: &mut Mouse, before: &Snapshot, changed: DpiSettings) -> Step<DpiSettings> {
    let original = &before.mouse.dpi;
    let mut step = Step::new(changed);
    let test = (|| -> Result<()> {
        ensure_fresh(&before.mouse_identity)?;
        let active = mouse.read_settings()?;
        ensure!(
            active.profile_index == original.profile_index,
            "Активный профиль изменился после резервной копии"
        );
        same_dpi(&mouse.read_dpi(original.profile_index)?, original)?;
        step.write_attempted = true;
        mouse.write_dpi(&step.expected_test_value)?;
        let actual = mouse.read_dpi(original.profile_index)?;
        let verified = same_dpi(&actual, &step.expected_test_value);
        step.after_test = Some(actual);
        verified?;
        step.test_verified = true;
        Ok(())
    })();
    step.test_error = test.err().map(|error| format!("{error:#}"));
    if !step.write_attempted {
        return step;
    }

    step.restore_attempted = true;
    if let Err(error) = ensure_fresh(&before.mouse_identity) {
        step.restore_write_error = Some(format!("{error:#}"));
        return step;
    }
    step.restore_write_error = mouse
        .write_dpi(original)
        .err()
        .map(|error| format!("{error:#}"));
    let readback = (|| -> Result<()> {
        let actual = mouse.read_dpi(original.profile_index)?;
        let verified = same_dpi(&actual, original);
        step.after_restore = Some(actual);
        verified?;
        step.restore_verified = true;
        Ok(())
    })();
    step.restore_read_error = readback.err().map(|error| format!("{error:#}"));
    step
}

fn verify_dock(dock: &mut Dock, before: &Snapshot) -> Step<DockSettings> {
    let mut changed = before.dock.clone();
    changed.lighting.brightness = (changed.lighting.brightness + 1) % 5;
    let mut step = Step::new(changed);
    let test = (|| -> Result<()> {
        ensure_fresh(&before.dock_identity)?;
        ensure!(
            dock.read_settings()?.lighting == before.dock.lighting,
            "Параметры дока изменились после резервной копии"
        );
        step.write_attempted = true;
        let actual = dock.set_lighting(&step.expected_test_value.lighting)?;
        step.test_verified = actual.lighting == step.expected_test_value.lighting;
        step.after_test = Some(actual);
        ensure!(step.test_verified, "Док не подтвердил тестовую яркость");
        Ok(())
    })();
    step.test_error = test.err().map(|error| format!("{error:#}"));
    if !step.write_attempted {
        return step;
    }

    step.restore_attempted = true;
    if let Err(error) = ensure_fresh(&before.dock_identity) {
        step.restore_write_error = Some(format!("{error:#}"));
        return step;
    }
    step.restore_write_error = dock
        .set_lighting(&before.dock.lighting)
        .err()
        .map(|error| format!("{error:#}"));
    let readback = (|| -> Result<()> {
        let actual = dock.read_settings()?;
        step.restore_verified = actual.lighting == before.dock.lighting;
        step.after_restore = Some(actual);
        ensure!(
            step.restore_verified,
            "Док не подтвердил восстановленную подсветку"
        );
        Ok(())
    })();
    step.restore_read_error = readback.err().map(|error| format!("{error:#}"));
    step
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let execute = match args.as_slice() {
        [] => false,
        [flag] if flag == "--execute" => true,
        _ => bail!(
            "Использование: verify_roundtrip [--execute]. Без флага выполняется только чтение и печать плана."
        ),
    };
    let devices = transport::enumerate()?;
    let mice: Vec<_> = devices
        .iter()
        .filter(|info| info.kind == DeviceKind::Mouse)
        .collect();
    let docks: Vec<_> = devices
        .iter()
        .filter(|info| info.kind == DeviceKind::Dock)
        .collect();
    ensure!(
        mice.len() == 1 && docks.len() == 1,
        "Для однозначного теста требуется одна мышь и один MagDock; найдено мышей {}, доков {}",
        mice.len(),
        docks.len()
    );
    let mouse_identity = identity(mice[0])?;
    let dock_identity = identity(docks[0])?;
    let mut mouse = Mouse::new(Box::new(HidTransport::open(mice[0])?));
    let mut dock = Dock::new(Box::new(HidTransport::open(docks[0])?));
    let before = Snapshot {
        unix_nanos: timestamp()?,
        mouse: mouse.read_snapshot_with_receiver(mice[0].product_id != 0x4033)?,
        dock: dock.read_settings()?,
        mouse_identity,
        dock_identity,
    };
    ensure!(
        mice[0].product_id == 0x4033 || before.mouse.status.battery_percent > 0,
        "Приёмник не сообщает заряд мыши; разбудите её и повторите чтение плана"
    );
    ensure!(
        before.mouse.settings.profile_index == before.mouse.dpi.profile_index
            && before.mouse.settings.dpi_index == before.mouse.dpi.selected,
        "Состояние мыши изменилось во время чтения; повторите план"
    );
    let changed = changed_dpi(&before.mouse.dpi)?;
    let stage = usize::from(changed.selected);
    let plan = serde_json::json!({
        "mode": if execute { "execute_requested" } else { "read_only_plan" },
        "execute_flag_required": "--execute",
        "mouse": {"identity":before.mouse_identity,"profile":changed.profile_index,"stage":changed.selected,
            "x_before":before.mouse.dpi.x[stage],"x_test":changed.x[stage],"x_restore":before.mouse.dpi.x[stage],
            "separate_y":changed.separate_y,"y_before":before.mouse.dpi.y[stage],"y_test":changed.y[stage]},
        "before":before,
        "dock": {"identity":before.dock_identity,"brightness_before":before.dock.lighting.brightness,
            "brightness_test":(before.dock.lighting.brightness+1)%5,"brightness_restore":before.dock.lighting.brightness},
        "steps":["fresh identity check", "new backup before.json under mchose-native/qa", "write complete DPI structure changing only selected stage X +/-50 (and linked Y if enabled)", "read and verify", "restore complete original DPI structure even after a failure", "read and verify restoration", "change only dock brightness", "read and verify", "restore complete original dock lighting even after a failure", "read full final snapshot and save report.json"],
        "excluded":["reset", "macros", "firmware", "buttons", "profile switching", "audio capture"]
    });
    if !execute {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }

    ensure_fresh(&before.mouse_identity)?;
    ensure_fresh(&before.dock_identity)?;
    let directory =
        data_dir()
            .join("qa")
            .join(format!("roundtrip-{}-{}", timestamp()?, std::process::id()));
    std::fs::create_dir_all(directory.parent().context("Нет каталога QA")?)?;
    std::fs::create_dir(&directory)
        .context("Не удалось создать уникальный каталог резервной копии")?;
    save_new_json(&directory.join("before.json"), &before)?;
    save_new_json(&directory.join("plan.json"), &plan)?;

    let mouse_result = verify_mouse(&mut mouse, &before, changed);
    let dock_result = if mouse_result.restore_verified {
        Some(verify_dock(&mut dock, &before))
    } else {
        None
    };
    let after = (|| -> Result<Snapshot> {
        ensure_fresh(&before.mouse_identity)?;
        ensure_fresh(&before.dock_identity)?;
        Ok(Snapshot {
            unix_nanos: timestamp()?,
            mouse_identity: identity(mice[0])?,
            dock_identity: identity(docks[0])?,
            mouse: mouse.read_snapshot_with_receiver(mice[0].product_id != 0x4033)?,
            dock: dock.read_settings()?,
        })
    })();
    let (after, after_error) = match after {
        Ok(snapshot) => (Some(snapshot), None),
        Err(error) => (None, Some(format!("{error:#}"))),
    };
    let final_settings_verified = after.as_ref().is_some_and(|after| {
        same_dpi(&after.mouse.dpi, &before.mouse.dpi).is_ok()
            && after.mouse.settings == before.mouse.settings
            && after.mouse.keys == before.mouse.keys
            && after.mouse.lod_index == before.mouse.lod_index
            && after.mouse.profile_names == before.mouse.profile_names
            && after.mouse.mouse_firmware == before.mouse.mouse_firmware
            && after.mouse.receiver_firmware == before.mouse.receiver_firmware
            && after.dock == before.dock
    });
    let restored = mouse_result.restore_verified
        && dock_result
            .as_ref()
            .is_some_and(|result| result.restore_verified)
        && final_settings_verified;
    let verified = restored
        && mouse_result.test_verified
        && mouse_result.test_error.is_none()
        && mouse_result.restore_write_error.is_none()
        && mouse_result.restore_read_error.is_none()
        && dock_result.as_ref().is_some_and(|result| {
            result.test_verified
                && result.test_error.is_none()
                && result.restore_write_error.is_none()
                && result.restore_read_error.is_none()
        });
    let report = serde_json::json!({"backup_directory":directory,"before":before,"mouse":mouse_result,"dock":dock_result,
        "dock_skipped_reason":if dock_result.is_none() { Some("Восстановление мыши не подтверждено; дальнейшие изменения остановлены") } else { None },
        "after":after,"after_error":after_error,"all_changes_and_restoration_verified":verified,"all_original_settings_restored":restored});
    save_new_json(&directory.join("report.json"), &report)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    ensure!(
        verified,
        "Проверка не завершена успешно. Подробности и независимые ошибки восстановления: {}",
        directory.join("report.json").display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpi_plan_changes_only_selected_axis_and_stays_in_range() {
        let original = DpiSettings {
            profile_index: 0,
            count: 6,
            selected: 2,
            separate_y: true,
            x: [200, 400, 50_000, 800, 1600, 3200],
            y: [200, 400, 800, 1600, 3200, 6400],
            raw_x: vec![],
            raw_y: vec![],
        };
        let changed = changed_dpi(&original).unwrap();
        assert_eq!(changed.x, [200, 400, 49_950, 800, 1600, 3200]);
        assert_eq!(changed.y, original.y);
        let mut linked = original;
        linked.selected = 0;
        linked.separate_y = false;
        linked.y = linked.x;
        let changed = changed_dpi(&linked).unwrap();
        assert_eq!(changed.x[0], 250);
        assert_eq!(changed.x, changed.y);
    }
}
