use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    io::{Cursor, Read, Write},
    path::Path,
    time::Duration,
};

pub const MODEL: &str = "MCHOSE A7 V3 Ultra+";
pub const CATALOG_URL: &str = "https://cdn.mchose.com.cn/configCenter/custom/newMouseConfig.json";
pub const DOCK_IMAGE_URL: &str =
    "https://cdn.mchose.com.cn/configCenter/public/newBin/MagDock0004.bin";
pub const DOCK_LEGACY_URL: &str =
    "https://cdn.mchose.com.cn/configCenter/public/exe/MagDock-upgrade.zip";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockFirmwareImage {
    pub version: u32,
    pub url: String,
    pub legacy_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareImage {
    pub role: String,
    pub version: u32,
    pub main_url: String,
    pub sub_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirmwareCatalog {
    pub model: String,
    pub images: Vec<FirmwareImage>,
    pub source: String,
    pub dock: DockFirmwareImage,
}

fn get(url: &str, max_bytes: u64) -> Result<Vec<u8>> {
    ensure!(
        url.starts_with("https://cdn.mchose.com.cn/"),
        "Допустим только официальный CDN MCHOSE"
    );
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(45)))
        .build();
    let agent: ureq::Agent = config.into();
    Ok(agent
        .get(url)
        .call()
        .context("Не удалось получить файл с CDN MCHOSE")?
        .body_mut()
        .with_config()
        .limit(max_bytes)
        .read_to_vec()?)
}

pub fn decode_catalog(bytes: &[u8]) -> Result<Value> {
    let wrapper: Value = serde_json::from_slice(bytes)?;
    if let Some(encoded) = wrapper.get("__compressBase64__").and_then(Value::as_str) {
        ensure!(encoded.len() < 4_000_000, "Каталог слишком большой");
        let archive = STANDARD.decode(encoded)?;
        let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
        let mut file = zip
            .by_name("data.json")
            .context("В каталоге нет data.json")?;
        ensure!(
            file.size() <= 4_000_000,
            "Распакованный каталог слишком большой"
        );
        let mut json = String::new();
        file.read_to_string(&mut json)?;
        Ok(serde_json::from_str(&json)?)
    } else {
        Ok(wrapper)
    }
}

fn version(value: &Value, key: &str) -> Result<u32> {
    let v = value.get(key).context("В каталоге нет версии")?;
    let number = v
        .as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .context("Неверный формат версии прошивки")?;
    Ok(u32::try_from(number)?)
}

fn image_url(value: &Value, key: &str) -> Result<String> {
    let path = value
        .get(key)
        .and_then(Value::as_str)
        .context("В каталоге нет файла прошивки")?;
    ensure!(
        path.starts_with("configCenter/static/binaries/") && !path.contains(".."),
        "Неизвестный путь прошивки"
    );
    Ok(format!("https://cdn.mchose.com.cn/{path}"))
}

pub fn catalog_from_json(value: &Value) -> Result<FirmwareCatalog> {
    let model = value
        .get(MODEL)
        .context("В каталоге отсутствует A7 V3 Ultra+")?;
    let mut images = Vec::new();
    for (role, prefix, suffix) in [
        ("Мышь", "Mouse", ""),
        ("Приёмник 8K", "Dongle", ""),
        ("Приёмник 1K", "Dongle", "_Low"),
    ] {
        images.push(FirmwareImage {
            role: role.into(),
            version: version(model, &format!("{prefix}_Version{suffix}"))?,
            main_url: image_url(model, &format!("{prefix}_Main{suffix}"))?,
            sub_url: image_url(model, &format!("{prefix}_Sub{suffix}"))?,
        });
    }
    Ok(FirmwareCatalog {
        model: MODEL.into(),
        images,
        source: CATALOG_URL.into(),
        dock: DockFirmwareImage {
            version: 4,
            url: DOCK_IMAGE_URL.into(),
            legacy_url: DOCK_LEGACY_URL.into(),
        },
    })
}

pub fn fetch_catalog() -> Result<FirmwareCatalog> {
    catalog_from_json(&decode_catalog(&get(CATALOG_URL, 4_000_000)?)?)
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadedImage {
    pub url: String,
    pub filename: String,
    pub bytes: usize,
    pub sha256: String,
}

pub fn download_pair(image: &FirmwareImage, directory: &Path) -> Result<Vec<DownloadedImage>> {
    let mut manifest = Vec::new();
    for url in [&image.main_url, &image.sub_url] {
        manifest.push(download_image(url, directory)?);
    }
    Ok(manifest)
}

pub fn download_dock(directory: &Path) -> Result<DownloadedImage> {
    download_image(DOCK_IMAGE_URL, directory)
}

fn download_image(url: &str, directory: &Path) -> Result<DownloadedImage> {
    std::fs::create_dir_all(directory)?;
    let filename = url.rsplit('/').next().context("Пустое имя прошивки")?;
    ensure!(filename.ends_with(".bin"), "Неверное расширение прошивки");
    let path = directory.join(filename);
    if path.exists() {
        bail!("Файл уже существует: {}", path.display());
    }
    let bytes = get(url, 8_000_000)?;
    ensure!(!bytes.is_empty(), "CDN вернул пустую прошивку");
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(DownloadedImage {
        url: url.into(),
        filename: filename.into(),
        bytes: bytes.len(),
        sha256,
    })
}

fn verify_bytes(actual: &[u8], official: &[u8]) -> Result<()> {
    ensure!(
        !actual.is_empty() && Sha256::digest(actual) == Sha256::digest(official),
        "Содержимое прошивки отличается от официального файла MCHOSE; запись отменена"
    );
    Ok(())
}

pub fn verify_official_pair(image: &FirmwareImage, main: &[u8], sub: &[u8]) -> Result<()> {
    verify_bytes(main, &get(&image.main_url, 8_000_000)?)?;
    verify_bytes(sub, &get(&image.sub_url, 8_000_000)?)
}

pub fn verify_official_dock(bytes: &[u8]) -> Result<()> {
    verify_bytes(bytes, &get(DOCK_IMAGE_URL, 8_000_000)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_snapshot_selects_exact_model_and_both_receivers() {
        let raw = include_bytes!("../tests/fixtures/newMouseConfig.json");
        let catalog = catalog_from_json(&decode_catalog(raw).unwrap()).unwrap();
        assert_eq!(catalog.images.len(), 3);
        assert_eq!(catalog.images[0].version, 30);
        assert!(
            catalog.images[0]
                .main_url
                .contains("A7V3Ultra_Mouse_Main_V0030")
        );
        assert!(catalog.images[2].sub_url.contains("564C"));
    }

    #[test]
    fn refuses_wrong_model_or_untrusted_binary_path() {
        assert!(catalog_from_json(&serde_json::json!({})).is_err());
        assert!(image_url(&serde_json::json!({"x":"https://example.com/a.bin"}), "x").is_err());
        assert!(
            image_url(
                &serde_json::json!({"x":"configCenter/static/binaries/../a.bin"}),
                "x"
            )
            .is_err()
        );
    }

    #[test]
    fn firmware_content_must_match_not_just_filename() {
        assert!(verify_bytes(b"official image", b"official image").is_ok());
        assert!(verify_bytes(b"official imagf", b"official image").is_err());
        assert!(verify_bytes(b"", b"").is_err());
    }
}
