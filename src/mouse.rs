use crate::transport::HidIo;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const REPORT_ID: u8 = 0x4d;
pub const POLLING_RATES: [u16; 6] = [125, 500, 1000, 2000, 4000, 8000];
pub const LOD_MM: [f32; 5] = [0.7, 0.9, 1.2, 1.4, 1.7];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Response {
    pub command: u16,
    pub flags: u8,
    pub business_code: u8,
    pub sequence: u8,
    pub checksum_verified: bool,
    pub payload: Vec<u8>,
    pub raw: Vec<u8>,
}

pub fn encode_frame(command: u16, payload: &[u8], length: usize) -> Result<Vec<u8>> {
    ensure!(
        [32, 64].contains(&length),
        "Длина отчёта должна быть 32 или 64"
    );
    ensure!(payload.len() + 9 <= length, "Данные не помещаются в отчёт");
    let mut frame = vec![0; length];
    frame[..8].copy_from_slice(&[
        REPORT_ID,
        1,
        1,
        payload.len() as u8,
        command as u8,
        (command >> 8) as u8,
        0,
        0,
    ]);
    frame[8..8 + payload.len()].copy_from_slice(payload);
    frame[8 + payload.len()] = frame[2..8 + payload.len()].iter().fold(0, |a, b| a ^ b);
    Ok(frame)
}

pub fn decode_frame(frame: &[u8]) -> Result<Response> {
    ensure!(frame.len() >= 8, "Короткий HID-ответ: {} байт", frame.len());
    ensure!(
        frame[0] == REPORT_ID && frame[1] == 1,
        "Неизвестный HID-отчёт"
    );
    let len = frame[3] as usize;
    let command = u16::from_le_bytes([frame[4], frame[5]]);
    ensure!(frame.len() >= 8 + len, "Обрезанные данные ответа");
    let mut checksum_verified = false;
    if frame[2] == 1 {
        ensure!(frame.len() > 8 + len, "В ответе отсутствует XOR");
        let checksum = frame[2..8 + len].iter().fold(0, |a, b| a ^ b);
        checksum_verified = checksum == frame[8 + len];
        // A7 V3 Ultra+ firmware 6 returns 0901 with four version bytes and
        // zero padding, omitting its advertised XOR.
        let version_without_checksum =
            command == 0x0901 && len == 4 && frame[8 + len..].iter().all(|b| *b == 0);
        // The receiver's MCU emits this shape when a mouse command times out.
        let timeout_without_checksum = matches!(
            command,
            0x0001 | 0x0002 | 0x0003 | 0x0009 | 0x000a | 0x0900 | 0x0901 | 0x0905 | 0x090c
        ) && len == 1
            && frame[6..8] == [0, 0]
            && frame[8] == 0xff
            && frame[9..].iter().all(|b| *b == 0);
        ensure!(
            checksum_verified || version_without_checksum || timeout_without_checksum,
            "Неверный XOR ответа: вычислено {checksum:02x}, получено {:02x}, raw={frame:02x?}",
            frame[8 + len]
        );
    }
    Ok(Response {
        command,
        flags: frame[2],
        business_code: frame[6],
        sequence: frame[7],
        checksum_verified,
        payload: frame[8..8 + len].to_vec(),
        raw: frame.to_vec(),
    })
}

fn u16_at(bytes: &[u8], index: usize) -> u16 {
    u16::from_le_bytes([bytes[index], bytes[index + 1]])
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceStatus {
    pub vendor_id: u16,
    pub product_id: u16,
    pub profile_count: u8,
    pub macro_storage_type: u8,
    pub shared_macro_bytes: u16,
    pub macro_unit_bytes: u16,
    pub connection_status: u8,
    pub charge_status: u8,
    pub battery_percent: u8,
    pub game_mode: u8,
    pub raw: Vec<u8>,
}

impl DeviceStatus {
    pub fn decode(data: &[u8]) -> Result<Self> {
        ensure!(data.len() >= 14, "Ответ состояния короче 14 байт");
        Ok(Self {
            vendor_id: u16_at(data, 0),
            product_id: u16_at(data, 2),
            profile_count: data[4],
            macro_storage_type: data[5],
            shared_macro_bytes: u16_at(data, 6),
            macro_unit_bytes: u16_at(data, 8),
            connection_status: data[10],
            charge_status: data[11],
            battery_percent: data[12],
            game_mode: data[13],
            raw: data.to_vec(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MouseSettings {
    pub profile_index: u8,
    pub dpi_index: u8,
    pub usb_rate_index: u8,
    pub wireless_rate_index: u8,
    pub sleep_minutes: u8,
    pub sleep_disabled: bool,
    pub sensor: u8,
    pub angle: i8,
    pub left_debounce_ms: u8,
    pub right_debounce_ms: u8,
    pub raw: Vec<u8>,
}

fn rate_decode(value: u8) -> u8 {
    if value > 1 { value - 1 } else { value }
}
fn rate_encode(value: u8) -> u8 {
    if value > 0 { value + 1 } else { value }
}

impl MouseSettings {
    pub fn decode(data: &[u8]) -> Result<Self> {
        ensure!(data.len() >= 9, "Ответ настроек короче 9 байт");
        ensure!(data[0] < 3, "Неверный индекс профиля");
        Ok(Self {
            profile_index: data[0],
            dpi_index: data[1] & 15,
            usb_rate_index: rate_decode(data[1] >> 4),
            wireless_rate_index: rate_decode(data[2] >> 4),
            sleep_minutes: data[3],
            sleep_disabled: data[4] & 1 != 0,
            sensor: data[5],
            angle: data[6] as i8,
            left_debounce_ms: data[7],
            right_debounce_ms: data[8],
            raw: data.to_vec(),
        })
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        profile_valid(self.profile_index)?;
        ensure!(
            self.dpi_index < 6 && self.usb_rate_index < 6 && self.wireless_rate_index < 6,
            "Неверный индекс DPI или частоты"
        );
        ensure!(
            (-30..=30).contains(&self.angle),
            "Угол вне диапазона −30…30°"
        );
        let mut data = self.raw.clone();
        data.resize(19, 0);
        data.truncate(19);
        data[0] = self.profile_index;
        data[1] = rate_encode(self.usb_rate_index) << 4 | self.dpi_index;
        data[2] = rate_encode(self.wireless_rate_index) << 4 | self.dpi_index;
        data[3] = self.sleep_minutes;
        data[4] = (data[4] & !1) | u8::from(self.sleep_disabled);
        data[5] = self.sensor;
        data[6] = self.angle as u8;
        data[7] = self.left_debounce_ms;
        data[8] = self.right_debounce_ms;
        Ok(data)
    }
    pub fn game_mode(&self) -> u8 {
        self.sensor & 3
    }
    pub fn set_game_mode(&mut self, mode: u8) -> Result<()> {
        ensure!(mode <= 2, "Неизвестный режим сенсора");
        self.sensor = self.sensor & !3 | mode;
        Ok(())
    }
    pub fn ripple_control(&self) -> bool {
        self.sensor & 4 != 0
    }
    pub fn angle_snapping(&self) -> bool {
        self.sensor & 8 != 0
    }
    pub fn motion_sync(&self) -> bool {
        self.sensor & 16 != 0
    }
    pub fn glass_mode(&self) -> bool {
        self.sensor & 128 != 0
    }
    pub fn set_sensor_bit(&mut self, bit: u8, enabled: bool) -> Result<()> {
        ensure!([2, 3, 4, 7].contains(&bit), "Неизвестная функция сенсора");
        self.sensor = self.sensor & !(1 << bit) | (u8::from(enabled) << bit);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DpiSettings {
    pub profile_index: u8,
    pub count: u8,
    pub selected: u8,
    pub separate_y: bool,
    pub x: [u16; 6],
    pub y: [u16; 6],
    pub raw_x: Vec<u8>,
    pub raw_y: Vec<u8>,
}

impl DpiSettings {
    pub fn decode(data: &[u8]) -> Result<Self> {
        ensure!(data.len() >= 17, "Ответ DPI короче 17 байт");
        let mut values = [0; 6];
        for (index, value) in values.iter_mut().enumerate() {
            *value = u16_at(data, 5 + index * 2);
        }
        Ok(Self {
            profile_index: data[0],
            count: data[2],
            selected: data[3],
            separate_y: data[4] != 0,
            x: values,
            y: values,
            raw_x: data.to_vec(),
            raw_y: Vec::new(),
        })
    }
    pub fn encode(&self, direction: u8) -> Result<Vec<u8>> {
        profile_valid(self.profile_index)?;
        ensure!(
            (1..=6).contains(&self.count) && self.selected < self.count,
            "Некорректные ступени DPI"
        );
        ensure!(direction <= 1, "Неизвестная ось DPI");
        let values = if direction == 0 { &self.x } else { &self.y };
        ensure!(
            values.iter().all(|v| (200..=50000).contains(v)),
            "DPI вне диапазона 200…50000"
        );
        let mut data = vec![
            self.profile_index,
            direction,
            u8::from(self.separate_y),
            self.count,
            self.selected,
        ];
        for value in values {
            data.extend_from_slice(&value.to_le_bytes());
        }
        Ok(data)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyBinding {
    pub kind: u8,
    /// Value bytes in device order, excluding kind. Preserves unmapped bindings.
    pub data: Vec<u8>,
}

impl KeyBinding {
    pub fn mouse(mask: u8) -> Self {
        Self {
            kind: 0,
            data: vec![mask, 0],
        }
    }
    pub fn keyboard(usage: u8) -> Self {
        Self {
            kind: 0x11,
            data: vec![usage, 0],
        }
    }
    pub fn chord(modifiers: u8, usage: u8) -> Self {
        Self {
            kind: 0x13,
            data: vec![modifiers, usage],
        }
    }
    pub fn modifiers(mask: u8) -> Self {
        Self {
            kind: 0x12,
            data: vec![mask, 0],
        }
    }
    pub fn wheel(down: bool) -> Self {
        Self {
            kind: 0x05,
            data: vec![u8::from(down), 0],
        }
    }
    pub fn consumer(usage: u16) -> Self {
        Self {
            kind: 0x14,
            data: usage.to_le_bytes().to_vec(),
        }
    }
    pub fn macro_id(id: u16) -> Self {
        Self {
            kind: 0x21,
            data: id.to_le_bytes().to_vec(),
        }
    }
    pub fn profile(index: u8) -> Self {
        Self {
            kind: 0x33,
            data: vec![index, 0],
        }
    }
    pub fn dpi(action: u8) -> Self {
        Self {
            kind: 1,
            data: vec![action, 0, 0],
        }
    }
    pub fn disabled() -> Self {
        Self {
            kind: 0xfe,
            data: vec![0, 0],
        }
    }
    pub fn decode_all(data: &[u8], count: usize) -> Result<Vec<Self>> {
        let mut keys = Vec::new();
        let mut offset = 0;
        for _ in 0..count {
            ensure!(offset < data.len(), "Обрезанный список кнопок");
            let kind = data[offset];
            let len = binding_size(kind);
            ensure!(offset + len <= data.len(), "Обрезанные данные кнопки");
            keys.push(Self {
                kind,
                data: data[offset + 1..offset + len].to_vec(),
            });
            offset += len;
        }
        Ok(keys)
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.data.len() + 1 == binding_size(self.kind),
            "Неверная длина назначения кнопки"
        );
        let mut data = vec![self.kind];
        data.extend_from_slice(&self.data);
        Ok(data)
    }
}

fn binding_size(kind: u8) -> usize {
    match kind {
        1 | 0x22 => 4,
        0x23 | 0x24 => 8,
        0xff => 1,
        _ => 3,
    }
}
fn profile_valid(index: u8) -> Result<()> {
    ensure!(index < 3, "Профиль должен быть 0, 1 или 2");
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdaptivePower {
    pub active_report_mode: u8,
    pub enabled: bool,
    pub rate_index: u8,
    pub sensor_mode: u8,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Telemetry {
    pub dpi_index: Option<u8>,
    pub battery_percent: Option<u8>,
    pub profile_index: Option<u8>,
    pub charge_status: Option<u8>,
    pub raw: Vec<u8>,
}

impl Telemetry {
    pub fn decode(data: &[u8]) -> Result<Self> {
        ensure!(data.len() >= 13, "Короткое событие состояния");
        Ok(Self {
            dpi_index: (data[0] & 1 != 0).then_some(data[2]),
            battery_percent: (data[0] & 2 != 0).then_some(data[3]),
            profile_index: (data[0] & 8 != 0).then_some(data[6]),
            charge_status: (data[1] & 2 != 0).then_some(data[12]),
            raw: data.to_vec(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MacroEvent {
    pub delay_ms: u16,
    pub kind: u8,
    pub pressed: bool,
    pub value: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MouseMacro {
    pub name: String,
    pub repeat_mode: u8,
    pub events: Vec<MacroEvent>,
    pub reserved: [u8; 5],
}

impl MouseMacro {
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(self.name.len() <= 56, "Имя макроса превышает 56 байт UTF-8");
        ensure!(self.repeat_mode <= 3, "Неизвестный режим повтора макроса");
        ensure!(
            self.events.len() <= 1008,
            "Макрос превышает 1008 событий / 4096 байт"
        );
        let mut data = vec![0; 64];
        data[..2].copy_from_slice(&((self.events.len() * 4) as u16).to_le_bytes());
        data[2..4].copy_from_slice(&self.reserved[..2]);
        data[4] = self.repeat_mode;
        data[5..8].copy_from_slice(&self.reserved[2..]);
        data[8..8 + self.name.len()].copy_from_slice(self.name.as_bytes());
        for (index, event) in self.events.iter().enumerate() {
            ensure!(event.kind < 64, "Тип события макроса превышает 6 бит");
            data.extend_from_slice(&event.delay_ms.to_le_bytes());
            data.push(
                event.kind
                    | u8::from(event.pressed) << 6
                    | u8::from(index + 1 == self.events.len()) << 7,
            );
            data.push(event.value);
        }
        Ok(data)
    }
    pub fn decode(data: &[u8]) -> Result<Self> {
        ensure!(data.len() >= 64, "Короткий заголовок макроса");
        let size = u16_at(data, 0) as usize;
        ensure!(
            size <= 4032 && size % 4 == 0 && data.len() >= 64 + size,
            "Некорректная длина событий макроса"
        );
        let name_bytes = &data[8..64];
        let end = name_bytes.iter().position(|b| *b == 0).unwrap_or(56);
        let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
        let events = data[64..64 + size]
            .chunks_exact(4)
            .map(|part| MacroEvent {
                delay_ms: u16_at(part, 0),
                kind: part[2] & 63,
                pressed: part[2] & 64 != 0,
                value: part[3],
            })
            .collect();
        Ok(Self {
            name,
            repeat_mode: data[4],
            events,
            reserved: [data[2], data[3], data[5], data[6], data[7]],
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseSnapshot {
    pub status: DeviceStatus,
    pub settings: MouseSettings,
    pub dpi: DpiSettings,
    pub lod_index: u8,
    pub keys: Vec<KeyBinding>,
    pub profile_names: Vec<String>,
    pub mouse_firmware: String,
    pub receiver_firmware: String,
}

pub struct Mouse {
    io: Box<dyn HidIo>,
    telemetry: VecDeque<Telemetry>,
    unsolicited: VecDeque<Response>,
}

impl Mouse {
    pub fn new(io: Box<dyn HidIo>) -> Self {
        Self {
            io,
            telemetry: VecDeque::new(),
            unsolicited: VecDeque::new(),
        }
    }
    fn exchange(
        &mut self,
        command: u16,
        payload: &[u8],
        length: usize,
        timeout: Duration,
    ) -> Result<Response> {
        let frame = encode_frame(command, payload, length)?;
        self.io.write_output(REPORT_ID, &frame[1..])?;
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let remaining = deadline
                .saturating_duration_since(Instant::now())
                .as_millis()
                .min(100) as i32;
            let raw = self.io.read_input(remaining.max(1))?;
            if raw.is_empty() {
                continue;
            }
            if raw[0] != REPORT_ID {
                continue;
            }
            let response = decode_frame(&raw)
                .with_context(|| format!("Разбор ответа команды {command:04x}"))?;
            if response.command == 0x0600 {
                self.telemetry
                    .push_back(Telemetry::decode(&response.payload)?);
                if self.telemetry.len() > 128 {
                    self.telemetry.pop_front();
                }
            } else if response.command == command {
                return Ok(response);
            } else {
                self.unsolicited.push_back(response);
                if self.unsolicited.len() > 32 {
                    self.unsolicited.pop_front();
                }
            }
        }
        bail!("Тайм-аут команды мыши 0x{command:04x}")
    }
    fn read(&mut self, command: u16, data: &[u8], length: usize) -> Result<Vec<u8>> {
        for attempt in 0..5 {
            let response = self.exchange(command, data, length, Duration::from_secs(3))?;
            if response.payload.first() != Some(&255) {
                return Ok(response.payload);
            }
            if attempt == 4 {
                bail!(
                    "Мышь не отвечает на команду 0x{command:04x} после 5 попыток (FF). Проверьте подключение и питание мыши"
                );
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        unreachable!()
    }
    fn write(&mut self, command: u16, data: &[u8], length: usize) -> Result<()> {
        self.exchange(command, data, length, Duration::from_secs(3))?;
        Ok(())
    }
    pub fn take_telemetry(&mut self) -> Vec<Telemetry> {
        self.telemetry.drain(..).collect()
    }
    pub fn take_unsolicited(&mut self) -> Vec<Response> {
        self.unsolicited.drain(..).collect()
    }
    pub fn poll_telemetry(&mut self, timeout_ms: i32) -> Result<Option<Telemetry>> {
        if let Some(event) = self.telemetry.pop_front() {
            return Ok(Some(event));
        }
        let raw = self.io.read_input(timeout_ms)?;
        if raw.is_empty() || raw[0] != REPORT_ID {
            return Ok(None);
        }
        let response = decode_frame(&raw)?;
        if response.command == 0x0600 {
            Ok(Some(Telemetry::decode(&response.payload)?))
        } else {
            self.unsolicited.push_back(response);
            Ok(None)
        }
    }
    pub fn read_status(&mut self) -> Result<DeviceStatus> {
        DeviceStatus::decode(&self.read(0x0900, &[], 64)?)
    }
    pub fn read_settings(&mut self) -> Result<MouseSettings> {
        MouseSettings::decode(&self.read(0x0002, &[], 64)?)
    }
    pub fn write_settings(&mut self, settings: &MouseSettings) -> Result<()> {
        self.write(0x0102, &settings.encode()?, 64)
    }
    pub fn read_dpi(&mut self, profile: u8) -> Result<DpiSettings> {
        profile_valid(profile)?;
        let mut dpi = DpiSettings::decode(&self.read(0x0003, &[profile, 0], 64)?)?;
        if dpi.separate_y {
            let raw = self.read(0x0003, &[profile, 1], 64)?;
            dpi.y = DpiSettings::decode(&raw)?.x;
            dpi.raw_y = raw;
        }
        Ok(dpi)
    }
    pub fn write_dpi(&mut self, dpi: &DpiSettings) -> Result<()> {
        let x = dpi.encode(0)?;
        let y = dpi.separate_y.then(|| dpi.encode(1)).transpose()?;
        self.write(0x0103, &x, 64)?;
        if let Some(y) = y {
            self.write(0x0103, &y, 64)?;
        }
        Ok(())
    }
    pub fn write_single_dpi(
        &mut self,
        profile: u8,
        stage: u8,
        direction: u8,
        value: u16,
    ) -> Result<()> {
        profile_valid(profile)?;
        ensure!(
            stage < 6 && direction < 2 && (200..=50000).contains(&value),
            "Некорректные параметры DPI"
        );
        self.write(
            0x0104,
            &[profile, stage, direction, value as u8, (value >> 8) as u8],
            64,
        )
    }
    pub fn read_lod(&mut self, profile: u8) -> Result<u8> {
        profile_valid(profile)?;
        self.read(0x0009, &[profile], 64)?
            .first()
            .copied()
            .context("Пустой ответ LOD")
    }
    pub fn write_lod(&mut self, profile: u8, index: u8) -> Result<()> {
        profile_valid(profile)?;
        ensure!(index < 5, "Неверный индекс LOD");
        self.write(0x0109, &[profile, index], 64)
    }
    pub fn read_keys(&mut self, profile: u8) -> Result<Vec<KeyBinding>> {
        profile_valid(profile)?;
        KeyBinding::decode_all(&self.read(0x0001, &[profile, 0, 6], 64)?, 6)
    }
    pub fn write_keys(&mut self, profile: u8, first: u8, keys: &[KeyBinding]) -> Result<()> {
        profile_valid(profile)?;
        ensure!(
            !keys.is_empty() && first as usize + keys.len() <= 6,
            "Некорректный диапазон кнопок"
        );
        let mut data = vec![profile, first, (keys.len() as u8) << 3 | 3];
        for key in keys {
            data.extend(key.encode()?);
        }
        self.write(0x0101, &data, 64)
    }
    pub fn select_profile(&mut self, profile: u8) -> Result<()> {
        profile_valid(profile)?;
        self.write(0x0906, &[profile], 64)
    }
    pub fn read_profile_name(&mut self, profile: u8) -> Result<String> {
        profile_valid(profile)?;
        let mut data = Vec::new();
        for offset in [0, 23, 46] {
            let count = (60 - offset).min(23);
            let part = self.read(0x0905, &[profile, offset, count], 32)?;
            ensure!(
                part.len() >= count as usize,
                "Короткий фрагмент имени профиля"
            );
            data.extend_from_slice(&part[..count as usize]);
        }
        while data.last() == Some(&0) {
            data.pop();
        }
        Ok(String::from_utf8(data).context("Имя профиля содержит некорректный UTF-8")?)
    }
    pub fn write_profile_name(&mut self, profile: u8, name: &str) -> Result<()> {
        profile_valid(profile)?;
        ensure!(
            !name.is_empty() && name.len() <= 60,
            "Имя профиля должно занимать 1…60 байт UTF-8"
        );
        for (index, part) in name.as_bytes().chunks(20).enumerate() {
            let offset = index * 20;
            let control = (part.len() as u8) << 2
                | u8::from(index == 0)
                | u8::from(offset + part.len() == name.len()) << 1;
            let mut data = vec![profile, offset as u8, control];
            data.extend_from_slice(part);
            self.write(0x0910, &data, 64)?;
        }
        Ok(())
    }
    pub fn read_firmware(&mut self, target: u8) -> Result<String> {
        ensure!(target <= 1, "Цель: 0 = мышь, 1 = приёмник");
        let data = self.read(0x0901, &[target], 64)?;
        ensure!(data.len() >= 2, "Короткий ответ версии");
        Ok(format!("{:x}", u16::from_be_bytes([data[0], data[1]])))
    }
    pub fn read_adaptive_power(&mut self, profile: u8) -> Result<AdaptivePower> {
        profile_valid(profile)?;
        let data = self.read(0x000a, &[profile, 0], 64)?;
        ensure!(data.len() >= 4, "Короткий ответ энергосбережения");
        Ok(AdaptivePower {
            active_report_mode: data[0],
            enabled: data[1] != 0,
            rate_index: rate_decode(data[2]),
            sensor_mode: data[3],
            raw: data,
        })
    }
    pub fn write_adaptive_power(&mut self, profile: u8, power: &AdaptivePower) -> Result<()> {
        profile_valid(profile)?;
        ensure!(power.rate_index < 6, "Неверная частота энергосбережения");
        self.write(
            0x010a,
            &[
                profile,
                u8::from(power.enabled),
                rate_encode(power.rate_index),
                power.sensor_mode,
            ],
            64,
        )
    }
    pub fn adaptive_power_heartbeat(&mut self, mode: u8) -> Result<()> {
        self.write(0x010b, &[mode], 64)
    }
    pub fn read_macro_raw(&mut self, id: u8, offset: u16, count: u8) -> Result<Vec<u8>> {
        ensure!(id < 18, "Неверный слот макроса");
        ensure!((1..=22).contains(&count), "Чтение макроса: 1…22 байта");
        ensure!(
            offset as usize + count as usize <= 4096,
            "Чтение за пределами слота макроса"
        );
        let data = self.read(0x090c, &[id, offset as u8, (offset >> 8) as u8, count], 32)?;
        ensure!(data.len() > count as usize, "Короткий ответ макроса");
        Ok(data[1..1 + count as usize].to_vec())
    }
    pub fn read_macro(&mut self, id: u8) -> Result<MouseMacro> {
        ensure!(id < 18, "Неверный слот макроса");
        let mut data = Vec::new();
        while data.len() < 64 {
            let n = (64 - data.len()).min(22);
            data.extend(self.read_macro_raw(id, data.len() as u16, n as u8)?);
        }
        let length = 64 + u16_at(&data, 0) as usize;
        ensure!(
            length <= 4096 && length % 4 == 0,
            "Некорректная длина макроса"
        );
        while data.len() < length {
            let n = (length - data.len()).min(22);
            data.extend(self.read_macro_raw(id, data.len() as u16, n as u8)?);
        }
        MouseMacro::decode(&data)
    }
    pub fn write_macro(&mut self, id: u8, value: &MouseMacro) -> Result<()> {
        ensure!(id < 18, "Неверный слот макроса");
        let data = value.encode()?;
        for (index, part) in data.chunks(19).enumerate() {
            let offset = index * 19;
            let flags = if index == 0 {
                1
            } else if offset + part.len() == data.len() {
                2
            } else {
                0
            };
            let mut payload = vec![
                id,
                offset as u8,
                (offset >> 8) as u8,
                (part.len() as u8) << 2 | flags,
            ];
            payload.extend_from_slice(part);
            self.write(0x090b, &payload, 32)?;
        }
        Ok(())
    }
    pub fn reset_profiles(&mut self, profiles: &[u8], buttons_only: bool) -> Result<()> {
        ensure!(!profiles.is_empty(), "Не выбран профиль для сброса");
        let mut data = [0; 5];
        for profile in profiles {
            profile_valid(*profile)?;
            data[0] |= 1 << profile;
            data[*profile as usize + 1] = if buttons_only { 1 } else { 255 };
        }
        self.write(0x0105, &data, 64)
    }
    pub fn read_snapshot(&mut self) -> Result<MouseSnapshot> {
        self.read_snapshot_with_receiver(true)
    }
    pub fn read_snapshot_with_receiver(&mut self, has_receiver: bool) -> Result<MouseSnapshot> {
        let status = self.read_status()?;
        let settings = self.read_settings()?;
        let index = settings.profile_index;
        let dpi = self.read_dpi(index)?;
        let lod_index = self.read_lod(index)?;
        let keys = self.read_keys(index)?;
        let profile_names = (0..3)
            .map(|p| self.read_profile_name(p))
            .collect::<Result<Vec<_>>>()?;
        let mouse_firmware = self.read_firmware(0)?;
        let receiver_firmware = if has_receiver {
            self.read_firmware(1)?
        } else {
            String::new()
        };
        Ok(MouseSnapshot {
            status,
            settings,
            dpi,
            lod_index,
            keys,
            profile_names,
            mouse_firmware,
            receiver_firmware,
        })
    }
    pub fn firmware_begin(&mut self, target: FirmwareTarget, image: &[u8]) -> Result<()> {
        let response = self.exchange(
            target.command(),
            &firmware_begin_payload(image)?,
            64,
            Duration::from_secs(3),
        )?;
        firmware_ack(&response, 0)
    }
    pub fn firmware_chunk(
        &mut self,
        target: FirmwareTarget,
        index: u16,
        chunk: &[u8],
    ) -> Result<()> {
        let payload = firmware_chunk_payload(index, chunk)?;
        for attempt in 0..3 {
            let response =
                self.exchange(target.command(), &payload, 64, Duration::from_millis(300))?;
            match firmware_ack(&response, 1) {
                Ok(()) => return Ok(()),
                Err(error) if attempt == 2 => {
                    return Err(error)
                        .context(format!("Фрагмент прошивки {index} отклонён 3 раза"));
                }
                Err(_) => {}
            }
        }
        unreachable!()
    }
    pub fn firmware_finish(&mut self, target: FirmwareTarget, image: &[u8]) -> Result<()> {
        let mut data = vec![2];
        data.extend_from_slice(&crc32(image).to_le_bytes());
        let response = self.exchange(target.command(), &data, 64, Duration::from_secs(3))?;
        firmware_ack(&response, 2)
    }
    /// Each physical device has Main and Sub images. The caller selects the
    /// matching wired-mouse or receiver images before invoking this method.
    pub fn update_firmware<F>(&mut self, main: &[u8], sub: &[u8], mut progress: F) -> Result<()>
    where
        F: FnMut(usize, usize),
    {
        firmware_begin_payload(main)?;
        firmware_begin_payload(sub)?;
        let total = main.len() + sub.len();
        let mut completed = 0;
        progress(0, total);
        for (target, image) in [(FirmwareTarget::Main, main), (FirmwareTarget::Sub, sub)] {
            self.firmware_begin(target, image)?;
            for (index, chunk) in image.chunks(49).enumerate() {
                self.firmware_chunk(target, index as u16, chunk)?;
                completed += chunk.len();
                progress(completed, total);
            }
            self.firmware_finish(target, image)?;
            if matches!(target, FirmwareTarget::Main) {
                std::thread::sleep(Duration::from_secs(3));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum FirmwareTarget {
    Main,
    Sub,
}
impl FirmwareTarget {
    fn command(self) -> u16 {
        match self {
            Self::Main => 0x09f0,
            Self::Sub => 0x09f5,
        }
    }
}

fn firmware_ack(response: &Response, phase: u8) -> Result<()> {
    ensure!(
        response.payload.len() >= 2,
        "Обрезанный ответ обновления прошивки"
    );
    ensure!(
        response.payload[0] == phase,
        "Неожиданная фаза обновления прошивки"
    );
    ensure!(
        response.payload[1] == 1,
        "Устройство отклонило операцию прошивки, код {}",
        response.payload[1]
    );
    Ok(())
}

pub fn firmware_begin_payload(image: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        !image.is_empty() && image.len().div_ceil(49) <= u16::MAX as usize,
        "Недопустимый размер прошивки"
    );
    let mut data = vec![0];
    data.extend_from_slice(&(image.len() as u32).to_le_bytes());
    data.extend_from_slice(&[0, 1]);
    data.extend_from_slice(&(image.len().div_ceil(49) as u16).to_le_bytes());
    data.extend_from_slice(&[49, 0]);
    Ok(data)
}
pub fn firmware_chunk_payload(index: u16, chunk: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        !chunk.is_empty() && chunk.len() <= 49,
        "Фрагмент прошивки должен занимать 1…49 байт"
    );
    let mut data = vec![1];
    data.extend_from_slice(&index.to_le_bytes());
    data.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
    data.extend_from_slice(chunk);
    Ok(data)
}
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn frame_matches_upstream_encoder() {
        let frame = encode_frame(0x0003, &[2, 1], 64).unwrap();
        assert_eq!(&frame[..11], &[0x4d, 1, 1, 2, 3, 0, 0, 0, 2, 1, 3]);
        assert!(frame[11..].iter().all(|b| *b == 0));
        assert_eq!(decode_frame(&frame).unwrap().payload, [2, 1]);
        let mut corrupt = frame;
        corrupt[8] ^= 1;
        assert!(decode_frame(&corrupt).is_err());
        assert!(encode_frame(0, &[0; 24], 32).is_err());
    }
    #[test]
    fn settings_preserve_unknown_bits_and_read_write_difference() {
        let mut raw = vec![0; 19];
        raw[..9].copy_from_slice(&[2, 0x62, 0x42, 3, 0xa0, 0x91, 246, 2, 3]);
        raw[18] = 0x77;
        let mut settings = MouseSettings::decode(&raw).unwrap();
        assert_eq!(
            (
                settings.usb_rate_index,
                settings.wireless_rate_index,
                settings.angle
            ),
            (5, 3, -10)
        );
        settings.sleep_disabled = true;
        settings.set_sensor_bit(2, true).unwrap();
        let encoded = settings.encode().unwrap();
        assert_eq!(encoded[4], 0xa1);
        assert_eq!(encoded[5], 0x95);
        assert_eq!(encoded[18], 0x77);
    }
    #[test]
    fn dpi_write_has_different_field_order_than_read() {
        let mut raw = vec![2, 0, 3, 1, 1];
        for v in [400u16, 800, 1600, 3200, 6400, 50000] {
            raw.extend(v.to_le_bytes());
        }
        let dpi = DpiSettings::decode(&raw).unwrap();
        assert_eq!(&dpi.encode(0).unwrap()[..5], &[2, 0, 1, 3, 1]);
        assert_eq!(dpi.x[5], 50000);
    }
    #[test]
    fn macro_encoding_and_crc_reference() {
        let value = MouseMacro {
            name: "Тест".into(),
            repeat_mode: 1,
            reserved: [0; 5],
            events: vec![
                MacroEvent {
                    delay_ms: 0,
                    kind: 17,
                    pressed: true,
                    value: 4,
                },
                MacroEvent {
                    delay_ms: 100,
                    kind: 63,
                    pressed: false,
                    value: 0,
                },
                MacroEvent {
                    delay_ms: 0,
                    kind: 17,
                    pressed: false,
                    value: 4,
                },
            ],
        };
        let encoded = value.encode().unwrap();
        assert_eq!(
            &encoded[64..],
            &[0, 0, 0x51, 4, 100, 0, 63, 0, 0, 0, 0x91, 4]
        );
        assert_eq!(MouseMacro::decode(&encoded).unwrap(), value);
        assert_eq!(crc32(b"123456789"), 0xcbf43926);
    }
    #[test]
    fn receiver_firmware_checksum_quirk_is_narrowly_scoped() {
        let mut response = vec![0; 64];
        response[..12].copy_from_slice(&[0x4d, 1, 1, 4, 1, 9, 0, 0, 0, 6, 0, 6]);
        let decoded = decode_frame(&response).unwrap();
        assert_eq!(decoded.payload, [0, 6, 0, 6]);
        assert!(!decoded.checksum_verified);
        response[4] = 0;
        assert!(decode_frame(&response).is_err());
        response[4] = 1;
        response[3] = 3;
        assert!(decode_frame(&response).is_err());
        response[3] = 4;
        response[63] = 1;
        assert!(decode_frame(&response).is_err());
    }
    #[test]
    fn firmware_negative_ack_is_not_success() {
        let failure = decode_frame(&encode_frame(0x09f0, &[0, 0], 64).unwrap()).unwrap();
        assert!(firmware_ack(&failure, 0).is_err());
        let success = decode_frame(&encode_frame(0x09f0, &[0, 1], 64).unwrap()).unwrap();
        assert!(firmware_ack(&success, 0).is_ok());
        assert!(firmware_ack(&success, 1).is_err());
        assert_eq!(
            firmware_begin_payload(&[0; 50]).unwrap(),
            [0, 50, 0, 0, 0, 0, 1, 2, 0, 49, 0]
        );
    }
    #[test]
    fn complete_upstream_action_palette_has_valid_wire_lengths() {
        let actions: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../assets/hub-actions.json")).unwrap();
        assert_eq!(actions.len(), 184);
        for action in actions {
            let key = KeyBinding {
                kind: action["kind"].as_u64().unwrap() as u8,
                data: serde_json::from_value(action["data"].clone()).unwrap(),
            };
            key.encode()
                .unwrap_or_else(|error| panic!("{}: {error}", action["id"]));
        }
        assert_eq!(KeyBinding::modifiers(3).encode().unwrap(), [0x12, 3, 0]);
        assert_eq!(KeyBinding::wheel(true).encode().unwrap(), [5, 1, 0]);
    }
    struct Fake {
        inputs: Mutex<VecDeque<Vec<u8>>>,
        outputs: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl HidIo for Fake {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            bail!("unexpected feature")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            bail!("unexpected feature")
        }
        fn write_output(&self, id: u8, data: &[u8]) -> Result<()> {
            let mut frame = vec![id];
            frame.extend_from_slice(data);
            self.outputs.lock().unwrap().push(frame);
            Ok(())
        }
        fn read_input(&self, _: i32) -> Result<Vec<u8>> {
            Ok(self.inputs.lock().unwrap().pop_front().unwrap_or_default())
        }
    }
    #[test]
    fn invalid_y_dpi_cannot_partially_write_x() {
        let outputs = Arc::new(Mutex::new(Vec::new()));
        let mut mouse = Mouse::new(Box::new(Fake {
            inputs: Mutex::new(VecDeque::new()),
            outputs: outputs.clone(),
        }));
        let dpi = DpiSettings {
            profile_index: 0,
            count: 1,
            selected: 0,
            separate_y: true,
            x: [800; 6],
            y: [0; 6],
            raw_x: vec![],
            raw_y: vec![],
        };
        assert!(mouse.write_dpi(&dpi).is_err());
        assert!(outputs.lock().unwrap().is_empty());
    }
    #[test]
    fn corrupt_macro_length_stops_after_header() {
        let mut header = [0; 64];
        header[..2].copy_from_slice(&4096u16.to_le_bytes());
        let replies = header
            .chunks(22)
            .map(|part| {
                let mut payload = vec![0];
                payload.extend_from_slice(part);
                encode_frame(0x090c, &payload, 32).unwrap()
            })
            .collect();
        let outputs = Arc::new(Mutex::new(Vec::new()));
        let mut mouse = Mouse::new(Box::new(Fake {
            inputs: Mutex::new(replies),
            outputs: outputs.clone(),
        }));
        assert!(mouse.read_macro(0).is_err());
        assert_eq!(outputs.lock().unwrap().len(), 3);
        assert!(mouse.read_macro_raw(18, 0, 1).is_err());
        assert!(mouse.read_macro_raw(0, 4095, 2).is_err());
        assert_eq!(outputs.lock().unwrap().len(), 3);
    }
    #[test]
    fn driver_keeps_unsolicited_telemetry_and_retries_busy_reads() {
        let mut event = [0; 13];
        event[0] = 3;
        event[2] = 2;
        event[3] = 84;
        let status = [0x37, 0x38, 0x33, 0x40, 3, 1, 0, 16, 0, 4, 1, 0, 84, 2];
        let outputs = Arc::new(Mutex::new(Vec::new()));
        let fake = Fake {
            inputs: Mutex::new(VecDeque::from([
                encode_frame(0x0600, &event, 64).unwrap(),
                encode_frame(0x0900, &[255], 64).unwrap(),
                encode_frame(0x0900, &status, 64).unwrap(),
            ])),
            outputs: outputs.clone(),
        };
        let mut mouse = Mouse::new(Box::new(fake));
        let result = mouse.read_status().unwrap();
        assert_eq!(result.battery_percent, 84);
        assert_eq!(result.product_id, 0x4033);
        assert_eq!(mouse.take_telemetry()[0].dpi_index, Some(2));
        assert_eq!(outputs.lock().unwrap().len(), 2);
    }

    #[test]
    fn driver_retries_captured_busy_status_without_checksum() {
        let mut busy = vec![0; 64];
        busy[..9].copy_from_slice(&[0x4d, 1, 1, 1, 0, 9, 0, 0, 0xff]);
        let status = [0x37, 0x38, 0x33, 0x40, 3, 1, 0, 16, 0, 4, 1, 0, 84, 2];
        let outputs = Arc::new(Mutex::new(Vec::new()));
        let mut mouse = Mouse::new(Box::new(Fake {
            inputs: Mutex::new(VecDeque::from([
                busy.clone(),
                encode_frame(0x0900, &status, 64).unwrap(),
            ])),
            outputs: outputs.clone(),
        }));
        assert_eq!(mouse.read_status().unwrap().battery_percent, 84);
        assert_eq!(outputs.lock().unwrap().len(), 2);
        assert!(!decode_frame(&busy).unwrap().checksum_verified);

        let attempts = Arc::new(Mutex::new(Vec::new()));
        let mut unavailable = Mouse::new(Box::new(Fake {
            inputs: Mutex::new(VecDeque::from(vec![busy.clone(); 5])),
            outputs: attempts.clone(),
        }));
        let error = unavailable.read_status().unwrap_err().to_string();
        assert!(error.contains("5 попыток (FF)"));
        assert!(!error.contains("XOR"));
        assert_eq!(attempts.lock().unwrap().len(), 5);

        busy[4] = 3;
        busy[5] = 1;
        assert!(decode_frame(&busy).is_err());
        busy[4] = 0;
        busy[5] = 9;
        busy[63] = 1;
        assert!(decode_frame(&busy).is_err());
    }

    #[test]
    fn receiver_timeout_is_retried_for_firmware_and_lod_reads() {
        let timeout = |command: u16| {
            let mut raw = vec![0; 64];
            raw[..9].copy_from_slice(&[
                0x4d,
                1,
                1,
                1,
                command as u8,
                (command >> 8) as u8,
                0,
                0,
                0xff,
            ]);
            raw
        };
        let outputs = Arc::new(Mutex::new(Vec::new()));
        let mut mouse = Mouse::new(Box::new(Fake {
            inputs: Mutex::new(VecDeque::from([
                timeout(0x0901),
                encode_frame(0x0901, &[0, 0x17, 0, 0x17], 64).unwrap(),
                timeout(0x0009),
                encode_frame(0x0009, &[4], 64).unwrap(),
            ])),
            outputs: outputs.clone(),
        }));
        assert_eq!(mouse.read_firmware(0).unwrap(), "17");
        assert_eq!(mouse.read_lod(0).unwrap(), 4);
        assert_eq!(outputs.lock().unwrap().len(), 4);
        assert!(
            outputs.lock().unwrap().iter().all(|frame| {
                matches!(u16::from_le_bytes([frame[4], frame[5]]), 0x0901 | 0x0009)
            })
        );
    }
}
