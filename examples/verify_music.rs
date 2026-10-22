use anyhow::{Context, Result, ensure};
use mchose_native::{
    controller::{Command, Controller, save_new_json},
    dock::{Dock, DockMode, DockSettings},
    transport::{self, DeviceKind, HidIo, HidTransport},
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn wait_idle(controller: &mut Controller) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        controller.poll();
        if !controller.snapshot.busy {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Controller did not complete its operation"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn controller_roundtrip(before: &DockSettings) -> Result<serde_json::Value> {
    let mut controller = Controller::new(false);
    wait_idle(&mut controller)?;
    let mut music = before.clone();
    music.lighting.enabled = true;
    music.lighting.mode = DockMode::Music;
    // Exercise the same staged input as the old UI: the controller must set sync.
    music.lighting.music_sync = false;
    controller.send(Command::ApplyDock(music.clone()));
    wait_idle(&mut controller)?;
    ensure!(
        controller.snapshot.music_active,
        "Music did not start: {:?}",
        controller.snapshot.error
    );
    let started = controller.snapshot.dock.clone();
    std::thread::sleep(Duration::from_secs(5));
    music.lighting.brightness = (before.lighting.brightness + 1) % 5;
    music.lighting.color = [0, 80, 255];
    controller.poll();
    wait_idle(&mut controller)?;
    controller.send(Command::ApplyDock(music.clone()));
    wait_idle(&mut controller)?;
    ensure!(
        controller.snapshot.music_active,
        "Changing Music settings stopped capture: {:?}; dock: {:?}",
        controller.snapshot.error,
        controller.snapshot.dock
    );
    let updated = controller
        .snapshot
        .dock
        .clone()
        .context("Missing updated settings")?;
    ensure!(
        updated.lighting.color == music.lighting.color
            && updated.lighting.brightness == music.lighting.brightness,
        "Music ignored the staged color or brightness"
    );
    std::thread::sleep(Duration::from_secs(5));
    wait_idle(&mut controller)?;
    controller.send(Command::StopMusic);
    wait_idle(&mut controller)?;
    ensure!(!controller.snapshot.music_active, "Music did not stop");
    ensure!(
        controller
            .snapshot
            .dock
            .as_ref()
            .is_some_and(|actual| actual.lighting == before.lighting),
        "StopMusic did not restore the previous lighting: {:?}",
        controller.snapshot.error
    );
    Ok(
        serde_json::json!({"started": started, "updated": updated, "stopped": controller.snapshot.dock}),
    )
}

struct CaptureIo {
    io: HidTransport,
    reports: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl HidIo for CaptureIo {
    fn send_feature(&self, id: u8, payload: &[u8]) -> Result<()> {
        self.io.send_feature(id, payload)
    }
    fn get_feature(&self, id: u8, length: usize) -> Result<Vec<u8>> {
        self.io.get_feature(id, length)
    }
    fn write_output(&self, id: u8, payload: &[u8]) -> Result<()> {
        self.io.write_output(id, payload)
    }
    fn read_input(&self, timeout: i32) -> Result<Vec<u8>> {
        let report = self.io.read_input(timeout)?;
        if !report.is_empty() {
            self.reports.lock().unwrap().push(report.clone());
        }
        Ok(report)
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(
        args.iter().any(|arg| arg == "--execute"),
        "Use --execute for a reversible MagDock music test"
    );
    let device = transport::enumerate()?
        .into_iter()
        .find(|device| device.kind == DeviceKind::Dock)
        .context("MagDock is not connected")?;
    let reports = Arc::new(Mutex::new(Vec::new()));
    let mut dock = Dock::new(Box::new(CaptureIo {
        io: HidTransport::open(&device)?,
        reports: reports.clone(),
    }));
    let before = dock.read_settings()?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let directory = std::path::PathBuf::from(format!("research/evidence/music-{timestamp}"));
    std::fs::create_dir_all(&directory)?;
    save_new_json(&directory.join("before.json"), &before)?;
    let mut requested = before.lighting.clone();
    requested.mode = DockMode::Music;
    requested.music_sync = args.iter().any(|arg| arg == "--sync");
    requested.enabled = true;
    let result = if args.iter().any(|arg| arg == "--controller") {
        controller_roundtrip(&before)
    } else {
        dock.set_lighting(&requested)
            .and_then(|settings| Ok(serde_json::to_value(settings)?))
    };
    let after = dock.read_settings();
    let restore = dock.set_lighting(&before.lighting);
    let restored = dock.read_settings()?;
    let verified = result.is_ok();
    let restored_ok = restored.lighting == before.lighting;
    let report = serde_json::json!({
        "device": device, "requested": requested,
        "confirmed": result.as_ref().ok(),
        "error": result.as_ref().err().map(|error| format!("{error:#}")),
        "after": after.as_ref().ok(),
        "restore_error": restore.as_ref().err().map(|error| format!("{error:#}")),
        "restored": restored, "restored_ok": restored_ok,
        "verified": verified, "input_reports": *reports.lock().unwrap(),
    });
    save_new_json(&directory.join("result.json"), &report)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    println!("Evidence: {}", directory.display());
    ensure!(restored_ok, "Original MagDock lighting was not restored");
    ensure!(verified, "MagDock did not confirm Music");
    Ok(())
}
