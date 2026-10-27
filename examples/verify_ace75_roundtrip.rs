use anyhow::{Context, Result, ensure};
use mchose_hub_rust::{
    ace75::{Ace75, KeyboardBase},
    keyboard_controller::write_keyboard_backup,
    transport::{self, DeviceKind, HidTransport},
};
use std::path::Path;

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let devices = transport::enumerate()?;
    let keyboards: Vec<_> = devices
        .iter()
        .filter(|device| device.kind == DeviceKind::Keyboard)
        .collect();
    ensure!(keyboards.len() == 1, "Connect exactly one Ace 75 8K");
    let mut keyboard = Ace75::new(Box::new(HidTransport::open(keyboards[0])?));
    let base = KeyboardBase::decode(&keyboard.read_base()?)?;
    let original = keyboard
        .read_profile(base.active_profile)?
        .functions
        .lighting;
    if !arguments.iter().any(|value| value == "--write-lighting") {
        println!("{}", serde_json::to_string_pretty(&original)?);
        return Ok(());
    }
    let backup = arguments
        .windows(2)
        .find(|pair| pair[0] == "--backup")
        .map(|pair| pair[1].as_str())
        .context("--write-lighting requires --backup NEW_JSON_PATH")?;
    write_keyboard_backup(&mut keyboard, Path::new(backup))?;
    ensure!(
        KeyboardBase::decode(&keyboard.read_base()?)?.active_profile == base.active_profile,
        "Active profile changed before the test"
    );
    let mut temporary = original.clone();
    temporary.main.brightness = if original.main.brightness < 100 {
        original.main.brightness + 1
    } else {
        99
    };
    let change = keyboard.apply_lighting(base.active_profile, &temporary);
    let restore = keyboard.apply_lighting(base.active_profile, &original);
    let actual = keyboard
        .read_profile(base.active_profile)?
        .functions
        .lighting;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "profile": base.active_profile,
            "original": original,
            "temporary": temporary,
            "change_verified": change.is_ok(),
            "change_error": change.as_ref().err().map(|error| format!("{error:#}")),
            "restore_verified": restore.is_ok(),
            "restore_error": restore.as_ref().err().map(|error| format!("{error:#}")),
            "actual": actual,
            "backup": backup,
        }))?
    );
    restore.context("Lighting restoration was not confirmed")?;
    ensure!(actual == original, "Lighting differs after restoration");
    change.context("Temporary lighting setting was not confirmed")?;
    Ok(())
}
