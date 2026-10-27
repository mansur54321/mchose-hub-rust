use super::advanced::*;
use super::model::*;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

const PERFORMANCE_FIELDS: &[(&str, &str, bool)] = &[
    ("macMode", "mac_mode", false),
    ("reporteRate", "report_rate", false),
    ("tickRate", "scan_rate", false),
    ("deadZone", "dead_zone", false),
    ("lockWin", "lock_win", true),
    ("lockAltTap", "lock_alt_tab", true),
    ("lockAltF4", "lock_alt_f4", true),
    ("tachyonMode", "tachyon_mode", true),
    ("bottomrapidtriggerMode", "bottom_rapid_trigger", true),
    ("travelCheck", "travel_check", true),
    ("debugMode", "debug_mode", true),
    ("debounceLevel", "debounce_level", false),
    ("rtsmart", "rt_smart", false),
    ("roller_type", "roller_type", false),
    ("sleepTime", "sleep_time", false),
    ("sleepMode", "sleep_mode", false),
    ("rf_bt_ch", "bluetooth_channel", false),
    ("report_rate_2_4G", "wireless_report_rate", false),
];
const TRIGGER_FIELDS: &[(&str, &str, bool)] = &[
    ("switchType", "switch_type", false),
    ("keyMode", "key_mode", false),
    ("priority", "priority", false),
    ("keyActuation", "actuation", false),
    ("pressPrecision", "press_precision", false),
    ("releasePrecision", "release_precision", false),
    ("rtPress", "rt_press", false),
    ("rtRelease", "rt_release", false),
    ("pressDeadzone", "press_deadzone", false),
    ("releaseDeadzone", "release_deadzone", false),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HubProfileScope {
    pub keys: bool,
    pub triggers: bool,
    pub advanced: bool,
    pub performance: bool,
    pub lighting: bool,
    pub custom_params: bool,
    pub color_indices: Vec<usize>,
}

impl HubProfileScope {
    pub fn merge_into(
        &self,
        imported: &KeyboardProfile,
        current: &KeyboardProfile,
    ) -> Result<KeyboardProfile> {
        validate_profile_shape(imported)?;
        validate_profile_shape(current)?;
        let mut result = current.clone();
        if self.keys {
            result.layers = imported.layers.clone();
            for (layer, keys) in result.layers.iter_mut().enumerate() {
                for index in [93, 94] {
                    keys[index] = current.layers[layer][index];
                }
            }
        }
        if self.triggers {
            result.triggers = imported.triggers.clone();
            result.trigger_raw = update_trigger_records(&current.trigger_raw, &result.triggers)?;
        }
        if self.advanced {
            result.advanced = imported.advanced.clone();
            result.advanced.mt_reserved = current.advanced.mt_reserved.clone();
            result.advanced.tgl_reserved = current.advanced.tgl_reserved.clone();
            for (value, before) in result.advanced.dks.iter_mut().zip(&current.advanced.dks) {
                value.reserved_masks = before.reserved_masks;
            }
        }
        if self.custom_params {
            result.custom_params = imported.custom_params.clone();
            result.custom_params.raw = current.custom_params.raw.clone();
            let bytes = result.custom_params.encode()?;
            result.custom_raw[CUSTOM_PARAMS_OFFSET..CUSTOM_PARAMS_OFFSET + CUSTOM_PARAMS_SIZE]
                .copy_from_slice(&bytes);
        }
        if self.performance {
            result.functions = FunctionConfig::decode(
                &result
                    .functions
                    .with_performance(&imported.functions.performance, false)?,
            )?;
        }
        if self.lighting {
            result.functions = FunctionConfig::decode(
                &result
                    .functions
                    .with_lighting(&imported.functions.lighting)?,
            )?;
        }
        for &index in &self.color_indices {
            ensure!(index < KEY_COUNT, "ACE75: индекс цвета вне диапазона");
            result.colors[index] = imported.colors[index];
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedHubProfile {
    pub profile: KeyboardProfile,
    pub macros: Option<MacroBank>,
    pub document: HubProfileDocument,
    pub scope: HubProfileScope,
}

pub fn import_hub_profile(
    document: &HubProfileDocument,
    baseline: &KeyboardProfile,
) -> Result<ImportedHubProfile> {
    HubProfileDocument::from_json(&document.to_json()?)?;
    if is_legacy_profile(&document.content) {
        let converted = convert_legacy_profile(&document.content, baseline)?;
        let mut imported = import_hub_profile(&converted, baseline)?;
        imported.document = document.clone();
        return Ok(imported);
    }
    ensure!(
        baseline.index < 4
            && baseline.triggers.len() == KEY_COUNT
            && baseline.colors.len() == KEY_COUNT
            && baseline.trigger_raw.len() == KEY_COUNT * 8
            && baseline.custom_raw.len() == CUSTOM_AREA_SIZE,
        "ACE75: неверный целевой профиль"
    );
    let root = &document.content;
    let data = root.get("data").unwrap_or(root);
    validate_identity(root)?;
    if !std::ptr::eq(data, root) {
        validate_identity(data)?;
    }
    ensure!(
        data.get("userKeys").is_some()
            || data.get("allDefaultKeys").is_some()
            || data.get("triggerTravel").is_some()
            || data.get("defaultTrigger").is_some()
            || data.get("performance").is_some()
            || data.get("funcConfigStruct").is_some()
            || data.get("light").is_some()
            || data.get("advancedKeys").is_some()
            || data.get("customParam").is_some()
            || data.get("lightColors").is_some(),
        "ACE75: профиль не содержит поддерживаемых настроек"
    );
    validate_profile_shape(baseline)?;
    let mut scope = HubProfileScope::default();
    let mut profile = baseline.clone();
    if let Some(keys) = data.get("userKeys") {
        profile.layers = import_layers(keys, baseline)?;
        scope.keys = true;
    } else if let Some(keys) = data.get("allDefaultKeys") {
        let all = keys.as_array().context("ACE75: неверные default layers")?;
        ensure!(
            all.len() == 16,
            "ACE75: SDK preset должен содержать 16 слоёв"
        );
        profile.layers = import_layers(
            &Value::Array(
                all[baseline.index as usize * 4..baseline.index as usize * 4 + 4].to_vec(),
            ),
            baseline,
        )?;
        scope.keys = true;
    }
    if let Some(config) = data.get("funcConfigStruct") {
        ensure!(config.is_object(), "ACE75: неверный funcConfigStruct");
        if let Some(value) = config.get("performanceInfo") {
            let updated = import_fields(
                serde_json::to_value(&profile.functions.performance)?,
                value,
                PERFORMANCE_FIELDS,
            )?;
            let mut performance: Performance = serde_json::from_value(updated)?;
            performance.mac_mode = baseline.index * 4 + performance.mac_mode % 4;
            let raw = profile.functions.with_performance(&performance, false)?;
            scope.performance = true;
            profile.functions = FunctionConfig::decode(&raw)?;
        }
        if let Some(value) = config.get("lightConfig") {
            let light = import_lighting(value, &profile.functions.lighting)?;
            profile.functions = FunctionConfig::decode(&profile.functions.with_lighting(&light)?)?;
            scope.lighting = true;
        }
    }
    if let Some(value) = data.get("performance") {
        let mut performance: Performance = serde_json::from_value(import_fields(
            serde_json::to_value(&profile.functions.performance)?,
            value,
            PERFORMANCE_FIELDS,
        )?)?;
        performance.mac_mode = baseline.index * 4 + performance.mac_mode % 4;
        profile.functions =
            FunctionConfig::decode(&profile.functions.with_performance(&performance, false)?)?;
        scope.performance = true;
    }
    if let Some(value) = data.pointer("/light/config") {
        let light = import_lighting(value, &profile.functions.lighting)?;
        profile.functions = FunctionConfig::decode(&profile.functions.with_lighting(&light)?)?;
        scope.lighting = true;
    }
    if let Some(trigger) = data.pointer("/triggerTravel/travelKeysV2") {
        profile.triggers = import_sparse_triggers(trigger, &baseline.triggers)?;
        scope.triggers = true;
    } else if let Some(trigger) = data.pointer("/triggerTravel/travelKeys") {
        let values = trigger
            .as_array()
            .context("ACE75: неверные triggerTravel.travelKeys")?;
        ensure!(
            values.len() == KEY_COUNT,
            "ACE75: нужны trigger-настройки 128 позиций"
        );
        profile.triggers = values
            .iter()
            .zip(&baseline.triggers)
            .map(|(value, before)| import_trigger(value, before))
            .collect::<Result<_>>()?;
        scope.triggers = true;
    } else if let Some(defaults) = data.get("defaultTrigger") {
        let per_profile = defaults
            .as_array()
            .context("ACE75: неверный defaultTrigger SDK preset")?;
        let selected = per_profile
            .get(baseline.index as usize)
            .context("ACE75: отсутствует defaultTrigger целевого профиля")?;
        let default = selected.get("default").unwrap_or(selected);
        for (index, trigger) in profile.triggers.iter_mut().enumerate() {
            *trigger = import_trigger(default, trigger)?;
            if let Some(code) = data
                .pointer(&format!(
                    "/allDefaultKeys/{}/{}",
                    baseline.index as usize * 4,
                    index
                ))
                .and_then(|key| key.get("code"))
                .and_then(Value::as_u64)
            {
                if let Some(override_value) = selected.get(code.to_string()) {
                    *trigger = import_trigger(override_value, trigger)?;
                }
            }
        }
        scope.triggers = true;
    }
    profile.trigger_raw = update_trigger_records(&baseline.trigger_raw, &profile.triggers)?;
    if let Some(value) = data.get("advancedKeys") {
        import_advanced(value, &mut profile.advanced)?;
        scope.advanced = true;
    }
    if data.get("userKeys").is_some() {
        validate_defined_advanced(&profile.layers, data.get("advancedKeys"))?;
    }
    if let Some(value) = data.get("customParam") {
        ensure!(value.is_object(), "ACE75: неверные customParam");
        scope.custom_params = true;
        if let Some(v) = value.get("rtPressPrecisionMode") {
            profile.custom_params.press_precision = byte(v)?;
        }
        if let Some(v) = value.get("rtReleasePrecisionMode") {
            profile.custom_params.release_precision = byte(v)?;
        }
        if let Some(v) = value.get("rtSmartCacheValue") {
            profile.custom_params.rt_smart_cache = byte(v)?;
        }
        if let Some(v) = value.get("cbKeyIndexList") {
            let values = v.as_array().context("ACE75: неверные combo indices")?;
            profile.custom_params.combo_indices = if values.first().is_some_and(Value::is_array) {
                ensure!(values.len() == 4, "ACE75: нужны combo lists четырёх слоёв");
                serde_json::from_value(v.clone())?
            } else {
                vec![serde_json::from_value(v.clone())?, vec![], vec![], vec![]]
            };
        }
    }
    let custom = profile.custom_params.encode()?;
    ensure!(
        profile.custom_raw.len() == CUSTOM_AREA_SIZE,
        "ACE75: неверный raw custom area"
    );
    profile.custom_raw[CUSTOM_PARAMS_OFFSET..CUSTOM_PARAMS_OFFSET + CUSTOM_PARAMS_SIZE]
        .copy_from_slice(&custom);
    if let Some(value) = data
        .pointer("/light/colors")
        .or_else(|| data.get("lightColors"))
    {
        profile.colors = import_colors(value, KEY_COUNT)?;
        scope.color_indices.extend(0..KEY_COUNT);
    }
    for (field, start, count) in [("lightSideColors", 95, 28), ("lightSide2Colors", 123, 1)] {
        let canonical = match field {
            "lightSideColors" => "sideColors",
            _ => "side2Colors",
        };
        if let Some(value) = data
            .get("light")
            .and_then(|v| v.get(canonical))
            .or_else(|| data.get(field))
        {
            let values = value
                .as_array()
                .context("ACE75: неверная таблица side light colors")?;
            let colors = if values.len() == KEY_COUNT {
                import_colors(value, KEY_COUNT)?[start..start + count].to_vec()
            } else {
                import_colors(value, count)?
            };
            profile.colors[start..start + count].copy_from_slice(&colors);
            scope.color_indices.extend(start..start + count);
        }
    }
    scope.color_indices.sort_unstable();
    scope.color_indices.dedup();
    visit_assignments_mut(&mut profile, |key| {
        if key.kind == 240 && key.code1 == 255 {
            key.code2 = baseline.index * 4 + key.code2 % 4;
        }
    });
    profile.advanced.validate_references(&profile.layers)?;
    for trigger in &profile.triggers {
        trigger.validate()?;
    }
    let definitions = data.get("macros").or_else(|| root.get("macros"));
    let macros = if let Some(value) = definitions {
        let values = value
            .as_array()
            .context("ACE75: неверный список macro definitions")?;
        ensure!(values.len() <= MACRO_COUNT, "ACE75: макросов больше 16");
        let mut bank = MacroBank::decode(&vec![0; 8192])?;
        let mut used = BTreeSet::new();
        for (position, value) in values.iter().enumerate() {
            let mut entry = value
                .as_object()
                .context("ACE75: macro definition должен быть объектом")?
                .clone();
            entry.entry("macroIndex").or_insert(Value::from(position));
            entry.entry("name").or_insert(Value::from(""));
            entry.entry("type").or_insert(Value::from(0));
            entry.entry("defaultDelay").or_insert(Value::from(50));
            entry
                .entry("enableDefaultDelay")
                .or_insert(Value::from(true));
            let entry: MacroDefinition = serde_json::from_value(Value::Object(entry))?;
            ensure!(
                (entry.macro_index as usize) < MACRO_COUNT && used.insert(entry.macro_index),
                "ACE75: неверный или повторный macro index"
            );
            let slot = entry.macro_index as usize;
            bank.macros[slot] = entry;
        }
        bank.apply_local_metadata(&bank.local_metadata())?;
        bank.encode()?;
        Some(bank)
    } else {
        None
    };
    Ok(ImportedHubProfile {
        profile,
        macros,
        document: document.clone(),
        scope,
    })
}

pub fn export_hub_profile(
    profile: &KeyboardProfile,
    macros: Option<&MacroBank>,
    original: Option<&HubProfileDocument>,
) -> Result<HubProfileDocument> {
    validate_profile_shape(profile)?;
    profile.advanced.validate_references(&profile.layers)?;
    ensure!(
        profile.colors.len() == KEY_COUNT,
        "ACE75: нужны цвета всех 128 позиций"
    );
    let mut root = original
        .map(|value| value.content.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    if !root
        .as_object()
        .context("ACE75: неверный исходный Hub envelope")?
        .contains_key("data")
    {
        let data = root.clone();
        root.as_object_mut().unwrap().insert("data".into(), data);
    }
    let root_object = root
        .as_object_mut()
        .context("ACE75: неверный исходный Hub envelope")?;
    root_object.insert("dataScope".into(), Value::from("KeyboardProfile"));
    root_object.insert("type".into(), Value::from("keyboard"));
    root_object.insert("version".into(), Value::from(3));
    root_object.insert("productName".into(), Value::from("Ace 75"));
    root_object.insert(
        "identity".into(),
        serde_json::json!({"vendorId":0x3837,"productId":0x303c,"productName":"Ace 75"}),
    );
    root_object.insert("vendorId".into(), Value::from(0x3837));
    root_object.insert("productId".into(), Value::from(0x303c));
    let mut extra = root_object
        .get("extra")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    extra
        .as_object_mut()
        .context("ACE75: неверный extra")?
        .insert(
            "identity".into(),
            serde_json::json!({"vendorId":0x3837,"productId":0x303c,"productName":"Ace 75"}),
        );
    root_object.insert("extra".into(), extra);
    let nested = root_object.contains_key("data");
    let _ = root_object;
    let data = if nested {
        root.get_mut("data").unwrap()
    } else {
        &mut root
    };
    let object = data
        .as_object_mut()
        .context("ACE75: неверные данные Hub envelope")?;
    if object.get("name").and_then(Value::as_str).is_none() {
        object.insert("name".into(), Value::from("Ace 75 8K"));
    }
    let layers = profile
        .layers
        .iter()
        .enumerate()
        .map(|(layer, keys)| {
            Value::Array(
                keys.iter()
                    .enumerate()
                    .map(|(index, key)| {
                        let previous = object
                            .get("userKeys")
                            .and_then(|v| v.get(layer))
                            .and_then(|v| v.get(index));
                        export_assignment(*key, previous)
                    })
                    .collect(),
            )
        })
        .collect();
    object.insert("userKeys".into(), Value::Array(layers));
    let mut config = object
        .get("funcConfigStruct")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let config_object = config
        .as_object_mut()
        .context("ACE75: неверный funcConfigStruct")?;
    let mut performance = config_object
        .get("performanceInfo")
        .or_else(|| object.get("performance"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    export_fields(
        &mut performance,
        &serde_json::to_value(&profile.functions.performance)?,
        PERFORMANCE_FIELDS,
    )?;
    config_object.insert("performanceInfo".into(), performance);
    let light = export_lighting(
        &profile.functions.lighting,
        config_object
            .get("lightConfig")
            .or_else(|| object.get("light").and_then(|v| v.get("config"))),
    )?;
    config_object.insert("lightConfig".into(), light);
    object.insert("performance".into(), config["performanceInfo"].clone());
    let mut light = object
        .get("light")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let light_object = light.as_object_mut().context("ACE75: неверный light")?;
    light_object.insert("config".into(), config["lightConfig"].clone());
    light_object.insert(
        "colors".into(),
        Value::Array(
            profile
                .colors
                .iter()
                .map(|rgb| Value::from(hex_color(*rgb)))
                .collect(),
        ),
    );
    light_object.insert(
        "sideColors".into(),
        Value::Array(
            profile.colors[95..123]
                .iter()
                .map(|rgb| Value::from(hex_color(*rgb)))
                .collect(),
        ),
    );
    light_object.insert(
        "side2Colors".into(),
        Value::Array(
            profile.colors[123..124]
                .iter()
                .map(|rgb| Value::from(hex_color(*rgb)))
                .collect(),
        ),
    );
    object.insert("light".into(), light);
    if object.contains_key("funcConfigStruct") {
        object.insert("funcConfigStruct".into(), config);
    }
    let mut travel = object
        .get("triggerTravel")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let travel_object = travel
        .as_object_mut()
        .context("ACE75: неверный triggerTravel")?;
    let triggers = profile
        .triggers
        .iter()
        .enumerate()
        .map(|(index, trigger)| {
            let mut value = travel_object
                .get("travelKeys")
                .and_then(|v| v.get(index))
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            export_fields(&mut value, &serde_json::to_value(trigger)?, TRIGGER_FIELDS)?;
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    travel_object.remove("travelKeysV2");
    travel_object.insert("travelKeys".into(), Value::Array(triggers));
    object.insert("triggerTravel".into(), travel);
    object.insert(
        "advancedKeys".into(),
        export_advanced(&profile.advanced, object.get("advancedKeys"))?,
    );
    let mut custom = object
        .get("customParam")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let custom_object = custom
        .as_object_mut()
        .context("ACE75: неверный customParam")?;
    custom_object.insert(
        "rtPressPrecisionMode".into(),
        Value::from(profile.custom_params.press_precision),
    );
    custom_object.insert(
        "rtReleasePrecisionMode".into(),
        Value::from(profile.custom_params.release_precision),
    );
    custom_object.insert(
        "rtSmartCacheValue".into(),
        Value::from(profile.custom_params.rt_smart_cache),
    );
    custom_object.insert(
        "cbKeyIndexList".into(),
        serde_json::to_value(&profile.custom_params.combo_indices)?,
    );
    object.insert("customParam".into(), custom);
    ensure!(
        profile.colors.len() == KEY_COUNT,
        "ACE75: нужны цвета всех 128 позиций"
    );
    object.insert(
        "lightColors".into(),
        Value::Array(
            profile
                .colors
                .iter()
                .map(|rgb| Value::from(hex_color(*rgb)))
                .collect(),
        ),
    );
    object.insert(
        "lightSideColors".into(),
        Value::Array(
            profile.colors[95..123]
                .iter()
                .map(|rgb| Value::from(hex_color(*rgb)))
                .collect(),
        ),
    );
    object.insert(
        "lightSide2Colors".into(),
        Value::Array(
            profile.colors[123..124]
                .iter()
                .map(|rgb| Value::from(hex_color(*rgb)))
                .collect(),
        ),
    );
    if let Some(macros) = macros {
        macros.encode()?;
        if object.contains_key("macros") {
            object.insert("macros".into(), serde_json::to_value(&macros.macros)?);
        }
    }
    if let Some(macros) = macros {
        root.as_object_mut()
            .unwrap()
            .insert("macros".into(), serde_json::to_value(&macros.macros)?);
    }
    let document = HubProfileDocument { content: root };
    document.to_json()?;
    Ok(document)
}

fn is_legacy_profile(value: &Value) -> bool {
    value
        .get("version")
        .and_then(Value::as_u64)
        .is_none_or(|version| version < 2)
        && value.get("data").is_none()
        && ["productInfo", "configData", "colorKeys", "travelKeys"]
            .iter()
            .any(|field| value.get(field).is_some())
}

fn legacy_truthy(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
        _ => false,
    }
}

fn convert_legacy_profile(
    source: &Value,
    baseline: &KeyboardProfile,
) -> Result<HubProfileDocument> {
    validate_profile_shape(baseline)?;
    validate_identity(source)?;
    if let Some(identity) = source.get("productInfo") {
        ensure!(
            identity.is_object(),
            "ACE75: неверный productInfo legacy профиля"
        );
        validate_identity(identity)?;
    }
    let source_layers = source
        .get("userKeys")
        .and_then(Value::as_array)
        .context("ACE75: legacy профиль должен содержать userKeys")?;
    ensure!(
        source_layers.len() <= LAYER_COUNT,
        "ACE75: legacy профиль содержит лишние слои"
    );
    let mut layers = Vec::with_capacity(LAYER_COUNT);
    for layer in 0..LAYER_COUNT {
        let defaults = &factory_layers()[layer];
        let keys = if let Some(value) = source_layers.get(layer).filter(|value| !value.is_null()) {
            let values = value
                .as_array()
                .context("ACE75: неверный слой legacy профиля")?;
            ensure!(
                values.len() == KEY_COUNT,
                "ACE75: legacy слой должен содержать 128 позиций"
            );
            values
                .iter()
                .map(import_assignment)
                .collect::<Result<Vec<_>>>()?
        } else {
            defaults.clone()
        };
        layers.push(keys);
    }
    let records = source
        .get("advancedKeys")
        .and_then(Value::as_array)
        .context("ACE75: неверные advancedKeys legacy профиля")?;
    ensure!(
        records.len() <= 128,
        "ACE75: слишком много legacy advanced keys"
    );
    let mut groups = BTreeMap::<&str, Vec<&Value>>::new();
    let mut combos = vec![Vec::<u8>::new(); LAYER_COUNT];
    for record in records {
        let kind = record
            .get("type")
            .and_then(Value::as_str)
            .context("ACE75: нет типа legacy advanced key")?;
        let layer = byte(
            record
                .get("layer")
                .context("ACE75: нет слоя legacy advanced key")?,
        )? as usize;
        let index = byte(
            record
                .get("index1")
                .context("ACE75: нет индекса legacy advanced key")?,
        )? as usize;
        ensure!(
            layer < LAYER_COUNT && index < KEY_COUNT,
            "ACE75: legacy advanced key вне матрицы"
        );
        let group = match kind {
            "dks" => "dks",
            "tgl" => "tgl",
            "mt" | "rs" | "socd" | "lt" => "mt",
            "cb" => {
                combos[layer].push(index as u8);
                continue;
            }
            _ => anyhow::bail!("ACE75: неизвестный тип legacy advanced key {kind}"),
        };
        groups.entry(group).or_default().push(record);
    }
    for records in groups.values_mut() {
        records.sort_by_key(|record| {
            layers[record["layer"].as_u64().unwrap() as usize]
                [record["index1"].as_u64().unwrap() as usize]
                .code1
        });
    }
    let mut advanced = AdvancedState::decode(&vec![0; 768], &vec![0; 256], &vec![0; 128])?;
    let dks = groups.get("dks").map(Vec::as_slice).unwrap_or_default();
    ensure!(
        dks.len() <= ADVANCED_SLOT_COUNT,
        "ACE75: legacy DKS превышает 32 слота"
    );
    for (index, record) in dks.iter().enumerate() {
        advanced.dks[index] = convert_legacy_dks(record)?;
    }
    let tgl = groups.get("tgl").map(Vec::as_slice).unwrap_or_default();
    ensure!(
        tgl.len() <= ADVANCED_SLOT_COUNT,
        "ACE75: legacy TGL превышает 32 слота"
    );
    for (index, record) in tgl.iter().enumerate() {
        advanced.tgl[index] =
            import_assignment(record.get("tglKey").context("ACE75: нет legacy TGL key")?)?;
    }
    let mut mt = Vec::new();
    for record in groups.get("mt").map(Vec::as_slice).unwrap_or_default() {
        if record["type"] == "mt" {
            mt.push(MtKey {
                click: import_assignment(
                    record
                        .get("mtClickKey")
                        .context("ACE75: нет legacy MT click")?,
                )?,
                hold: import_assignment(
                    record
                        .get("mtDownKey")
                        .context("ACE75: нет legacy MT hold")?,
                )?,
            });
        } else {
            let keys = [
                import_assignment(record.get("key1").context("ACE75: нет legacy pair key1")?)?,
                import_assignment(record.get("key2").context("ACE75: нет legacy pair key2")?)?,
            ];
            mt.extend(paired_mt_records(keys));
        }
    }
    ensure!(
        mt.len() <= ADVANCED_SLOT_COUNT,
        "ACE75: legacy MT превышает 32 слота"
    );
    advanced.mt[..mt.len()].clone_from_slice(&mt);
    let mut data = serde_json::json!({
        "name":source.pointer("/detail/name").and_then(Value::as_str).unwrap_or("Ace 75 8K"),
        "userKeys":layers.iter().map(|layer| layer.iter().map(|key| export_assignment(*key,None)).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "advancedKeys":export_advanced(&advanced,None)?,
        "customParam":{"rtPressPrecisionMode":0,"rtReleasePrecisionMode":0,"rtSmartCacheValue":if baseline.functions.performance.rt_smart == 0 {1} else {baseline.functions.performance.rt_smart},"cbKeyIndexList":combos},
    });
    if let Some(performance) = source.get("configData").filter(|value| !value.is_null()) {
        data["performance"] = performance.clone();
    }
    if let Some(trigger) = source.get("travelKeys") {
        data["triggerTravel"] = serde_json::json!({"travelKeys":trigger});
    }
    if let Some(light) = source.get("light") {
        let mut config = light
            .as_object()
            .context("ACE75: неверная подсветка legacy профиля")?
            .clone();
        for field in ["h", "s", "v", "singleColor", "side_singleColor"] {
            config.remove(field);
        }
        config.insert(
            "customColorDisabled".into(),
            Value::from((!legacy_truthy(light.get("singleColor"))) as u8),
        );
        config.insert(
            "sideCustomColorDisabled".into(),
            Value::from(legacy_truthy(light.get("side_singleColor")) as u8),
        );
        for field in ["side_effect", "side_brightness", "side_speed"] {
            if config.get(field).is_none_or(Value::is_null) {
                config.insert(field.into(), Value::from(0));
            }
        }
        if config.get("side_hexColor").is_none_or(Value::is_null) {
            config.insert("side_hexColor".into(), Value::from("#000000"));
        }
        let colors = (0..KEY_COUNT)
            .map(|index| {
                source
                    .pointer("/colorKeys/0")
                    .and_then(|value| value.get(index))
                    .filter(|value| value.as_str().is_some_and(|value| !value.is_empty()))
                    .cloned()
                    .unwrap_or_else(|| Value::from("#000000"))
            })
            .collect::<Vec<_>>();
        data["light"] = serde_json::json!({"config":config,"colors":colors});
    }
    let mut converted = serde_json::json!({"dataScope":"KeyboardProfile","version":3,"vendorId":0x3837,"productId":0x303c,"productName":"Ace 75","data":data});
    if let Some(macros) = source.get("macros") {
        converted["macros"] = macros.clone();
    }
    HubProfileDocument::from_json(&serde_json::to_vec(&converted)?)
}

fn convert_legacy_dks(value: &Value) -> Result<DksKey> {
    let mut result = DksKey::decode(&[0; 24])?;
    let points = value
        .get("dksPoint")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value::<[u8; 4]>(value.clone()))
        .transpose()?
        .unwrap_or([10, 30, 30, 10]);
    for (index, point) in points.into_iter().enumerate() {
        result.points[index] = if point == 0 {
            [10, 30, 30, 10][index]
        } else {
            point
        };
    }
    let actions = value
        .get("dksKeys")
        .and_then(Value::as_array)
        .context("ACE75: нет legacy DKS actions")?;
    ensure!(
        actions.len() <= 4,
        "ACE75: legacy DKS содержит больше четырёх действий"
    );
    let mask = |value: u8| {
        (if value > 0 { 1 } else { 0 })
            | (if value > 1 { 2 } else { 0 })
            | (if value > 2 { 4 } else { 0 })
    };
    for (index, value) in actions.iter().enumerate() {
        result.actions[index] =
            import_assignment(value.get("key").context("ACE75: нет legacy DKS key")?)?;
        let get = |name: &str| {
            value
                .get(name)
                .map(byte)
                .transpose()
                .map(|value| value.unwrap_or(0))
        };
        let mut down_start = get("downStart")?;
        let mut down_end = get("downEnd")?;
        let mut up_start = get("upStart")?;
        let up_end = get("upEnd")?;
        if down_start == 4 {
            down_start = 3;
            down_end = 3;
            up_start = 2;
        } else if down_start == 3 {
            down_end = 2;
        }
        if get("downEnd")? == 3 {
            up_start = 2;
        }
        result.masks[index] = [
            mask(down_start),
            mask(down_end),
            mask(up_start),
            mask(up_end) & 1,
        ];
    }
    result.encode()?;
    Ok(result)
}

fn validate_profile_shape(profile: &KeyboardProfile) -> Result<()> {
    ensure!(
        profile.index < 4
            && profile.layers.len() == LAYER_COUNT
            && profile.layers.iter().all(|layer| layer.len() == KEY_COUNT)
            && profile.triggers.len() == KEY_COUNT
            && profile.trigger_raw.len() == KEY_COUNT * 8
            && profile.colors.len() == KEY_COUNT
            && profile.custom_raw.len() == CUSTOM_AREA_SIZE,
        "ACE75: неверные размеры профиля"
    );
    Ok(())
}

fn import_layers(value: &Value, baseline: &KeyboardProfile) -> Result<Vec<Vec<KeyAssignment>>> {
    let layers = value
        .as_array()
        .context("ACE75: неверные слои Hub profile")?;
    ensure!(
        layers.len() == LAYER_COUNT,
        "ACE75: нужны четыре слоя Hub profile"
    );
    let defaults = factory_layers();
    let lookup = storage_lookup(false);
    layers
        .iter()
        .enumerate()
        .map(|(layer_index, layer)| {
            let mut keys = defaults[baseline.index as usize * 4 + layer_index].clone();
            if let Some(values) = layer.as_array() {
                ensure!(
                    values.len() == KEY_COUNT,
                    "ACE75: нужно 128 позиций на слой"
                );
                for (index, value) in values.iter().enumerate() {
                    keys[index] = import_assignment(value)?;
                }
            } else {
                ensure!(
                    layer.get("type").and_then(Value::as_str) == Some("diff-keys"),
                    "ACE75: неизвестный формат слоя Hub"
                );
                if let Some(data) = layer.get("dataV2") {
                    ensure!(data.is_object(), "ACE75: неверные diff-keys dataV2");
                    for (index, location) in lookup.iter().enumerate() {
                        if let Some((name, occurrence)) = location {
                            if let Some(value) = data
                                .get(name)
                                .and_then(|v| v.get(*occurrence))
                                .filter(|v| !v.is_null())
                            {
                                keys[index] = import_assignment(value)?;
                            }
                        }
                    }
                } else if let Some(data) = layer.get("data") {
                    let data = data.as_object().context("ACE75: неверные diff-keys data")?;
                    for (index, value) in data {
                        let index: usize =
                            index.parse().context("ACE75: неверный индекс diff-keys")?;
                        ensure!(index < KEY_COUNT, "ACE75: индекс diff-keys вне диапазона");
                        keys[index] = import_assignment(value)?;
                    }
                }
            }
            for (index, key) in keys.iter_mut().enumerate() {
                if [93, 94].contains(&index) {
                    *key = baseline.layers[layer_index][index];
                }
                if key.kind == 240 && key.code1 == 255 {
                    key.code2 = baseline.index * 4 + key.code2 % 4;
                }
                if key.kind >= 240 && key.kind == key.code1 && key.kind == key.code2 {
                    key.kind = 240 + baseline.index * 4 + layer_index as u8;
                    key.code1 = key.kind;
                    key.code2 = key.kind;
                }
            }
            Ok(keys)
        })
        .collect()
}

fn factory_layers() -> &'static Vec<Vec<KeyAssignment>> {
    static LAYERS: OnceLock<Vec<Vec<KeyAssignment>>> = OnceLock::new();
    LAYERS.get_or_init(|| {
        let raw: Vec<Vec<[u8; 3]>> =
            serde_json::from_str(include_str!("../../assets/ace75-default-key-layers.json"))
                .expect("bundled ACE75 factory layout");
        raw.into_iter()
            .map(|layer| {
                layer
                    .into_iter()
                    .map(|[kind, code1, code2]| KeyAssignment { kind, code1, code2 })
                    .collect()
            })
            .collect()
    })
}

fn storage_lookup(trigger: bool) -> Vec<Option<(String, usize)>> {
    let mut occurrences = BTreeMap::<String, usize>::new();
    factory_layers()[0]
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let name = if key.kind == 240 && key.code1 == 255 {
                "fn-key".to_owned()
            } else if key.kind >= 240 && key.kind == key.code1 && key.kind == key.code2 {
                format!("side-light-key:relative@{}", index - 95)
            } else if [92, 93, 94].contains(&index) {
                if trigger {
                    return None;
                }
                match index {
                    92 => "code:3809",
                    93 => "code:252",
                    _ => "rt-dbclick",
                }
                .to_owned()
            } else {
                let value = (key.kind as u32) << 16 | (key.code1 as u32) << 8 | key.code2 as u32;
                if value == 0 {
                    return None;
                }
                format!("0x{value:x}")
            };
            let occurrence = occurrences.entry(name.clone()).or_default();
            let result = Some((name, *occurrence));
            *occurrence += 1;
            result
        })
        .collect()
}

fn import_sparse_triggers(value: &Value, before: &[KeyTrigger]) -> Result<Vec<KeyTrigger>> {
    let default = value
        .get("default")
        .context("ACE75: нет default travelKeysV2")?;
    let data = value
        .get("data")
        .and_then(Value::as_object)
        .context("ACE75: неверные travelKeysV2.data")?;
    storage_lookup(true)
        .iter()
        .zip(before)
        .map(|(location, before)| {
            let specified = location
                .as_ref()
                .and_then(|(name, occurrence)| data.get(name).and_then(|v| v.get(*occurrence)));
            let selected = specified
                .filter(|v| !v.is_null() && v.as_str() != Some("default-value"))
                .unwrap_or(default);
            import_trigger(selected, before)
        })
        .collect()
}

fn validate_identity(value: &Value) -> Result<()> {
    for identity in [
        Some(value),
        value.pointer("/extra/identity"),
        value.get("identity"),
    ]
    .into_iter()
    .flatten()
    {
        for (field, expected) in [("vendorId", 0x3837), ("productId", 0x303c)] {
            if let Some(value) = identity.get(field) {
                ensure!(
                    value.as_u64() == Some(expected),
                    "ACE75: профиль другой модели ({field})"
                );
            }
        }
        if let Some(name) = identity.get("productName") {
            ensure!(
                name.as_str() == Some("Ace 75"),
                "ACE75: профиль другой модели (productName)"
            );
        }
    }
    Ok(())
}

fn import_assignment(value: &Value) -> Result<KeyAssignment> {
    Ok(KeyAssignment {
        kind: byte(
            value
                .get("type")
                .context("ACE75: отсутствует type назначения")?,
        )?,
        code1: byte(
            value
                .get("code1")
                .context("ACE75: отсутствует code1 назначения")?,
        )?,
        code2: byte(
            value
                .get("code2")
                .context("ACE75: отсутствует code2 назначения")?,
        )?,
    })
}

fn export_assignment(key: KeyAssignment, previous: Option<&Value>) -> Value {
    let mut result = previous
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    result.insert("type".into(), Value::from(key.kind));
    result.insert("code1".into(), Value::from(key.code1));
    result.insert("code2".into(), Value::from(key.code2));
    Value::Object(result)
}

fn import_trigger(value: &Value, before: &KeyTrigger) -> Result<KeyTrigger> {
    let result: KeyTrigger = serde_json::from_value(import_fields(
        serde_json::to_value(before)?,
        value,
        TRIGGER_FIELDS,
    )?)?;
    result.validate()?;
    ensure!(
        &result == before || (result.press_precision < 3 && result.release_precision < 3),
        "ACE75: профиль использует неподдерживаемую точность"
    );
    Ok(result)
}

fn import_fields(
    mut destination: Value,
    source: &Value,
    fields: &[(&str, &str, bool)],
) -> Result<Value> {
    let source = source
        .as_object()
        .context("ACE75: ожидается объект настроек Hub")?;
    let target = destination.as_object_mut().unwrap();
    for (official, native, boolean) in fields {
        if let Some(value) = source.get(*official) {
            let value = if *boolean {
                Value::Bool(match value {
                    Value::Bool(v) => *v,
                    _ => {
                        let number = value.as_u64().context("ACE75: неверный флаг Hub")?;
                        ensure!(number <= 1, "ACE75: флаг должен быть 0 или 1");
                        number != 0
                    }
                })
            } else {
                Value::from(value.as_u64().context("ACE75: неверное число Hub")?)
            };
            target.insert((*native).into(), value);
        }
    }
    Ok(destination)
}

fn export_fields(
    destination: &mut Value,
    source: &Value,
    fields: &[(&str, &str, bool)],
) -> Result<()> {
    let target = destination
        .as_object_mut()
        .context("ACE75: неверный объект Hub")?;
    for (official, native, boolean) in fields {
        let value = source
            .get(*native)
            .context("ACE75: отсутствует native field")?;
        target.insert(
            (*official).into(),
            if *boolean {
                Value::from(value.as_bool().unwrap() as u8)
            } else {
                value.clone()
            },
        );
    }
    Ok(())
}

fn import_lighting(value: &Value, before: &Lighting) -> Result<Lighting> {
    ensure!(value.is_object(), "ACE75: неверный lightConfig");
    let mut result = before.clone();
    import_zone(value, &mut result.main, "")?;
    import_zone(value, &mut result.side, "side_")?;
    if let Some(logo) = value.get("sideEffect2") {
        import_zone(logo, &mut result.side2, "")?;
    }
    if let Some(scope) = value.get("lightScopeType") {
        result.scope = match scope.as_str() {
            Some("main") => LightScope::Main,
            Some("side") => LightScope::Side,
            Some("side2") => LightScope::Side2,
            _ => anyhow::bail!("ACE75: неизвестная область подсветки"),
        };
    }
    if let Some(rgb) = value.get("calibrationRgb") {
        result.calibration_color = [
            byte(rgb.get("r").context("ACE75: нет calibration.r")?)?,
            byte(rgb.get("g").context("ACE75: нет calibration.g")?)?,
            byte(rgb.get("b").context("ACE75: нет calibration.b")?)?,
        ];
    }
    result.validate()?;
    Ok(result)
}

fn import_zone(value: &Value, zone: &mut LightZone, prefix: &str) -> Result<()> {
    let fields = [
        (format!("{prefix}effect"), "effect"),
        (format!("{prefix}brightness"), "brightness"),
        (format!("{prefix}speed"), "speed"),
        (
            if prefix.is_empty() {
                "direct".into()
            } else {
                "side_direct".into()
            },
            "direction",
        ),
    ];
    for (name, destination) in fields {
        if let Some(value) = value.get(&name) {
            let v = byte(value)?;
            match destination {
                "effect" => zone.effect = v,
                "brightness" => zone.brightness = v,
                "speed" => zone.speed = v as i16,
                "direction" => zone.direction = v,
                _ => unreachable!(),
            }
        }
    }
    let rainbow = if prefix.is_empty() {
        "customColorDisabled"
    } else {
        "sideCustomColorDisabled"
    };
    if let Some(value) = value.get(rainbow) {
        zone.rainbow = match value {
            Value::Bool(v) => *v,
            _ => {
                let v = byte(value)?;
                ensure!(v <= 1, "ACE75: неверный rainbow flag");
                v != 0
            }
        };
    }
    if let Some(value) = value.get(format!("{prefix}hexColor")) {
        zone.color = parse_color(value)?;
    }
    if let Some(value) = value.get(format!("{prefix}colorIndex")) {
        zone.color_index = byte(value)?;
    }
    zone.validate()
}

fn export_lighting(light: &Lighting, previous: Option<&Value>) -> Result<Value> {
    let mut value = previous.cloned().unwrap_or_else(|| serde_json::json!({}));
    export_zone(&mut value, &light.main, "")?;
    export_zone(&mut value, &light.side, "side_")?;
    let object = value.as_object_mut().unwrap();
    let mut logo = object
        .get("sideEffect2")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    export_zone(&mut logo, &light.side2, "")?;
    object.insert("sideEffect2".into(), logo);
    object.insert(
        "lightScopeType".into(),
        Value::from(match light.scope {
            LightScope::Main => "main",
            LightScope::Side => "side",
            LightScope::Side2 => "side2",
        }),
    );
    object.insert("calibrationRgb".into(), serde_json::json!({"r":light.calibration_color[0],"g":light.calibration_color[1],"b":light.calibration_color[2]}));
    Ok(value)
}

fn export_zone(value: &mut Value, zone: &LightZone, prefix: &str) -> Result<()> {
    zone.validate()?;
    let object = value
        .as_object_mut()
        .context("ACE75: неверная область lightConfig")?;
    for (name, value) in [
        ("effect", Value::from(zone.effect)),
        ("brightness", Value::from(zone.brightness)),
        ("speed", Value::from(zone.speed)),
        ("hexColor", Value::from(hex_color(zone.color))),
        ("colorIndex", Value::from(zone.color_index)),
    ] {
        object.insert(format!("{prefix}{name}"), value);
    }
    object.insert(
        if prefix.is_empty() {
            "customColorDisabled"
        } else {
            "sideCustomColorDisabled"
        }
        .into(),
        Value::from(zone.rainbow as u8),
    );
    object.insert(
        if prefix.is_empty() {
            "direct"
        } else {
            "side_direct"
        }
        .into(),
        Value::from(zone.direction),
    );
    Ok(())
}

fn import_advanced(value: &Value, target: &mut AdvancedState) -> Result<()> {
    ensure!(value.is_object(), "ACE75: неверные advancedKeys");
    for name in ["dks", "mt", "tgl"] {
        let Some(values) = value.get(name) else {
            continue;
        };
        let values = values
            .as_array()
            .context("ACE75: advanced bank должен быть массивом")?;
        ensure!(
            values.len() <= ADVANCED_SLOT_COUNT,
            "ACE75: advanced bank превышает 32 записи"
        );
        for (index, value) in values.iter().enumerate() {
            match name {
                "dks" => {
                    let points: [u8; 4] = serde_json::from_value(
                        value.get("point").context("ACE75: нет DKS point")?.clone(),
                    )?;
                    target.dks[index].points = points;
                    for action in 0..4 {
                        target.dks[index].actions[action] = import_assignment(
                            value
                                .get(format!("action{action}"))
                                .context("ACE75: нет DKS action")?,
                        )?;
                        for point in 0..4 {
                            target.dks[index].masks[action][point] = byte(
                                value
                                    .get(format!("point{action}"))
                                    .and_then(|v| v.get(format!("action{point}")))
                                    .context("ACE75: нет DKS mask")?,
                            )?;
                        }
                    }
                }
                "mt" => {
                    target.mt[index] = MtKey {
                        click: import_assignment(
                            value.get("clickKey").context("ACE75: нет MT clickKey")?,
                        )?,
                        hold: import_assignment(
                            value.get("downKey").context("ACE75: нет MT downKey")?,
                        )?,
                    }
                }
                "tgl" => target.tgl[index] = import_assignment(value)?,
                _ => unreachable!(),
            }
        }
    }
    target.encode()?;
    Ok(())
}

fn export_advanced(value: &AdvancedState, previous: Option<&Value>) -> Result<Value> {
    value.encode()?;
    let mut result = previous.cloned().unwrap_or_else(|| serde_json::json!({}));
    let object = result
        .as_object_mut()
        .context("ACE75: неверный advancedKeys")?;
    let mut dks = vec![];
    for (index, key) in value.dks.iter().enumerate() {
        let mut item = object
            .get("dks")
            .and_then(|v| v.get(index))
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        item.insert("point".into(), serde_json::to_value(key.points)?);
        for action in 0..4 {
            item.insert(
                format!("action{action}"),
                export_assignment(key.actions[action], item.get(&format!("action{action}"))),
            );
            let mut masks = item
                .get(&format!("point{action}"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            for point in 0..4 {
                masks.insert(
                    format!("action{point}"),
                    Value::from(key.masks[action][point]),
                );
            }
            item.insert(format!("point{action}"), Value::Object(masks));
        }
        dks.push(Value::Object(item));
    }
    object.insert("dks".into(), Value::Array(dks));
    let mt = value
        .mt
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let mut item = object
                .get("mt")
                .and_then(|v| v.get(index))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            item.insert(
                "clickKey".into(),
                export_assignment(key.click, item.get("clickKey")),
            );
            item.insert(
                "downKey".into(),
                export_assignment(key.hold, item.get("downKey")),
            );
            Value::Object(item)
        })
        .collect();
    object.insert("mt".into(), Value::Array(mt));
    let tgl = value
        .tgl
        .iter()
        .enumerate()
        .map(|(index, key)| export_assignment(*key, object.get("tgl").and_then(|v| v.get(index))))
        .collect();
    object.insert("tgl".into(), Value::Array(tgl));
    Ok(result)
}

fn validate_defined_advanced(
    layers: &[Vec<KeyAssignment>],
    definitions: Option<&Value>,
) -> Result<()> {
    for key in layers.iter().flatten() {
        if (144..=149).contains(&key.kind) {
            let bank = match key.kind {
                144 => "dks",
                145 => "tgl",
                _ => "mt",
            };
            ensure!(
                definitions
                    .and_then(|value| value.get(bank))
                    .and_then(Value::as_array)
                    .is_some_and(|v| (key.code1 as usize) < v.len()),
                "ACE75: профиль ссылается на advanced key без определения"
            );
        }
    }
    Ok(())
}

fn import_colors(value: &Value, count: usize) -> Result<Vec<[u8; 3]>> {
    let values = value
        .as_array()
        .context("ACE75: цвета должны быть массивом")?;
    ensure!(
        values.len() == count,
        "ACE75: неверное число цветов в профиле"
    );
    values.iter().map(parse_color).collect()
}

fn parse_color(value: &Value) -> Result<[u8; 3]> {
    if let Some(values) = value.as_array() {
        ensure!(
            values.len() == 3,
            "ACE75: RGB должен содержать три компонента"
        );
        return Ok([byte(&values[0])?, byte(&values[1])?, byte(&values[2])?]);
    }
    let text = value
        .as_str()
        .context("ACE75: цвет должен быть HEX или RGB")?;
    let text = text
        .strip_prefix('#')
        .context("ACE75: цвет должен начинаться с #")?;
    ensure!(
        text.len() == 6 && text.is_ascii(),
        "ACE75: нужен цвет #RRGGBB"
    );
    Ok([
        u8::from_str_radix(&text[..2], 16)?,
        u8::from_str_radix(&text[2..4], 16)?,
        u8::from_str_radix(&text[4..], 16)?,
    ])
}

fn hex_color(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}
fn byte(value: &Value) -> Result<u8> {
    value
        .as_u64()
        .filter(|v| *v <= 255)
        .map(|v| v as u8)
        .context("ACE75: значение должно быть байтом 0..255")
}

pub fn plan_macro_reference_updates(
    profile: &KeyboardProfile,
    factory_layers: &[Vec<KeyAssignment>],
    bank: &MacroBank,
) -> Result<KeyboardProfile> {
    validate_profile_shape(profile)?;
    bank.encode()?;
    let defaults = match factory_layers.len() {
        LAYER_COUNT => factory_layers,
        16 => &factory_layers[profile.index as usize * 4..profile.index as usize * 4 + 4],
        _ => anyhow::bail!("ACE75: неверное число заводских слоёв"),
    };
    ensure!(
        defaults.iter().all(|layer| layer.len() == KEY_COUNT),
        "ACE75: неполные заводские слои"
    );
    let update = |key: &mut KeyAssignment, fallback: KeyAssignment| -> Result<()> {
        if key.kind != 0x70 {
            return Ok(());
        }
        let definition = bank
            .macros
            .get(key.code1 as usize)
            .context("ACE75: ссылка на макрос вне диапазона 0..15")?;
        if definition.macro_actions.is_empty() {
            *key = fallback;
        } else {
            key.code2 = definition.mode;
        }
        Ok(())
    };
    let mut result = profile.clone();
    for (layer, keys) in result.layers.iter_mut().enumerate() {
        for (index, key) in keys.iter_mut().enumerate() {
            update(key, defaults[layer][index])?;
        }
    }
    // The Hub normalizes advanced action arrays with layer0 and action-relative indices.
    for entry in &mut result.advanced.dks {
        for (index, key) in entry.actions.iter_mut().enumerate() {
            update(key, defaults[0][index])?;
        }
    }
    for entry in &mut result.advanced.mt {
        update(&mut entry.click, defaults[0][0])?;
        update(&mut entry.hold, defaults[0][1])?;
    }
    for entry in &mut result.advanced.tgl {
        update(entry, defaults[0][0])?;
    }
    result.advanced.validate_references(&result.layers)?;
    Ok(result)
}

pub fn rebase_profile(profile: &mut KeyboardProfile, target_index: u8) -> Result<()> {
    ensure!(
        target_index < PROFILE_COUNT as u8,
        "ACE75: неверный целевой профиль"
    );
    validate_profile_shape(profile)?;
    let mut result = profile.clone();
    result.index = target_index;
    let mut performance = result.functions.performance.clone();
    performance.mac_mode = target_index * 4 + performance.mac_mode % 4;
    result.functions =
        FunctionConfig::decode(&result.functions.with_performance(&performance, false)?)?;
    visit_assignments_mut(&mut result, |key| {
        if key.kind == 240 && key.code1 == 255 {
            key.code2 = target_index * 4 + key.code2 % 4;
        }
    });
    for (layer, keys) in result.layers.iter_mut().enumerate() {
        for key in keys {
            if key.kind >= 240 && key.kind == key.code1 && key.kind == key.code2 {
                key.kind = 240 + target_index * 4 + layer as u8;
                key.code1 = key.kind;
                key.code2 = key.kind;
            }
        }
    }
    *profile = result;
    Ok(())
}

pub fn merge_profile_macros(
    profile: &mut KeyboardProfile,
    source: Option<&MacroBank>,
    current: &MacroBank,
) -> Result<MacroBank> {
    merge_scoped_profile_macros(profile, source, current, None)
}

pub fn macro_slots(profile: &KeyboardProfile) -> BTreeSet<u8> {
    profile
        .layers
        .iter()
        .flatten()
        .chain(profile.advanced.dks.iter().flat_map(|entry| &entry.actions))
        .chain(
            profile
                .advanced
                .mt
                .iter()
                .flat_map(|entry| [&entry.click, &entry.hold]),
        )
        .chain(&profile.advanced.tgl)
        .filter(|key| key.kind == 0x70)
        .map(|key| key.code1)
        .collect()
}

pub fn merge_scoped_profile_macros(
    profile: &mut KeyboardProfile,
    source: Option<&MacroBank>,
    current: &MacroBank,
    scope: Option<&HubProfileScope>,
) -> Result<MacroBank> {
    merge_scoped_profile_macros_reserved(profile, source, current, scope, &BTreeSet::new())
}

pub fn merge_scoped_profile_macros_reserved(
    profile: &mut KeyboardProfile,
    source: Option<&MacroBank>,
    current: &MacroBank,
    scope: Option<&HubProfileScope>,
    reserved: &BTreeSet<u8>,
) -> Result<MacroBank> {
    current.encode()?;
    let mut protected = reserved.clone();
    if let Some(scope) = scope {
        for keys in &profile.layers {
            for (index, key) in keys.iter().enumerate() {
                if key.kind == 0x70 && (!scope.keys || [93, 94].contains(&index)) {
                    protected.insert(key.code1);
                }
            }
        }
        if !scope.advanced {
            for key in profile
                .advanced
                .dks
                .iter()
                .flat_map(|entry| &entry.actions)
                .chain(
                    profile
                        .advanced
                        .mt
                        .iter()
                        .flat_map(|entry| [&entry.click, &entry.hold]),
                )
                .chain(&profile.advanced.tgl)
            {
                if key.kind == 0x70 {
                    protected.insert(key.code1);
                }
            }
        }
    }
    ensure!(
        protected.iter().all(|slot| (*slot as usize) < MACRO_COUNT),
        "ACE75: резервная ссылка на макрос вне диапазона 0..15"
    );
    profile.advanced.validate_references(&profile.layers)?;
    let mut references = BTreeSet::new();
    visit_scoped_assignments(profile, scope, |key| {
        if key.kind == 0x70 {
            ensure!(
                (key.code1 as usize) < MACRO_COUNT,
                "ACE75: ссылка на макрос вне диапазона 0..15"
            );
            references.insert(key.code1);
        }
        Ok(())
    })?;
    if references.is_empty() {
        return Ok(current.clone());
    }
    let source = source.context("ACE75: профиль ссылается на макросы без их определений")?;
    source.encode()?;
    let mut result = current.clone();
    let mut mapping = BTreeMap::new();
    for index in references {
        let imported = &source.macros[index as usize];
        ensure!(
            !imported.macro_actions.is_empty(),
            "ACE75: профиль ссылается на пустой макрос {}",
            index + 1
        );
        let wire = encode_macro_actions(&imported.macro_actions)?;
        let mut existing = None;
        for (slot, item) in result.macros.iter().enumerate() {
            if item.mode == imported.mode && encode_macro_actions(&item.macro_actions)? == wire {
                existing = Some(slot);
                break;
            }
        }
        let slot = match existing {
            Some(slot) => slot,
            None => {
                let slot = result
                    .macros
                    .iter()
                    .enumerate()
                    .position(|(slot, item)| {
                        item.macro_actions.is_empty() && !protected.contains(&(slot as u8))
                    })
                    .context(
                        "ACE75: нет свободных слотов макросов; существующие макросы сохранены",
                    )?;
                result.macros[slot] = imported.clone();
                result.macros[slot].macro_index = slot as u8;
                slot
            }
        };
        mapping.insert(index, (slot as u8, imported.mode));
    }
    result.encode()?;
    let mut rewritten = profile.clone();
    visit_scoped_assignments_mut(&mut rewritten, scope, |key| {
        if key.kind == 0x70 {
            let (index, mode) = mapping[&key.code1];
            key.code1 = index;
            key.code2 = mode;
        }
    });
    *profile = rewritten;
    Ok(result)
}

fn visit_scoped_assignments(
    profile: &KeyboardProfile,
    scope: Option<&HubProfileScope>,
    mut visit: impl FnMut(&KeyAssignment) -> Result<()>,
) -> Result<()> {
    if scope.is_none_or(|scope| scope.keys) {
        for keys in &profile.layers {
            for (index, key) in keys.iter().enumerate() {
                if scope.is_some() && [93, 94].contains(&index) {
                    continue;
                }
                visit(key)?;
            }
        }
    }
    if scope.is_none_or(|scope| scope.advanced) {
        for key in &profile.advanced.dks {
            for action in &key.actions {
                visit(action)?;
            }
        }
        for key in &profile.advanced.mt {
            visit(&key.click)?;
            visit(&key.hold)?;
        }
        for key in &profile.advanced.tgl {
            visit(key)?;
        }
    }
    Ok(())
}

fn visit_scoped_assignments_mut(
    profile: &mut KeyboardProfile,
    scope: Option<&HubProfileScope>,
    mut visit: impl FnMut(&mut KeyAssignment),
) {
    if scope.is_none_or(|scope| scope.keys) {
        for keys in &mut profile.layers {
            for (index, key) in keys.iter_mut().enumerate() {
                if scope.is_some() && [93, 94].contains(&index) {
                    continue;
                }
                visit(key);
            }
        }
    }
    if scope.is_none_or(|scope| scope.advanced) {
        for key in &mut profile.advanced.dks {
            for action in &mut key.actions {
                visit(action);
            }
        }
        for key in &mut profile.advanced.mt {
            visit(&mut key.click);
            visit(&mut key.hold);
        }
        for key in &mut profile.advanced.tgl {
            visit(key);
        }
    }
}

fn visit_assignments_mut(profile: &mut KeyboardProfile, mut visit: impl FnMut(&mut KeyAssignment)) {
    for key in profile.layers.iter_mut().flatten() {
        visit(key);
    }
    for key in &mut profile.advanced.dks {
        for action in &mut key.actions {
            visit(action);
        }
    }
    for key in &mut profile.advanced.mt {
        visit(&mut key.click);
        visit(&mut key.hold);
    }
    for key in &mut profile.advanced.tgl {
        visit(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn baseline(index: u8) -> KeyboardProfile {
        let trigger = KeyTrigger {
            switch_type: 0,
            key_mode: 1,
            priority: 0,
            actuation: 100,
            press_precision: 0,
            release_precision: 0,
            rt_press: 20,
            rt_release: 30,
            press_deadzone: 5,
            release_deadzone: 6,
        };
        let mut functions = vec![0; 64];
        functions[1] = index * 4;
        functions[4] = 1;
        RawKeyboardProfile {
            index,
            functions,
            layers: factory_layers()[index as usize * 4..index as usize * 4 + 4]
                .iter()
                .map(|v| KeyAssignment::encode_all(v))
                .collect(),
            triggers: KeyTrigger::encode_all(&vec![trigger; KEY_COUNT]).unwrap(),
            colors: vec![0; KEY_COUNT * 3],
            dks: vec![0; 768],
            mt: vec![0; 256],
            tgl: vec![0; 128],
            custom: vec![0; CUSTOM_AREA_SIZE],
        }
        .decode()
        .unwrap()
    }

    fn macro_action(code: u16) -> Vec<MacroAction> {
        vec![
            MacroAction::Action {
                action: MacroKeyAction::Keydown,
                code,
            },
            MacroAction::Time { delay: 50 },
            MacroAction::Action {
                action: MacroKeyAction::Keyup,
                code,
            },
        ]
    }

    fn macro_ref(slot: u8) -> KeyAssignment {
        KeyAssignment {
            kind: 0x70,
            code1: slot,
            code2: 0,
        }
    }

    #[test]
    fn original_sdk_sparse_keys_triggers_and_storage_lookup() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/ace75/hub-profile-sdk-vectors.json"
        ))
        .unwrap();
        for (trigger, name) in [(false, "keyLookup"), (true, "triggerLookup")] {
            let actual: Vec<Value> = storage_lookup(trigger).into_iter().map(|entry| entry.map(|(key, occurrence)| json!({"storageKeyByte":key,"occurrence":occurrence})).unwrap_or(Value::Null)).collect();
            assert_eq!(Value::Array(actual), fixture[name]);
        }
        let document =
            HubProfileDocument::from_json(&serde_json::to_vec(&fixture["document"]).unwrap())
                .unwrap();
        let imported = import_hub_profile(&document, &baseline(0)).unwrap();
        let expected: Vec<Vec<[u8; 3]>> =
            serde_json::from_value(fixture["expected"]["keys"].clone()).unwrap();
        for (actual, expected) in imported.profile.layers.iter().zip(expected) {
            assert_eq!(KeyAssignment::encode_all(actual), expected.concat());
        }
        for (actual, expected) in imported
            .profile
            .triggers
            .iter()
            .zip(fixture["expected"]["triggers"].as_array().unwrap())
        {
            assert_eq!(actual.actuation as u64, expected["keyActuation"]);
            assert_eq!(actual.rt_press as u64, expected["rtPress"]);
            assert_eq!(actual.rt_release as u64, expected["rtRelease"]);
            assert_eq!(actual.switch_type as u64, expected["switchType"]);
            assert_eq!(actual.key_mode as u64, expected["keyMode"]);
            assert_eq!(actual.priority as u64, expected["priority"]);
        }
        assert_eq!(imported.profile.functions.performance.report_rate, 1);
        assert!(
            imported.scope.keys
                && imported.scope.triggers
                && imported.scope.performance
                && imported.scope.lighting
        );
    }

    #[test]
    fn official_default_bundle_imports_for_any_onboard_profile() {
        let document = HubProfileDocument::from_json(include_bytes!(
            "../../tests/fixtures/ace75/official-preset114.json"
        ))
        .unwrap();
        for index in 0..4 {
            let imported = import_hub_profile(&document, &baseline(index)).unwrap();
            assert_eq!(
                imported.profile.layers[0][88],
                KeyAssignment {
                    kind: 16,
                    code1: 0,
                    code2: 4
                }
            );
            assert_eq!(imported.profile.layers[0][22].code2, index * 4 + 1);
            assert!(imported.scope.keys && imported.scope.triggers && imported.scope.performance);
            assert_eq!(imported.profile.functions.performance.report_rate, 1);
        }
    }

    #[test]
    fn export_roundtrip_retains_unknown_envelope_fields_and_updates_v2() {
        let before = baseline(2);
        let original = HubProfileDocument::from_json(br##"{"dataScope":"KeyboardProfile","vendorId":14391,"productId":12348,"future":{"v":[1,255]},"data":{"triggerTravel":{"travelKeysV2":{"default":{},"data":{}}},"performance":{"unknownPerf":42},"light":{"config":{"futureLight":17}},"opaque":true}}"##).unwrap();
        let mut profile = before.clone();
        profile.layers[0][88] = KeyAssignment {
            kind: 16,
            code1: 0,
            code2: 5,
        };
        profile.triggers[88].actuation = 123;
        profile.trigger_raw =
            update_trigger_records(&before.trigger_raw, &profile.triggers).unwrap();
        profile.colors[99] = [1, 2, 3];
        let exported = export_hub_profile(&profile, None, Some(&original)).unwrap();
        assert_eq!(exported.content["version"], 3);
        assert_eq!(exported.content["productName"], "Ace 75");
        assert_eq!(exported.content["identity"]["productId"], 0x303c);
        assert_eq!(exported.content["future"], original.content["future"]);
        assert_eq!(exported.content["data"]["opaque"], true);
        assert_eq!(exported.content["data"]["performance"]["unknownPerf"], 42);
        assert_eq!(
            exported.content["data"]["light"]["config"]["futureLight"],
            17
        );
        assert!(
            exported.content["data"]["triggerTravel"]
                .get("travelKeysV2")
                .is_none()
        );
        let imported = import_hub_profile(&exported, &before).unwrap();
        assert_eq!(imported.profile, profile);
    }

    #[test]
    fn partial_import_rebases_unmentioned_sections_and_colors() {
        let before = baseline(0);
        let document = HubProfileDocument::from_json(br##"{"dataScope":"KeyboardProfile","data":{"light":{"sideColors":["#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203","#010203"]}}}"##).unwrap();
        let imported = import_hub_profile(&document, &before).unwrap();
        let mut current = before.clone();
        current.layers[0][88].code2 = 9;
        current.colors[0] = [7, 8, 9];
        current.colors[123] = [4, 5, 6];
        let merged = imported
            .scope
            .merge_into(&imported.profile, &current)
            .unwrap();
        assert_eq!(merged.layers, current.layers);
        assert_eq!(merged.colors[0], [7, 8, 9]);
        assert_eq!(merged.colors[95], [1, 2, 3]);
        assert_eq!(merged.colors[123], [4, 5, 6]);
        assert!(!imported.scope.keys && !imported.scope.lighting && !imported.scope.performance);
    }

    #[test]
    fn another_keyboard_identity_and_malformed_scopes_are_rejected() {
        let wrong = HubProfileDocument::from_json(br#"{"dataScope":"KeyboardProfile","extra":{"identity":{"vendorId":14391,"productId":12317,"productName":"Ace 75"}},"data":{"performance":{"lockWin":1}}}"#).unwrap();
        assert!(import_hub_profile(&wrong, &baseline(0)).is_err());
        assert!(
            HubProfileDocument::from_json(br#"{"dataScope":"LightingEffectProfile","data":{}}"#)
                .is_err()
        );
        let empty = HubProfileDocument::from_json(br#"{"data":{"unknown":1}}"#).unwrap();
        assert!(import_hub_profile(&empty, &baseline(0)).is_err());
    }

    #[test]
    fn macro_merge_reuses_existing_allocates_empty_and_rewrites_advanced_refs() {
        let mut source = MacroBank::decode(&vec![0; 8192]).unwrap();
        source.macros[3].macro_actions = macro_action(4);
        source.macros[3].mode = 1;
        source.macros[5].macro_actions = macro_action(5);
        let mut current = MacroBank::decode(&vec![0; 8192]).unwrap();
        current.macros[0].macro_actions = macro_action(9);
        current.macros[7].macro_actions = macro_action(4);
        current.macros[7].mode = 1;
        let mut profile = baseline(0);
        profile.layers[0][88] = macro_ref(3);
        profile.advanced.dks[1].actions[2] = macro_ref(5);
        profile.advanced.mt[3].click = macro_ref(3);
        profile.advanced.mt[3].hold = macro_ref(5);
        profile.advanced.tgl[4] = macro_ref(3);
        let merged = merge_profile_macros(&mut profile, Some(&source), &current).unwrap();
        assert_eq!(merged.macros[0], current.macros[0]);
        assert_eq!(
            profile.layers[0][88],
            KeyAssignment {
                kind: 0x70,
                code1: 7,
                code2: 1
            }
        );
        assert_eq!(profile.advanced.dks[1].actions[2].code1, 1);
        assert_eq!(profile.advanced.mt[3].click.code1, 7);
        assert_eq!(profile.advanced.mt[3].hold.code1, 1);
        assert_eq!(profile.advanced.tgl[4].code1, 7);
        assert_eq!(merged.macros[1].macro_actions, macro_action(5));
    }

    #[test]
    fn failed_macro_merge_keeps_profile_and_all_existing_slots() {
        let mut source = MacroBank::decode(&vec![0; 8192]).unwrap();
        source.macros[3].macro_actions = macro_action(99);
        let mut current = MacroBank::decode(&vec![0; 8192]).unwrap();
        for (index, item) in current.macros.iter_mut().enumerate() {
            item.macro_actions = macro_action(index as u16 + 4);
        }
        let mut profile = baseline(0);
        profile.layers[0][88] = macro_ref(3);
        let before = profile.clone();
        assert!(merge_profile_macros(&mut profile, Some(&source), &current).is_err());
        assert_eq!(profile, before);
        assert!(merge_profile_macros(&mut profile, None, &current).is_err());
        assert_eq!(profile, before);
    }
    #[test]
    fn scoped_macro_merge_preserves_inherited_refs_and_requires_imported_definitions() {
        let current = MacroBank::decode(&vec![0; 8192]).unwrap();
        let mut profile = baseline(0);
        profile.layers[0][88] = macro_ref(5);
        profile.layers[0][93] = macro_ref(4);
        profile.advanced.mt[0].click = macro_ref(3);
        let before = profile.clone();
        let mut scope = HubProfileScope {
            performance: true,
            ..Default::default()
        };
        assert_eq!(
            merge_scoped_profile_macros(&mut profile, None, &current, Some(&scope)).unwrap(),
            current
        );
        assert_eq!(profile, before);
        scope.keys = true;
        assert!(merge_scoped_profile_macros(&mut profile, None, &current, Some(&scope)).is_err());
        let mut source = current.clone();
        source.macros[5].macro_actions = macro_action(9);
        merge_scoped_profile_macros(&mut profile, Some(&source), &current, Some(&scope)).unwrap();
        assert_eq!(profile.layers[0][88].code1, 0);
        assert_eq!(profile.layers[0][93], before.layers[0][93]);
        assert_eq!(profile.advanced, before.advanced);
    }

    #[test]
    fn profile_rebase_updates_fn_in_every_group_and_side_markers() {
        let mut profile = baseline(0);
        let fn_key = KeyAssignment {
            kind: 240,
            code1: 255,
            code2: 1,
        };
        profile.advanced.dks[0].actions[0] = fn_key;
        profile.advanced.mt[0] = MtKey {
            click: fn_key,
            hold: fn_key,
        };
        profile.advanced.tgl[0] = fn_key;
        rebase_profile(&mut profile, 3).unwrap();
        assert_eq!(profile.index, 3);
        assert_eq!(profile.functions.performance.mac_mode, 12);
        assert_eq!(profile.layers[0][22].code2, 13);
        assert_eq!(
            profile.layers[2][100],
            KeyAssignment {
                kind: 254,
                code1: 254,
                code2: 254
            }
        );
        assert_eq!(profile.advanced.dks[0].actions[0].code2, 13);
        assert_eq!(profile.advanced.mt[0].click.code2, 13);
        assert_eq!(profile.advanced.mt[0].hold.code2, 13);
        assert_eq!(profile.advanced.tgl[0].code2, 13);
    }
    #[test]
    fn original_sdk_macro_mode_change_and_clear_reference_fallbacks() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/ace75/hub-profile-sdk-vectors.json"
        ))
        .unwrap();
        for case in fixture["macroCases"].as_array().unwrap() {
            let mut profile = baseline(0);
            profile.layers[2][88] = macro_ref(3);
            profile.advanced.dks[0].actions = [macro_ref(3); 4];
            profile.advanced.mt[0] = MtKey {
                click: macro_ref(3),
                hold: macro_ref(3),
            };
            profile.advanced.tgl[0] = macro_ref(3);
            let mut bank = MacroBank::decode(&vec![0; 8192]).unwrap();
            bank.macros[3].mode = 1;
            if !case["clear"].as_bool().unwrap() {
                bank.macros[3].macro_actions = macro_action(4);
            }
            let result = plan_macro_reference_updates(&profile, factory_layers(), &bank).unwrap();
            let expected: Vec<Vec<[u8; 3]>> =
                serde_json::from_value(case["expectedKeys"].clone()).unwrap();
            for (actual, expected) in result.layers.iter().zip(expected) {
                assert_eq!(KeyAssignment::encode_all(actual), expected.concat());
            }
            let expected_dks: Vec<[u8; 3]> =
                serde_json::from_value(case["expectedDks"].clone()).unwrap();
            assert_eq!(
                KeyAssignment::encode_all(&result.advanced.dks[0].actions),
                expected_dks.concat()
            );
            let expected_mt: Vec<[u8; 3]> =
                serde_json::from_value(case["expectedMt"].clone()).unwrap();
            assert_eq!(
                KeyAssignment::encode_all(&[
                    result.advanced.mt[0].click,
                    result.advanced.mt[0].hold
                ]),
                expected_mt.concat()
            );
            let expected_tgl: Vec<[u8; 3]> =
                serde_json::from_value(case["expectedTgl"].clone()).unwrap();
            assert_eq!(
                KeyAssignment::encode_all(&[result.advanced.tgl[0]]),
                expected_tgl.concat()
            );
        }
    }
    #[test]
    fn scoped_rebase_preserves_current_global_keys_and_reserved_bytes() {
        let imported = baseline(0);
        let mut current = imported.clone();
        current.layers[0][93] = KeyAssignment {
            kind: 16,
            code1: 0,
            code2: 9,
        };
        current.advanced.mt_reserved[0] = 17;
        current.advanced.tgl_reserved[0] = 18;
        current.advanced.dks[0].reserved_masks[0] = 4;
        current.custom_params.raw[55] = 19;
        current.custom_raw[CUSTOM_PARAMS_OFFSET + 55] = 19;
        let scope = HubProfileScope {
            keys: true,
            advanced: true,
            custom_params: true,
            ..Default::default()
        };
        let merged = scope.merge_into(&imported, &current).unwrap();
        assert_eq!(merged.layers[0][93], current.layers[0][93]);
        assert_eq!(merged.advanced.mt_reserved, current.advanced.mt_reserved);
        assert_eq!(merged.advanced.tgl_reserved, current.advanced.tgl_reserved);
        assert_eq!(
            merged.advanced.dks[0].reserved_masks,
            current.advanced.dks[0].reserved_masks
        );
        assert_eq!(merged.custom_raw[CUSTOM_PARAMS_OFFSET + 55], 19);
    }
    #[test]
    fn original_next_legacy_converter_all_six_advanced_types_and_combo() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/ace75/hub-profile-legacy-sdk-vector.json"
        ))
        .unwrap();
        let document =
            HubProfileDocument::from_json(&serde_json::to_vec(&fixture["legacy"]).unwrap())
                .unwrap();
        let mut before = baseline(0);
        before.functions.performance.rt_smart = 5;
        before.functions = FunctionConfig::decode(
            &before
                .functions
                .with_performance(&before.functions.performance, false)
                .unwrap(),
        )
        .unwrap();
        let imported = import_hub_profile(&document, &before).unwrap();
        for (actual, expected) in imported
            .profile
            .layers
            .iter()
            .zip(fixture["expected"]["userKeys"].as_array().unwrap())
        {
            for (actual, expected) in actual.iter().zip(expected.as_array().unwrap()) {
                assert_eq!(actual.kind as u64, expected["type"]);
                assert_eq!(actual.code1 as u64, expected["code1"]);
                assert_eq!(actual.code2 as u64, expected["code2"]);
            }
        }
        let expected_dks: Vec<u8> =
            serde_json::from_value(fixture["expectedDksBytes"].clone()).unwrap();
        assert_eq!(
            imported.profile.advanced.dks[0].encode().unwrap(),
            expected_dks
        );
        let expected_mt = fixture["expected"]["advancedKeys"]["mt"]
            .as_array()
            .unwrap();
        for (actual, expected) in imported.profile.advanced.mt.iter().zip(expected_mt) {
            assert_eq!(
                actual.click,
                import_assignment(&expected["clickKey"]).unwrap()
            );
            assert_eq!(
                actual.hold,
                import_assignment(&expected["downKey"]).unwrap()
            );
        }
        assert_eq!(imported.profile.custom_params.combo_indices[0], vec![52]);
        assert_eq!(imported.profile.custom_params.rt_smart_cache, 5);
        assert!(imported.profile.functions.performance.lock_win);
        assert!(!imported.profile.functions.lighting.main.rainbow);
        assert!(!imported.profile.functions.lighting.side.rainbow);
        assert_eq!(imported.profile.colors, vec![[0x12, 0x34, 0x56]; 128]);
        assert_eq!(imported.document, document);
    }

    #[test]
    fn legacy_missing_layers_use_factory_and_wrong_model_fails() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/ace75/hub-profile-legacy-sdk-vector.json"
        ))
        .unwrap();
        let mut legacy = fixture["legacy"].clone();
        legacy["userKeys"] = Value::Array(vec![Value::Null]);
        legacy["advancedKeys"] = Value::Array(vec![]);
        legacy["colorKeys"] = Value::Array(vec![]);
        let document =
            HubProfileDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        let imported = import_hub_profile(&document, &baseline(2)).unwrap();
        assert_eq!(imported.profile.layers[0][88].code2, 4);
        assert_eq!(imported.profile.layers[0][22].code2, 9);
        assert_eq!(imported.profile.colors, vec![[0; 3]; 128]);
        legacy["productInfo"]["productName"] = Value::from("Ace 75 16K");
        let document =
            HubProfileDocument::from_json(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(import_hub_profile(&document, &baseline(0)).is_err());
    }
    #[test]
    fn macro_merge_reserves_other_profiles_and_inherited_empty_references() {
        let mut other = baseline(1);
        other.advanced.tgl[0] = macro_ref(0);
        let reserved = macro_slots(&other);
        assert_eq!(reserved, BTreeSet::from([0]));
        let current = MacroBank::decode(&vec![0; 8192]).unwrap();
        let mut source = current.clone();
        source.macros[3].macro_actions = macro_action(4);
        let mut profile = baseline(0);
        profile.layers[0][88] = macro_ref(3);
        profile.layers[0][93] = macro_ref(1);
        profile.advanced.mt[0].click = macro_ref(2);
        let scope = HubProfileScope {
            keys: true,
            ..Default::default()
        };
        let bank = merge_scoped_profile_macros_reserved(
            &mut profile,
            Some(&source),
            &current,
            Some(&scope),
            &reserved,
        )
        .unwrap();
        assert_eq!(profile.layers[0][88].code1, 3);
        assert_eq!(profile.layers[0][93].code1, 1);
        assert_eq!(profile.advanced.mt[0].click.code1, 2);
        for index in 0..3 {
            assert!(bank.macros[index].macro_actions.is_empty());
        }
        assert_eq!(bank.macros[3].macro_actions, macro_action(4));
    }

    #[test]
    fn protected_empty_slots_cannot_be_allocated_but_unchanged_definition_can_be_reused() {
        let mut current = MacroBank::decode(&vec![0; 8192]).unwrap();
        let mut source = current.clone();
        source.macros[3].macro_actions = macro_action(4);
        let mut profile = baseline(0);
        profile.layers[0][88] = macro_ref(3);
        let before = profile.clone();
        let reserved = (0..16).collect();
        assert!(
            merge_scoped_profile_macros_reserved(
                &mut profile,
                Some(&source),
                &current,
                None,
                &reserved
            )
            .is_err()
        );
        assert_eq!(profile, before);
        current.macros[7].macro_actions = macro_action(4);
        let merged = merge_scoped_profile_macros_reserved(
            &mut profile,
            Some(&source),
            &current,
            None,
            &reserved,
        )
        .unwrap();
        assert_eq!(profile.layers[0][88].code1, 7);
        assert_eq!(merged, current);
    }
}
