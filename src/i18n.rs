use std::{collections::BTreeMap, sync::{OnceLock, atomic::{AtomicU8, Ordering}}};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    Ru,
    En,
}

impl Language {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ru" | "RU" => Some(Self::Ru),
            "en" | "EN" => Some(Self::En),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self { Self::Ru => "ru", Self::En => "en" }
    }
}

static LANGUAGE: AtomicU8 = AtomicU8::new(0);

pub fn set_language(value: Language) {
    LANGUAGE.store(u8::from(value == Language::En), Ordering::Relaxed);
}

pub fn language() -> Language {
    if LANGUAGE.load(Ordering::Relaxed) == 1 { Language::En } else { Language::Ru }
}

fn catalog() -> &'static BTreeMap<String, String> {
    static CATALOG: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut values: BTreeMap<String, String> = serde_json::from_str(include_str!("../assets/locales/backend-en.json")).expect("Embedded backend translations");
        values.extend(serde_json::from_str::<BTreeMap<String, String>>(include_str!("../assets/locales/ui-en.json")).expect("Embedded interface translations"));
        values.extend(serde_json::from_str::<BTreeMap<String, String>>(include_str!("../assets/locales/ace75-backend-en.json")).expect("Embedded Ace75 translations"));
        values
    })
}

pub fn t(text: &'static str) -> &'static str {
    if language() == Language::En {
        catalog().get(text).map(String::as_str).unwrap_or(text)
    } else {
        text
    }
}

#[derive(Clone, Debug)]
enum Part {
    Text(String),
    Argument(String),
}

fn parts(template: &str) -> Vec<Part> {
    let mut result = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    let mut index = 0;
    while let Some(ch) = chars.next() {
        if ch == '{' && chars.peek() == Some(&'{') {
            chars.next();
            text.push('{');
        } else if ch == '}' && chars.peek() == Some(&'}') {
            chars.next();
            text.push('}');
        } else if ch == '{' {
            if !text.is_empty() { result.push(Part::Text(std::mem::take(&mut text))); }
            let mut field = String::new();
            for next in chars.by_ref() {
                if next == '}' { break; }
                field.push(next);
            }
            let field = field.split(':').next().unwrap_or_default();
            let name = if field.is_empty() {
                let name = index.to_string();
                index += 1;
                name
            } else {
                field.to_owned()
            };
            result.push(Part::Argument(name));
        } else {
            text.push(ch);
        }
    }
    if !text.is_empty() { result.push(Part::Text(text)); }
    result
}

struct Template {
    source: Vec<Part>,
    translated: Vec<Part>,
    literal_length: usize,
}

fn templates() -> &'static Vec<Template> {
    static TEMPLATES: OnceLock<Vec<Template>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        let mut values = catalog().iter().filter_map(|(ru, en)| {
            let source = parts(ru);
            if !source.iter().any(|part| matches!(part, Part::Argument(_))) { return None; }
            let literal_length = source.iter().map(|part| match part { Part::Text(text) => text.len(), _ => 0 }).sum();
            if literal_length == 0 { return None; }
            Some(Template { source, translated: parts(en), literal_length })
        }).collect::<Vec<_>>();
        values.sort_by_key(|item| std::cmp::Reverse(item.literal_length));
        values
    })
}

fn capture(parts: &[Part], input: &str, values: &mut BTreeMap<String, String>) -> bool {
    let Some((first, rest)) = parts.split_first() else { return input.is_empty(); };
    match first {
        Part::Text(text) => input.strip_prefix(text.as_str()).is_some_and(|tail| capture(rest, tail, values)),
        Part::Argument(name) => {
            let ends = match rest.first() {
                Some(Part::Text(next)) => input.match_indices(next.as_str()).map(|(index, _)| index).collect::<Vec<_>>(),
                None => vec![input.len()],
                Some(Part::Argument(_)) => return false,
            };
            for end in ends {
                let value = &input[..end];
                if values.get(name).is_some_and(|previous| previous != value) { continue; }
                let mut next = values.clone();
                next.insert(name.clone(), value.to_owned());
                if capture(rest, &input[end..], &mut next) {
                    *values = next;
                    return true;
                }
            }
            false
        }
    }
}

fn english_message(text: &str, depth: u8) -> String {
    if let Some(value) = catalog().get(text) { return value.clone(); }
    if depth >= 8 { return text.to_owned(); }
    for template in templates() {
        let mut values = BTreeMap::new();
        if !capture(&template.source, text, &mut values) { continue; }
        let mut output = String::new();
        for part in &template.translated {
            match part {
                Part::Text(literal) => output.push_str(literal),
                Part::Argument(name) => {
                    let Some(value) = values.get(name) else { return text.to_owned(); };
                    // Only error parameters contain another system message. Names and paths stay literal.
                    if matches!(name.as_str(), "error" | "err" | "e" | "source" | "advice" | "restore" | "second" | "cleanup" | "flash_error" | "enter_error") {
                        output.push_str(&english_message(value, depth + 1));
                    } else {
                        output.push_str(value);
                    }
                }
            }
        }
        return output;
    }
    if let Some((context, detail)) = text.split_once(": ") {
        if let Some(translated) = catalog().get(context) {
            return format!("{translated}: {}", english_message(detail, depth + 1));
        }
    }
    if text.contains('\n') {
        return text.split('\n').map(|line| english_message(line, depth + 1)).collect::<Vec<_>>().join("\n");
    }
    text.to_owned()
}

pub fn message(text: &str) -> String {
    message_for(language(), text)
}

pub fn message_for(locale: Language, text: &str) -> String {
    match locale { Language::Ru => text.to_owned(), Language::En => english_message(text, 0) }
}

pub fn error_chain(contexts: &[String]) -> String {
    contexts.iter().map(|context| message(context)).collect::<Vec<_>>().join(": ")
}

pub fn load_preference() -> Language {
    let path = crate::controller::data_dir().join("ui-preferences.json");
    std::fs::read(path).ok()
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|value| value.get("language").and_then(|value| value.as_str()).and_then(Language::parse))
        .unwrap_or_default()
}

pub fn save_preference(locale: Language) -> std::io::Result<()> {
    save_ui_preference("language", serde_json::Value::String(locale.code().into()))
}

pub(crate) fn save_ui_preference(key: &str, setting: serde_json::Value) -> std::io::Result<()> {
    use std::io::Write;
    let directory = crate::controller::data_dir();
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("ui-preferences.json");
    let mut value = match std::fs::read(&path) {
        Ok(data) => serde_json::from_slice::<serde_json::Value>(&data).map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(error) => return Err(error),
    };
    let object = value.as_object_mut().ok_or_else(|| std::io::Error::other("Invalid UI preferences"))?;
    object.insert(key.into(), setting);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let pending = directory.join(format!(".ui-preferences-{}-{stamp}.json", std::process::id()));
    let mut file = std::fs::OpenOptions::new().create_new(true).write(true).open(&pending)?;
    file.write_all(&serde_json::to_vec_pretty(&value).map_err(std::io::Error::other)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(pending, path)
}

#[macro_export]
macro_rules! tf {
    ($($args:tt)*) => { $crate::i18n::message(&format!($($args)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_preserve_all_argument_names_and_occurrences() {
        for (ru, en) in catalog() {
            let arguments = |text: &str| {
                let mut result = parts(text).into_iter().filter_map(|part| match part { Part::Argument(name) => Some(name), _ => None }).collect::<Vec<_>>();
                result.sort(); result
            };
            assert_eq!(arguments(ru), arguments(en), "Translation placeholders: {ru}");
        }
    }

    #[test]
    fn user_names_and_paths_are_never_translated_as_parameters() {
        assert_eq!(message_for(Language::En, "Открыт локальный макрос «Нет данных / Новый макрос»"), "Opened local macro «Нет данных / Новый макрос»");
        assert_eq!(message_for(Language::En, "Макрос сохранён: C:\\Пользователи\\Мышь\\Нет данных.json"), "Macro saved: C:\\Пользователи\\Мышь\\Нет данных.json");
    }

    #[test]
    fn runtime_status_and_nested_errors_follow_the_requested_language() {
        assert_eq!(message_for(Language::En, "Зарядка: 50% — выбранный эффект временно приостановлен."), "Charging: 50% — the selected effect is temporarily paused.");
        assert_eq!(message_for(Language::En, "Запись событий остановлена: Достигнут предел размера макроса."), "Event recording stopped: Macro size limit reached.");
        assert_eq!(message_for(Language::Ru, "Макрос сохранён: Мышь.json"), "Макрос сохранён: Мышь.json");
    }

    #[test]
    fn locale_codes_are_explicit_and_russian_is_the_default() {
        assert_eq!(Language::default(), Language::Ru);
        assert_eq!(Language::parse("en"), Some(Language::En));
        assert_eq!(Language::parse("RU"), Some(Language::Ru));
        assert_eq!(Language::parse("de"), None);
    }

    #[test]
    fn hid_access_advice_is_translated_without_changing_the_device_path() {
        for (advice, expected_advice) in [
            ("Проверьте подключение и права hidraw", "Check the connection and HID permissions"),
            ("Проверьте подключение и закройте другие программы управления MCHOSE", "Check the connection and close other MCHOSE configuration applications"),
        ] {
            let source = format!("Нет доступа к Моя мышь (C:\\Нет данных\\Мышь). {advice}");
            assert_eq!(message_for(Language::En, &source), format!("Cannot access Моя мышь (C:\\Нет данных\\Мышь). {expected_advice}"));
        }
    }

    #[test]
    fn structured_error_contexts_preserve_paths_and_translate_the_cause() {
        let contexts = ["Не удалось прочитать C:\\Нет данных\\Мышь.json", "Мышь не подключена"];
        let output = contexts.iter().map(|context| message_for(Language::En, context)).collect::<Vec<_>>().join(": ");
        assert_eq!(output, "Could not read C:\\Нет данных\\Мышь.json: The mouse is not connected");
        assert_eq!(message_for(Language::En, "MagDock: Мышь не подключена"), "MagDock: The mouse is not connected");
        assert_eq!(message_for(Language::En, "WASAPI: Не удалось инициализировать COM"), "WASAPI: Could not initialize COM");
        assert_eq!(message_for(Language::En, "Не удалось восстановить подсветку: Мышь не подключена"), "Could not restore the lighting: The mouse is not connected");
        assert_eq!(message_for(Language::En, "ACE75: восстановление настроек не подтверждено: ACE75 не подключена"), "ACE75: settings restoration was not confirmed: ACE75 is not connected");
    }

    #[test]
    fn every_russian_interface_literal_has_a_catalog_entry() {
        let source = [include_str!("ui.rs"), include_str!("ui/ace75.rs"), include_str!("ui/ace75/settings.rs"), include_str!("ui/ace75/lighting.rs"), include_str!("ui/ace75/components.rs"), include_str!("ui/ace75/diagram.rs")].join("\n");
        assert_translated_literals(&source);
    }

    #[test]
    fn every_russian_keyboard_backend_literal_has_a_catalog_entry() {
        let source = [include_str!("ace75.rs"), include_str!("keyboard_controller.rs"),
            include_str!("ace75/advanced.rs"), include_str!("ace75/advanced_io.rs"),
            include_str!("ace75/bootloader.rs"), include_str!("ace75/model.rs"),
            include_str!("ace75/operations.rs"), include_str!("ace75/hub_profile.rs"),
            include_str!("ace75/firmware_catalog.rs"), include_str!("ace75/firmware_service.rs"),
            include_str!("ace75/host_lighting.rs")].join("\n");
        assert_translated_literals(&source);
    }

    fn assert_translated_literals(source: &str) {
        let mut start = None;
        let mut escaped = false;
        for (index, ch) in source.char_indices() {
            if let Some(begin) = start {
                if escaped { escaped = false; continue; }
                if ch == '\\' { escaped = true; }
                if ch == '"' {
                    let literal = &source[begin..=index];
                    if let Ok(text) = serde_json::from_str::<String>(literal) {
                        if text.chars().any(|ch| ('\u{0400}'..='\u{04ff}').contains(&ch)) {
                            assert!(catalog().contains_key(&text), "Missing UI translation: {text}");
                        }
                    }
                    start = None;
                }
            } else if ch == '"' {
                start = Some(index);
            }
        }
    }
}
