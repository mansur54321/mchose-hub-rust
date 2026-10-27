use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const PROFILE_COUNT: usize = 4;
pub const LAYER_COUNT: usize = 4;
pub const KEY_COUNT: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardInfo {
    pub firmware_version: String,
    pub report_rate_enhanced: bool,
    pub usb3_mode: bool,
    pub raw: Vec<u8>,
}
impl KeyboardInfo {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() >= 7, "ACE75: неполная информация устройства");
        Ok(Self {
            firmware_version: format!("{:x}", u16::from_le_bytes([raw[0], raw[1]])),
            report_rate_enhanced: raw[6] & 1 != 0,
            usb3_mode: raw[6] & 2 != 0,
            raw: raw.to_vec(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardBase {
    pub active_profile: u8,
    pub profile_count: u8,
    pub profile_order: Vec<u8>,
    pub max_profiles: u8,
    pub supports_order: bool,
    pub supports_multiple: bool,
    pub raw: Vec<u8>,
}
impl KeyboardBase {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() == 56, "ACE75: неверный размер базовых настроек");
        let mut sorted = raw[2..6].to_vec();
        sorted.sort_unstable();
        let supports_order = sorted == [0, 1, 2, 3];
        let supports_multiple = raw[0] < 4 && (!supports_order || raw[1] <= 4);
        let profile_count = if supports_multiple && supports_order {
            raw[1]
        } else if supports_multiple {
            2
        } else {
            1
        };
        ensure!(
            profile_count > 0,
            "ACE75: пустой список аппаратных профилей"
        );
        if supports_multiple && supports_order {
            ensure!(
                raw[0] < profile_count,
                "ACE75: активный профиль за пределами списка"
            );
        }
        Ok(Self {
            active_profile: if supports_multiple && supports_order {
                raw[2 + raw[0] as usize]
            } else if supports_multiple {
                raw[0]
            } else {
                0
            },
            profile_count,
            profile_order: if supports_order {
                raw[2..6].to_vec()
            } else {
                (0..profile_count).collect()
            },
            max_profiles: if supports_multiple && supports_order {
                4
            } else {
                profile_count
            },
            supports_order,
            supports_multiple,
            raw: raw.to_vec(),
        })
    }

    pub fn encode_selection(&self, active_profile: u8, count: u8, order: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            self.supports_order && self.supports_multiple,
            "ACE75: порядок профилей не поддерживается устройством"
        );
        ensure!(
            (1..=self.max_profiles).contains(&count),
            "ACE75: неверное число профилей"
        );
        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        ensure!(
            sorted == (0..self.max_profiles).collect::<Vec<_>>(),
            "ACE75: порядок должен содержать каждый профиль ровно один раз"
        );
        let active_position = order.iter().position(|value| *value == active_profile);
        ensure!(
            active_position.is_some_and(|index| index < count as usize),
            "ACE75: выбранный профиль скрыт или отсутствует"
        );
        let mut raw = self.raw.clone();
        ensure!(raw.len() == 56, "ACE75: неверный размер базовых настроек");
        raw[0] = active_position.unwrap() as u8;
        raw[1] = count;
        raw[2..2 + order.len()].copy_from_slice(order);
        Ok(raw)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Performance {
    pub mac_mode: u8,
    pub report_rate: u8,
    pub scan_rate: u8,
    pub dead_zone: u8,
    pub lock_win: bool,
    pub lock_alt_tab: bool,
    pub lock_alt_f4: bool,
    pub tachyon_mode: bool,
    pub bottom_rapid_trigger: bool,
    pub travel_check: bool,
    pub debug_mode: bool,
    pub debounce_level: u8,
    pub rt_smart: u8,
    pub battery_percent: u8,
    pub charging: bool,
    pub roller_type: u8,
    pub work_mode: u8,
    pub sleep_time: u8,
    pub sleep_mode: u8,
    pub bluetooth_channel: u8,
    pub wireless_report_rate: u8,
}
impl Performance {
    fn decode(raw: &[u8]) -> Self {
        Self {
            mac_mode: raw[1] & 15,
            report_rate: raw[4] & 15,
            scan_rate: raw[4] >> 4,
            dead_zone: raw[5],
            lock_win: raw[6] & 1 != 0,
            lock_alt_tab: raw[6] & 2 != 0,
            lock_alt_f4: raw[6] & 4 != 0,
            tachyon_mode: raw[7] & 1 != 0,
            bottom_rapid_trigger: raw[7] & 2 != 0,
            travel_check: raw[7] & 4 != 0,
            debug_mode: raw[7] & 8 != 0,
            debounce_level: raw[7] >> 5,
            rt_smart: raw[20],
            battery_percent: raw[32] & 127,
            charging: raw[32] & 128 != 0,
            roller_type: raw[33],
            work_mode: raw[34],
            sleep_time: raw[35],
            sleep_mode: raw[36],
            bluetooth_channel: raw[37],
            wireless_report_rate: raw[38],
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.mac_mode < 16
                && self.report_rate < 16
                && self.scan_rate < 16
                && self.debounce_level < 8,
            "ACE75: значение настройки не помещается в поле протокола"
        );
        Ok(())
    }

    fn apply_to(&self, raw: &mut [u8], update_report_rate: bool) -> Result<()> {
        self.validate()?;
        raw[1] = (raw[1] & 0xf0) | self.mac_mode;
        if update_report_rate {
            raw[4] = self.scan_rate << 4 | self.report_rate;
            raw[38] = self.wireless_report_rate;
        }
        raw[5] = self.dead_zone;
        raw[6] = raw[6] & 0xf8
            | (self.lock_alt_f4 as u8) << 2
            | (self.lock_alt_tab as u8) << 1
            | self.lock_win as u8;
        // SDK uses -240 & byte7: preserve bit4; the other bits are defined here.
        raw[7] = raw[7] & 0x10
            | self.debounce_level << 5
            | (self.debug_mode as u8) << 3
            | (self.travel_check as u8) << 2
            | (self.bottom_rapid_trigger as u8) << 1
            | self.tachyon_mode as u8;
        raw[20] = self.rt_smart;
        raw[33] = self.roller_type;
        raw[35] = self.sleep_time;
        raw[36] = self.sleep_mode;
        raw[37] = self.bluetooth_channel;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LightZone {
    pub effect: u8,
    pub brightness: u8,
    pub speed: i16,
    pub direction: u8,
    pub rainbow: bool,
    pub color_index: u8,
    pub color: [u8; 3],
}
impl LightZone {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.brightness <= 100,
            "ACE75: яркость должна быть от 0 до 100 процентов"
        );
        ensure!(
            (0..=4).contains(&self.speed),
            "ACE75: скорость подсветки должна быть от 0 до 4"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LightScope {
    Main,
    Side2,
    Side,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Lighting {
    pub main: LightZone,
    pub side: LightZone,
    pub side2: LightZone,
    pub scope: LightScope,
    pub calibration_color: [u8; 3],
}
impl Lighting {
    fn decode(raw: &[u8]) -> Self {
        Self {
            main: LightZone {
                effect: raw[8],
                brightness: raw[9],
                speed: 4 - i16::from(raw[10]),
                direction: raw[11],
                rainbow: raw[12] != 0,
                color_index: raw[13],
                color: [raw[14], raw[15], raw[16]],
            },
            side: LightZone {
                effect: raw[24],
                brightness: raw[25],
                speed: 4 - i16::from(raw[26]),
                direction: 0,
                rainbow: raw[27] != 0,
                color_index: raw[28],
                color: [raw[29], raw[30], raw[31]],
            },
            side2: LightZone {
                effect: raw[44],
                brightness: raw[45],
                speed: 4 - i16::from(raw[46]),
                direction: 0,
                rainbow: raw[47] != 0,
                color_index: raw[48],
                color: [raw[49], raw[50], raw[51]],
            },
            scope: match raw[23] {
                1 => LightScope::Side2,
                2 => LightScope::Side,
                _ => LightScope::Main,
            },
            calibration_color: [raw[40], raw[41], raw[42]],
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.main.effect <= 22 && self.side.effect <= 5 && self.side2.effect <= 3,
            "ACE75: неизвестный эффект подсветки"
        );
        self.main.validate()?;
        self.side.validate()?;
        self.side2.validate()
    }

    fn apply_to(&self, raw: &mut [u8]) -> Result<()> {
        self.validate()?;
        raw[8] = self.main.effect;
        raw[9] = self.main.brightness;
        raw[10] = (4 - self.main.speed) as u8;
        raw[11] = self.main.direction;
        raw[12] = self.main.rainbow as u8;
        raw[14..17].copy_from_slice(&self.main.color);
        raw[24] = self.side.effect;
        raw[25] = self.side.brightness;
        raw[26] = (4 - self.side.speed) as u8;
        raw[27] = self.side.rainbow as u8;
        raw[29..32].copy_from_slice(&self.side.color);
        raw[44] = self.side2.effect;
        raw[45] = self.side2.brightness;
        raw[46] = (4 - self.side2.speed) as u8;
        raw[47] = self.side2.rainbow as u8;
        raw[49..52].copy_from_slice(&self.side2.color);
        raw[23] = match self.scope {
            LightScope::Main => 0,
            LightScope::Side2 => 1,
            LightScope::Side => 2,
        };
        raw[40..43].copy_from_slice(&self.calibration_color);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionConfig {
    pub performance: Performance,
    pub lighting: Lighting,
    pub raw: Vec<u8>,
}
impl FunctionConfig {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() == 64, "ACE75: неверный размер функций профиля");
        Ok(Self {
            performance: Performance::decode(raw),
            lighting: Lighting::decode(raw),
            raw: raw.to_vec(),
        })
    }
    pub fn with_performance(
        &self,
        value: &Performance,
        update_report_rate: bool,
    ) -> Result<Vec<u8>> {
        ensure!(
            self.raw.len() == 64,
            "ACE75: неверный размер функций профиля"
        );
        let mut raw = self.raw.clone();
        value.apply_to(&mut raw, update_report_rate)?;
        Ok(raw)
    }
    pub fn with_lighting(&self, value: &Lighting) -> Result<Vec<u8>> {
        ensure!(
            self.raw.len() == 64,
            "ACE75: неверный размер функций профиля"
        );
        let mut raw = self.raw.clone();
        value.apply_to(&mut raw)?;
        Ok(raw)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyAssignment {
    pub kind: u8,
    pub code1: u8,
    pub code2: u8,
}
impl KeyAssignment {
    pub fn decode_all(raw: &[u8]) -> Result<Vec<Self>> {
        ensure!(raw.len() % 3 == 0, "ACE75: обрезанная таблица клавиш");
        Ok(raw
            .chunks_exact(3)
            .map(|key| Self {
                kind: key[0],
                code1: key[1],
                code2: key[2],
            })
            .collect())
    }
    pub fn encode_all(keys: &[Self]) -> Vec<u8> {
        keys.iter()
            .flat_map(|key| [key.kind, key.code1, key.code2])
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyTrigger {
    pub switch_type: u8,
    pub key_mode: u8,
    pub priority: u8,
    pub actuation: u16,
    pub press_precision: u8,
    pub release_precision: u8,
    pub rt_press: u16,
    pub rt_release: u16,
    pub press_deadzone: u8,
    pub release_deadzone: u8,
}
impl KeyTrigger {
    pub fn decode_all(raw: &[u8]) -> Result<Vec<Self>> {
        ensure!(
            raw.len() % 8 == 0 && raw.len() <= KEY_COUNT * 8,
            "ACE75: неверный размер магнитных настроек"
        );
        Ok(raw
            .chunks_exact(8)
            .map(|key| Self {
                switch_type: key[0] & 15,
                key_mode: key[1] & 15,
                priority: key[1] >> 4,
                actuation: u16::from_le_bytes([key[2], key[3] & 1]) + 1,
                press_precision: (key[3] >> 3) & 3,
                release_precision: (key[3] >> 1) & 3,
                rt_press: u16::from_le_bytes([key[4], key[5] & 1]) + 1,
                rt_release: u16::from_le_bytes([key[6], key[7] & 1]) + 1,
                press_deadzone: key[5] >> 1,
                release_deadzone: key[7] >> 1,
            })
            .collect())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.switch_type < 16
                && self.key_mode < 16
                && self.priority < 16
                && self.press_precision < 4
                && self.release_precision < 4,
            "ACE75: неверное битовое поле магнитных настроек"
        );
        ensure!(
            [self.actuation, self.rt_press, self.rt_release]
                .iter()
                .all(|v| (1..=512).contains(v)),
            "ACE75: точка срабатывания должна быть от 1 до 512"
        );
        ensure!(
            self.press_deadzone < 128 && self.release_deadzone < 128,
            "ACE75: мёртвая зона должна быть меньше 128"
        );
        Ok(())
    }
    pub fn encode_all(values: &[Self]) -> Result<Vec<u8>> {
        let mut raw = Vec::with_capacity(values.len() * 8);
        for value in values {
            value.validate()?;
            let actuation = (value.actuation - 1).to_le_bytes();
            let press = (value.rt_press - 1).to_le_bytes();
            let release = (value.rt_release - 1).to_le_bytes();
            raw.extend_from_slice(&[
                0xa0 | value.switch_type,
                value.priority << 4 | value.key_mode,
                actuation[0],
                actuation[1] | value.press_precision << 3 | value.release_precision << 1,
                press[0],
                press[1] | value.press_deadzone << 1,
                release[0],
                release[1] | value.release_deadzone << 1,
            ]);
        }
        Ok(raw)
    }
}

pub fn update_trigger_records(current: &[u8], values: &[KeyTrigger]) -> Result<Vec<u8>> {
    ensure!(
        current.len() == KEY_COUNT * 8 && values.len() == KEY_COUNT,
        "ACE75: необходимы настройки всех 128 позиций"
    );
    let decoded = KeyTrigger::decode_all(current)?;
    let mut expected = current.to_vec();
    for (index, (before, wanted)) in decoded.iter().zip(values).enumerate() {
        if before == wanted {
            continue;
        }
        ensure!(
            wanted.press_precision < 3 && wanted.release_precision < 3,
            "ACE75: неизвестный режим точности Rapid Trigger"
        );
        let mut encoded = KeyTrigger::encode_all(std::slice::from_ref(wanted))?;
        let start = index * 8;
        encoded[0] = current[start] & 0xf0 | encoded[0] & 0x0f;
        encoded[3] = current[start + 3] & 0xe0 | encoded[3] & 0x1f;
        expected[start..start + 8].copy_from_slice(&encoded);
    }
    Ok(expected)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RawKeyboardProfile {
    pub index: u8,
    pub functions: Vec<u8>,
    pub layers: Vec<Vec<u8>>,
    pub triggers: Vec<u8>,
    pub colors: Vec<u8>,
    pub dks: Vec<u8>,
    pub mt: Vec<u8>,
    pub tgl: Vec<u8>,
    pub custom: Vec<u8>,
}
impl RawKeyboardProfile {
    pub fn decode(&self) -> Result<KeyboardProfile> {
        ensure!(
            self.index < PROFILE_COUNT as u8
                && self.layers.len() == LAYER_COUNT
                && self.layers.iter().all(|layer| layer.len() == KEY_COUNT * 3),
            "ACE75: неверная таблица слоёв профиля"
        );
        ensure!(
            self.triggers.len() == KEY_COUNT * 8
                && self.colors.len() == KEY_COUNT * 3
                && self.custom.len() == super::advanced::CUSTOM_AREA_SIZE,
            "ACE75: неверный размер необработанных областей профиля"
        );
        Ok(KeyboardProfile {
            index: self.index,
            functions: FunctionConfig::decode(&self.functions)?,
            layers: self
                .layers
                .iter()
                .map(|layer| KeyAssignment::decode_all(layer))
                .collect::<Result<_>>()?,
            triggers: KeyTrigger::decode_all(&self.triggers)?,
            trigger_raw: self.triggers.clone(),
            colors: self
                .colors
                .chunks_exact(3)
                .map(|rgb| [rgb[0], rgb[1], rgb[2]])
                .collect(),
            advanced: super::advanced::AdvancedState::decode(&self.dks, &self.mt, &self.tgl)?,
            custom_params: super::advanced::CustomParams::decode(
                &self.custom[super::advanced::CUSTOM_PARAMS_OFFSET
                    ..super::advanced::CUSTOM_PARAMS_OFFSET + super::advanced::CUSTOM_PARAMS_SIZE],
            )?,
            custom_raw: self.custom.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardProfile {
    pub index: u8,
    pub functions: FunctionConfig,
    pub layers: Vec<Vec<KeyAssignment>>,
    pub triggers: Vec<KeyTrigger>,
    pub trigger_raw: Vec<u8>,
    pub colors: Vec<[u8; 3]>,
    pub custom_params: super::advanced::CustomParams,
    pub advanced: super::advanced::AdvancedState,
    pub custom_raw: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardSnapshot {
    pub info: KeyboardInfo,
    pub base: KeyboardBase,
    pub profile: KeyboardProfile,
    pub profile_names: super::advanced::ProfileNames,
    pub macros: super::advanced::MacroBank,
    pub macro_layout: super::advanced::MacroLayout,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn golden() -> serde_json::Value {
        serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/ace75/sdk-golden-vectors.json"
        ))
        .unwrap()
    }
    fn bytes(value: &serde_json::Value) -> Vec<u8> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect()
    }

    #[test]
    fn info_and_base_match_live_captures_without_bcd_decimal_confusion() {
        let info = crate::ace75::decode_response(include_bytes!(
            "../../tests/fixtures/ace75/info-response.bin"
        ))
        .unwrap();
        assert_eq!(
            KeyboardInfo::decode(&info.payload)
                .unwrap()
                .firmware_version,
            "117"
        );
        let base = crate::ace75::decode_response(include_bytes!(
            "../../tests/fixtures/ace75/base-response.bin"
        ))
        .unwrap();
        let decoded = KeyboardBase::decode(&base.payload).unwrap();
        assert_eq!(
            (
                decoded.active_profile,
                decoded.profile_count,
                decoded.max_profiles
            ),
            (0, 3, 4)
        );
        assert_eq!(decoded.profile_order, [0, 1, 2, 3]);
        assert!(decoded.encode_selection(3, 3, &[0, 1, 2, 3]).is_err());
        assert!(decoded.encode_selection(0, 3, &[0, 1, 1, 3]).is_err());
        let changed = decoded.encode_selection(3, 3, &[3, 1, 2, 0]).unwrap();
        assert_eq!(&changed[..6], &[0, 3, 3, 1, 2, 0]);
        assert_eq!(&changed[6..], &base.payload[6..]);
    }

    #[test]
    fn performance_decode_and_rmw_match_original_sdk() {
        let fixture = golden();
        let data = &fixture["typed"];
        let config = FunctionConfig::decode(&bytes(&data["raw"])).unwrap();
        let source = &data["performance"];
        macro_rules! check {
            ($field:ident,$name:literal) => {
                assert_eq!(
                    config.performance.$field as u64,
                    source[$name].as_u64().unwrap(),
                    $name
                );
            };
        }
        check!(mac_mode, "macMode");
        check!(report_rate, "reporteRate");
        check!(scan_rate, "tickRate");
        check!(dead_zone, "deadZone");
        check!(lock_win, "lockWin");
        check!(lock_alt_tab, "lockAltTap");
        check!(lock_alt_f4, "lockAltF4");
        check!(tachyon_mode, "tachyonMode");
        check!(bottom_rapid_trigger, "bottomrapidtriggerMode");
        check!(travel_check, "travelCheck");
        check!(debug_mode, "debugMode");
        check!(debounce_level, "debounceLevel");
        check!(rt_smart, "rtsmart");
        check!(battery_percent, "rf_battery");
        check!(charging, "isCharging");
        check!(roller_type, "roller_type");
        check!(work_mode, "work_mode");
        check!(sleep_time, "sleepTime");
        check!(sleep_mode, "sleepMode");
        check!(bluetooth_channel, "rf_bt_ch");
        check!(wireless_report_rate, "report_rate_2_4G");
        let mut changed = config.performance.clone();
        changed.mac_mode = 2;
        changed.report_rate = 3;
        changed.scan_rate = 4;
        changed.dead_zone = 7;
        changed.lock_win = true;
        changed.lock_alt_tab = false;
        changed.lock_alt_f4 = true;
        changed.tachyon_mode = false;
        changed.bottom_rapid_trigger = true;
        changed.travel_check = false;
        changed.debug_mode = true;
        changed.debounce_level = 5;
        changed.rt_smart = 6;
        changed.roller_type = 2;
        changed.sleep_time = 25;
        changed.sleep_mode = 2;
        changed.bluetooth_channel = 3;
        changed.wireless_report_rate = 4;
        assert_eq!(
            config.with_performance(&changed, false).unwrap(),
            bytes(&data["perfPreserve"])
        );
        assert_eq!(
            config.with_performance(&changed, true).unwrap(),
            bytes(&data["perfRate"])
        );
    }

    #[test]
    fn three_lighting_zones_and_scope_rmw_match_original_sdk() {
        let fixture = golden();
        let data = &fixture["typed"];
        let config = FunctionConfig::decode(&bytes(&data["raw"])).unwrap();
        assert_eq!(
            config.lighting.main.speed,
            data["lighting"]["speed"].as_i64().unwrap() as i16
        );
        assert_eq!(
            config.lighting.side2.speed,
            data["lighting"]["sideEffect2"]["speed"].as_i64().unwrap() as i16
        );
        assert_eq!(config.lighting.scope, LightScope::Side2);
        let mut changed = config.lighting.clone();
        changed.main.effect = 4;
        changed.main.brightness = 3;
        changed.main.speed = 1;
        changed.main.direction = 2;
        changed.main.rainbow = true;
        changed.main.color = [0x12, 0x34, 0x56];
        changed.side.effect = 5;
        changed.side.brightness = 2;
        changed.side.speed = 4;
        changed.side.rainbow = false;
        changed.side.color = [0xaa, 0xbb, 0xcc];
        changed.side2.effect = 3;
        changed.side2.brightness = 1;
        changed.side2.speed = 2;
        changed.side2.rainbow = true;
        changed.side2.color = [0x98, 0x76, 0x54];
        changed.scope = LightScope::Side;
        changed.calibration_color = [1, 2, 3];
        assert_eq!(
            config.with_lighting(&changed).unwrap(),
            bytes(&data["lightBytes"])
        );
        changed.main.speed = 5;
        assert!(config.with_lighting(&changed).is_err());
    }

    #[test]
    fn trigger_bitfields_and_roundtrip_match_original_sdk() {
        let fixture = golden();
        let data = &fixture["typed"];
        let decoded = KeyTrigger::decode_all(&bytes(&data["triggerBytes"])).unwrap();
        for (key, source) in decoded.iter().zip(data["triggers"].as_array().unwrap()) {
            macro_rules! check {
                ($field:ident,$name:literal) => {
                    assert_eq!(key.$field as u64, source[$name].as_u64().unwrap(), $name);
                };
            }
            check!(switch_type, "switchType");
            check!(key_mode, "keyMode");
            check!(priority, "priority");
            check!(actuation, "keyActuation");
            check!(press_precision, "pressPrecision");
            check!(release_precision, "releasePrecision");
            check!(rt_press, "rtPress");
            check!(rt_release, "rtRelease");
            check!(press_deadzone, "pressDeadzone");
            check!(release_deadzone, "releaseDeadzone");
        }
        assert_eq!(
            KeyTrigger::encode_all(&decoded).unwrap(),
            bytes(&data["triggerRoundtrip"])
        );
        let mut invalid = decoded;
        invalid[0].actuation = 513;
        assert!(KeyTrigger::encode_all(&invalid).is_err());
    }
    #[test]
    fn invalid_typed_custom_area_remains_lossless_in_raw_backup() {
        let mut raw = RawKeyboardProfile {
            index: 3,
            functions: vec![0xa2; 64],
            layers: vec![vec![0xa2; 384]; 4],
            triggers: vec![0xa2; 1024],
            colors: vec![0xa2; 384],
            dks: vec![0xa2; 768],
            mt: vec![0xa2; 256],
            tgl: vec![0xa2; 128],
            custom: vec![0xa2; 1024],
        };
        raw.custom[280 + 3] = 255;
        raw.custom[280 + 4] = 39;
        raw.custom[280 + 44] = 39;
        assert!(raw.decode().is_err());
        let serialized = serde_json::to_vec(&raw).unwrap();
        let restored: RawKeyboardProfile = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(restored, raw);
        assert_eq!(restored.custom[280 + 44], 39);
        assert!(restored.functions.iter().all(|byte| *byte == 0xa2));
    }
}
