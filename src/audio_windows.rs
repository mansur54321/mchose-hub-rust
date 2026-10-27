use super::{FFT_SIZE, SAMPLE_RATE, spectrum, upstream_colors, upstream_level};
use anyhow::{Context, Result, bail, ensure};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};
use windows::Win32::Media::Audio::{AUDCLNT_BUFFERFLAGS_SILENT, IAudioCaptureClient};

#[derive(Default)]
struct CaptureState {
    frequencies: Vec<u8>,
    generation: u64,
    error: Option<String>,
}

pub struct MusicCapture {
    reader: Option<JoinHandle<()>>,
    stopped: Arc<AtomicBool>,
    state: Arc<Mutex<CaptureState>>,
    history: VecDeque<f32>,
    generation: u64,
    sink_name: String,
}

impl MusicCapture {
    pub fn start() -> Result<Self> {
        let stopped = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(CaptureState::default()));
        let thread_stopped = Arc::clone(&stopped);
        let thread_state = Arc::clone(&state);
        let (ready, receiver) = mpsc::sync_channel(1);
        let reader = std::thread::Builder::new()
            .name("mchose-wasapi".into())
            .spawn(move || {
                if let Err(error) = capture_loop(&thread_stopped, &thread_state, &ready) {
                    let message = format!("WASAPI: {error:#}");
                    let _ = ready.try_send(Err(message.clone()));
                    if let Ok(mut state) = thread_state.lock() {
                        state.error = Some(message);
                    }
                }
            })
            .context("Не удалось запустить поток WASAPI")?;
        match receiver.recv() {
            Ok(Ok(sink_name)) => Ok(Self {
                reader: Some(reader),
                stopped,
                state,
                history: VecDeque::with_capacity(150),
                generation: 0,
                sink_name,
            }),
            result => {
                stopped.store(true, Ordering::Relaxed);
                reader.thread().unpark();
                let _ = reader.join();
                match result {
                    Ok(Err(message)) => bail!("{message}"),
                    _ => bail!("Поток WASAPI завершился до начала захвата"),
                }
            }
        }
    }

    pub fn sink_name(&self) -> &str {
        &self.sink_name
    }

    pub fn take_error(&mut self) -> Option<String> {
        let error = self.state.lock().unwrap().error.take();
        error.or_else(|| {
            self.reader
                .as_ref()
                .is_some_and(JoinHandle::is_finished)
                .then(|| "Поток звука WASAPI остановлен".into())
        })
    }

    pub fn latest_colors(&mut self, color: [u8; 3], brightness: u8) -> Option<[[u8; 3]; 12]> {
        let frequencies = self.latest_frequencies()?;
        let level = upstream_level(&frequencies, &mut self.history);
        Some(upstream_colors(level, color, brightness))
    }

    pub fn latest_frequencies(&mut self) -> Option<Vec<u8>> {
        let frequencies = {
            let state = self.state.lock().unwrap();
            if state.generation == self.generation
                || state.error.is_some()
                || self.reader.as_ref().is_some_and(JoinHandle::is_finished)
            {
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
        if let Some(reader) = self.reader.take() {
            reader.thread().unpark();
            let _ = reader.join();
        }
    }
}

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        wasapi::deinitialize();
    }
}

fn publish(state: &Mutex<CaptureState>, samples: &[f32; FFT_SIZE]) {
    let frequencies = spectrum(samples);
    let mut state = state.lock().unwrap();
    state.frequencies = frequencies;
    state.generation = state.generation.wrapping_add(1);
}

/// `pointer` must contain `frames` mono f32 samples until this call returns,
/// except for SILENT packets, which may have a null pointer.
unsafe fn append_packet(
    queue: &mut VecDeque<f32>,
    pointer: *const u8,
    frames: u32,
    flags: u32,
) -> Result<()> {
    ensure!(frames <= SAMPLE_RATE as u32, "Слишком большой пакет WASAPI");
    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
        queue.extend(std::iter::repeat_n(0.0_f32, frames as usize));
    } else if frames > 0 {
        ensure!(!pointer.is_null(), "WASAPI вернул пустой аудиобуфер");
        let bytes = unsafe { std::slice::from_raw_parts(pointer, frames as usize * 4) };
        queue.extend(bytes.chunks_exact(4).map(|bytes| {
            let sample = f32::from_le_bytes(bytes.try_into().unwrap());
            if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }));
    }
    Ok(())
}

fn capture_loop(
    stopped: &AtomicBool,
    state: &Mutex<CaptureState>,
    ready: &mpsc::SyncSender<std::result::Result<String, String>>,
) -> Result<()> {
    wasapi::initialize_mta()
        .ok()
        .context("Не удалось инициализировать COM")?;
    let _apartment = ComApartment;
    let enumerator = DeviceEnumerator::new()?;
    // A render endpoint combined with Direction::Capture selects WASAPI loopback.
    let device = enumerator
        .get_default_device(&Direction::Render)
        .context("Выход воспроизведения Windows недоступен")?;
    let sink_name = device.get_friendlyname()?;
    let mut client = device.get_iaudioclient()?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE as usize, 1, None);
    client.initialize_client(
        &format,
        &Direction::Capture,
        &StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 1_000_000,
        },
    )?;
    // Handle SILENT before inspecting the pointer: WASAPI may return null for silence.
    let capture: IAudioCaptureClient = unsafe { client.as_iaudioclient().GetService()? };
    client.start_stream()?;
    let result = (|| -> Result<()> {
        ready
            .send(Ok(sink_name))
            .context("Запуск музыкальной подсветки отменён")?;
        let mut queue = VecDeque::with_capacity(FFT_SIZE * 4);
        let mut last_packet = Instant::now();
        let mut last_silence = Instant::now();
        while !stopped.load(Ordering::Relaxed) {
            while unsafe { capture.GetNextPacketSize()? } > 0 {
                let mut pointer = std::ptr::null_mut();
                let mut frames = 0;
                let mut flags = 0;
                unsafe { capture.GetBuffer(&mut pointer, &mut frames, &mut flags, None, None)? };
                // WASAPI owns the buffer until ReleaseBuffer, on this same thread.
                let read = unsafe { append_packet(&mut queue, pointer, frames, flags) };
                let release = unsafe { capture.ReleaseBuffer(frames) };
                read?;
                release?;
                last_packet = Instant::now();
                while queue.len() >= FFT_SIZE {
                    let samples = std::array::from_fn(|_| queue.pop_front().unwrap());
                    publish(state, &samples);
                }
                if stopped.load(Ordering::Relaxed) {
                    break;
                }
            }
            if last_packet.elapsed() >= Duration::from_millis(200)
                && last_silence.elapsed() >= Duration::from_millis(32)
            {
                queue.clear();
                publish(state, &[0.0; FFT_SIZE]);
                last_silence = Instant::now();
            }
            std::thread::park_timeout(Duration::from_millis(10));
        }
        Ok(())
    })();
    let stop = client.stop_stream();
    result?;
    stop?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_packets_accept_null_and_invalid_non_silent_packets_leave_queue_unchanged() {
        let mut queue = VecDeque::from([0.5]);
        unsafe {
            append_packet(
                &mut queue,
                std::ptr::null(),
                3,
                AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
            )
        }
        .unwrap();
        assert_eq!(queue, [0.5, 0.0, 0.0, 0.0]);
        assert!(unsafe { append_packet(&mut queue, std::ptr::null(), 1, 0) }.is_err());
        assert!(
            unsafe {
                append_packet(
                    &mut queue,
                    std::ptr::null(),
                    SAMPLE_RATE as u32 + 1,
                    AUDCLNT_BUFFERFLAGS_SILENT.0 as u32,
                )
            }
            .is_err()
        );
        assert_eq!(queue, [0.5, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn float_packets_preserve_amplitude_and_sanitize_non_finite_values() {
        let samples = [0.25_f32, -0.75, 2.0, f32::NAN, f32::INFINITY];
        let bytes: Vec<u8> = samples.into_iter().flat_map(f32::to_le_bytes).collect();
        let mut queue = VecDeque::new();
        unsafe { append_packet(&mut queue, bytes.as_ptr(), samples.len() as u32, 0) }.unwrap();
        assert_eq!(queue, [0.25, -0.75, 1.0, 0.0, 0.0]);
    }
}
