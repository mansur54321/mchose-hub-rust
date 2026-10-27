pub mod advanced;
mod advanced_io;
mod bootloader;
pub mod firmware_catalog;
pub mod firmware_service;
pub mod host_lighting;
pub mod hub_profile;
pub mod model;
mod operations;
pub use advanced::*;
pub use bootloader::{Ace75Bootloader, KeyboardFirmwarePlan, boot_data_packet};
pub use model::*;
pub use operations::{KeyboardNotification, decode_notification, music_requests};

use crate::transport::HidIo;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const MODEL: &str = "Ace 75 8K";
pub const USB_PRODUCT: &str = "Ace 75";
pub const PRODUCT_ID: u16 = 0x303c;
pub const BOOTLOADER_PRODUCT_ID: u16 = 0x303d;
pub const REPORT_ID: u8 = 0;
pub const REPORT_LENGTH: usize = 64;
pub const CHUNK_LENGTH: usize = 56;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub command: u8,
    pub offset: u16,
    pub length: u8,
    pub packet: [u8; REPORT_LENGTH],
}

impl Request {
    pub fn read(command: u8, offset: u16, length: usize) -> Result<Self> {
        ensure!(
            (1..=CHUNK_LENGTH).contains(&length),
            "ACE75: неверная длина чтения {length}"
        );
        Self::encode(command, offset, length, &[])
    }

    pub fn write(command: u8, offset: u16, payload: &[u8]) -> Result<Self> {
        ensure!(
            !payload.is_empty() && payload.len() <= CHUNK_LENGTH,
            "ACE75: неверная длина записи {}",
            payload.len()
        );
        Self::encode(command, offset, payload.len(), payload)
    }

    fn encode(command: u8, offset: u16, length: usize, payload: &[u8]) -> Result<Self> {
        let mut packet = [0; REPORT_LENGTH];
        packet[..8].copy_from_slice(&[
            0x55,
            command,
            0,
            0,
            length as u8,
            offset as u8,
            (offset >> 8) as u8,
            0,
        ]);
        packet[8..8 + payload.len()].copy_from_slice(payload);
        packet[3] = packet[4..8 + payload.len()]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        Ok(Self {
            command,
            offset,
            length: length as u8,
            packet,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub command: u8,
    pub offset: u16,
    pub reserved_header: u8,
    pub reserved: u8,
    pub payload: Vec<u8>,
    pub raw: Vec<u8>,
}

pub fn decode_response(raw: &[u8]) -> Result<Response> {
    ensure!(
        raw.len() >= 8 && raw.len() <= REPORT_LENGTH,
        "ACE75: неверная длина ответа {}",
        raw.len()
    );
    ensure!(
        raw[0] == 0xaa,
        "ACE75: неизвестный заголовок ответа {:02x}",
        raw[0]
    );
    let length = raw[4] as usize;
    ensure!(
        length <= CHUNK_LENGTH && raw.len() >= 8 + length,
        "ACE75: обрезанные данные ответа"
    );
    let checksum = raw[4..8 + length]
        .iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
    ensure!(
        checksum == raw[3],
        "ACE75: неверная сумма ответа: {checksum:02x} != {:02x}",
        raw[3]
    );
    Ok(Response {
        command: raw[1],
        offset: u16::from_le_bytes([raw[5], raw[6]]),
        reserved_header: raw[2],
        reserved: raw[7],
        payload: raw[8..8 + length].to_vec(),
        raw: raw.to_vec(),
    })
}

pub struct Ace75 {
    io: Box<dyn HidIo>,
    notifications: VecDeque<Vec<u8>>,
}

impl Ace75 {
    pub fn new(io: Box<dyn HidIo>) -> Self {
        Self {
            io,
            notifications: VecDeque::new(),
        }
    }

    pub fn take_notifications(&mut self) -> Vec<Vec<u8>> {
        self.notifications.drain(..).collect()
    }

    fn preserve_notification(&mut self, report: Vec<u8>) {
        if self.notifications.len() == 64 {
            self.notifications.pop_front();
        }
        self.notifications.push_back(report);
    }

    fn request(&mut self, request: &Request, attempts: usize) -> Result<Response> {
        for attempt in 0..attempts {
            if attempt != 0 {
                std::thread::sleep(Duration::from_millis(20));
            }
            self.io
                .write_output(REPORT_ID, &request.packet)
                .context("ACE75: отправка запроса")?;
            let deadline = Instant::now() + Duration::from_secs(1);
            for _ in 0..128 {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let raw = self
                    .io
                    .read_input(remaining.as_millis().clamp(1, 1000) as i32)
                    .context("ACE75: чтение ответа")?;
                if raw.is_empty() {
                    break;
                }
                if raw.first() != Some(&0xaa) || raw.get(1) != Some(&request.command) {
                    self.preserve_notification(raw);
                    continue;
                }
                if raw.len() >= 7 && u16::from_le_bytes([raw[5], raw[6]]) != request.offset {
                    self.preserve_notification(raw);
                    continue;
                }
                let response = decode_response(&raw)?;
                ensure!(
                    response.payload.len() <= request.length as usize,
                    "ACE75: ответ длиннее запроса"
                );
                return Ok(response);
            }
        }
        bail!(
            "ACE75: нет ответа на команду {:02x}, смещение {}",
            request.command,
            request.offset
        )
    }

    fn read_range(&mut self, command: u8, offset: usize, length: usize) -> Result<Vec<u8>> {
        ensure!(
            length != 0 && offset.checked_add(length).is_some_and(|end| end <= 65536),
            "ACE75: диапазон чтения выходит за адресное пространство"
        );
        let mut result = Vec::with_capacity(length);
        while result.len() < length {
            let size = (length - result.len()).min(CHUNK_LENGTH);
            let request = Request::read(command, (offset + result.len()) as u16, size)?;
            let response = self.request(&request, 4)?;
            ensure!(
                response.payload.len() == size,
                "ACE75: получено {} из {size} байт по смещению {}",
                response.payload.len(),
                request.offset
            );
            result.extend(response.payload);
        }
        Ok(result)
    }

    pub fn read_info(&mut self) -> Result<Vec<u8>> {
        // Firmware 0x0117 returns 31 valid bytes to the SDK's 56-byte request.
        Ok(self.request(&Request::read(0x03, 0, 56)?, 4)?.payload)
    }

    pub fn read_base(&mut self) -> Result<Vec<u8>> {
        self.read_range(0x04, 0, 56)
    }

    pub fn read_function_config(&mut self, profile: u8) -> Result<Vec<u8>> {
        validate_profile(profile)?;
        self.read_range(0x05, 64 * profile as usize, 64)
    }

    pub fn read_key_matrix(
        &mut self,
        layer: u8,
        key_count: usize,
        factory: bool,
    ) -> Result<Vec<u8>> {
        ensure!(
            layer < (PROFILE_COUNT * LAYER_COUNT) as u8,
            "ACE75: неверный слой клавиш"
        );
        ensure!(
            key_count > 0 && key_count <= 512 / 3,
            "ACE75: неверное число клавиш {key_count}"
        );
        self.read_range(
            if factory { 0x07 } else { 0x08 },
            512 * layer as usize,
            key_count * 3,
        )
    }

    pub fn read_key_triggers(&mut self, profile: u8) -> Result<Vec<u8>> {
        validate_profile(profile)?;
        self.read_range(0xa0, 1024 * profile as usize, 1024)
    }

    pub fn read_key_trigger(&mut self, profile: u8, key_index: u8) -> Result<Vec<u8>> {
        validate_profile(profile)?;
        ensure!(
            key_index < 128,
            "ACE75: неверный индекс клавиши {key_index}"
        );
        self.read_range(0xa0, 1024 * profile as usize + 8 * key_index as usize, 8)
    }

    pub fn read_custom_data(
        &mut self,
        profile: u8,
        profile_size: usize,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>> {
        validate_profile(profile)?;
        ensure!(
            profile_size != 0
                && offset
                    .checked_add(length)
                    .is_some_and(|end| end <= profile_size),
            "ACE75: диапазон пользовательских данных выходит за профиль"
        );
        let start = profile_size
            .checked_mul(profile as usize)
            .and_then(|base| base.checked_add(offset))
            .context("ACE75: переполнение адреса профиля")?;
        self.read_range(0xf1, start, length)
    }
}

fn validate_profile(profile: u8) -> Result<()> {
    ensure!(
        (profile as usize) < PROFILE_COUNT,
        "ACE75: неверный индекс профиля {profile}"
    );
    Ok(())
}

pub fn write_requests(
    command: u8,
    offset: usize,
    data: &[u8],
    element_size: usize,
) -> Result<Vec<Request>> {
    ensure!(
        !data.is_empty()
            && (1..=CHUNK_LENGTH).contains(&element_size)
            && data.len() % element_size == 0,
        "ACE75: неверный размер блока записи"
    );
    ensure!(
        offset
            .checked_add(data.len())
            .is_some_and(|end| end <= 65536),
        "ACE75: адрес записи вне диапазона"
    );
    let chunk = CHUNK_LENGTH / element_size * element_size;
    let mut result = Vec::new();
    for start in (0..data.len()).step_by(chunk) {
        // The SDK overlaps the final partial packet to avoid a firmware memory leak.
        let start = if start > 0 && data.len() - start < chunk {
            data.len() - chunk
        } else {
            start
        };
        let end = (start + chunk).min(data.len());
        result.push(Request::write(
            command,
            (offset + start) as u16,
            &data[start..end],
        )?);
    }
    Ok(result)
}

impl Ace75 {
    pub fn read_identity(&mut self) -> Result<KeyboardInfo> {
        KeyboardInfo::decode(&self.read_info()?)
    }

    pub fn read_profile_raw(&mut self, profile: u8) -> Result<RawKeyboardProfile> {
        validate_profile(profile)?;
        let functions = self.read_function_config(profile)?;
        let mut layers = Vec::with_capacity(LAYER_COUNT);
        for layer in 0..LAYER_COUNT {
            layers.push(self.read_key_matrix(
                profile * LAYER_COUNT as u8 + layer as u8,
                KEY_COUNT,
                false,
            )?);
        }
        Ok(RawKeyboardProfile {
            index: profile,
            functions,
            layers,
            triggers: self.read_key_triggers(profile)?,
            colors: self.read_range(0x0a, profile as usize * 512, KEY_COUNT * 3)?,
            dks: self.read_range(0xa2, profile as usize * DKS_REGION_SIZE, DKS_REGION_SIZE)?,
            mt: self.read_range(0xa4, profile as usize * MT_REGION_SIZE, MT_REGION_SIZE)?,
            tgl: self.read_range(0xa6, profile as usize * TGL_REGION_SIZE, TGL_REGION_SIZE)?,
            custom: self.read_custom_area(profile)?,
        })
    }

    pub fn read_all_profiles_raw(&mut self) -> Result<Vec<RawKeyboardProfile>> {
        (0..PROFILE_COUNT as u8)
            .map(|index| self.read_profile_raw(index))
            .collect()
    }

    pub fn read_profile(&mut self, profile: u8) -> Result<KeyboardProfile> {
        self.read_profile_raw(profile)?.decode()
    }

    pub fn read_snapshot(&mut self) -> Result<KeyboardSnapshot> {
        let info = self.read_identity()?;
        let base = KeyboardBase::decode(&self.read_base()?)?;
        let profile = self.read_profile(base.active_profile)?;
        let profile_names = self.read_profile_names()?;
        let macro_layout = MacroLayout::for_base(base.supports_multiple, base.supports_order);
        let macro_offset = macro_layout.offset(base.active_profile, base.max_profiles)?;
        let macros = MacroBank::decode(&self.read_range(0x0c, macro_offset, macro_layout.size)?)?;
        Ok(KeyboardSnapshot {
            info,
            base,
            profile,
            profile_names,
            macros,
            macro_layout,
        })
    }

    pub fn read_all_profiles(&mut self) -> Result<Vec<KeyboardProfile>> {
        (0..PROFILE_COUNT as u8)
            .map(|index| self.read_profile(index))
            .collect()
    }

    fn write_range(
        &mut self,
        command: u8,
        offset: usize,
        data: &[u8],
        element_size: usize,
    ) -> Result<()> {
        let requests = write_requests(command, offset, data, element_size)?;
        for request in requests {
            self.request(&request, 1)?;
        }
        Ok(())
    }

    fn write_functions_verified(&mut self, profile: u8, expected: &[u8]) -> Result<FunctionConfig> {
        validate_profile(profile)?;
        ensure!(
            expected.len() == 64,
            "ACE75: неверный размер функций профиля"
        );
        self.write_range(0x06, profile as usize * 64, expected, 1)?;
        let actual = self.read_function_config(profile)?;
        // Bytes 32/34 report battery/connection status and may change independently.
        for index in 0..64 {
            if index != 32 && index != 34 {
                ensure!(
                    actual[index] == expected[index],
                    "ACE75: проверка функций профиля не прошла, байт {index}"
                );
            }
        }
        FunctionConfig::decode(&actual)
    }

    pub fn apply_lighting(&mut self, profile: u8, value: &Lighting) -> Result<FunctionConfig> {
        validate_profile(profile)?;
        value.validate()?;
        let current = FunctionConfig::decode(&self.read_function_config(profile)?)?;
        let expected = current.with_lighting(value)?;
        self.write_functions_verified(profile, &expected)
    }

    pub fn apply_performance(
        &mut self,
        profile: u8,
        value: &Performance,
    ) -> Result<FunctionConfig> {
        validate_profile(profile)?;
        value.validate()?;
        let current = FunctionConfig::decode(&self.read_function_config(profile)?)?;
        ensure!(
            value.report_rate == current.performance.report_rate
                && value.scan_rate == current.performance.scan_rate
                && value.wireless_report_rate == current.performance.wireless_report_rate,
            "ACE75: частоту опроса необходимо менять отдельной командой"
        );
        let expected = current.with_performance(value, false)?;
        self.write_functions_verified(profile, &expected)
    }

    pub fn apply_report_rates(
        &mut self,
        profile: u8,
        report_rate: u8,
        scan_rate: u8,
    ) -> Result<FunctionConfig> {
        validate_profile(profile)?;
        ensure!(
            (1..=4).contains(&report_rate) && scan_rate < 16,
            "ACE75: неверный код частоты опроса"
        );
        let current = FunctionConfig::decode(&self.read_function_config(profile)?)?;
        let mut value = current.performance.clone();
        value.report_rate = report_rate;
        value.scan_rate = scan_rate;
        let expected = current.with_performance(&value, true)?;
        self.write_functions_verified(profile, &expected)
    }

    pub fn apply_profile_selection(
        &mut self,
        active_profile: u8,
        count: u8,
        order: &[u8],
    ) -> Result<KeyboardBase> {
        validate_profile(active_profile)?;
        let current = KeyboardBase::decode(&self.read_base()?)?;
        let expected = current.encode_selection(active_profile, count, order)?;
        self.write_range(0x0e, 0, &expected, 1)?;
        let actual = self.read_base()?;
        ensure!(
            actual == expected,
            "ACE75: проверка выбора профиля не прошла"
        );
        KeyboardBase::decode(&actual)
    }

    pub fn apply_keys(
        &mut self,
        profile: u8,
        layer: u8,
        values: &[KeyAssignment],
    ) -> Result<Vec<KeyAssignment>> {
        validate_profile(profile)?;
        ensure!(
            (layer as usize) < LAYER_COUNT && values.len() == KEY_COUNT,
            "ACE75: неверный размер слоя клавиш"
        );
        let global_layer = profile * LAYER_COUNT as u8 + layer;
        let expected = KeyAssignment::encode_all(values);
        self.write_range(0x09, global_layer as usize * 512, &expected, 3)?;
        let actual = self.read_key_matrix(global_layer, KEY_COUNT, false)?;
        ensure!(
            actual == expected,
            "ACE75: проверка назначений клавиш не прошла"
        );
        KeyAssignment::decode_all(&actual)
    }

    pub fn apply_triggers(
        &mut self,
        profile: u8,
        values: &[KeyTrigger],
    ) -> Result<Vec<KeyTrigger>> {
        validate_profile(profile)?;
        ensure!(
            values.len() == KEY_COUNT,
            "ACE75: необходимы настройки всех 128 позиций"
        );
        let current = self.read_key_triggers(profile)?;
        let expected = update_trigger_records(&current, values)?;
        if expected == current {
            return KeyTrigger::decode_all(&current);
        }
        self.write_range(0xa1, profile as usize * 1024, &expected, 8)?;
        let actual = self.read_key_triggers(profile)?;
        ensure!(
            actual == expected,
            "ACE75: проверка магнитных настроек не прошла"
        );
        KeyTrigger::decode_all(&actual)
    }

    pub fn write_custom_data_verified(
        &mut self,
        profile: u8,
        profile_size: usize,
        offset: usize,
        data: &[u8],
    ) -> Result<Vec<u8>> {
        validate_profile(profile)?;
        ensure!(
            profile_size == 1024
                && offset
                    .checked_add(data.len())
                    .is_some_and(|end| end <= profile_size),
            "ACE75: запись выходит за пользовательскую область профиля"
        );
        self.write_range(0xf2, profile as usize * profile_size + offset, data, 1)?;
        let actual = self.read_custom_data(profile, profile_size, offset, data.len())?;
        ensure!(
            actual == data,
            "ACE75: проверка пользовательской области не прошла"
        );
        Ok(actual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn request_packets_match_executed_original_sdk() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../tests/fixtures/ace75/sdk-golden-vectors.json"
        ))
        .unwrap();
        for case in fixture["vectors"].as_array().unwrap() {
            let command = case["command"].as_u64().unwrap() as u8;
            let offset = case["offset"].as_u64().unwrap() as u16;
            let request = if let Some(data) = case["data"].as_array() {
                Request::write(
                    command,
                    offset,
                    &data
                        .iter()
                        .map(|byte| byte.as_u64().unwrap() as u8)
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            } else {
                Request::read(
                    command,
                    offset,
                    case["data"]["size"].as_u64().unwrap() as usize,
                )
                .unwrap()
            };
            let expected: Vec<u8> = case["bytes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|byte| byte.as_u64().unwrap() as u8)
                .collect();
            assert_eq!(request.packet.as_slice(), expected, "{}", case["name"]);
            assert_eq!(case["report_id"], 0);
        }
    }

    #[test]
    fn captured_info_short_payload_and_base_decode_without_fabricated_padding() {
        let info =
            decode_response(include_bytes!("../tests/fixtures/ace75/info-response.bin")).unwrap();
        assert_eq!(info.command, 3);
        assert_eq!(info.payload.len(), 31);
        assert_eq!(&info.payload[..2], &[0x17, 0x01]);
        let base =
            decode_response(include_bytes!("../tests/fixtures/ace75/base-response.bin")).unwrap();
        assert_eq!(base.payload.len(), 56);
        assert_eq!(&base.payload[..6], &[0, 3, 0, 1, 2, 3]);
    }

    #[test]
    fn corrupt_or_truncated_responses_and_oversized_requests_are_rejected() {
        let mut info = include_bytes!("../tests/fixtures/ace75/info-response.bin").to_vec();
        info[10] ^= 1;
        assert!(decode_response(&info).is_err());
        assert!(decode_response(&info[..7]).is_err());
        let base = include_bytes!("../tests/fixtures/ace75/base-response.bin");
        assert!(decode_response(&base[..63]).is_err());
        assert!(Request::read(3, 0, 57).is_err());
        assert!(Request::write(6, 0, &[0; 57]).is_err());
    }

    struct Mock {
        received: Mutex<VecDeque<Vec<u8>>>,
        written: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl HidIo for Mock {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            panic!("ACE75 has no Feature reports")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            panic!("ACE75 has no Feature reports")
        }
        fn write_output(&self, id: u8, packet: &[u8]) -> Result<()> {
            assert_eq!(id, 0);
            self.written.lock().unwrap().push(packet.to_vec());
            Ok(())
        }
        fn read_input(&self, _: i32) -> Result<Vec<u8>> {
            Ok(self
                .received
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }
    }

    #[test]
    fn read_info_accepts_actual_short_response_after_unrelated_notifications() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let stale = include_bytes!("../tests/fixtures/ace75/base-response.bin").to_vec();
        let notice = vec![0xa1, 0, 2];
        let mut driver = Ace75::new(Box::new(Mock {
            received: Mutex::new(VecDeque::from([
                notice.clone(),
                stale.clone(),
                include_bytes!("../tests/fixtures/ace75/info-response.bin").to_vec(),
            ])),
            written: written.clone(),
        }));
        assert_eq!(driver.read_info().unwrap().len(), 31);
        assert_eq!(written.lock().unwrap().len(), 1);
        assert_eq!(driver.take_notifications(), vec![notice, stale]);
    }

    #[test]
    fn invalid_memory_range_is_rejected_before_any_io() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut driver = Ace75::new(Box::new(Mock {
            received: Mutex::new(VecDeque::new()),
            written: written.clone(),
        }));
        assert!(driver.read_key_matrix(0, 171, false).is_err());
        assert!(driver.read_key_triggers(64).is_err());
        assert!(driver.read_custom_data(0, 1024, 1020, 10).is_err());
        assert!(written.lock().unwrap().is_empty());
    }

    #[test]
    fn overlapping_tail_and_element_boundaries_match_original_sdk() {
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../tests/fixtures/ace75/sdk-golden-vectors.json"
        ))
        .unwrap();
        for case in fixture["chunkVectors"].as_array().unwrap() {
            let data: Vec<u8> = case["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect();
            let requests = write_requests(
                case["command"].as_u64().unwrap() as u8,
                case["offset"].as_u64().unwrap() as usize,
                &data,
                case["element"].as_u64().unwrap() as usize,
            )
            .unwrap();
            let expected = case["packets"].as_array().unwrap();
            assert_eq!(requests.len(), expected.len());
            for (request, expected) in requests.iter().zip(expected) {
                let packet: Vec<u8> = expected
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(request.packet.as_slice(), packet, "{}", case["name"]);
            }
        }
    }

    fn response(command: u8, offset: u16, data: &[u8]) -> Vec<u8> {
        let mut raw = vec![0; 64];
        raw[..8].copy_from_slice(&[
            0xaa,
            command,
            0,
            0,
            data.len() as u8,
            offset as u8,
            (offset >> 8) as u8,
            0,
        ]);
        raw[8..8 + data.len()].copy_from_slice(data);
        raw[3] = raw[4..8 + data.len()]
            .iter()
            .fold(0u8, |sum, byte| sum.wrapping_add(*byte));
        raw
    }

    #[test]
    fn write_ack_cannot_hide_readback_mismatch_and_reserved_fields_survive() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut original = vec![0; 64];
        original[18] = 0xa7;
        original[13] = 0x9c;
        original[28] = 0x55;
        original[48] = 0x77;
        let mut wanted = FunctionConfig::decode(&original).unwrap().lighting;
        wanted.main.brightness = 3;
        wanted.main.color = [1, 2, 3];
        let mut driver = Ace75::new(Box::new(Mock {
            received: Mutex::new(VecDeque::from([
                response(5, 0, &original[..56]),
                response(5, 56, &original[56..]),
                response(6, 0, &[]),
                response(6, 8, &[]),
                response(5, 0, &original[..56]),
                response(5, 56, &original[56..]),
            ])),
            written: written.clone(),
        }));
        let error = driver.apply_lighting(0, &wanted).unwrap_err().to_string();
        assert!(error.contains("проверка функций"), "{error}");
        let writes = written.lock().unwrap();
        assert_eq!(writes.len(), 6);
        assert_eq!(writes[2][1], 6);
        assert_eq!(writes[2][8 + 9], 3);
        for index in [13, 18, 28, 48] {
            assert_eq!(writes[2][8 + index], original[index]);
        }
        assert_eq!(&writes[2][8 + 14..8 + 17], &[1, 2, 3]);
    }

    #[test]
    fn invalid_group_is_rejected_before_any_hid_report() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let mut driver = Ace75::new(Box::new(Mock {
            received: Mutex::new(VecDeque::new()),
            written: written.clone(),
        }));
        let mut lighting = FunctionConfig::decode(&[0; 64]).unwrap().lighting;
        lighting.side2.speed = -1;
        assert!(driver.apply_lighting(0, &lighting).is_err());
        assert!(driver.apply_keys(0, 4, &[]).is_err());
        assert!(driver.apply_triggers(4, &[]).is_err());
        assert!(written.lock().unwrap().is_empty());
    }
    #[test]
    fn changing_one_trigger_preserves_all_other_records_and_reserved_bits() {
        let mut raw = vec![0xa2; 1024];
        raw[0] = 0xb3;
        raw[1] = 0x21;
        raw[2] = 9;
        raw[3] = 0xe8;
        let mut keys = KeyTrigger::decode_all(&raw).unwrap();
        keys[0].actuation = 20;
        let changed = update_trigger_records(&raw, &keys).unwrap();
        assert_eq!(changed[0], 0xb3);
        assert_eq!(changed[2], 19);
        assert_eq!(changed[3], 0xe8);
        assert_eq!(&changed[4..], &raw[4..]);
        assert_eq!(
            update_trigger_records(&raw, &KeyTrigger::decode_all(&raw).unwrap()).unwrap(),
            raw
        );
        keys[0].release_precision = 3;
        assert!(update_trigger_records(&raw, &keys).is_err());
    }
}
