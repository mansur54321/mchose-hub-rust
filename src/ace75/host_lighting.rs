use super::{KEY_COUNT, LightScope, MODEL};
use crate::audio::MusicCapture;
use anyhow::{Context, Result, ensure};
use image::{AnimationDecoder, ImageDecoder, ImageReader, RgbaImage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
    path::Path,
    sync::OnceLock,
    time::{Duration, Instant},
};

const MAX_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FRAMES: usize = 512;
const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LightingFrame {
    pub duration_ms: u32,
    pub colors: Vec<[u8; 3]>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LightingAnimation {
    pub schema_version: u8,
    pub model: String,
    pub name: String,
    pub scopes: Vec<LightScope>,
    pub frames: Vec<LightingFrame>,
}

fn validate_scopes(scopes: &[LightScope]) -> Result<()> {
    ensure!(
        !scopes.is_empty() && scopes.len() <= 3,
        "ACE75: выберите от одной до трёх областей подсветки"
    );
    for (index, scope) in scopes.iter().enumerate() {
        ensure!(
            !scopes[..index].contains(scope),
            "ACE75: область подсветки указана дважды"
        );
    }
    Ok(())
}

impl LightingAnimation {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.model == MODEL,
            "ACE75: анимация предназначена для другой модели или версии формата"
        );
        ensure!(
            !self.name.trim().is_empty() && self.name.chars().count() <= 128,
            "ACE75: название анимации должно содержать от 1 до 128 символов"
        );
        validate_scopes(&self.scopes)?;
        ensure!(
            !self.frames.is_empty() && self.frames.len() <= MAX_FRAMES,
            "ACE75: анимация должна содержать от 1 до 512 кадров"
        );
        for frame in &self.frames {
            ensure!(
                frame.colors.len() == KEY_COUNT,
                "ACE75: кадр подсветки должен содержать 128 RGB-позиций"
            );
            ensure!(
                (5..=60_000).contains(&frame.duration_ms),
                "ACE75: длительность кадра должна быть от 5 до 60000 мс"
            );
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct Layout {
    canvas: Canvas,
    keys: Vec<Position>,
    controls: Vec<Position>,
    lighting_regions: Vec<Position>,
}
#[derive(Deserialize)]
struct Canvas {
    width: f32,
    height: f32,
    top_inset: f32,
}
#[derive(Deserialize)]
struct Position {
    matrix_index: usize,
    #[serde(default)]
    code: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    #[serde(default)]
    hidden: Vec<String>,
}

fn geometry() -> &'static Layout {
    static VALUE: OnceLock<Layout> = OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/ace75-layout.json"))
            .expect("bundled ACE75 layout")
    })
}

fn positions() -> impl Iterator<Item = &'static Position> {
    let layout = geometry();
    layout
        .keys
        .iter()
        .chain(&layout.controls)
        .chain(&layout.lighting_regions)
        .filter(|position| {
            position.matrix_index < KEY_COUNT
                && !position.hidden.iter().any(|value| value == "light")
        })
}

fn code_indices() -> &'static BTreeMap<u32, usize> {
    static VALUE: OnceLock<BTreeMap<u32, usize>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut values: BTreeMap<_, _> = positions()
            .filter(|p| p.code != 0)
            .map(|p| (p.code, p.matrix_index))
            .collect();
        if let Some(index) = values.get(&255).copied() {
            values.insert(1, index);
        }
        for (code, index) in [(300, 25), (301, 23), (302, 12), (303, 3)] {
            values.insert(code, index);
        }
        for offset in 0..28 {
            values.insert(400 + offset as u32, 95 + offset);
        }
        values.insert(428, 123);
        values
    })
}

fn parse_color(value: &Value) -> Result<[u8; 3]> {
    if let Some(text) = value.as_str() {
        let text = text.strip_prefix('#').unwrap_or(text);
        ensure!(
            text.len() == 6 && text.bytes().all(|v| v.is_ascii_hexdigit()),
            "ACE75: неверный цвет HEX"
        );
        return Ok([
            u8::from_str_radix(&text[0..2], 16)?,
            u8::from_str_radix(&text[2..4], 16)?,
            u8::from_str_radix(&text[4..6], 16)?,
        ]);
    }
    serde_json::from_value(value.clone()).context("ACE75: цвет должен быть HEX или RGB-массивом")
}

fn imported_hub_animation(root: &Value) -> Result<LightingAnimation> {
    let data = root
        .get("data")
        .filter(|value| value.get("dataScope").is_some())
        .unwrap_or(root);
    ensure!(
        data.get("dataScope").and_then(Value::as_str) == Some("LightingEffectProfile"),
        "ACE75: это не профиль подсветки Hub"
    );
    ensure!(
        matches!(
            data.get("type").and_then(Value::as_str),
            Some("still" | "gif")
        ),
        "ACE75: неизвестный тип профиля подсветки Hub"
    );
    for identity in [
        root,
        data,
        root.pointer("/extra/identity").unwrap_or(&Value::Null),
    ] {
        if let Some(product) = identity.get("productId") {
            ensure!(
                product.as_u64() == Some(super::PRODUCT_ID as u64),
                "ACE75: профиль подсветки другой модели"
            );
        }
        if let Some(vendor) = identity.get("vendorId") {
            ensure!(
                vendor.as_u64() == Some(crate::transport::VENDOR_ID as u64),
                "ACE75: профиль подсветки другого производителя"
            );
        }
    }
    let raw_frames = data
        .get("frames")
        .and_then(Value::as_array)
        .context("ACE75: отсутствуют кадры подсветки Hub")?;
    ensure!(
        raw_frames.len() <= MAX_FRAMES,
        "ACE75: слишком много кадров подсветки"
    );
    let mut frames = Vec::new();
    for raw in raw_frames {
        let entries = raw
            .get("data")
            .or_else(|| raw.get("colors"))
            .and_then(Value::as_array)
            .context("ACE75: неверный список цветов кадра Hub")?;
        ensure!(
            entries.len() <= KEY_COUNT * 2,
            "ACE75: слишком много цветов в кадре Hub"
        );
        let mut colors = vec![[0; 3]; KEY_COUNT];
        for (index, entry) in entries.iter().enumerate() {
            if entry.is_string() {
                ensure!(
                    index < KEY_COUNT,
                    "ACE75: кадр Hub выходит за матрицу клавиатуры"
                );
                colors[index] = parse_color(entry)?;
            } else {
                let code = entry
                    .get("code")
                    .and_then(Value::as_u64)
                    .filter(|v| *v <= u32::MAX as u64)
                    .context("ACE75: неверный код клавиши в кадре Hub")?
                    as u32;
                let index = code_indices()
                    .get(&code)
                    .context("ACE75: кадр Hub содержит неизвестный код клавиши")?;
                colors[*index] = parse_color(
                    entry
                        .get("selectColor")
                        .or_else(|| entry.get("color"))
                        .context("ACE75: отсутствует цвет клавиши Hub")?,
                )?;
            }
        }
        let duration = match raw.get("duration").or_else(|| raw.get("dur")) {
            None => 100,
            Some(value) => {
                let value = value
                    .as_f64()
                    .filter(|v| v.is_finite() && *v >= 0.0 && *v <= 60_000.0)
                    .context("ACE75: неверная длительность кадра Hub")?;
                if value == 0.0 {
                    100
                } else {
                    value.round().max(5.0) as u32
                }
            }
        };
        frames.push(LightingFrame {
            duration_ms: duration,
            colors,
        });
    }
    let name = root
        .get("name")
        .or_else(|| data.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Hub lighting")
        .to_owned();
    let animation = LightingAnimation {
        schema_version: 1,
        model: MODEL.into(),
        name,
        scopes: vec![LightScope::Main, LightScope::Side, LightScope::Side2],
        frames,
    };
    animation.validate()?;
    Ok(animation)
}

pub fn export_hub_animation(value: &LightingAnimation) -> Result<Value> {
    value.validate()?;
    let mut index_codes = BTreeMap::new();
    for (code, index) in code_indices() {
        index_codes.insert(*index, *code);
    }
    let frames: Vec<_> = value
        .frames
        .iter()
        .map(|frame| {
            let data: Vec<_> = index_codes.iter().map(|(index, code)| {
            let c = frame.colors[*index];
            json!({"code": code, "selectColor": format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])})
        }).collect();
            json!({"duration":frame.duration_ms,"data":data})
        })
        .collect();
    Ok(
        json!({"version":1,"name":value.name,"vendorId":0x3837,"productId":0x303c,
        "data":{"dataScope":"LightingEffectProfile","type":if value.frames.len()==1 {"still"} else {"gif"},"isPreset":false,"frames":frames},
        "extra":{"identity":{"vendorId":0x3837,"productId":0x303c,"productName":super::USB_PRODUCT}}}),
    )
}

fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

fn sample_image(image: &RgbaImage) -> Vec<[u8; 3]> {
    let layout = geometry();
    let mut colors = vec![[0; 3]; KEY_COUNT];
    for position in positions() {
        let x = ((position.x + position.width / 2.0) / layout.canvas.width * image.width() as f32)
            .floor()
            .clamp(0.0, image.width().saturating_sub(1) as f32) as u32;
        let y = ((position.y + position.height / 2.0 + layout.canvas.top_inset)
            / (layout.canvas.height + layout.canvas.top_inset)
            * image.height() as f32)
            .floor()
            .clamp(0.0, image.height().saturating_sub(1) as f32) as u32;
        let pixel = image.get_pixel(x, y).0;
        colors[position.matrix_index] = [pixel[0], pixel[1], pixel[2]]
            .map(|v| (u16::from(v) * u16::from(pixel[3]) / 255) as u8);
    }
    colors
}

pub fn load_animation(path: &Path) -> Result<LightingAnimation> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("ACE75: не удалось открыть {}", path.display()))?
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_INPUT_BYTES,
        "ACE75: файл анимации превышает 32 МиБ"
    );
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "json" {
        let raw: Value = serde_json::from_slice(&bytes).context("ACE75: неверный JSON анимации")?;
        let value = if raw.get("schema_version").is_some() {
            serde_json::from_value(raw)?
        } else {
            imported_hub_animation(&raw)?
        };
        value.validate()?;
        return Ok(value);
    }
    let mut frames = Vec::new();
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        let mut decoder = image::codecs::gif::GifDecoder::new(Cursor::new(bytes))
            .context("ACE75: не удалось прочитать GIF")?;
        decoder.set_limits(image_limits())?;
        let mut decoded_bytes = 0_u64;
        for frame in decoder.into_frames() {
            ensure!(
                frames.len() < MAX_FRAMES,
                "ACE75: GIF содержит больше 512 кадров"
            );
            let frame = frame.context("ACE75: не удалось декодировать кадр GIF")?;
            let (numerator, denominator) = frame.delay().numer_denom_ms();
            let duration_ms =
                (u64::from(numerator) / u64::from(denominator.max(1))).clamp(5, 60_000) as u32;
            decoded_bytes = decoded_bytes.saturating_add(frame.buffer().len() as u64);
            ensure!(
                decoded_bytes <= MAX_DECODED_BYTES,
                "ACE75: распакованный GIF превышает 64 МиБ"
            );
            frames.push(LightingFrame {
                duration_ms,
                colors: sample_image(frame.buffer()),
            });
        }
    } else {
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        reader.limits(image_limits());
        let image = reader
            .decode()
            .context("ACE75: не удалось прочитать изображение подсветки")?
            .to_rgba8();
        frames.push(LightingFrame {
            duration_ms: 100,
            colors: sample_image(&image),
        });
    }
    let value = LightingAnimation {
        schema_version: 1,
        model: MODEL.into(),
        name: path
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("Lighting")
            .chars()
            .take(128)
            .collect(),
        scopes: vec![LightScope::Main, LightScope::Side, LightScope::Side2],
        frames,
    };
    value.validate()?;
    Ok(value)
}

pub fn save_animation(path: &Path, value: &LightingAnimation) -> Result<()> {
    value.validate()?;
    let bytes = if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".hub.json"))
    {
        serde_json::to_vec_pretty(&export_hub_animation(value)?)?
    } else {
        serde_json::to_vec_pretty(value)?
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "ACE75: файл уже существует или недоступен: {}",
                path.display()
            )
        })?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub struct AnimationPlayer {
    value: LightingAnimation,
    started: Instant,
    last_sent: Option<(u128, usize, Instant)>,
}

impl AnimationPlayer {
    pub fn new(value: LightingAnimation) -> Result<Self> {
        value.validate()?;
        Ok(Self {
            value,
            started: Instant::now(),
            last_sent: None,
        })
    }
    pub fn scopes(&self) -> &[LightScope] {
        &self.value.scopes
    }
    pub fn next_frame(&mut self, now: Instant) -> Option<&LightingFrame> {
        let total: u128 = self
            .value
            .frames
            .iter()
            .map(|f| u128::from(f.duration_ms.max(30)))
            .sum();
        let elapsed = now.saturating_duration_since(self.started).as_millis();
        let cycle = elapsed / total;
        let mut within = elapsed % total;
        let mut index = 0;
        for (candidate, frame) in self.value.frames.iter().enumerate() {
            index = candidate;
            if within < u128::from(frame.duration_ms.max(30)) {
                break;
            }
            within -= u128::from(frame.duration_ms.max(30));
        }
        if self
            .last_sent
            .is_some_and(|(previous_cycle, previous_index, sent)| {
                previous_cycle == cycle
                    && previous_index == index
                    && now.saturating_duration_since(sent) < Duration::from_millis(250)
            })
        {
            return None;
        }
        self.last_sent = Some((cycle, index, now));
        Some(&self.value.frames[index])
    }
}

pub struct KeyboardMusic {
    capture: MusicCapture,
    scopes: Vec<LightScope>,
    levels: [f32; 24],
}
impl KeyboardMusic {
    pub fn start(scopes: Vec<LightScope>) -> Result<Self> {
        validate_scopes(&scopes)?;
        Ok(Self {
            capture: MusicCapture::start()?,
            scopes,
            levels: [0.0; 24],
        })
    }
    pub fn sink_name(&self) -> &str {
        self.capture.sink_name()
    }
    pub fn scopes(&self) -> &[LightScope] {
        &self.scopes
    }
    pub fn take_error(&mut self) -> Option<String> {
        self.capture.take_error()
    }
    pub fn next_frame(&mut self) -> Option<Vec<[u8; 3]>> {
        let frequencies = self.capture.latest_frequencies()?;
        Some(render_spectrum(&frequencies, &mut self.levels))
    }
}

fn render_spectrum(frequencies: &[u8], levels: &mut [f32; 24]) -> Vec<[u8; 3]> {
    for (index, level) in levels.iter_mut().enumerate() {
        let first = (2.0_f32 * 128.0_f32.powf(index as f32 / 24.0)).floor() as usize;
        let last = (2.0_f32 * 128.0_f32.powf((index + 1) as f32 / 24.0)).ceil() as usize;
        let maximum = frequencies
            .get(first.min(frequencies.len())..last.min(frequencies.len()))
            .unwrap_or(&[])
            .iter()
            .copied()
            .max()
            .unwrap_or(0);
        *level = (f32::from(maximum) / 255.0).max((*level - 0.075).max(0.0));
    }
    let layout = geometry();
    let mut colors = vec![[0; 3]; KEY_COUNT];
    for position in positions() {
        let column = (((position.x + position.width / 2.0) / layout.canvas.width) * 24.0)
            .floor()
            .clamp(0.0, 23.0) as usize;
        let level = levels[column];
        let y = ((position.y + position.height / 2.0 + layout.canvas.top_inset)
            / (layout.canvas.height + layout.canvas.top_inset))
            .clamp(0.0, 1.0);
        let strip_or_logo = (95..124).contains(&position.matrix_index);
        if level > 0.02 && (strip_or_logo || 1.0 - y <= level) {
            let t = column as f32 / 23.0;
            let intensity = if strip_or_logo { level } else { 1.0 };
            colors[position.matrix_index] = [
                ((1.0 - t) * 255.0 * intensity) as u8,
                ((1.0 - (2.0 * t - 1.0).abs()) * 255.0 * intensity) as u8,
                (t * 255.0 * intensity) as u8,
            ];
        }
    }
    colors
}

#[cfg(test)]
mod tests {
    use super::*;
    fn animation() -> LightingAnimation {
        LightingAnimation {
            schema_version: 1,
            model: MODEL.into(),
            name: "test".into(),
            scopes: vec![LightScope::Main],
            frames: vec![
                LightingFrame {
                    duration_ms: 100,
                    colors: vec![[255, 0, 0]; 128],
                },
                LightingFrame {
                    duration_ms: 200,
                    colors: vec![[0, 0, 255]; 128],
                },
            ],
        }
    }
    #[test]
    fn host_player_uses_frame_duration_and_skips_overdue_frames() {
        let mut player = AnimationPlayer::new(animation()).unwrap();
        let started = player.started;
        assert_eq!(player.next_frame(started).unwrap().colors[0], [255, 0, 0]);
        assert!(
            player
                .next_frame(started + Duration::from_millis(50))
                .is_none()
        );
        assert_eq!(
            player
                .next_frame(started + Duration::from_millis(100))
                .unwrap()
                .colors[0],
            [0, 0, 255]
        );
        assert_eq!(
            player
                .next_frame(started + Duration::from_millis(1000))
                .unwrap()
                .colors[0],
            [0, 0, 255]
        );
        assert_eq!(
            player
                .next_frame(started + Duration::from_millis(1200))
                .unwrap()
                .colors[0],
            [255, 0, 0]
        );
    }
    #[test]
    fn hub_codes_preserve_space_segments_strip_and_logo() {
        let raw = json!({"productId":0x303c,"data":{"dataScope":"LightingEffectProfile","type":"gif","frames":[{"duration":45,"data":[{"code":4,"selectColor":"#112233"},{"code":300,"selectColor":"#ABCDEF"},{"code":427,"selectColor":"#010203"},{"code":428,"selectColor":"#102030"}]}]}});
        let animation = imported_hub_animation(&raw).unwrap();
        assert_eq!(animation.frames[0].colors[88], [17, 34, 51]);
        assert_eq!(animation.frames[0].colors[25], [171, 205, 239]);
        assert_eq!(animation.frames[0].colors[122], [1, 2, 3]);
        assert_eq!(animation.frames[0].colors[123], [16, 32, 48]);
        let again = imported_hub_animation(&export_hub_animation(&animation).unwrap()).unwrap();
        assert_eq!(again.frames, animation.frames);
        let mut wrong = raw.clone();
        wrong["productId"] = json!(0x301d);
        assert!(imported_hub_animation(&wrong).is_err());
        let mut timing = raw;
        timing["data"]["frames"][0]["duration"] = json!(35.5);
        assert_eq!(
            imported_hub_animation(&timing).unwrap().frames[0].duration_ms,
            36
        );
        timing["data"]["frames"][0]["duration"] = json!(-5);
        assert!(imported_hub_animation(&timing).is_err());
    }
    #[test]
    fn bitmap_import_samples_real_positions_and_alpha() {
        let image = RgbaImage::from_pixel(2, 1, image::Rgba([100, 60, 20, 128]));
        let colors = sample_image(&image);
        assert_eq!(colors[88], [50, 30, 10]);
        assert_eq!(colors[122], [50, 30, 10]);
        assert_eq!(colors[123], [50, 30, 10]);
        assert_eq!(colors[94], [0, 0, 0]);
    }
    #[test]
    fn silent_audio_does_not_light_keys() {
        let mut levels = [0.0; 24];
        assert!(
            render_spectrum(&[0; 256], &mut levels)
                .iter()
                .all(|v| *v == [0, 0, 0])
        );
        assert!(
            render_spectrum(&[255; 256], &mut levels)
                .iter()
                .filter(|v| **v != [0, 0, 0])
                .count()
                > 100
        );
        for _ in 0..20 {
            render_spectrum(&[0; 256], &mut levels);
        }
        assert!(levels.iter().all(|v| *v == 0.0));
    }
    #[test]
    fn animation_rejects_wrong_model_malformed_frames_and_duplicate_scopes() {
        let mut value = animation();
        value.model = "Ace75 16K".into();
        assert!(value.validate().is_err());
        value.model = MODEL.into();
        value.frames[0].colors.pop();
        assert!(value.validate().is_err());
        value.frames[0].colors.push([0; 3]);
        value.scopes.push(LightScope::Main);
        assert!(value.validate().is_err());
    }
}
