use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
    time::Duration,
};

pub const KEYBOARD_CATALOG_URL: &str =
    "https://cdn.mchose.com.cn/configCenter/custom/keyboardConfig.json";
pub const KEYBOARD_MODEL_KEY: &str = "0x3837_0x303C";
const BINARY_PREFIX: &str = "https://cdn.mchose.com.cn/configCenter/static/binaries/";
const MAX_CATALOG_BYTES: usize = 4_000_000;
pub const MAX_KEYBOARD_IMAGE_BYTES: usize = 8_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KeyboardFirmwareInfo {
    pub version: String,
    pub url: String,
    pub bytes: Option<usize>,
    pub sha256: Option<String>,
}

fn binary_filename(url: &str) -> Result<&str> {
    let filename = url
        .strip_prefix(BINARY_PREFIX)
        .context("ACE75: прошивка должна находиться на официальном CDN MCHOSE")?;
    ensure!(
        !filename.is_empty()
            && filename.len() <= 255
            && filename.ends_with(".bin")
            && !filename.contains("..")
            && filename
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')),
        "ACE75: недопустимый путь образа прошивки"
    );
    let model: String = filename
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    ensure!(
        model.starts_with("ace758k")
            && !model.contains("16k")
            && !model.contains("dongle")
            && !model.contains("upgrader"),
        "ACE75: образ не предназначен для клавиатуры Ace 75 8K"
    );
    Ok(filename)
}

fn get_official(url: &str, max_bytes: usize) -> Result<Vec<u8>> {
    if url != KEYBOARD_CATALOG_URL {
        binary_filename(url)?;
    }
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(45)))
        .max_redirects(0)
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .call()
        .context("ACE75: не удалось получить файл с официального CDN")?;
    ensure!(
        response.status().as_u16() == 200,
        "ACE75: CDN вернул HTTP {}; перенаправления запрещены",
        response.status()
    );
    if let Some(length) = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        ensure!(
            length <= max_bytes as u64,
            "ACE75: файл CDN превышает допустимый размер"
        );
    }
    Ok(response
        .body_mut()
        .with_config()
        .limit(max_bytes as u64)
        .read_to_vec()
        .context("ACE75: не удалось прочитать ограниченный по размеру ответ CDN")?)
}

pub fn keyboard_firmware_from_catalog(bytes: &[u8]) -> Result<KeyboardFirmwareInfo> {
    ensure!(
        bytes.len() <= MAX_CATALOG_BYTES,
        "ACE75: каталог слишком большой"
    );
    let wrapper: Value = serde_json::from_slice(bytes).context("ACE75: неверный JSON каталога")?;
    let catalog = if let Some(encoded) = wrapper.get("__compressBase64__").and_then(Value::as_str) {
        ensure!(
            encoded.len() <= MAX_CATALOG_BYTES,
            "ACE75: сжатый каталог слишком большой"
        );
        let archive = STANDARD
            .decode(encoded)
            .context("ACE75: неверный base64 каталога")?;
        let mut zip =
            zip::ZipArchive::new(Cursor::new(archive)).context("ACE75: неверный ZIP каталога")?;
        let file = zip
            .by_name("data.json")
            .context("ACE75: в каталоге отсутствует data.json")?;
        ensure!(
            file.size() <= MAX_CATALOG_BYTES as u64,
            "ACE75: распакованный каталог слишком большой"
        );
        let mut raw = Vec::new();
        file.take(MAX_CATALOG_BYTES as u64 + 1)
            .read_to_end(&mut raw)
            .context("ACE75: не удалось распаковать каталог")?;
        ensure!(
            raw.len() <= MAX_CATALOG_BYTES,
            "ACE75: распакованный каталог превышает допустимый размер"
        );
        serde_json::from_slice(&raw).context("ACE75: неверный JSON внутри каталога")?
    } else {
        wrapper
    };
    let entry = catalog
        .get(KEYBOARD_MODEL_KEY)
        .context("ACE75: в официальном каталоге отсутствует модель 3837:303c")?;
    let value = entry
        .get("fwVersion")
        .context("ACE75: в каталоге отсутствует версия прошивки")?;
    let number = value
        .as_u64()
        .or_else(|| {
            value.as_str().and_then(|text| {
                if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
                    text.parse::<u64>().ok()
                } else {
                    None
                }
            })
        })
        .context("ACE75: неверный формат версии прошивки")?;
    ensure!(
        number > 0 && number <= u32::MAX as u64,
        "ACE75: версия прошивки вне допустимого диапазона"
    );
    let url = entry
        .get("updateFile")
        .and_then(Value::as_str)
        .context("ACE75: в каталоге отсутствует образ прошивки")?;
    binary_filename(url)?;
    Ok(KeyboardFirmwareInfo {
        version: number.to_string(),
        url: url.into(),
        bytes: None,
        sha256: None,
    })
}

pub fn fetch_keyboard_firmware() -> Result<KeyboardFirmwareInfo> {
    keyboard_firmware_from_catalog(&get_official(KEYBOARD_CATALOG_URL, MAX_CATALOG_BYTES)?)
}

fn checked_image_info(info: &KeyboardFirmwareInfo, bytes: &[u8]) -> Result<KeyboardFirmwareInfo> {
    binary_filename(&info.url)?;
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_KEYBOARD_IMAGE_BYTES,
        "ACE75: недопустимый размер образа прошивки"
    );
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    if let Some(expected) = info.bytes {
        ensure!(
            bytes.len() == expected,
            "ACE75: размер образа изменился после проверки каталога"
        );
    }
    if let Some(expected) = &info.sha256 {
        ensure!(
            sha256.eq_ignore_ascii_case(expected),
            "ACE75: SHA-256 образа изменился после проверки каталога"
        );
    }
    Ok(KeyboardFirmwareInfo {
        bytes: Some(bytes.len()),
        sha256: Some(sha256),
        ..info.clone()
    })
}

fn save_image(
    info: &KeyboardFirmwareInfo,
    bytes: &[u8],
    directory: &Path,
) -> Result<crate::firmware::DownloadedImage> {
    let checked = checked_image_info(info, bytes)?;
    let filename = binary_filename(&checked.url)?;
    std::fs::create_dir_all(directory)
        .with_context(|| format!("ACE75: не удалось создать каталог {}", directory.display()))?;
    let path = directory.join(filename);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| {
            format!(
                "ACE75: файл уже существует или недоступен: {}",
                path.display()
            )
        })?;
    file.write_all(bytes)
        .with_context(|| format!("ACE75: не удалось сохранить {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("ACE75: не удалось синхронизировать {}", path.display()))?;
    Ok(crate::firmware::DownloadedImage {
        url: checked.url.clone(),
        filename: filename.into(),
        bytes: bytes.len(),
        sha256: checked.sha256.unwrap(),
    })
}

pub fn download_keyboard_firmware(
    info: &KeyboardFirmwareInfo,
    directory: &Path,
) -> Result<crate::firmware::DownloadedImage> {
    binary_filename(&info.url)?;
    let current = fetch_keyboard_firmware()?;
    ensure!(
        info.version == current.version && info.url == current.url,
        "ACE75: официальный каталог изменился; повторите проверку прошивки"
    );
    let bytes = get_official(&current.url, MAX_KEYBOARD_IMAGE_BYTES)?;
    save_image(info, &bytes, directory)
}

fn verify_image_bytes(actual: &[u8], official: &[u8]) -> Result<()> {
    ensure!(
        !actual.is_empty()
            && actual.len() <= MAX_KEYBOARD_IMAGE_BYTES
            && !official.is_empty()
            && official.len() <= MAX_KEYBOARD_IMAGE_BYTES,
        "ACE75: недопустимый размер образа прошивки"
    );
    ensure!(
        actual.len() == official.len() && Sha256::digest(actual) == Sha256::digest(official),
        "ACE75: содержимое прошивки отличается от актуального официального образа Ace 75 8K; запись отменена"
    );
    Ok(())
}

pub fn verify_official_keyboard(bytes: &[u8]) -> Result<KeyboardFirmwareInfo> {
    ensure!(
        !bytes.is_empty() && bytes.len() <= MAX_KEYBOARD_IMAGE_BYTES,
        "ACE75: недопустимый размер образа прошивки"
    );
    let info = fetch_keyboard_firmware()?;
    let official = get_official(&info.url, MAX_KEYBOARD_IMAGE_BYTES)?;
    verify_image_bytes(bytes, &official)?;
    checked_image_info(&info, &official)
}

#[cfg(test)]
mod tests {
    use super::*;
    const URL: &str = "https://cdn.mchose.com.cn/configCenter/static/binaries/Ace75_8K_V117_CS6698734A20260727_okby9heultrya4ba.bin";
    fn catalog(version: Value, url: &str) -> Vec<u8> {
        serde_json::to_vec(
            &serde_json::json!({KEYBOARD_MODEL_KEY:{"fwVersion":version,"updateFile":url}}),
        )
        .unwrap()
    }
    fn info() -> KeyboardFirmwareInfo {
        keyboard_firmware_from_catalog(&catalog(serde_json::json!(117), URL)).unwrap()
    }

    #[test]
    fn official_compressed_catalog_selects_only_current_8k_identity() {
        let result = keyboard_firmware_from_catalog(include_bytes!(
            "../../tests/fixtures/ace75/keyboardConfig.json"
        ))
        .unwrap();
        assert_eq!(result.version, "117");
        assert_eq!(result.url, URL);
        assert_eq!(result.bytes, None);
        assert_eq!(result.sha256, None);
        let future = keyboard_firmware_from_catalog(&catalog(
            serde_json::json!("121"),
            &URL.replace("V117", "V121"),
        ))
        .unwrap();
        assert_eq!(future.version, "121");
        assert!(
            keyboard_firmware_from_catalog(
                br#"{"0x3837_0x301D":{"fwVersion":121,"updateFile":"ignored"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn wrong_roles_foreign_hosts_and_ambiguous_paths_are_rejected() {
        for url in [
            URL.replace("Ace75_8K", "Ace75_16K"),
            URL.replace("Ace75_8K", "Ace75_8K_Dongle"),
            URL.replace("cdn.mchose.com.cn/", "cdn.mchose.com.cn.evil.invalid/"),
            URL.replace("https://", "http://"),
            format!("{BINARY_PREFIX}../Ace75_8K.bin"),
            format!("{BINARY_PREFIX}Ace75_8K%2fother.bin"),
            format!("{URL}?redirect=other"),
            format!("{URL}#fragment"),
            format!("{BINARY_PREFIX}A7V3Ultra_Mouse_Main.bin"),
        ] {
            assert!(
                keyboard_firmware_from_catalog(&catalog(serde_json::json!(117), &url)).is_err(),
                "{url}"
            );
        }
        for version in [
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!("117.1"),
            serde_json::json!("0x117"),
            Value::Null,
        ] {
            assert!(keyboard_firmware_from_catalog(&catalog(version, URL)).is_err());
        }
    }

    #[test]
    fn exact_bytes_and_optional_pinned_metadata_are_required() {
        assert!(verify_image_bytes(b"official", b"official").is_ok());
        assert!(verify_image_bytes(b"officiaL", b"official").is_err());
        assert!(verify_image_bytes(b"", b"").is_err());
        let checked = checked_image_info(&info(), b"official").unwrap();
        assert_eq!(checked.bytes, Some(8));
        assert_eq!(checked.sha256.as_ref().unwrap().len(), 64);
        assert!(checked_image_info(&checked, b"officiaL").is_err());
        assert!(checked_image_info(&info(), &vec![0; MAX_KEYBOARD_IMAGE_BYTES + 1]).is_err());
    }

    #[test]
    fn download_storage_never_overwrites_existing_user_file() {
        let directory = std::env::temp_dir().join(format!(
            "mchose-ace75-firmware-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = save_image(&info(), b"official", &directory).unwrap();
        let path = directory.join(&first.filename);
        assert_eq!(std::fs::read(&path).unwrap(), b"official");
        assert!(save_image(&info(), b"replacement", &directory).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"official");
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn compressed_catalog_has_an_independent_expansion_limit() {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "data.json",
                zip::write::FileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer
            .write_all(&vec![b' '; MAX_CATALOG_BYTES + 1])
            .unwrap();
        let zipped = writer.finish().unwrap().into_inner();
        let wrapped =
            serde_json::to_vec(&serde_json::json!({"__compressBase64__":STANDARD.encode(zipped)}))
                .unwrap();
        assert!(wrapped.len() < MAX_CATALOG_BYTES);
        let error = keyboard_firmware_from_catalog(&wrapped)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("распакованный каталог слишком большой"),
            "{error}"
        );
    }
}
