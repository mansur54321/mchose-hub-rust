use crate::transport::HidIo;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

const REPORT_LEN: usize = 63;
const HEADER_LEN: usize = 6;
const OTA_CHUNK_LEN: usize = 56;
const BOOTLOADER_LEN: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum DockMode {
    Static = 0,
    Breathing = 1,
    Shining = 2,
    Cycle = 3,
    Flow = 4,
    Music = 5,
}

impl TryFrom<u8> for DockMode {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Static),
            1 => Ok(Self::Breathing),
            2 => Ok(Self::Shining),
            3 => Ok(Self::Cycle),
            4 => Ok(Self::Flow),
            5 => Ok(Self::Music),
            _ => bail!("Неизвестный режим MagDock: {value}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockLighting {
    pub enabled: bool,
    pub mode: DockMode,
    pub mode_count: u8,
    pub speed: u8,
    pub brightness: u8,
    pub music_sync: bool,
    pub color: [u8; 3],
    pub direction: u8,
}

impl DockLighting {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.speed <= 4, "Скорость MagDock должна быть 0–4");
        ensure!(self.brightness <= 4, "Яркость MagDock должна быть 0–4");
        ensure!(
            self.direction <= 1,
            "Направление MagDock должно быть 0 или 1"
        );
        ensure!(self.mode_count > 0, "Количество режимов MagDock равно нулю");
        ensure!(
            !self.music_sync || self.mode == DockMode::Music,
            "Музыкальная синхронизация требует режима Music"
        );
        Ok(())
    }

    fn parameters(&self) -> Result<[u8; 10]> {
        self.validate()?;
        Ok([
            u8::from(self.enabled),
            self.mode as u8,
            self.mode_count,
            self.speed,
            self.brightness,
            u8::from(self.music_sync),
            self.color[0],
            self.color[1],
            self.color[2],
            self.direction,
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockSettings {
    pub lighting: DockLighting,
    pub sleep_enabled: bool,
    pub sleep_time_raw: u8,
    pub firmware_version: String,
    pub firmware_raw: [u8; 4],
    pub raw_response: Vec<u8>,
}

pub fn encode_packet(command: u8, parameters: &[u8]) -> Result<[u8; REPORT_LEN]> {
    ensure!(
        parameters.len() <= REPORT_LEN - HEADER_LEN,
        "Слишком длинный пакет MagDock"
    );
    let mut packet = [0; REPORT_LEN];
    packet[0] = 0xaa;
    packet[1] = command;
    packet[5] = parameters.len() as u8;
    packet[HEADER_LEN..HEADER_LEN + parameters.len()].copy_from_slice(parameters);
    Ok(packet)
}

pub fn decode_settings(report: &[u8]) -> Result<DockSettings> {
    ensure!(
        report.len() >= HEADER_LEN + 30,
        "Неполный ответ MagDock: {} байт",
        report.len()
    );
    ensure!(
        report[..2] == [0xaa, 0x07],
        "Неверный ответ на запрос состояния MagDock"
    );
    ensure!(
        matches!(report[2], 0 | 2),
        "Неверный тип ответа MagDock: {}",
        report[2]
    );
    ensure!(
        report[5] >= 30,
        "Неполные параметры состояния MagDock: {}",
        report[5]
    );
    ensure!(
        HEADER_LEN + usize::from(report[5]) <= report.len(),
        "Длина параметров MagDock превышает ответ"
    );
    let p = &report[HEADER_LEN..];
    for index in [0, 5, 10] {
        ensure!(
            p[index] <= 1,
            "Неверное логическое поле MagDock {index}: {}",
            p[index]
        );
    }
    let hardware_mode = DockMode::try_from(p[1])?;
    let lighting = DockLighting {
        enabled: p[0] == 1,
        // Music is a host stream selected by sync; firmware retains its ordinary effect.
        mode: if p[5] == 1 {
            DockMode::Music
        } else {
            hardware_mode
        },
        mode_count: p[2],
        speed: p[3],
        brightness: p[4],
        music_sync: p[5] == 1,
        color: [p[6], p[7], p[8]],
        direction: p[29],
    };
    lighting.validate()?;
    let firmware_raw = [p[11], p[12], p[13], p[14]];
    let firmware_version = if firmware_raw == [30, 17, 0, 1] {
        "0000".to_owned()
    } else {
        let version = format!("{:03}{:03}", p[13], p[12]);
        version[version.len() - 4..].to_owned()
    };
    Ok(DockSettings {
        lighting,
        // p[9] repeats the blue channel on firmware 0003/0004.
        sleep_enabled: p[10] == 1,
        sleep_time_raw: p[11],
        firmware_version,
        firmware_raw,
        raw_response: report.to_vec(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct FirmwarePlan {
    pub image_size: usize,
    pub image_sha256: String,
    pub skipped_bootloader_bytes: usize,
    pub chunk_count: usize,
    #[serde(skip_serializing)]
    payload: Vec<u8>,
}

impl FirmwarePlan {
    pub fn from_full_image(image: &[u8]) -> Result<Self> {
        ensure!(
            image.len() > BOOTLOADER_LEN,
            "Файл MagDock должен содержать более 8192 байт"
        );
        let payload = image[BOOTLOADER_LEN..].to_vec();
        ensure!(
            payload.iter().any(|byte| *byte != 0 && *byte != 0xff),
            "Область прошивки пуста"
        );
        Ok(Self {
            image_size: image.len(),
            image_sha256: format!("{:x}", Sha256::digest(image)),
            skipped_bootloader_bytes: BOOTLOADER_LEN,
            chunk_count: payload.len().div_ceil(OTA_CHUNK_LEN),
            payload,
        })
    }

    pub fn chunk(&self, index: usize) -> Result<[u8; REPORT_LEN]> {
        let count = self.payload.len().div_ceil(OTA_CHUNK_LEN);
        ensure!(index < count, "Индекс пакета прошивки вне диапазона");
        let start = index * OTA_CHUNK_LEN;
        let bytes = &self.payload[start..(start + OTA_CHUNK_LEN).min(self.payload.len())];
        let mut packet = encode_packet(0x74, bytes)?;
        // The upstream sender stores both counters in one-byte Uint8Array fields.
        packet[3] = index as u8;
        packet[4] = (count - 1) as u8;
        packet[62] = crc8(bytes);
        Ok(packet)
    }
}

pub fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0x8c
            } else {
                crc >> 1
            };
        }
    }
    crc
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockFlashReceipt {
    pub image_sha256: String,
    pub chunks_sent: usize,
    pub payload_bytes_sent: usize,
    pub completion_command_sent: bool,
    pub requires_reconnect_and_version_check: bool,
}

pub struct Dock {
    io: Box<dyn HidIo>,
}

impl Dock {
    pub fn new(io: Box<dyn HidIo>) -> Self {
        Self { io }
    }

    fn request(
        &mut self,
        packet: &[u8],
        response_type: Option<u8>,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        // Drop queued notifications so an older 0x07 cannot confirm a new write.
        for _ in 0..32 {
            if self.io.read_input(0)?.is_empty() {
                break;
            }
        }
        self.io.write_output(0, packet)?;
        let deadline = Instant::now() + timeout;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            let report = self
                .io
                .read_input(remaining.as_millis().max(1).min(i32::MAX as u128) as i32)?;
            if report.len() >= HEADER_LEN
                && report[0] == 0xaa
                && report[1] == packet[1]
                && response_type.is_none_or(|kind| report[2] == kind)
            {
                return Ok(report);
            }
            if report.is_empty() {
                break;
            }
        }
        bail!("MagDock не ответил на команду 0x{:02x}", packet[1])
    }

    pub fn read_settings(&mut self) -> Result<DockSettings> {
        let report = self.request(&encode_packet(0x07, &[])?, None, Duration::from_secs(3))?;
        decode_settings(&report)
    }

    pub fn set_lighting(&mut self, settings: &DockLighting) -> Result<DockSettings> {
        self.io
            .write_output(0, &encode_packet(0x27, &settings.parameters()?)?)?;
        std::thread::sleep(Duration::from_millis(50));
        let actual = self.read_settings()?;
        ensure!(
            actual.lighting == *settings,
            "MagDock не подтвердил настройку. Фактические параметры: {:?}",
            actual.lighting
        );
        Ok(actual)
    }

    pub fn stream_leds(&mut self, colors: &[[u8; 3]; 12]) -> Result<()> {
        let bytes: Vec<u8> = colors.iter().flatten().copied().collect();
        self.io.write_output(0, &encode_packet(0x29, &bytes)?)
    }

    pub fn reset_lighting(&mut self) -> Result<DockSettings> {
        self.io.write_output(0, &encode_packet(0x2a, &[])?)?;
        std::thread::sleep(Duration::from_millis(100));
        self.read_settings()
    }

    /// The caller must select a compatible trusted MagDock image and obtain approval.
    /// A receipt confirms transfer, not the version after the device restarts.
    pub fn flash_firmware(
        &mut self,
        plan: &FirmwarePlan,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<DockFlashReceipt> {
        let settings = self.read_settings()?;
        let version: u32 = settings
            .firmware_version
            .parse()
            .context("Версия MagDock не распознана")?;
        ensure!(
            version >= 3,
            "Для MagDock ниже версии 0003 официальный клиент требует внешний загрузчик"
        );
        let ready = self.request(&encode_packet(0x2b, &[])?, Some(2), Duration::from_secs(3))?;
        ensure!(
            ready.get(6) == Some(&0),
            "MagDock не готов к обновлению прошивки"
        );
        let count = plan.payload.len().div_ceil(OTA_CHUNK_LEN);
        progress(0, count);
        for index in 0..count {
            let packet = plan.chunk(index)?;
            let mut last_error = None;
            let mut acknowledged = false;
            for attempt in 0..3 {
                match self.request(&packet, Some(2), Duration::from_secs(1)) {
                    Ok(reply) if reply.get(6) == Some(&0) => {
                        acknowledged = true;
                        break;
                    }
                    Ok(reply) => last_error = Some(format!("статус {:?}", reply.get(6))),
                    Err(error) => last_error = Some(error.to_string()),
                }
                if attempt < 2 {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            ensure!(
                acknowledged,
                "Пакет прошивки {}/{} не подтверждён: {}",
                index + 1,
                count,
                last_error.unwrap_or_default()
            );
            progress(index + 1, count);
            if index + 1 < count {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.io.write_output(0, &encode_packet(0x75, &[])?)?;
        Ok(DockFlashReceipt {
            image_sha256: plan.image_sha256.clone(),
            chunks_sent: count,
            payload_bytes_sent: plan.payload.len(),
            completion_command_sent: true,
            requires_reconnect_and_version_check: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    fn live_response() -> Vec<u8> {
        let capture: serde_json::Value =
            serde_json::from_str(include_str!("../research/dock-readonly-live.json")).unwrap();
        let hex = capture["raw_input"][0].as_str().unwrap();
        hex.as_bytes()
            .chunks_exact(2)
            .map(|bytes| u8::from_str_radix(std::str::from_utf8(bytes).unwrap(), 16).unwrap())
            .collect()
    }

    struct MockHid {
        responses: Mutex<VecDeque<Vec<u8>>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl HidIo for MockHid {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            bail!("Unexpected feature write")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            bail!("Unexpected feature read")
        }
        fn write_output(&self, id: u8, payload: &[u8]) -> Result<()> {
            ensure!(id == 0, "MagDock uses unnumbered reports");
            self.writes.lock().unwrap().push(payload.to_vec());
            Ok(())
        }
        fn read_input(&self, timeout: i32) -> Result<Vec<u8>> {
            if timeout == 0 {
                return Ok(vec![]);
            }
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }
    }

    fn mock(responses: Vec<Vec<u8>>) -> (Dock, Arc<Mutex<Vec<Vec<u8>>>>) {
        let writes = Arc::new(Mutex::new(vec![]));
        let dock = Dock::new(Box::new(MockHid {
            responses: Mutex::new(responses.into()),
            writes: writes.clone(),
        }));
        (dock, writes)
    }

    fn music_response(name: &str) -> Vec<u8> {
        let capture: serde_json::Value =
            serde_json::from_str(include_str!("../research/fixtures/dock-music-0003.json"))
                .unwrap();
        serde_json::from_value(capture[name].clone()).unwrap()
    }

    #[test]
    fn decodes_actual_dock_capture() {
        let settings = decode_settings(&live_response()).unwrap();
        assert_eq!(settings.firmware_version, "0003");
        assert_eq!(settings.firmware_raw, [30, 3, 0, 0]);
        assert_eq!(settings.lighting.mode, DockMode::Flow);
        assert_eq!(settings.lighting.mode_count, 6);
        assert_eq!(settings.lighting.brightness, 2);
        assert_eq!(settings.lighting.color, [255, 0, 0]);
        assert_eq!(settings.lighting.direction, 1);
        assert!(!settings.sleep_enabled);
    }

    #[test]
    fn rejects_truncated_and_misframed_responses() {
        let report = live_response();
        for len in 0..36 {
            assert!(decode_settings(&report[..len]).is_err());
        }
        let mut bad = report.clone();
        bad[1] = 0x27;
        assert!(decode_settings(&bad).is_err());
        bad = report.clone();
        bad[5] = 58;
        assert!(decode_settings(&bad).is_err());
        bad = report;
        bad[10] = 5;
        assert!(decode_settings(&bad).is_err());
    }

    #[test]
    fn read_only_query_ignores_unrelated_reports() {
        let (mut dock, writes) = mock(vec![vec![0xaa, 0x29, 2, 0, 0, 0], live_response()]);
        assert_eq!(dock.read_settings().unwrap().firmware_version, "0003");
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][..6], &[0xaa, 7, 0, 0, 0, 0]);
        assert!(writes[0][6..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn lighting_write_requires_readback_match() {
        let original = decode_settings(&live_response()).unwrap().lighting;
        let (mut dock, writes) = mock(vec![live_response()]);
        dock.set_lighting(&original).unwrap();
        assert_eq!(
            &writes.lock().unwrap()[0][..16],
            &[0xaa, 0x27, 0, 0, 0, 10, 1, 4, 6, 1, 2, 0, 255, 0, 0, 1]
        );
        let (mut dock, _) = mock(vec![live_response()]);
        let mut changed = original;
        changed.brightness = 4;
        assert!(dock.set_lighting(&changed).is_err());
    }

    #[test]
    fn music_write_accepts_captured_sync_with_retained_hardware_effect() {
        // Firmware 0003 retains mode=2 when mode=5 is sent, but confirms sync=1.
        let report = music_response("started");
        let mut expected = decode_settings(&live_response()).unwrap().lighting;
        expected.mode = DockMode::Music;
        expected.music_sync = true;
        expected.brightness = 4;
        let (mut dock, _) = mock(vec![report.clone()]);
        let confirmed = dock.set_lighting(&expected).unwrap();
        assert_eq!(confirmed.lighting, expected);
        assert_eq!(confirmed.raw_response[7], DockMode::Shining as u8);

        let mut changed = report;
        changed[10] = 3;
        let (mut dock, _) = mock(vec![changed]);
        assert!(dock.set_lighting(&expected).is_err());
    }

    #[test]
    fn music_color_update_accepts_captured_sleep_status_byte() {
        let mut expected = decode_settings(&music_response("started")).unwrap().lighting;
        expected.color = [0, 80, 255];
        expected.brightness = 0;
        let (mut dock, _) = mock(vec![music_response("updated")]);
        let confirmed = dock.set_lighting(&expected).unwrap();
        assert_eq!(confirmed.lighting, expected);
        assert_eq!(confirmed.raw_response[15], 255);
        assert!(!confirmed.sleep_enabled);
        assert_eq!(confirmed.sleep_time_raw, 30);
    }

    #[test]
    fn led_stream_is_twelve_rgb_triplets() {
        let (mut dock, writes) = mock(vec![]);
        dock.stream_leds(&[[7, 8, 9]; 12]).unwrap();
        let writes = writes.lock().unwrap();
        assert_eq!(&writes[0][..6], &[0xaa, 0x29, 0, 0, 0, 36]);
        assert_eq!(&writes[0][6..42], [7, 8, 9].repeat(12));
        assert!(writes[0][42..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn firmware_packets_skip_bootloader_and_match_crc_reference() {
        assert_eq!(crc8(b"123456789"), 0xa1);
        let mut image = vec![0xee; BOOTLOADER_LEN];
        image.extend(0_u8..57);
        let plan = FirmwarePlan::from_full_image(&image).unwrap();
        assert_eq!(plan.chunk_count, 2);
        let first = plan.chunk(0).unwrap();
        assert_eq!(&first[..6], &[0xaa, 0x74, 0, 0, 1, 56]);
        assert_eq!(&first[6..62], &(0_u8..56).collect::<Vec<_>>());
        let last = plan.chunk(1).unwrap();
        assert_eq!(&last[..7], &[0xaa, 0x74, 0, 1, 1, 1, 56]);
        assert!(last[7..62].iter().all(|byte| *byte == 0));
        assert_eq!(last[62], crc8(&[56]));
        assert!(plan.chunk(2).is_err());
    }

    #[test]
    fn firmware_does_not_finalize_failed_chunk() {
        let mut image = vec![0xee; BOOTLOADER_LEN];
        image.extend([1, 2, 3]);
        let plan = FirmwarePlan::from_full_image(&image).unwrap();
        let (mut dock, writes) = mock(vec![live_response(), vec![0xaa, 0x2b, 2, 0, 0, 1, 0]]);
        assert!(dock.flash_firmware(&plan, |_, _| {}).is_err());
        let writes = writes.lock().unwrap();
        assert_eq!(writes.iter().filter(|p| p[1] == 0x74).count(), 3);
        assert!(!writes.iter().any(|p| p[1] == 0x75));
    }

    #[test]
    fn firmware_completion_is_a_transfer_receipt() {
        let mut image = vec![0xee; BOOTLOADER_LEN];
        image.extend([1, 2, 3]);
        let plan = FirmwarePlan::from_full_image(&image).unwrap();
        let (mut dock, writes) = mock(vec![
            live_response(),
            vec![0xaa, 0x2b, 2, 0, 0, 1, 0],
            vec![0xaa, 0x74, 2, 0, 0, 1, 0],
        ]);
        let receipt = dock.flash_firmware(&plan, |_, _| {}).unwrap();
        assert_eq!(receipt.payload_bytes_sent, 3);
        assert!(receipt.requires_reconnect_and_version_check);
        assert_eq!(writes.lock().unwrap().last().unwrap()[1], 0x75);
    }
}
