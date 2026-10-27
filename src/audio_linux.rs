use super::{FFT_SIZE, SAMPLE_RATE, spectrum, upstream_colors, upstream_level};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

#[derive(Default)]
struct CaptureState {
    frequencies: Vec<u8>,
    generation: u64,
    error: Option<String>,
    stderr_tail: String,
}

pub struct MusicCapture {
    child: Child,
    reader: Option<JoinHandle<()>>,
    stderr_reader: Option<JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    state: Arc<Mutex<CaptureState>>,
    history: VecDeque<f32>,
    generation: u64,
    exit_reported: bool,
    sink_name: String,
}

impl MusicCapture {
    /// Called only after the user enables music lighting. No audio is written to disk.
    pub fn start() -> Result<Self> {
        let output = Command::new("pw-dump")
            .stdin(Stdio::null())
            .output()
            .context("Для музыкальной подсветки нужен установленный pw-dump (PipeWire)")?;
        ensure!(
            output.status.success(),
            "Не удалось прочитать PipeWire: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        let graph: Value = serde_json::from_slice(&output.stdout)
            .context("PipeWire вернул некорректный список устройств")?;
        let (target, sink_name) = default_sink(&graph)?;
        let properties = serde_json::json!({
            "stream.capture.sink": true,
            "node.dont-fallback": true,
            "node.dont-reconnect": true,
            "application.name": "MCHOSE Hub Rust music lighting",
            "node.name": "mchose-native-music",
            "media.name": "MagDock lighting from speaker output"
        })
        .to_string();
        let mut child = Command::new("pw-cat")
            .args([
                "--record",
                "--raw",
                "--target",
                &target,
                "--rate",
                &SAMPLE_RATE.to_string(),
                "--channels",
                "1",
                "--format",
                "f32",
                "--latency",
                "50ms",
                "--properties",
                &properties,
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Не удалось запустить pw-cat для чтения звука выхода PipeWire")?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        let stopped = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(CaptureState::default()));
        let reader_state = Arc::clone(&state);
        let reader_stopped = Arc::clone(&stopped);
        let reader = std::thread::spawn(move || {
            let mut bytes = [0_u8; FFT_SIZE * 4];
            while !reader_stopped.load(Ordering::Relaxed) {
                if let Err(error) = stdout.read_exact(&mut bytes) {
                    if !reader_stopped.load(Ordering::Relaxed) {
                        reader_state.lock().unwrap().error =
                            Some(format!("Поток звука PipeWire остановлен: {error}"));
                    }
                    break;
                }
                let mut samples = [0_f32; FFT_SIZE];
                for (sample, bytes) in samples.iter_mut().zip(bytes.chunks_exact(4)) {
                    let value = f32::from_ne_bytes(bytes.try_into().unwrap());
                    *sample = if value.is_finite() {
                        value.clamp(-1.0, 1.0)
                    } else {
                        0.0
                    };
                }
                let frequencies = spectrum(&samples);
                let mut state = reader_state.lock().unwrap();
                state.frequencies = frequencies;
                state.generation = state.generation.wrapping_add(1);
            }
        });
        let stderr_state = Arc::clone(&state);
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = [0_u8; 1024];
            while let Ok(count) = stderr.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                let mut state = stderr_state.lock().unwrap();
                state
                    .stderr_tail
                    .push_str(&String::from_utf8_lossy(&bytes[..count]));
                if state.stderr_tail.len() > 4096 {
                    let mut start = state.stderr_tail.len() - 4096;
                    while !state.stderr_tail.is_char_boundary(start) {
                        start += 1;
                    }
                    state.stderr_tail.drain(..start);
                }
            }
        });
        Ok(Self {
            child,
            reader: Some(reader),
            stderr_reader: Some(stderr_reader),
            stopped,
            state,
            history: VecDeque::with_capacity(150),
            generation: 0,
            exit_reported: false,
            sink_name,
        })
    }

    pub fn sink_name(&self) -> &str {
        &self.sink_name
    }

    fn check_process(&mut self) {
        if self.exit_reported {
            return;
        }
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.exit_reported = true;
                let mut state = self.state.lock().unwrap();
                state.error = Some(format!(
                    "Захват звука завершился ({status}): {}",
                    state.stderr_tail.trim()
                ));
            }
            Err(error) => {
                self.exit_reported = true;
                self.state.lock().unwrap().error =
                    Some(format!("Не удалось проверить процесс звука: {error}"));
            }
            Ok(None) => {}
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.check_process();
        self.state.lock().unwrap().error.take()
    }

    pub fn latest_colors(&mut self, color: [u8; 3], brightness: u8) -> Option<[[u8; 3]; 12]> {
        let frequencies = self.latest_frequencies()?;
        let level = upstream_level(&frequencies, &mut self.history);
        Some(upstream_colors(level, color, brightness))
    }

    pub fn latest_frequencies(&mut self) -> Option<Vec<u8>> {
        self.check_process();
        let frequencies = {
            let state = self.state.lock().unwrap();
            if state.generation == self.generation || self.exit_reported || state.error.is_some() {
                return None;
            }
            self.generation = state.generation;
            state.frequencies.clone()
        };
        Some(frequencies)
    }
}

impl Drop for MusicCapture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // Only this instance's capture child is owned; other audio processes remain running.
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn default_sink(graph: &Value) -> Result<(String, String)> {
    let entries = graph
        .as_array()
        .context("PipeWire graph должен быть массивом")?;
    let default = entries
        .iter()
        .find(|entry| {
            entry["type"] == "PipeWire:Interface:Metadata"
                && entry["props"]["metadata.name"] == "default"
        })
        .and_then(|entry| entry["metadata"].as_array())
        .and_then(|metadata| {
            metadata
                .iter()
                .find(|entry| entry["key"] == "default.audio.sink")
        })
        .context("В PipeWire не выбран выход воспроизведения; микрофон использоваться не будет")?;
    let value = match default.get("value") {
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)?,
        Some(value) => value.clone(),
        None => bail!("В PipeWire не указано имя выхода воспроизведения"),
    };
    let name = value["name"]
        .as_str()
        .context("Некорректное имя выхода PipeWire")?;
    let node = entries
        .iter()
        .find(|entry| {
            entry["type"] == "PipeWire:Interface:Node"
                && entry["info"]["props"]["node.name"] == name
                && entry["info"]["props"]["media.class"] == "Audio/Sink"
        })
        .context("Выбранный выход PipeWire недоступен; захват не запущен")?;
    let props = &node["info"]["props"];
    let serial = match &props["object.serial"] {
        Value::Number(number) => number.to_string(),
        Value::String(number) => number.clone(),
        _ => bail!("У выхода PipeWire нет object.serial"),
    };
    ensure!(
        serial.parse::<u64>().is_ok(),
        "Некорректный object.serial PipeWire"
    );
    let description = props["node.description"]
        .as_str()
        .unwrap_or(name)
        .to_owned();
    Ok((serial, description))
}
