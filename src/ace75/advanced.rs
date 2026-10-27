use super::model::{KEY_COUNT, KeyAssignment, LAYER_COUNT};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const ADVANCED_SLOT_COUNT: usize = 32;
pub const DKS_REGION_SIZE: usize = 768;
pub const MT_REGION_SIZE: usize = 256;
pub const TGL_REGION_SIZE: usize = 128;
pub const CUSTOM_AREA_SIZE: usize = 1024;
pub const CUSTOM_PARAMS_OFFSET: usize = 280;
pub const CUSTOM_PARAMS_SIZE: usize = 56;
pub const PROFILE_NAMES_OFFSET: usize = 336;
pub const PROFILE_NAMES_SIZE: usize = 280;
pub const FEATURE_METADATA_OFFSET: usize = 616;
pub const FEATURE_METADATA_SIZE: usize = 56;
pub const MACRO_COUNT: usize = 16;
pub const MIN_MACRO_DELAY: u16 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DksKey {
    pub points: [u8; 4],
    pub actions: [KeyAssignment; 4],
    pub masks: [[u8; 4]; 4],
    pub reserved_masks: [u16; 4],
}

impl DksKey {
    pub fn ui_phases(&self, action_index: usize) -> Result<[u8; 4]> {
        ensure!(action_index < 4, "ACE75: DKS action вне диапазона 0..3");
        let masks = self.masks[action_index];
        let highest = |mask: u8| {
            (1..=3)
                .rev()
                .find(|bit| mask & (1 << (bit - 1)) != 0)
                .unwrap_or(0)
        };
        let mut phases = [
            highest(masks[0]),
            highest(masks[1]),
            highest(masks[2]),
            masks[3] & 1,
        ];
        if phases[0] == 3 {
            if phases[1] == 3 {
                phases[0] = 4;
                phases[1] = 0;
                phases[2] = 0;
            } else {
                phases[1] = 0;
            }
        } else if phases[1] == 3 {
            phases[2] = 0;
        }
        Ok(phases)
    }

    pub fn set_ui_phases(&mut self, action_index: usize, phases: [u8; 4]) -> Result<()> {
        ensure!(
            action_index < 4
                && phases[0] <= 4
                && phases[1] <= 3
                && phases[2] <= 2
                && phases[3] <= 1,
            "ACE75: неверные фазы DKS"
        );
        if self.ui_phases(action_index)? == phases {
            return Ok(());
        }
        let [mut down_start, mut down_end, mut up_start, up_end] = phases;
        if phases[0] == 4 {
            down_start = 3;
            down_end = 3;
            up_start = 2;
        }
        if phases[0] == 3 {
            down_end = 2;
        }
        if phases[1] == 3 {
            up_start = 2;
        }
        let thermometer =
            |value: u8| (value > 0) as u8 | ((value > 1) as u8) << 1 | ((value > 2) as u8) << 2;
        self.masks[action_index] = [
            thermometer(down_start),
            thermometer(down_end),
            thermometer(up_start),
            thermometer(up_end),
        ];
        Ok(())
    }

    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(raw.len() == 24, "ACE75: DKS должен содержать 24 байта");
        let empty = KeyAssignment {
            kind: 0,
            code1: 0,
            code2: 0,
        };
        let mut result = Self {
            points: raw[..4].try_into().unwrap(),
            actions: [empty; 4],
            masks: [[0; 4]; 4],
            reserved_masks: [0; 4],
        };
        for index in 0..4 {
            let start = 4 + index * 5;
            result.actions[index] = assignment(&raw[start..start + 3]);
            let word = u16::from_le_bytes([raw[start + 3], raw[start + 4]]);
            result.masks[index] = [
                (word & 7) as u8,
                ((word >> 3) & 7) as u8,
                ((word >> 6) & 7) as u8,
                ((word >> 9) & 1) as u8,
            ];
            result.reserved_masks[index] = word & !0x03ff;
        }
        Ok(result)
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut raw = self.points.to_vec();
        for index in 0..4 {
            let masks = self.masks[index];
            ensure!(
                masks[..3].iter().all(|v| *v <= 7) && masks[3] <= 1,
                "ACE75: неверная маска переходов DKS"
            );
            ensure!(
                self.reserved_masks[index] & 0x03ff == 0,
                "ACE75: резервные биты DKS перекрывают действия"
            );
            let word = self.reserved_masks[index]
                | masks[0] as u16
                | (masks[1] as u16) << 3
                | (masks[2] as u16) << 6
                | (masks[3] as u16) << 9;
            raw.extend(key_bytes(self.actions[index]));
            raw.extend(word.to_le_bytes());
        }
        Ok(raw)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MtKey {
    pub click: KeyAssignment,
    pub hold: KeyAssignment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvancedState {
    pub dks: Vec<DksKey>,
    pub mt: Vec<MtKey>,
    pub tgl: Vec<KeyAssignment>,
    pub mt_reserved: Vec<u8>,
    pub tgl_reserved: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdvancedRegions {
    pub dks: Vec<u8>,
    pub mt: Vec<u8>,
    pub tgl: Vec<u8>,
}

impl AdvancedState {
    pub fn decode(dks: &[u8], mt: &[u8], tgl: &[u8]) -> Result<Self> {
        ensure!(
            dks.len() == DKS_REGION_SIZE
                && mt.len() == MT_REGION_SIZE
                && tgl.len() == TGL_REGION_SIZE,
            "ACE75: неверные размеры областей advanced keys"
        );
        Ok(Self {
            dks: dks
                .chunks_exact(24)
                .map(DksKey::decode)
                .collect::<Result<_>>()?,
            mt: mt[..192]
                .chunks_exact(6)
                .map(|raw| MtKey {
                    click: assignment(&raw[..3]),
                    hold: assignment(&raw[3..]),
                })
                .collect(),
            tgl: tgl[..96].chunks_exact(3).map(assignment).collect(),
            mt_reserved: mt[192..].to_vec(),
            tgl_reserved: tgl[96..].to_vec(),
        })
    }

    pub fn encode(&self) -> Result<AdvancedRegions> {
        ensure!(
            self.dks.len() == ADVANCED_SLOT_COUNT
                && self.mt.len() == ADVANCED_SLOT_COUNT
                && self.tgl.len() == ADVANCED_SLOT_COUNT
                && self.mt_reserved.len() == 64
                && self.tgl_reserved.len() == 32,
            "ACE75: нужны все 32 записи advanced keys и резервные байты"
        );
        let dks = self
            .dks
            .iter()
            .map(DksKey::encode)
            .collect::<Result<Vec<_>>>()?
            .concat();
        let mut mt: Vec<_> = self
            .mt
            .iter()
            .flat_map(|key| [key_bytes(key.click), key_bytes(key.hold)].concat())
            .collect();
        mt.extend(&self.mt_reserved);
        let mut tgl = KeyAssignment::encode_all(&self.tgl);
        tgl.extend(&self.tgl_reserved);
        Ok(AdvancedRegions { dks, mt, tgl })
    }

    pub fn validate_references(&self, layers: &[Vec<KeyAssignment>]) -> Result<()> {
        self.encode()?;
        ensure!(
            layers.len() == LAYER_COUNT && layers.iter().all(|keys| keys.len() == KEY_COUNT),
            "ACE75: нужны четыре слоя по 128 позиций"
        );
        for keys in layers {
            for (index, key) in keys.iter().enumerate() {
                if let Some(binding) = AdvancedBinding::decode(*key)? {
                    if let Some((kind, partner)) = binding.pair() {
                        ensure!(
                            partner as usize != index,
                            "ACE75: advanced пара ссылается сама на себя"
                        );
                        let other = keys[partner as usize];
                        ensure!(
                            other.kind == kind
                                && other.code2 as usize == index
                                && other.code1 < ADVANCED_SLOT_COUNT as u8
                                && other.code1 != key.code1,
                            "ACE75: разорванная ссылка advanced пары"
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdvancedBinding {
    Dks { slot: u8, reserved: u8 },
    Tgl { slot: u8, reserved: u8 },
    Mt { slot: u8, delay_ms: u16 },
    Rs { slot: u8, partner: u8 },
    Socd { slot: u8, partner: u8 },
    Lt { slot: u8, partner: u8 },
}

impl AdvancedBinding {
    pub fn decode(key: KeyAssignment) -> Result<Option<Self>> {
        if !(144..=149).contains(&key.kind) {
            return Ok(None);
        }
        ensure!(
            key.code1 < ADVANCED_SLOT_COUNT as u8,
            "ACE75: advanced slot должен быть меньше 32"
        );
        if key.kind >= 147 {
            ensure!(
                key.code2 < KEY_COUNT as u8,
                "ACE75: advanced partner должен быть меньше 128"
            );
        }
        Ok(Some(match key.kind {
            144 => Self::Dks {
                slot: key.code1,
                reserved: key.code2,
            },
            145 => Self::Tgl {
                slot: key.code1,
                reserved: key.code2,
            },
            146 => Self::Mt {
                slot: key.code1,
                delay_ms: key.code2 as u16 * 10,
            },
            147 => Self::Rs {
                slot: key.code1,
                partner: key.code2,
            },
            148 => Self::Socd {
                slot: key.code1,
                partner: key.code2,
            },
            149 => Self::Lt {
                slot: key.code1,
                partner: key.code2,
            },
            _ => unreachable!(),
        }))
    }

    pub fn encode(&self) -> Result<KeyAssignment> {
        let (kind, slot, code2) = match *self {
            Self::Dks { slot, reserved } => (144, slot, reserved),
            Self::Tgl { slot, reserved } => (145, slot, reserved),
            Self::Mt { slot, delay_ms } => {
                ensure!(
                    delay_ms <= 2550 && delay_ms % 10 == 0,
                    "ACE75: MT задержка должна быть кратна 10 мс и не больше 2550 мс"
                );
                (146, slot, (delay_ms / 10) as u8)
            }
            Self::Rs { slot, partner } => (147, slot, partner),
            Self::Socd { slot, partner } => (148, slot, partner),
            Self::Lt { slot, partner } => (149, slot, partner),
        };
        let result = KeyAssignment {
            kind,
            code1: slot,
            code2,
        };
        Self::decode(result)?;
        Ok(result)
    }

    fn pair(&self) -> Option<(u8, u8)> {
        match *self {
            Self::Rs { partner, .. } => Some((147, partner)),
            Self::Socd { partner, .. } => Some((148, partner)),
            Self::Lt { partner, .. } => Some((149, partner)),
            _ => None,
        }
    }
}

pub fn paired_assignments(
    kind: u8,
    indices: [u8; 2],
    slots: [u8; 2],
) -> Result<[KeyAssignment; 2]> {
    ensure!(
        (147..=149).contains(&kind) && indices[0] != indices[1] && slots[0] != slots[1],
        "ACE75: нужна пара разных клавиш и двух разных MT slots"
    );
    let result = [
        KeyAssignment {
            kind,
            code1: slots[0],
            code2: indices[1],
        },
        KeyAssignment {
            kind,
            code1: slots[1],
            code2: indices[0],
        },
    ];
    ensure!(
        indices.iter().all(|index| (*index as usize) < KEY_COUNT),
        "ACE75: клавиша advanced пары вне матрицы"
    );
    for key in result {
        AdvancedBinding::decode(key)?;
    }
    Ok(result)
}

pub fn paired_mt_records(actions: [KeyAssignment; 2]) -> [MtKey; 2] {
    [
        MtKey {
            click: actions[0],
            hold: actions[1],
        },
        MtKey {
            click: actions[1],
            hold: actions[0],
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomParams {
    pub press_precision: u8,
    pub release_precision: u8,
    pub rt_smart_cache: u8,
    pub combo_indices: Vec<Vec<u8>>,
    pub raw: Vec<u8>,
}

impl CustomParams {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(
            raw.len() == CUSTOM_PARAMS_SIZE,
            "ACE75: custom params должны содержать 56 байт"
        );
        let mut combo_indices = vec![vec![]; LAYER_COUNT];
        let mut cursor = if raw[3] == 255 { 4 } else { 3 };
        let count = if raw[3] == 255 { LAYER_COUNT } else { 1 };
        for indices in combo_indices.iter_mut().take(count) {
            if cursor >= raw.len() || raw[cursor] >= 40 {
                break;
            }
            let length = raw[cursor] as usize;
            ensure!(
                cursor + 1 + length <= raw.len(),
                "ACE75: обрезанный список combo keys"
            );
            *indices = raw[cursor + 1..cursor + 1 + length].to_vec();
            cursor += length + 1;
        }
        Ok(Self {
            press_precision: raw[0],
            release_precision: raw[1],
            rt_smart_cache: raw[2],
            combo_indices,
            raw: raw.to_vec(),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.raw.len() == CUSTOM_PARAMS_SIZE && self.combo_indices.len() == LAYER_COUNT,
            "ACE75: неверная структура custom params"
        );
        let original = Self::decode(&self.raw)?;
        if &original == self {
            return Ok(self.raw.clone());
        }
        ensure!(
            (self.press_precision < 3 || self.press_precision == original.press_precision)
                && (self.release_precision < 3
                    || self.release_precision == original.release_precision),
            "ACE75: неподдерживаемая точность RT"
        );
        for indices in &self.combo_indices {
            ensure!(
                indices.len() < 40 && indices.iter().all(|index| (*index as usize) < KEY_COUNT),
                "ACE75: неверные индексы combo keys"
            );
        }
        let mut prefix = vec![
            self.press_precision,
            self.release_precision,
            self.rt_smart_cache,
            255,
        ];
        for indices in &self.combo_indices {
            prefix.push(indices.len() as u8);
            prefix.extend(indices);
        }
        ensure!(
            prefix.len() <= CUSTOM_PARAMS_SIZE,
            "ACE75: combo keys превышают custom params"
        );
        let mut result = self.raw.clone();
        result[..prefix.len()].copy_from_slice(&prefix);
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileNames {
    pub names: Vec<String>,
    pub raw: Vec<u8>,
}

impl ProfileNames {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(
            raw.len() == PROFILE_NAMES_SIZE,
            "ACE75: неверный размер имён профилей"
        );
        let value = decode_custom_json(raw)?.unwrap_or_else(|| Value::Array(vec![]));
        let names: Vec<String> =
            serde_json::from_value(value).context("ACE75: неверные имена профилей")?;
        ensure!(names.len() <= 4, "ACE75: имён профилей больше четырёх");
        Ok(Self {
            names,
            raw: raw.to_vec(),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.names.len() <= 4 && self.raw.len() == PROFILE_NAMES_SIZE,
            "ACE75: неверный список имён профилей"
        );
        if Self::decode(&self.raw)?.names == self.names {
            return Ok(self.raw.clone());
        }
        encode_custom_json(&self.raw, &serde_json::to_value(&self.names)?)
    }
}

pub fn decode_custom_json(raw: &[u8]) -> Result<Option<Value>> {
    let length = raw
        .iter()
        .position(|byte| *byte == 255)
        .unwrap_or(raw.len());
    let data = &raw[..length];
    if data.is_empty() || data.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    Ok(Some(
        serde_json::from_slice(data).context("ACE75: неверный JSON в custom area")?,
    ))
}

pub fn encode_custom_json(current: &[u8], value: &Value) -> Result<Vec<u8>> {
    if decode_custom_json(current)?.as_ref() == Some(value) {
        return Ok(current.to_vec());
    }
    let data = serde_json::to_vec(value)?;
    ensure!(
        data.len() < current.len(),
        "ACE75: JSON превышает размер custom area"
    );
    let mut result = current.to_vec();
    result[..data.len()].copy_from_slice(&data);
    result[data.len()] = 255;
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeatureMetadata {
    pub macro_updated_at: u64,
    pub profile_name_updated_at: u64,
    pub browser_id: String,
    pub browser_id_expired_at: u64,
    pub extra: Vec<Value>,
    pub raw: Vec<u8>,
}

impl FeatureMetadata {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(
            raw.len() == FEATURE_METADATA_SIZE,
            "ACE75: неверный размер sync metadata"
        );
        let value = decode_custom_json(raw)?.unwrap_or_else(|| serde_json::json!([0, 0, ""]));
        let values = value
            .as_array()
            .context("ACE75: sync metadata должны быть массивом")?;
        let stamp = |index| -> Result<u64> {
            match values.get(index) {
                None | Some(Value::Null) => Ok(0),
                Some(v) => v
                    .as_u64()
                    .context("ACE75: неверная отметка времени sync metadata"),
            }
        };
        let browser = match values.get(2) {
            None | Some(Value::Null) => "",
            Some(v) => v.as_str().context("ACE75: неверный browser ID")?,
        };
        let (browser_id, expires) = browser.split_once('_').unwrap_or((browser, ""));
        let browser_id_expired_at = if expires.is_empty() {
            0
        } else {
            u64::from_str_radix(expires, 36).context("ACE75: неверное время browser lease")?
        };
        Ok(Self {
            macro_updated_at: stamp(0)?,
            profile_name_updated_at: stamp(1)?,
            browser_id: browser_id.into(),
            browser_id_expired_at,
            extra: values.iter().skip(3).cloned().collect(),
            raw: raw.to_vec(),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.raw.len() == FEATURE_METADATA_SIZE && !self.browser_id.contains('_'),
            "ACE75: неверные sync metadata"
        );
        if Self::decode(&self.raw)? == *self {
            return Ok(self.raw.clone());
        }
        let mut values = vec![
            Value::from(self.macro_updated_at),
            Value::from(self.profile_name_updated_at),
            Value::from(format!(
                "{}_{}",
                self.browser_id,
                base36(self.browser_id_expired_at)
            )),
        ];
        values.extend(self.extra.clone());
        encode_custom_json(&self.raw, &Value::Array(values))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct MacroLayout {
    pub size: usize,
    pub shared: bool,
}

impl MacroLayout {
    pub fn for_base(supports_multiple: bool, supports_order: bool) -> Self {
        if supports_multiple && supports_order {
            Self {
                size: 8192,
                shared: true,
            }
        } else {
            Self {
                size: 2048,
                shared: false,
            }
        }
    }

    pub fn offset(&self, profile: u8, max_profiles: u8) -> Result<usize> {
        ensure!(
            (1..=4).contains(&max_profiles) && profile < max_profiles,
            "ACE75: профиль макросов вне доступного диапазона"
        );
        ensure!(
            [2048, 8192].contains(&self.size),
            "ACE75: неверный размер области макросов"
        );
        Ok(if self.shared {
            0
        } else {
            profile as usize * self.size
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MacroKeyAction {
    Keydown,
    Keyup,
    Mousedown,
    Mouseup,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MacroAction {
    Action { action: MacroKeyAction, code: u16 },
    Time { delay: u16 },
    Raw { bytes: [u8; 4] },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MacroDefinition {
    pub macro_index: u8,
    pub name: String,
    #[serde(rename = "type")]
    pub mode: u8,
    pub default_delay: u16,
    pub enable_default_delay: bool,
    pub macro_actions: Vec<MacroAction>,
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MacroBank {
    pub macros: Vec<MacroDefinition>,
    pub raw: Vec<u8>,
}

impl MacroBank {
    pub fn decode(raw: &[u8]) -> Result<Self> {
        ensure!(
            raw.len() >= 68 && raw.len() <= 8192,
            "ACE75: неверный размер банка макросов"
        );
        let initialized =
            !raw[..32].iter().all(|b| *b == 0) && !raw[..32].iter().all(|b| *b == 255);
        let types_present = raw[32..34] == [129, 126];
        let mut macros = Vec::with_capacity(MACRO_COUNT);
        for index in 0..MACRO_COUNT {
            let actions = if initialized {
                let offset = u16::from_le_bytes([raw[index * 2], raw[index * 2 + 1]]) as usize;
                ensure!(
                    offset >= 64 && offset % 4 == 0 && offset < raw.len(),
                    "ACE75: неверный указатель макроса {}",
                    index + 1
                );
                decode_macro_actions(&raw[offset..])?
            } else {
                vec![]
            };
            macros.push(MacroDefinition {
                macro_index: index as u8,
                name: String::new(),
                mode: if types_present { raw[34 + index] } else { 0 },
                default_delay: 50,
                enable_default_delay: true,
                macro_actions: actions,
                extra: Map::new(),
            });
        }
        Ok(Self {
            macros,
            raw: raw.to_vec(),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure!(
            self.macros.len() == MACRO_COUNT,
            "ACE75: нужны все 16 макросов"
        );
        let original = Self::decode(&self.raw)?;
        for (index, entry) in self.macros.iter().enumerate() {
            ensure!(
                entry.macro_index as usize == index,
                "ACE75: нарушен порядок индексов макросов"
            );
        }
        if self
            .macros
            .iter()
            .zip(&original.macros)
            .all(|(a, b)| a.mode == b.mode && a.macro_actions == b.macro_actions)
        {
            return Ok(self.raw.clone());
        }
        let mut result = self.raw.clone();
        result[32..34].copy_from_slice(&[129, 126]);
        let mut streams = vec![(vec![], 64usize)];
        let mut content = vec![0, 0, 128, 0];
        for (index, entry) in self.macros.iter().enumerate() {
            let bytes = encode_macro_actions(&entry.macro_actions)?;
            let offset = if let Some((_, offset)) = streams
                .iter()
                .find(|(actions, _)| actions == &entry.macro_actions)
            {
                *offset
            } else {
                let offset = 64 + content.len();
                streams.push((entry.macro_actions.clone(), offset));
                content.extend(bytes);
                offset
            };
            result[index * 2..index * 2 + 2].copy_from_slice(&(offset as u16).to_le_bytes());
            result[34 + index] = entry.mode;
        }
        ensure!(
            64 + content.len() <= result.len(),
            "ACE75: макросы превышают вместимость банка"
        );
        result[64..64 + content.len()].copy_from_slice(&content);
        Ok(result)
    }

    pub fn local_metadata(&self) -> Value {
        Value::Array(
            self.macros
                .iter()
                .map(|entry| {
                    serde_json::json!([
                        entry.name,
                        entry.mode,
                        entry.default_delay,
                        entry.enable_default_delay
                    ])
                })
                .collect(),
        )
    }

    pub fn apply_local_metadata(&mut self, metadata: &Value) -> Result<()> {
        let values = metadata
            .as_array()
            .context("ACE75: неверные локальные метаданные макросов")?;
        ensure!(
            values.len() <= MACRO_COUNT,
            "ACE75: локальных макросов больше 16"
        );
        let mut parsed = Vec::with_capacity(values.len());
        for value in values {
            let (name, mode, delay, enabled): (String, u8, u16, bool) =
                serde_json::from_value(value.clone())?;
            ensure!(
                delay >= MIN_MACRO_DELAY,
                "ACE75: задержка макроса должна быть не меньше 5 мс"
            );
            parsed.push((name, mode, delay, enabled));
        }
        for (entry, (name, mode, delay, enabled)) in self.macros.iter_mut().zip(parsed) {
            entry.name = name;
            entry.mode = mode;
            entry.default_delay = delay;
            entry.enable_default_delay = enabled;
        }
        Ok(())
    }
}

pub fn decode_macro_actions(raw: &[u8]) -> Result<Vec<MacroAction>> {
    let mut result = vec![];
    for bytes in raw.chunks_exact(4) {
        let word: [u8; 4] = bytes.try_into().unwrap();
        let delay = u16::from_le_bytes([word[0], word[1]]);
        let kind = word[2] & 63;
        let down = word[2] & 64 != 0;
        let known = match kind {
            1 if word[3].is_power_of_two() => Some((
                if down {
                    MacroKeyAction::Keydown
                } else {
                    MacroKeyAction::Keyup
                },
                224 + word[3].trailing_zeros() as u16,
            )),
            2 => Some((
                if down {
                    MacroKeyAction::Keydown
                } else {
                    MacroKeyAction::Keyup
                },
                word[3] as u16,
            )),
            3 => Some((
                if down {
                    MacroKeyAction::Mousedown
                } else {
                    MacroKeyAction::Mouseup
                },
                word[3] as u16,
            )),
            _ => None,
        };
        if let Some((action, code)) = known {
            if code != 0 {
                result.push(MacroAction::Action { action, code });
            }
            if delay >= MIN_MACRO_DELAY {
                result.push(MacroAction::Time { delay });
            }
        } else if word != [0, 0, 128, 0] {
            result.push(MacroAction::Raw { bytes: word });
        }
        if word[2] & 128 != 0 {
            return Ok(result);
        }
    }
    bail!("ACE75: макрос не содержит завершающего маркера")
}

pub fn encode_macro_actions(actions: &[MacroAction]) -> Result<Vec<u8>> {
    ensure!(
        actions.len() <= 4096,
        "ACE75: слишком много действий макроса"
    );
    if actions.is_empty() {
        return Ok(vec![0, 0, 128, 0]);
    }
    let mut result: Vec<[u8; 4]> = vec![];
    let mut index = 0;
    while index < actions.len() {
        let mut delay = MIN_MACRO_DELAY - 1;
        let mut word = match &actions[index] {
            MacroAction::Raw { bytes } => *bytes,
            MacroAction::Time { delay: value } => {
                ensure!(
                    *value >= MIN_MACRO_DELAY,
                    "ACE75: задержка макроса должна быть не меньше 5 мс"
                );
                delay = *value;
                [0, 0, 2, 0]
            }
            MacroAction::Action { action, code } => {
                let mouse = matches!(action, MacroKeyAction::Mousedown | MacroKeyAction::Mouseup);
                ensure!(
                    *code > 0 && *code <= if mouse { 255 } else { 231 },
                    "ACE75: неверный код действия макроса"
                );
                let (kind, value) = if !mouse && *code >= 224 {
                    (1, 1u8 << (*code - 224))
                } else {
                    (if mouse { 3 } else { 2 }, *code as u8)
                };
                let flags = if matches!(action, MacroKeyAction::Keydown | MacroKeyAction::Mousedown)
                {
                    64
                } else {
                    0
                };
                if let Some(MacroAction::Time { delay: value }) = actions.get(index + 1) {
                    ensure!(
                        *value >= MIN_MACRO_DELAY,
                        "ACE75: задержка макроса должна быть не меньше 5 мс"
                    );
                    delay = *value;
                    index += 1;
                }
                [0, 0, kind | flags, value]
            }
        };
        if !matches!(&actions[index], MacroAction::Raw { .. }) {
            word[..2].copy_from_slice(&delay.to_le_bytes());
        }
        word[2] &= 127;
        result.push(word);
        index += 1;
    }
    result.last_mut().unwrap()[2] |= 128;
    Ok(result.concat())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HubProfileDocument {
    pub content: Value,
}

impl HubProfileDocument {
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "ACE75: файл профиля слишком велик"
        );
        let content: Value =
            serde_json::from_slice(bytes).context("ACE75: неверный JSON профиля")?;
        let object = content
            .as_object()
            .context("ACE75: профиль должен быть объектом JSON")?;
        ensure!(
            object
                .get("dataScope")
                .is_none_or(|v| v == "KeyboardProfile"),
            "ACE75: это не профиль клавиатуры"
        );
        ensure!(
            object.contains_key("data")
                || object.contains_key("userKeys")
                || object.contains_key("keyByteList")
                || object.contains_key("triggerTravel")
                || object.contains_key("allDefaultKeys")
                || object.contains_key("performance")
                || object.contains_key("light")
                || object.contains_key("advancedKeys")
                || object.contains_key("customParam")
                || object.contains_key("funcConfigStruct"),
            "ACE75: профиль не содержит настроек клавиатуры"
        );
        if let Some(vendor) = object.get("vendorId") {
            ensure!(
                vendor.as_u64() == Some(0x3837),
                "ACE75: профиль другого производителя"
            );
        }
        if let Some(product) = object.get("productId") {
            ensure!(
                product.as_u64() == Some(0x303c),
                "ACE75: профиль другой модели"
            );
        }
        let data = object.get("data").unwrap_or(&content);
        ensure!(
            data.is_object(),
            "ACE75: данные профиля должны быть объектом JSON"
        );
        let legacy = object
            .get("version")
            .and_then(Value::as_u64)
            .is_none_or(|version| version < 2)
            && object.get("data").is_none()
            && ["productInfo", "configData", "colorKeys", "travelKeys"]
                .iter()
                .any(|field| object.contains_key(*field));
        if let Some(layers) = data.get("userKeys") {
            let layers = layers
                .as_array()
                .context("ACE75: неверная таблица клавиш профиля")?;
            ensure!(
                if legacy {
                    layers.len() <= LAYER_COUNT
                } else {
                    layers.len() == LAYER_COUNT
                },
                "ACE75: профиль должен содержать четыре слоя"
            );
            for layer in layers {
                if legacy && layer.is_null() {
                    continue;
                }
                if layer.get("type").and_then(Value::as_str) == Some("diff-keys") {
                    ensure!(
                        layer.get("dataV2").is_none_or(Value::is_object)
                            && layer
                                .get("data")
                                .is_none_or(|v| v.is_null() || v.is_object()),
                        "ACE75: неверные данные diff-keys"
                    );
                    continue;
                }
                let layer = layer.as_array().context("ACE75: неверный слой профиля")?;
                ensure!(
                    layer.len() == KEY_COUNT,
                    "ACE75: слой должен содержать 128 позиций"
                );
                for key in layer {
                    for field in ["type", "code1", "code2"] {
                        ensure!(
                            key.get(field)
                                .and_then(Value::as_u64)
                                .is_some_and(|v| v <= 255),
                            "ACE75: неверное поле назначения клавиши {field}"
                        );
                    }
                }
            }
        }
        Ok(Self { content })
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec_pretty(&self.content)?;
        Self::from_json(&bytes)?;
        Ok(bytes)
    }
}

fn assignment(raw: &[u8]) -> KeyAssignment {
    KeyAssignment {
        kind: raw[0],
        code1: raw[1],
        code2: raw[2],
    }
}

fn key_bytes(key: KeyAssignment) -> [u8; 3] {
    [key.kind, key.code1, key.code2]
}

fn base36(mut value: u64) -> String {
    if value == 0 {
        return "0".into();
    }
    let mut bytes = vec![];
    while value > 0 {
        let digit = (value % 36) as u8;
        bytes.push(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        });
        value /= 36;
    }
    bytes.reverse();
    String::from_utf8(bytes).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Original SDK20721 pure encoders; source SHA c16055520ed1c43483a5357f74e265e3a356b0f7d2a4298a3d90ad6cf0a127ff.
    const DKS: [u8; 24] = [
        10, 30, 29, 9, 16, 0, 4, 217, 3, 16, 1, 6, 71, 0, 48, 226, 0, 248, 2, 112, 3, 1, 11, 0,
    ];
    const BANK: [u8; 76] = [
        68, 0, 68, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0, 64, 0,
        64, 0, 64, 0, 64, 0, 129, 126, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 0, 50, 0, 66, 4, 4, 0, 130, 4,
    ];

    #[test]
    fn original_sdk_dks_vector_and_unknown_mask_bits() {
        let mut key = DksKey::decode(&DKS).unwrap();
        assert_eq!(key.points, [10, 30, 29, 9]);
        assert_eq!(
            key.masks,
            [[1, 3, 7, 1], [7, 0, 1, 0], [0, 7, 3, 1], [3, 1, 0, 0]]
        );
        assert_eq!(
            key.actions[2],
            KeyAssignment {
                kind: 48,
                code1: 226,
                code2: 0
            }
        );
        assert_eq!(key.encode().unwrap(), DKS);
        let mut unknown = DKS;
        unknown[8] |= 0xe4;
        let mut decoded = DksKey::decode(&unknown).unwrap();
        assert_eq!(decoded.encode().unwrap(), unknown);
        decoded.actions[0].code2 = 5;
        let encoded = decoded.encode().unwrap();
        assert_eq!(encoded[8], unknown[8]);
        key.masks[0][3] = 2;
        assert!(key.encode().is_err());
    }

    #[test]
    fn all_advanced_regions_preserve_reserved_bytes_and_unknown_actions() {
        let mut dks = DKS.repeat(ADVANCED_SLOT_COUNT);
        dks[4] = 0xfb;
        let mt: Vec<u8> = (0..256).map(|v| v as u8).collect();
        let tgl: Vec<u8> = (0..128).map(|v| 255 - v as u8).collect();
        let state = AdvancedState::decode(&dks, &mt, &tgl).unwrap();
        assert_eq!(state.encode().unwrap(), AdvancedRegions { dks, mt, tgl });
        assert!(AdvancedState::decode(&[], &[0; 256], &[0; 128]).is_err());
    }

    #[test]
    fn sdk_mt_tgl_and_all_six_binding_types() {
        let state = AdvancedState::decode(
            &[0; 768],
            &[16, 0, 4, 16, 1, 6].repeat(43)[..256],
            &[48, 226, 0].repeat(43)[..128],
        )
        .unwrap();
        assert_eq!(
            state.mt[0],
            MtKey {
                click: KeyAssignment {
                    kind: 16,
                    code1: 0,
                    code2: 4
                },
                hold: KeyAssignment {
                    kind: 16,
                    code1: 1,
                    code2: 6
                }
            }
        );
        assert_eq!(
            state.tgl[0],
            KeyAssignment {
                kind: 48,
                code1: 226,
                code2: 0
            }
        );
        for kind in 144..=149 {
            let key = KeyAssignment {
                kind,
                code1: 31,
                code2: 127,
            };
            assert_eq!(
                AdvancedBinding::decode(key)
                    .unwrap()
                    .unwrap()
                    .encode()
                    .unwrap(),
                key
            );
        }
        assert!(
            AdvancedBinding::Mt {
                slot: 0,
                delay_ms: 151
            }
            .encode()
            .is_err()
        );
        assert!(
            AdvancedBinding::Dks {
                slot: 32,
                reserved: 0
            }
            .encode()
            .is_err()
        );
        assert!(paired_assignments(148, [128, 1], [0, 1]).is_err());
    }

    #[test]
    fn paired_rs_socd_lt_require_reciprocal_bounded_indices() {
        let state = AdvancedState::decode(&[0; 768], &[0; 256], &[0; 128]).unwrap();
        let empty = KeyAssignment {
            kind: 0,
            code1: 0,
            code2: 0,
        };
        for kind in 147..=149 {
            let mut layers = vec![vec![empty; 128]; 4];
            let pair = paired_assignments(kind, [88, 71], [2, 3]).unwrap();
            layers[0][88] = pair[0];
            layers[0][71] = pair[1];
            state.validate_references(&layers).unwrap();
            layers[0][71].code2 = 70;
            assert!(state.validate_references(&layers).is_err());
        }
    }

    #[test]
    fn original_sdk_custom_params_and_legacy_format() {
        let prefix = [2, 1, 17, 255, 2, 1, 88, 1, 22, 0, 2, 92, 93];
        let mut raw = vec![0xa6; 56];
        raw[..prefix.len()].copy_from_slice(&prefix);
        let mut value = CustomParams::decode(&raw).unwrap();
        assert_eq!(
            value.combo_indices,
            vec![vec![1, 88], vec![22], vec![], vec![92, 93]]
        );
        assert_eq!(value.encode().unwrap(), raw);
        value.rt_smart_cache = 18;
        let result = value.encode().unwrap();
        assert_eq!(&result[3..], &raw[3..]);
        let mut legacy = vec![0; 56];
        legacy[3..6].copy_from_slice(&[2, 88, 22]);
        assert_eq!(
            CustomParams::decode(&legacy).unwrap().combo_indices[0],
            [88, 22]
        );
        assert_eq!(
            CustomParams::decode(&legacy).unwrap().encode().unwrap(),
            legacy
        );
        value.combo_indices[0] = vec![128];
        assert!(value.encode().is_err());
    }

    #[test]
    fn original_sdk_macro_action_vectors() {
        let cases: &[&[u8]] = &[
            &[0, 0, 128, 0],
            &[50, 0, 66, 4, 4, 0, 130, 4],
            &[255, 255, 65, 1, 4, 0, 129, 1],
            &[5, 0, 67, 1, 4, 0, 131, 1],
            &[123, 0, 2, 0, 5, 0, 130, 0],
        ];
        for bytes in cases {
            let actions = decode_macro_actions(bytes).unwrap();
            assert_eq!(encode_macro_actions(&actions).unwrap(), *bytes);
        }
        assert_eq!(
            decode_macro_actions(cases[1]).unwrap(),
            vec![
                MacroAction::Action {
                    action: MacroKeyAction::Keydown,
                    code: 4
                },
                MacroAction::Time { delay: 50 },
                MacroAction::Action {
                    action: MacroKeyAction::Keyup,
                    code: 4
                }
            ]
        );
        assert!(encode_macro_actions(&[MacroAction::Time { delay: 4 }]).is_err());
        assert!(decode_macro_actions(&[50, 0, 66, 4]).is_err());
    }

    #[test]
    fn original_sdk_macro_bank_aliases_types_and_capacity() {
        let mut raw = vec![0; 8192];
        raw[..BANK.len()].copy_from_slice(&BANK);
        raw[55] = 0xb7;
        raw[8100] = 0x79;
        let bank = MacroBank::decode(&raw).unwrap();
        assert_eq!(bank.macros[0].macro_actions, bank.macros[1].macro_actions);
        assert_eq!(bank.macros[1].mode, 255);
        assert_eq!(bank.encode().unwrap(), raw);
        let mut empty = MacroBank::decode(&vec![0; 8192]).unwrap();
        empty.macros[0].macro_actions = bank.macros[0].macro_actions.clone();
        empty.macros[1].macro_actions = bank.macros[1].macro_actions.clone();
        empty.macros[1].mode = 255;
        assert_eq!(&empty.encode().unwrap()[..BANK.len()], &BANK);
        empty.macros[0].macro_actions = vec![MacroAction::Time { delay: 5 }; 4096];
        assert!(empty.encode().is_err());
        assert_eq!(MacroLayout::for_base(true, true).offset(3, 4).unwrap(), 0);
        assert_eq!(
            MacroLayout::for_base(true, false).offset(1, 2).unwrap(),
            2048
        );
        assert!(MacroLayout::for_base(true, false).offset(2, 2).is_err());
    }

    #[test]
    fn names_metadata_and_hub_json_keep_unknown_content() {
        let mut raw = vec![255; 280];
        let names = br#"["one","two","three",""]"#;
        raw[..names.len()].copy_from_slice(names);
        raw[279] = 0x81;
        let mut value = ProfileNames::decode(&raw).unwrap();
        assert_eq!(value.encode().unwrap(), raw);
        value.names[1] = "Изменён".into();
        let result = value.encode().unwrap();
        assert_eq!(result[279], 0x81);
        assert_eq!(ProfileNames::decode(&result).unwrap().names[1], "Изменён");
        let source = br#"{"dataScope":"KeyboardProfile","vendorId":14391,"productId":12348,"data":{"opaque":{"future":[255,1,2]}}}"#;
        let document = HubProfileDocument::from_json(source).unwrap();
        assert_eq!(
            HubProfileDocument::from_json(&document.to_json().unwrap()).unwrap(),
            document
        );
        assert!(
            HubProfileDocument::from_json(br#"{"dataScope":"LightingEffectProfile","data":{}}"#)
                .is_err()
        );
        assert!(HubProfileDocument::from_json(br#"{"productId":12317,"data":{}}"#).is_err());
    }

    #[test]
    fn sync_metadata_preserves_unknown_fields_and_macro_metadata_is_atomic() {
        let mut raw = vec![255; 56];
        let encoded = br#"[100,200,"browser_rs",{"future":1}]"#;
        raw[..encoded.len()].copy_from_slice(encoded);
        let mut value = FeatureMetadata::decode(&raw).unwrap();
        assert_eq!(value.browser_id_expired_at, 1000);
        assert_eq!(value.encode().unwrap(), raw);
        value.macro_updated_at = 123;
        let encoded = value.encode().unwrap();
        assert_eq!(
            FeatureMetadata::decode(&encoded).unwrap().extra,
            vec![serde_json::json!({"future": 1})]
        );
        let mut bank = MacroBank::decode(&vec![0; 8192]).unwrap();
        let before = bank.clone();
        assert!(
            bank.apply_local_metadata(&serde_json::json!([
                ["valid", 0, 50, true],
                ["invalid", 0, 4, true]
            ]))
            .is_err()
        );
        assert_eq!(bank, before);
        let actions = decode_macro_actions(&[51, 0, 0x8f, 177]).unwrap();
        assert_eq!(
            actions,
            [MacroAction::Raw {
                bytes: [51, 0, 0x8f, 177]
            }]
        );
        assert_eq!(encode_macro_actions(&actions).unwrap(), [51, 0, 0x8f, 177]);
    }
}
