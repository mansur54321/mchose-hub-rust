use crate::mouse::{KeyBinding, MacroEvent, MouseMacro, MouseSnapshot};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;

pub const HUB_MODEL: &str = "MCHOSE A7 V3 Ultra+";
const BUTTONS: [&str; 6] = ["left", "right", "mid", "forward", "retreat", "dpi"];

fn keyboard_names() -> &'static BTreeMap<String, u8> {
    static NAMES: OnceLock<BTreeMap<String, u8>> = OnceLock::new();
    NAMES.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/hub-keycodes.json"))
            .expect("embedded Hub HID key map")
    })
}

fn number(value: &Value) -> Result<u64> {
    if let Some(number) = value.as_u64() {
        return Ok(number);
    }
    if let Some(text) = value.as_str() {
        return text.parse().context("Ожидалось целое положительное число");
    }
    bail!("Ожидалось целое положительное число")
}

fn field(data: &Map<String, Value>, name: &str) -> Result<u64> {
    number(
        data.get(name)
            .with_context(|| format!("В профиле отсутствует {name}"))?,
    )
    .with_context(|| name.to_owned())
}
fn byte(data: &Map<String, Value>, name: &str) -> Result<u8> {
    u8::try_from(field(data, name)?).with_context(|| format!("{name} превышает один байт"))
}
fn word(data: &Map<String, Value>, name: &str) -> Result<u16> {
    u16::try_from(field(data, name)?).with_context(|| format!("{name} превышает два байта"))
}

fn hex_bytes(text: &str) -> Result<Vec<u8>> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    ensure!(
        text.len() % 2 == 0 && text.is_ascii(),
        "Неверная шестнадцатеричная строка"
    );
    text.as_bytes()
        .chunks_exact(2)
        .map(|part| {
            u8::from_str_radix(std::str::from_utf8(part)?, 16)
                .context("Неверная шестнадцатеричная строка")
        })
        .collect()
}

fn binding_from_hub(data: &Map<String, Value>, name: &str) -> Result<KeyBinding> {
    let kind_name = format!("{name}KeyType");
    let kind_text = data
        .get(&kind_name)
        .and_then(Value::as_str)
        .with_context(|| format!("Нет {kind_name}"))?;
    let kind = u8::from_str_radix(kind_text.trim_start_matches("0x"), 16)?;
    let value_name = format!("{name}KeyVal");
    let text = data.get(&value_name).and_then(Value::as_str).unwrap_or("");
    let mut bytes = hex_bytes(text)?;
    if kind != 0x13 && kind != 0x16 {
        bytes.reverse();
    }
    let binding = KeyBinding { kind, data: bytes };
    binding.encode()?;
    Ok(binding)
}

pub fn profile_from_hub_json(value: &Value, base: &MouseSnapshot) -> Result<MouseSnapshot> {
    let wrapper = value
        .as_object()
        .context("Профиль Hub должен быть объектом JSON")?;
    ensure!(wrapper.len() == 1, "Ожидался один профиль с именем модели");
    let data = wrapper
        .get(HUB_MODEL)
        .and_then(Value::as_object)
        .with_context(|| format!("Профиль должен принадлежать {HUB_MODEL}"))?;
    let mut result = base.clone();
    let profile = byte(data, "profileIndex")?;
    let settings = &mut result.settings;
    settings.profile_index = profile;
    settings.dpi_index = byte(data, "usbDpiIndex")?;
    settings.usb_rate_index = byte(data, "usbRateIndex")?;
    settings.wireless_rate_index = byte(data, "gRateIndex")?;
    settings.sleep_minutes = byte(data, "sleep")?;
    settings.sleep_disabled = byte(data, "sleepMode")? & 1 != 0;
    settings.sensor = byte(data, "sensor")?;
    settings.angle = byte(data, "rotateVal")? as i8;
    settings.left_debounce_ms = byte(data, "leftKeyDebounce")?;
    settings.right_debounce_ms = byte(data, "rightKeyDebounce")?;
    settings.encode()?;
    result.dpi.profile_index = profile;
    result.dpi.count = byte(data, "dpiSum")?;
    result.dpi.selected = settings.dpi_index;
    result.dpi.separate_y = byte(data, "dpiHasY")? != 0;
    for index in 0..6 {
        result.dpi.x[index] = word(data, &format!("dpi{index}"))?;
        result.dpi.y[index] = if result.dpi.separate_y {
            word(data, &format!("dpiY{index}"))?
        } else {
            result.dpi.x[index]
        };
    }
    result.dpi.encode(0)?;
    if result.dpi.separate_y {
        result.dpi.encode(1)?;
    }
    result.lod_index = byte(data, "lodIndex")?;
    ensure!(result.lod_index < 5, "Неверный индекс LOD");
    result.keys = BUTTONS
        .iter()
        .enumerate()
        .map(|(index, name)| {
            if index == 5 && !data.contains_key("dpiKeyType") {
                return Ok(KeyBinding {
                    kind: 255,
                    data: Vec::new(),
                });
            }
            binding_from_hub(data, name)
        })
        .collect::<Result<Vec<_>>>()?;
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .context("Нет имени профиля")?;
    ensure!(
        !name.is_empty() && name.len() <= 60,
        "Имя профиля должно занимать 1…60 байт UTF-8"
    );
    result.profile_names.resize(3, String::new());
    result.profile_names[profile as usize] = name.into();
    Ok(result)
}

pub fn profile_to_hub_json(snapshot: &MouseSnapshot) -> Value {
    let settings = &snapshot.settings;
    let dpi = &snapshot.dpi;
    let name = snapshot
        .profile_names
        .get(settings.profile_index as usize)
        .filter(|name| !name.is_empty())
        .cloned()
        .unwrap_or_else(|| format!("Profile {}", settings.profile_index + 1));
    let mut data = json!({
        "name": name, "profileIndex": settings.profile_index, "usbDpiIndex": settings.dpi_index,
        "usbRateIndex": settings.usb_rate_index, "gRateIndex": settings.wireless_rate_index,
        "sleep": settings.sleep_minutes, "sleepMode": u8::from(settings.sleep_disabled),
        "sensor": settings.sensor, "rotateVal": settings.angle as u8,
        "leftKeyDebounce": settings.left_debounce_ms, "rightKeyDebounce": settings.right_debounce_ms,
        "dpiDirection": 0, "dpiHasY": u8::from(dpi.separate_y), "dpiSum": dpi.count,
        "lodIndex": snapshot.lod_index, "macroInfo": {}
    }).as_object().unwrap().clone();
    for index in 0..6 {
        data.insert(format!("dpi{index}"), json!(dpi.x[index]));
        if dpi.separate_y {
            data.insert(format!("dpiY{index}"), json!(dpi.y[index]));
        }
    }
    for (name, binding) in BUTTONS.iter().zip(&snapshot.keys) {
        let mut bytes = binding.data.clone();
        if binding.kind != 0x13 && binding.kind != 0x16 {
            bytes.reverse();
        }
        data.insert(
            format!("{name}KeyType"),
            json!(format!("{:02x}", binding.kind)),
        );
        data.insert(
            format!("{name}KeyVal"),
            json!(bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()),
        );
    }
    json!({ HUB_MODEL: data })
}

pub fn hub_profile_macro_names(value: &Value) -> Result<BTreeMap<u8, String>> {
    let Some(data) = value
        .get(HUB_MODEL)
        .and_then(|v| v.get("macroInfo"))
        .and_then(Value::as_object)
    else {
        return Ok(BTreeMap::new());
    };
    data.iter()
        .map(|(key, value)| {
            let id = u8::from_str_radix(key, 16)?;
            ensure!(id < 18, "Неверный слот макроса {id}");
            Ok((
                id,
                value.as_str().context("Ожидалось имя макроса")?.to_owned(),
            ))
        })
        .collect()
}

pub fn macro_from_hub_json(value: &Value) -> Result<MouseMacro> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .context("Макрос не содержит имени")?;
    let repeat_mode = u8::try_from(number(
        value.get("type").context("Макрос не содержит type")?,
    )?)?;
    let key_list = value
        .get("info")
        .and_then(|v| v.get("keyList"))
        .and_then(Value::as_array)
        .context("Макрос не содержит info.keyList")?;
    let events = key_list
        .iter()
        .map(|item| {
            let kind = item
                .get("type")
                .and_then(Value::as_str)
                .context("Событие не содержит type")?;
            let val = item.get("val").context("Событие не содержит val")?;
            let pressed = item.get("press").and_then(Value::as_str) == Some("down");
            let (kind, value, delay_ms) = match kind {
                "time" => (
                    63,
                    0,
                    u16::try_from(number(val)?).context("Задержка превышает 65535 мс")?,
                ),
                "key" => {
                    let name = val.as_str().context("Имя клавиши должно быть строкой")?;
                    (
                        17,
                        *keyboard_names()
                            .get(name)
                            .with_context(|| format!("Неизвестная клавиша Hub: {name}"))?,
                        0,
                    )
                }
                "mouse" => (
                    0,
                    match val.as_str() {
                        Some("mouseLeft") => 1,
                        Some("mouseRight") => 2,
                        Some("mouseMid") => 4,
                        Some("mouseBack") => 8,
                        Some("mouseForward") => 16,
                        _ => bail!("Неизвестная кнопка мыши"),
                    },
                    0,
                ),
                "mousescroll" => (
                    5,
                    match val.as_str() {
                        Some("mouseScrollUp") => 0,
                        Some("mouseScrollDown") => 1,
                        _ => bail!("Неизвестное направление колеса"),
                    },
                    0,
                ),
                _ => bail!("Неизвестный тип события Hub: {kind}"),
            };
            Ok(MacroEvent {
                delay_ms,
                kind,
                pressed,
                value,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let result = MouseMacro {
        name: name.to_owned(),
        repeat_mode,
        events,
        reserved: [0; 5],
    };
    result.encode()?;
    Ok(result)
}

pub fn macro_to_hub_json(value: &MouseMacro) -> Result<Value> {
    let mut keys = Vec::new();
    for event in &value.events {
        ensure!(
            event.kind == 63 || event.delay_ms == 0,
            "Формат Hub не поддерживает задержку внутри события клавиши"
        );
        let (kind, val) = match event.kind {
            63 => ("time", json!(event.delay_ms)),
            17 => {
                let name = keyboard_names()
                    .iter()
                    .find(|(_, usage)| **usage == event.value)
                    .map(|(name, _)| name)
                    .with_context(|| {
                        format!("В формате Hub отсутствует клавиша HID {}", event.value)
                    })?;
                ("key", json!(name))
            }
            0 => (
                "mouse",
                json!(match event.value {
                    1 => "mouseLeft",
                    2 => "mouseRight",
                    4 => "mouseMid",
                    8 => "mouseBack",
                    16 => "mouseForward",
                    _ => bail!("Неизвестная кнопка мыши"),
                }),
            ),
            5 => (
                "mousescroll",
                json!(match event.value {
                    0 => "mouseScrollUp",
                    1 => "mouseScrollDown",
                    _ => bail!("Неизвестное направление колеса"),
                }),
            ),
            _ => bail!("Тип события {} отсутствует в формате Hub", event.kind),
        };
        keys.push(
            json!({ "type": kind, "val": val, "press": if event.pressed { "down" } else { "up" } }),
        );
    }
    Ok(
        json!({"name": value.name, "type": value.repeat_mode, "delay": 50, "info": {"keyList": keys}}),
    )
}

pub fn macros_from_hub_json(value: &Value) -> Result<Vec<MouseMacro>> {
    if let Some(list) = value.as_array() {
        return list.iter().map(macro_from_hub_json).collect();
    }
    if value.get("name").is_some() && value.get("type").is_some() {
        return Ok(vec![macro_from_hub_json(value)?]);
    }
    if let Some(wrapper) = value.as_object() {
        for item in wrapper.values() {
            if item.is_array() {
                return macros_from_hub_json(item);
            }
        }
    }
    bail!("Файл не содержит макросов M Hub")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_hub_macro_roundtrip_preserves_device_events() {
        let original = json!({"name":"Проверка", "type":2, "info":{"keyList":[
            {"type":"key","val":"a","press":"down"},
            {"type":"time","val":120,"press":"up"},
            {"type":"key","val":"A","press":"up"},
            {"type":"mouse","val":"mouseForward","press":"down"},
            {"type":"mousescroll","val":"mouseScrollDown","press":"up"}
        ]}});
        let decoded = macro_from_hub_json(&original).unwrap();
        let encoded = macro_to_hub_json(&decoded).unwrap();
        assert_eq!(macro_from_hub_json(&encoded).unwrap(), decoded);
        assert_eq!(&decoded.encode().unwrap()[64..68], &[0, 0, 0x51, 4]);
        assert!(macro_from_hub_json(&json!({"name":"bad","type":0,"info":{"keyList":[{"type":"key","val":"not a key"}]}})).is_err());
    }

    #[test]
    fn original_profile_roundtrip_preserves_wire_bindings_and_values() {
        let evidence: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/device-snapshot.json"
        ))
        .unwrap();
        let mut original: MouseSnapshot =
            serde_json::from_value(evidence["mouse"].clone()).unwrap();
        original.profile_names[0] = "Profile 1".into();
        original.keys[1] = KeyBinding::chord(5, 4);
        original.keys[2] = KeyBinding::consumer(0xe9);
        original.settings.angle = -12;
        let json = profile_to_hub_json(&original);
        assert_eq!(json[HUB_MODEL]["rightKeyVal"], "0504");
        assert_eq!(json[HUB_MODEL]["midKeyVal"], "00e9");
        let restored = profile_from_hub_json(&json, &original).unwrap();
        assert_eq!(restored.settings, original.settings);
        assert_eq!(restored.keys, original.keys);
        assert_eq!(restored.dpi, original.dpi);
        assert!(profile_from_hub_json(&json!({"OTHER MODEL": {}}), &original).is_err());
    }
}
