use crate::{
    dock::{Dock, DockLighting, DockMode, DockSettings, FirmwarePlan},
    firmware::{self, FirmwareCatalog},
    hub_format,
    keyboard_controller::{KeyboardCommand, KeyboardSession, KeyboardState},
    mouse::{
        AdaptivePower, DpiSettings, KeyBinding, Mouse, MouseMacro, MouseSettings, MouseSnapshot,
    },
    transport::{self, DeviceInfo, DeviceKind, HidTransport},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Default)]
pub struct AppSnapshot {
    pub ace75: KeyboardState,
    pub devices: Vec<DeviceInfo>,
    pub active_mouse_path: Option<String>,
    pub mouse: Option<MouseSnapshot>,
    pub dock: Option<DockSettings>,
    pub adaptive: Option<AdaptivePower>,
    pub macro_value: Option<(u8, MouseMacro)>,
    pub firmware: Option<FirmwareCatalog>,
    pub busy: bool,
    pub error: Option<String>,
    pub error_chain: Vec<String>,
    pub message: String,
    pub last_update: Option<Instant>,
    pub music_active: bool,
    pub charging_indicator_enabled: bool,
    pub charging_indicator_active: bool,
    pub charging_percent: Option<u8>,
    pub presets: Vec<PresetInfo>,
    pub preview: Option<ProfileFile>,
    pub macro_library: Vec<String>,
    pub local_macro_value: Option<MouseMacro>,
    pub firmware_progress: Option<(usize, usize)>,
}

#[derive(Debug)]
pub enum Command {
    Ace75(KeyboardCommand),
    Refresh,
    SelectMouse(String),
    ApplySettings(MouseSettings),
    ApplyPerformance {
        settings: MouseSettings,
        lod: u8,
    },
    ApplyDpi(DpiSettings),
    ApplyKeys {
        profile: u8,
        keys: Vec<KeyBinding>,
    },
    ApplyProfileCycleAcrossAllSlots {
        key_index: u8,
    },
    SetLod {
        profile: u8,
        lod: u8,
    },
    SelectProfile(u8),
    RenameProfile {
        profile: u8,
        name: String,
    },
    ApplyAdaptive {
        profile: u8,
        power: AdaptivePower,
    },
    ApplyDock(DockSettings),
    ReadMacro(u8),
    WriteMacro {
        id: u8,
        value: MouseMacro,
    },
    SaveLocalMacro(MouseMacro),
    OpenLocalMacro(String),
    UpdateLocalMacro {
        name: String,
        value: MouseMacro,
    },
    DeleteLocalMacro(String),
    RenameLocalMacro {
        from: String,
        to: String,
    },
    ExportProfile(PathBuf),
    ImportProfile(PathBuf),
    SavePreset(String),
    PreviewPreset(PathBuf),
    SaveEditedPreset {
        path: PathBuf,
        value: ProfileFile,
    },
    RenamePreset {
        path: PathBuf,
        name: String,
    },
    DeletePreset(PathBuf),
    ResetProfile {
        profile: u8,
        buttons_only: bool,
    },
    ResetAllProfiles,
    ResetDock,
    CheckFirmware,
    DownloadFirmware {
        index: usize,
        directory: PathBuf,
    },
    DownloadDockFirmware(PathBuf),
    FlashMouse {
        device_path: String,
        main: PathBuf,
        sub: PathBuf,
    },
    FlashDock(PathBuf),
    StartMusic,
    StopMusic,
    SetChargingIndicator(bool),
    Stop,
}

impl Command {
    fn writes_hardware(&self) -> bool {
        if let Self::Ace75(command) = self {
            return command.writes_hardware();
        }
        !matches!(
            self,
            Self::Refresh
                | Self::SelectMouse(_)
                | Self::ReadMacro(_)
                | Self::SaveLocalMacro(_)
                | Self::OpenLocalMacro(_)
                | Self::UpdateLocalMacro { .. }
                | Self::DeleteLocalMacro(_)
                | Self::RenameLocalMacro { .. }
                | Self::ExportProfile(_)
                | Self::SavePreset(_)
                | Self::PreviewPreset(_)
                | Self::SaveEditedPreset { .. }
                | Self::RenamePreset { .. }
                | Self::DeletePreset(_)
                | Self::CheckFirmware
                | Self::DownloadFirmware { .. }
                | Self::DownloadDockFirmware(_)
                | Self::StopMusic
                | Self::Stop
        )
    }
}

pub struct Controller {
    pub snapshot: AppSnapshot,
    sender: Sender<Command>,
    receiver: Receiver<AppSnapshot>,
    worker: Option<thread::JoinHandle<()>>,
}

fn combine_results(first: Result<()>, second: Result<()>) -> Result<()> {
    match (first, second) {
        (Ok(()), result) | (result, Ok(())) => result,
        (Err(first), Err(second)) => Err(first.context(format!("Дополнительная ошибка: {second:#}"))),
    }
}

impl Controller {
    pub fn new(read_only: bool) -> Self {
        Self::spawn(read_only, false)
    }

    fn spawn(read_only: bool, offline: bool) -> Self {
        let (sender, commands) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("mchose-hid".into())
            .spawn(move || {
                let mut worker = Worker::new(events, read_only);
                worker.offline = offline;
                worker.run(commands);
            })
            .expect("HID worker thread");
        Self {
            snapshot: AppSnapshot {
                busy: true,
                charging_indicator_enabled: true,
                message: "Чтение устройств MCHOSE…".into(),
                ..Default::default()
            },
            sender,
            receiver,
            worker: Some(worker),
        }
    }

    pub fn new_offline() -> Self {
        Self::spawn(true, true)
    }

    pub fn send(&mut self, command: Command) {
        if self.snapshot.busy {
            return;
        }
        match self.sender.send(command) {
            Ok(()) => {
                self.snapshot.busy = true;
                self.snapshot.error = None;
                self.snapshot.error_chain.clear();
            }
            Err(_) => {
                self.snapshot.error = Some("Нет соединения с устройствами".into());
                self.snapshot.error_chain.clear();
            }
        }
    }

    pub fn poll(&mut self) {
        while let Ok(snapshot) = self.receiver.try_recv() {
            self.snapshot = snapshot;
        }
        self.snapshot.ace75.pull_telemetry();
        if let Some(progress) = self.snapshot.ace75.firmware_progress() {
            self.snapshot.firmware_progress = Some(progress);
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Stop);
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                eprintln!(
                    "Поток MCHOSE завершился с паникой; восстановление подсветки не подтверждено"
                );
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFile {
    pub schema_version: u32,
    pub model: String,
    pub snapshot: MouseSnapshot,
    #[serde(default)]
    pub macros: BTreeMap<u8, MouseMacro>,
    #[serde(default)]
    pub adaptive: Option<AdaptivePower>,
}

#[derive(Debug, Clone)]
pub struct PresetInfo {
    pub name: String,
    pub path: PathBuf,
}

pub fn preset_path(name: &str) -> Result<PathBuf> {
    let name = name.trim();
    ensure!(
        !name.is_empty() && name.chars().count() <= 64,
        "Имя должно содержать от 1 до 64 символов"
    );
    ensure!(
        !name.contains(['/', '\\', '\0']) && name != "." && name != "..",
        "Имя не должно содержать разделители пути"
    );
    Ok(data_dir().join("presets").join(format!("{name}.json")))
}

fn existing_preset(path: &Path) -> Result<()> {
    ensure!(
        path.parent() == Some(data_dir().join("presets").as_path()),
        "Файл находится вне библиотеки профилей"
    );
    ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "Профиль должен быть обычным файлом"
    );
    Ok(())
}

pub fn list_presets() -> Result<Vec<PresetInfo>> {
    let root = data_dir().join("presets");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && entry.path().extension().is_some_and(|e| e == "json") {
            files.push(PresetInfo {
                name: entry
                    .path()
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: entry.path(),
            });
        }
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

pub fn save_new_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| {
            format!(
                "Не удалось создать {}. Существующие файлы не перезаписываются",
                path.display()
            )
        })?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    return std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .map(|profile| PathBuf::from(profile).join("AppData/Local"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("mchose-native");
    #[cfg(not(windows))]
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("mchose-native")
}

fn read_json(path: &Path) -> Result<serde_json::Value> {
    ensure!(
        fs::metadata(path)?.len() <= 4_000_000,
        "Файл JSON слишком большой: {}",
        path.display()
    );
    serde_json::from_slice(&fs::read(path)?)
        .with_context(|| format!("Не удалось прочитать {}", path.display()))
}

pub fn load_macro_library(directory: &Path) -> Result<BTreeMap<String, MouseMacro>> {
    if !directory.exists() {
        return Ok(BTreeMap::new());
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|value| value == "json")
        {
            files.push((entry.metadata()?.modified()?, entry.path()));
        }
    }
    files.sort();
    let mut macros = BTreeMap::new();
    for (_, path) in files {
        for value in read_macro_file(&path)? {
            macros.insert(value.name.clone(), value);
        }
    }
    Ok(macros)
}

fn read_macro_file(path: &Path) -> Result<Vec<MouseMacro>> {
    let value = read_json(path)?;
    let values = if value.get("events").is_some() {
        vec![serde_json::from_value::<MouseMacro>(value)?]
    } else {
        hub_format::macros_from_hub_json(&value)
            .with_context(|| format!("Библиотека макросов: {}", path.display()))?
    };
    for value in &values {
        value.encode()?;
    }
    Ok(values)
}

fn validate_local_macro(value: &MouseMacro) -> Result<()> {
    ensure!(
        !value.name.trim().is_empty(),
        "Имя макроса не может быть пустым"
    );
    ensure!(
        !value.name.contains('\0'),
        "Имя макроса содержит нулевой символ"
    );
    value.encode()?;
    Ok(())
}

fn append_local_macro(directory: &Path, value: &MouseMacro) -> Result<PathBuf> {
    validate_local_macro(value)?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let path = directory.join(format!("{timestamp}-local-{}.json", std::process::id()));
    save_new_json(&path, value)?;
    Ok(path)
}

fn save_local_macro(directory: &Path, value: &MouseMacro, update: Option<&str>) -> Result<()> {
    validate_local_macro(value)?;
    let library = load_macro_library(directory)?;
    match update {
        Some(name) => {
            ensure!(
                library.contains_key(name),
                "Макрос «{name}» отсутствует в библиотеке"
            );
            ensure!(
                value.name == name,
                "Для смены имени используйте переименование макроса"
            );
        }
        None => ensure!(
            !library.contains_key(&value.name),
            "Макрос «{}» уже существует",
            value.name
        ),
    }
    append_local_macro(directory, value)?;
    Ok(())
}

fn archive_local_macro(
    directory: &Path,
    backups: &Path,
    name: &str,
    rename: Option<&str>,
) -> Result<PathBuf> {
    let library = load_macro_library(directory)?;
    let original = library
        .get(name)
        .context("Макрос отсутствует в библиотеке")?;
    let replacement = if let Some(target) = rename {
        ensure!(
            !library.contains_key(target),
            "Макрос «{target}» уже существует"
        );
        let mut value = original.clone();
        value.name = target.to_owned();
        validate_local_macro(&value)?;
        Some(value)
    } else {
        None
    };
    let mut sources = Vec::new();
    let mut survivors = BTreeMap::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || !entry
                .path()
                .extension()
                .is_some_and(|value| value == "json")
        {
            continue;
        }
        let values = read_macro_file(&entry.path())?;
        if values.iter().any(|value| value.name == name) {
            for value in values.iter().filter(|value| value.name != name) {
                survivors.insert(value.name.clone(), library[&value.name].clone());
            }
            sources.push(entry.path());
        }
    }
    ensure!(!sources.is_empty(), "Исходные файлы макроса не найдены");
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let archive = backups.join(format!("macros-{timestamp}-{}", std::process::id()));
    fs::create_dir_all(backups)?;
    fs::create_dir(&archive)?;
    let originals = archive.join("originals");
    fs::create_dir(&originals)?;
    save_new_json(&archive.join("library-before.json"), &library)?;
    save_new_json(
        &archive.join("operation.json"),
        &serde_json::json!({"name":name,"rename_to":rename,"source_files":sources}),
    )?;
    // A Hub export may contain several macros; retain the latest unrelated values.
    for value in survivors.values().chain(replacement.iter()) {
        append_local_macro(directory, value).with_context(|| {
            format!(
                "Не удалось подготовить библиотеку; копия: {}",
                archive.display()
            )
        })?;
    }
    for source in sources {
        let target = originals.join(source.file_name().context("Файл макроса без имени")?);
        fs::rename(&source, &target).with_context(|| {
            format!(
                "Архивирование не завершено; все данные сохранены в библиотеке или {}",
                archive.display()
            )
        })?;
    }
    Ok(archive)
}

fn macro_slots(keys: &[KeyBinding]) -> Result<Vec<u8>> {
    let mut ids = Vec::new();
    for key in keys.iter().filter(|key| key.kind == 0x21) {
        ensure!(key.data.len() == 2, "Некорректная ссылка на макрос");
        let id = u16::from_le_bytes([key.data[0], key.data[1]]);
        ensure!(id < 18, "Недопустимый слот макроса: {id}");
        if !ids.contains(&(id as u8)) {
            ids.push(id as u8);
        }
    }
    Ok(ids)
}

fn imported_profile(
    value: &serde_json::Value,
    base: &MouseSnapshot,
    library: &BTreeMap<String, MouseMacro>,
) -> Result<ProfileFile> {
    let native = value.get("schema_version").is_some();
    let mut file = if native {
        serde_json::from_value::<ProfileFile>(value.clone())?
    } else {
        ProfileFile {
            schema_version: 1,
            model: firmware::MODEL.into(),
            snapshot: hub_format::profile_from_hub_json(value, base)?,
            macros: BTreeMap::new(),
            adaptive: None,
        }
    };
    ensure!(
        file.schema_version == 1 && file.model == firmware::MODEL,
        "Файл относится к другой модели или версии формата"
    );
    file.snapshot.settings.encode()?;
    file.snapshot.dpi.encode(0)?;
    if file.snapshot.dpi.separate_y {
        file.snapshot.dpi.encode(1)?;
    }
    ensure!(
        file.snapshot.settings.profile_index == file.snapshot.dpi.profile_index,
        "Индексы профиля настроек и DPI различаются"
    );
    ensure!(
        file.snapshot.settings.dpi_index == file.snapshot.dpi.selected,
        "Выбранные ступени DPI различаются"
    );
    ensure!(
        !file.snapshot.keys.is_empty() && file.snapshot.keys.len() <= 6,
        "Профиль должен содержать от 1 до 6 назначений кнопок"
    );
    ensure!(file.snapshot.lod_index < 5, "Недопустимый LOD в профиле");
    for binding in &file.snapshot.keys {
        binding.encode()?;
    }
    let names = if native {
        BTreeMap::new()
    } else {
        hub_format::hub_profile_macro_names(value)?
    };
    let mut missing = Vec::new();
    for id in macro_slots(&file.snapshot.keys)? {
        if file.macros.contains_key(&id) {
            continue;
        }
        if let Some(name) = names.get(&id) {
            if let Some(value) = library.get(name) {
                file.macros.insert(id, value.clone());
            } else {
                missing.push(format!("«{name}» (слот {id})"));
            }
        } else {
            missing.push(format!("слот {id}: отсутствует имя или определение"));
        }
    }
    ensure!(
        missing.is_empty(),
        "Импорт отменён до записи: отсутствуют макросы {}. Импортируйте их в локальную библиотеку",
        missing.join(", ")
    );
    for (id, value) in &file.macros {
        ensure!(*id < 18, "Недопустимый слот макроса: {id}");
        value.encode()?;
    }
    ensure!(
        file.adaptive.is_none(),
        "Адаптивное энергосбережение не подтверждено для A7 V3 Ultra+; импорт такого профиля отменён"
    );
    Ok(file)
}

fn validate_local_profile(file: &ProfileFile) -> Result<()> {
    imported_profile(
        &serde_json::to_value(file)?,
        &file.snapshot,
        &BTreeMap::new(),
    )?;
    validated_keys(&file.snapshot.keys, &file.snapshot.keys)?;
    ensure!(
        file.snapshot.profile_names.len() == 3,
        "Профиль должен содержать три имени слотов"
    );
    ensure!(
        file.snapshot
            .profile_names
            .iter()
            .all(|name| name.len() <= 60 && !name.contains('\0')),
        "Имя профиля превышает 60 байт или содержит нулевой символ"
    );
    Ok(())
}

fn save_edited_preset(path: &Path, backups: &Path, value: &ProfileFile) -> Result<PathBuf> {
    validate_local_profile(value)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 4_000_000,
        "Профиль должен быть обычным JSON-файлом до 4 МБ"
    );
    let previous = fs::read(path)?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    fs::create_dir_all(backups)?;
    let backup = backups.join(format!(
        "preset-edit-{timestamp}-{}.json",
        std::process::id()
    ));
    let mut backup_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup)?;
    backup_file.write_all(&previous)?;
    backup_file.sync_all()?;
    let parent = path
        .parent()
        .context("У профиля нет родительского каталога")?;
    let temporary = parent.join(format!(
        ".preset-edit-{timestamp}-{}.tmp",
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let replace = (|| -> Result<()> {
        file.set_permissions(metadata.permissions())?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        ensure!(
            fs::symlink_metadata(path)?.is_file() && fs::read(path)? == previous,
            "Файл изменился во время сохранения; замена отменена"
        );
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = replace {
        let _ = fs::remove_file(&temporary);
        return Err(error.context(format!("Исходный профиль сохранён в {}", backup.display())));
    }
    Ok(backup)
}

fn remap_profile(file: &mut ProfileFile, target: u8) -> Result<String> {
    ensure!(target < 3, "Недопустимый целевой профиль");
    let source = file.snapshot.settings.profile_index as usize;
    let name = file
        .snapshot
        .profile_names
        .get(source)
        .cloned()
        .context("В файле отсутствует имя исходного профиля")?;
    ensure!(name.len() <= 60, "Имя исходного профиля превышает 60 байт");
    file.snapshot.settings.profile_index = target;
    file.snapshot.dpi.profile_index = target;
    Ok(name)
}

fn verify_settings(mouse: &mut Mouse, expected: &MouseSettings) -> Result<()> {
    let actual = mouse.read_settings()?;
    ensure!(
        actual.encode()? == expected.encode()?,
        "Мышь не подтвердила настройки после записи: {:?}",
        actual
    );
    Ok(())
}

fn write_settings_verified(mouse: &mut Mouse, expected: &MouseSettings) -> Result<()> {
    mouse.write_settings(expected)?;
    verify_settings(mouse, expected)
}

fn write_dpi_verified(mouse: &mut Mouse, expected: &DpiSettings) -> Result<()> {
    mouse.write_dpi(expected)?;
    let actual = mouse.read_dpi(expected.profile_index)?;
    ensure!(
        actual.encode(0)? == expected.encode(0)?,
        "Мышь не подтвердила ступени DPI X"
    );
    if expected.separate_y {
        ensure!(
            actual.encode(1)? == expected.encode(1)?,
            "Мышь не подтвердила ступени DPI Y"
        );
    }
    Ok(())
}

fn validated_keys<'a>(before: &[KeyBinding], keys: &'a [KeyBinding]) -> Result<&'a [KeyBinding]> {
    let count = keys
        .iter()
        .rposition(|key| key.kind != 0xff)
        .map_or(0, |index| index + 1);
    let keys = &keys[..count];
    ensure!(
        !keys.is_empty() && keys.iter().all(|key| key.kind != 0xff),
        "Отсутствующая кнопка не может получить назначение"
    );
    ensure!(
        before.len() >= keys.len() && before[..keys.len()].iter().all(|key| key.kind != 0xff),
        "В профиле есть назначение для отсутствующей кнопки"
    );
    let mut after = before.to_vec();
    after[..keys.len()].clone_from_slice(keys);
    ensure!(
        after
            .iter()
            .any(|key| key.kind == 0 && key.data.first() == Some(&1)),
        "Должна остаться хотя бы одна кнопка левого щелчка"
    );
    for key in keys {
        key.encode()?;
    }
    Ok(keys)
}

fn write_keys_verified(mouse: &mut Mouse, profile: u8, keys: &[KeyBinding]) -> Result<()> {
    let before = mouse.read_keys(profile)?;
    let keys = validated_keys(&before, keys)?;
    mouse.write_keys(profile, 0, keys)?;
    let actual = mouse.read_keys(profile)?;
    ensure!(
        actual.get(..keys.len()) == Some(keys),
        "Мышь не подтвердила назначения кнопок"
    );
    Ok(())
}

fn write_lod_verified(mouse: &mut Mouse, profile: u8, lod: u8) -> Result<()> {
    mouse.write_lod(profile, lod)?;
    ensure!(mouse.read_lod(profile)? == lod, "Мышь не подтвердила LOD");
    Ok(())
}

fn write_name_verified(mouse: &mut Mouse, profile: u8, name: &str) -> Result<()> {
    mouse.write_profile_name(profile, name)?;
    ensure!(
        mouse.read_profile_name(profile)? == name,
        "Мышь не подтвердила имя профиля"
    );
    Ok(())
}

fn same_device(left: &DeviceInfo, right: &DeviceInfo) -> bool {
    left.path == right.path
        && left.vendor_id == right.vendor_id
        && left.product_id == right.product_id
        && left.interface == right.interface
        && left.usage_page == right.usage_page
        && left.usage == right.usage
        && left.name == right.name
}

fn fresh_device(expected: &DeviceInfo) -> Result<DeviceInfo> {
    transport::enumerate()?.into_iter().find(|device| same_device(device, expected))
        .context("Устройство отключилось или его идентификатор изменился; повторите обнаружение перед прошивкой")
}

fn validate_firmware_files(
    info: &DeviceInfo,
    main: &Path,
    sub: &Path,
    catalog: &FirmwareCatalog,
) -> Result<()> {
    let index = match info.product_id {
        0x4033 => 0,
        0x1014 => 1,
        0x1018 => 2,
        _ => bail!("Неподдерживаемое устройство для прошивки"),
    };
    let image = catalog
        .images
        .get(index)
        .context("В каталоге нет прошивки выбранного устройства")?;
    for (path, expected_url, role) in [
        (main, &image.main_url, "Main"),
        (sub, &image.sub_url, "Sub"),
    ] {
        let expected = expected_url
            .rsplit('/')
            .next()
            .context("Неизвестное имя прошивки в каталоге")?;
        ensure!(
            path.file_name().and_then(|name| name.to_str()) == Some(expected),
            "Неверный файл {role} для {}: требуется {expected}",
            image.role
        );
        ensure!(
            fs::metadata(path)?.len() <= 8_000_000,
            "Файл прошивки слишком большой"
        );
    }
    Ok(())
}

struct Worker {
    keyboard: KeyboardSession,
    events: Sender<AppSnapshot>,
    state: AppSnapshot,
    mouse: Option<Mouse>,
    mouse_path: Option<String>,
    dock: Option<Dock>,
    dock_path: Option<String>,
    read_only: bool,
    offline: bool,
    music: Option<crate::audio::MusicCapture>,
    music_restore: Option<crate::dock::DockLighting>,
    charging_restore: Option<DockLighting>,
    dock_backup_directory: PathBuf,
    operation_error: Option<String>,
    operation_error_chain: Vec<String>,
}

impl Worker {
    fn new(events: Sender<AppSnapshot>, read_only: bool) -> Self {
        Self {
            keyboard: KeyboardSession::new(read_only),
            events,
            state: AppSnapshot {
                charging_indicator_enabled: true,
                ..AppSnapshot::default()
            },
            mouse: None,
            mouse_path: None,
            dock: None,
            dock_path: None,
            read_only,
            offline: false,
            music: None,
            music_restore: None,
            charging_restore: None,
            dock_backup_directory: data_dir().join("backups"),
            operation_error: None,
            operation_error_chain: Vec::new(),
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        let initial = (|| {
            let path = data_dir().join("charging-indicator.json");
            if path.exists() {
                self.state.charging_indicator_enabled = read_json(&path)?
                    .as_bool()
                    .context("Некорректная настройка индикации зарядки")?;
            }
            self.refresh()?;
            self.update_charging_indicator()
        })();
        self.finish_poll(initial);
        let mut last_poll = Instant::now();
        loop {
            let interval = if self.music.is_some() || self.state.charging_indicator_active || self.keyboard.needs_fast_tick() {
                Duration::from_millis(50)
            } else {
                Duration::from_secs(2)
            };
            match commands.recv_timeout(interval) {
                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if let Err(error) = self.keyboard.stop_activity() {
                        eprintln!("ACE75: восстановление настроек при выходе не подтверждено: {error:#}");
                    }
                    if let Err(error) = self.stop_charging_indicator() {
                        eprintln!("Не удалось восстановить подсветку после индикации зарядки: {error:#}");
                    }
                    if let Err(error) = self.stop_music() {
                        eprintln!(
                            "Не удалось восстановить подсветку MagDock при выходе: {error:#}"
                        );
                    }
                    break;
                }
                Ok(command) => {
                    let refreshing = matches!(&command, Command::Refresh | Command::SelectMouse(_));
                    self.state.busy = true;
                    self.operation_error = None;
                    self.operation_error_chain.clear();
                    self.state.error = None;
                    self.state.error_chain.clear();
                    let _ = self.events.send(self.state.clone());
                    let operation = self.execute(command);
                    let indicator = self.update_charging_indicator();
                    let result = combine_results(operation, indicator);
                    if refreshing {
                        self.finish_poll(result);
                    } else {
                        self.finish(result);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(error) = self.keyboard.tick() {
                        let cleanup = self.keyboard.stop_activity();
                        self.state.ace75 = self.keyboard.state.clone();
                        let result = match cleanup {
                            Ok(()) => Err(error),
                            Err(cleanup) => Err(error.context(format!("ACE75: восстановление настроек не подтверждено: {cleanup:#}"))),
                        };
                        self.finish(result);
                    }
                    let frame = if self.state.charging_indicator_active {
                        self.charging_frame()
                    } else if self.charging_restore.is_some() {
                        Ok(())
                    } else {
                        self.music_frame()
                    };
                    if let Err(error) = frame {
                        let charging = self.stop_charging_indicator();
                        let music = self.stop_music();
                        let restore = combine_results(charging, music);
                        let result = match restore {
                            Ok(()) => Err(error),
                            Err(restore) => Err(error.context(format!(
                                "Не удалось восстановить подсветку: {restore:#}"
                            ))),
                        };
                        self.finish(result);
                    }
                    if last_poll.elapsed() >= Duration::from_secs(2) {
                        let status = self.poll_status();
                        let indicator = self.update_charging_indicator();
                        let result = combine_results(status, indicator);
                        self.finish_poll(result);
                        last_poll = Instant::now();
                    }
                }
            }
        }
    }

    fn finish(&mut self, result: Result<()>) {
        self.state.busy = false;
        self.state.firmware_progress = None;
        let error = result.err();
        self.operation_error = error.as_ref().map(|error| format!("{error:#}"));
        self.operation_error_chain = error
            .map(|error| error.chain().map(ToString::to_string).collect())
            .unwrap_or_default();
        self.state.error = self.operation_error.clone();
        self.state.error_chain = self.operation_error_chain.clone();
        let _ = self.events.send(self.state.clone());
    }

    fn finish_poll(&mut self, result: Result<()>) {
        self.state.busy = false;
        if self.operation_error.is_some() {
            self.state.error = self.operation_error.clone();
            self.state.error_chain = self.operation_error_chain.clone();
        } else {
            let error = result.err();
            self.state.error = error.as_ref().map(|error| format!("{error:#}"));
            self.state.error_chain = error
                .map(|error| error.chain().map(ToString::to_string).collect())
                .unwrap_or_default();
        }
        let _ = self.events.send(self.state.clone());
    }

    fn mouse(&mut self) -> Result<&mut Mouse> {
        self.mouse.as_mut().context("Мышь не подключена")
    }
    fn dock(&mut self) -> Result<&mut Dock> {
        self.dock.as_mut().context("MagDock не подключена")
    }

    fn retire_mouse(&mut self) {
        self.mouse = None;
        self.mouse_path = None;
        self.state.active_mouse_path = None;
        self.state.mouse = None;
        self.state.adaptive = None;
    }

    fn retire_dock(&mut self) {
        self.dock = None;
        self.dock_path = None;
        self.state.dock = None;
    }

    fn discover(&mut self) -> Result<bool> {
        if self.offline {
            return Ok(false);
        }
        let devices = transport::enumerate()?;
        let mouse = devices
            .iter()
            .filter(|d| d.kind == DeviceKind::Mouse)
            .find(|d| Some(&d.path) == self.mouse_path.as_ref())
            .or_else(|| devices.iter().find(|d| d.kind == DeviceKind::Mouse));
        let dock = devices.iter().find(|d| d.kind == DeviceKind::Dock);
        let mut changed = false;
        if mouse.map(|d| &d.path) != self.mouse_path.as_ref() {
            self.mouse = None;
            self.mouse_path = None;
            self.state.active_mouse_path = None;
            self.state.mouse = None;
            self.state.adaptive = None;
            if let Some(info) = mouse {
                self.mouse = Some(Mouse::new(Box::new(HidTransport::open(info)?)));
                self.mouse_path = Some(info.path.clone());
                self.state.active_mouse_path = self.mouse_path.clone();
            }
            changed = true;
        }
        if dock.map(|d| &d.path) != self.dock_path.as_ref() {
            self.music = None;
            self.music_restore = None;
            self.charging_restore = None;
            self.state.charging_indicator_active = false;
            self.state.charging_percent = None;
            self.state.music_active = false;
            self.dock = None;
            self.dock_path = None;
            self.state.dock = None;
            if let Some(info) = dock {
                self.dock = Some(Dock::new(Box::new(HidTransport::open(info)?)));
                self.dock_path = Some(info.path.clone());
            }
            changed = true;
        }
        changed |= self.keyboard.discover(&devices)?;
        self.state.ace75 = self.keyboard.state.clone();
        self.state.devices = devices;
        Ok(changed)
    }

    fn refresh(&mut self) -> Result<()> {
        self.discover()?;
        self.state.presets = list_presets()?;
        self.state.macro_library = load_macro_library(&data_dir().join("macros"))?
            .into_keys()
            .collect();
        let mut errors = Vec::new();
        if let Err(error) = self.keyboard.refresh() {
            errors.push(format!("ACE75: {error:#}"));
        }
        self.state.ace75 = self.keyboard.state.clone();
        let has_receiver = self.has_receiver();
        if let Some(mouse) = &mut self.mouse {
            match mouse.read_snapshot_with_receiver(has_receiver) {
                Ok(snapshot) => {
                    self.state.adaptive = None;
                    self.state.mouse = Some(snapshot);
                }
                Err(error) => {
                    self.retire_mouse();
                    errors.push(format!("Мышь: {error:#}"));
                }
            }
        }
        if let Some(dock) = &mut self.dock {
            match dock.read_settings() {
                Ok(mut settings) => {
                    if let Some(configured) = &self.charging_restore {
                        settings.lighting = configured.clone();
                    }
                    self.state.dock = Some(settings);
                }
                Err(error) => {
                    if self.music.is_none() && self.charging_restore.is_none() {
                        self.retire_dock();
                    }
                    errors.push(format!("MagDock: {error:#}"));
                }
            }
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("\n"));
        }
        self.state.last_update = (!self.state.devices.is_empty()).then(Instant::now);
        self.state.message = if self.state.devices.is_empty() {
            "Подключите устройство MCHOSE через USB".into()
        } else {
            "Параметры прочитаны с устройств".into()
        };
        Ok(())
    }

    fn poll_status(&mut self) -> Result<()> {
        if self.discover()? {
            return self.refresh();
        }
        let mut profile_changed = false;
        let mut errors = Vec::new();
        if let Err(error) = self.keyboard.poll() {
            errors.push(format!("ACE75: {error:#}"));
        }
        self.state.ace75 = self.keyboard.state.clone();
        let configured = self.state.mouse.as_ref().map(|mouse| mouse.settings.clone());
        let charging_pending = self.charging_restore.is_some();
        let indicator_enabled = self.state.charging_indicator_enabled
            && !self.read_only && self.dock.is_some();
        if let Some(mouse) = &mut self.mouse {
            match mouse
                .read_status()
                .and_then(|status| {
                    let settings = if charging_pending
                        || indicator_enabled && matches!(status.charge_status, 1 | 2)
                    {
                        configured.clone().map(Ok).unwrap_or_else(|| mouse.read_settings())?
                    } else {
                        mouse.read_settings()?
                    };
                    Ok((status, settings))
                })
            {
                Ok((status, settings)) => {
                    if let Some(current) = &mut self.state.mouse {
                        profile_changed = current.settings.profile_index != settings.profile_index;
                        current.status = status;
                        current.dpi.selected = settings.dpi_index;
                        current.settings = settings;
                    } else {
                        profile_changed = true;
                    }
                }
                Err(error) => {
                    self.retire_mouse();
                    errors.push(format!("Мышь: {error:#}"));
                }
            }
        }
        if profile_changed {
            return self.refresh();
        }
        if self.music.is_none() && self.charging_restore.is_none() {
            if let Some(dock) = &mut self.dock {
                match dock.read_settings() {
                    Ok(settings) => self.state.dock = Some(settings),
                    Err(error) => {
                        self.retire_dock();
                        errors.push(format!("MagDock: {error:#}"));
                    }
                }
            }
        }
        if !errors.is_empty() {
            bail!("{}", errors.join("\n"));
        }
        self.state.last_update = (!self.state.devices.is_empty()).then(Instant::now);
        Ok(())
    }

    fn profile_file(&mut self) -> Result<ProfileFile> {
        let has_receiver = self.has_receiver();
        let mouse = self.mouse()?;
        let snapshot = mouse.read_snapshot_with_receiver(has_receiver)?;
        let adaptive = None;
        let mut macros = BTreeMap::new();
        for key in &snapshot.keys {
            if key.kind == 0x21 && key.data.len() >= 2 {
                let id = u16::from_le_bytes([key.data[0], key.data[1]]);
                ensure!(
                    id <= u8::MAX as u16,
                    "Индекс макроса не помещается в протокол"
                );
                if let std::collections::btree_map::Entry::Vacant(entry) = macros.entry(id as u8) {
                    entry.insert(mouse.read_macro(id as u8)?);
                }
            }
        }
        Ok(ProfileFile {
            schema_version: 1,
            model: firmware::MODEL.into(),
            snapshot,
            macros,
            adaptive,
        })
    }

    fn backup(&mut self) -> Result<()> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let snapshot = self.profile_file()?;
        save_new_json(
            &data_dir()
                .join("backups")
                .join(format!("profile-{timestamp}.json")),
            &snapshot,
        )
    }

    fn backup_profiles(&mut self, profiles: &[u8]) -> Result<()> {
        ensure!(
            !profiles.is_empty() && profiles.iter().all(|profile| *profile < 3),
            "Неверный список профилей для резервной копии"
        );
        let original = self.mouse()?.read_settings()?.profile_index;
        let result = (|| -> Result<()> {
            for profile in profiles {
                if self.mouse()?.read_settings()?.profile_index != *profile {
                    self.mouse()?.select_profile(*profile)?;
                    ensure!(
                        self.mouse()?.read_settings()?.profile_index == *profile,
                        "Не удалось выбрать профиль {} для копирования",
                        profile + 1
                    );
                }
                self.backup()?;
            }
            Ok(())
        })();
        let restore = (|| -> Result<()> {
            if self.mouse()?.read_settings()?.profile_index != original {
                self.mouse()?.select_profile(original)?;
                ensure!(
                    self.mouse()?.read_settings()?.profile_index == original,
                    "Не удалось вернуть исходный активный профиль"
                );
            }
            Ok(())
        })();
        match (result, restore) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error.context(
                "Резервные копии созданы, но исходный профиль не восстановлен; операция отменена",
            )),
            (Err(error), Err(restore)) => Err(error.context(format!(
                "Также не удалось вернуть исходный профиль: {restore:#}"
            ))),
        }
    }

    fn backup_macro_slots(&mut self, ids: &[u8]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        ensure!(ids.iter().all(|id| *id < 18), "Недопустимый слот макроса");
        let mut slots = BTreeMap::new();
        for id in ids {
            if slots.contains_key(id) {
                continue;
            }
            let mut bytes = Vec::new();
            while bytes.len() < 64 {
                let count = (64 - bytes.len()).min(22);
                bytes.extend(
                    self.mouse()?
                        .read_macro_raw(*id, bytes.len() as u16, count as u8)?,
                );
            }
            let event_bytes = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
            let empty =
                bytes.iter().all(|byte| *byte == 0xff) || bytes.iter().all(|byte| *byte == 0);
            if !empty {
                ensure!(
                    event_bytes % 4 == 0 && event_bytes <= 4096 - 64,
                    "Слот макроса {id} содержит неизвестную длину; запись отменена"
                );
                while bytes.len() < 64 + event_bytes {
                    let count = (64 + event_bytes - bytes.len()).min(22);
                    bytes.extend(self.mouse()?.read_macro_raw(
                        *id,
                        bytes.len() as u16,
                        count as u8,
                    )?);
                }
            }
            slots.insert(*id, bytes);
        }
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        save_new_json(
            &data_dir()
                .join("backups")
                .join(format!("macro-slots-{timestamp}.json")),
            &serde_json::json!({"schema_version":1,"model":firmware::MODEL,"raw_macro_slots":slots}),
        )
    }

    fn backup_dock(&mut self) -> Result<()> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let old = self.dock()?.read_settings()?;
        save_new_json(
            &self.dock_backup_directory
                .join(format!("dock-{timestamp}.json")),
            &old,
        )
    }

    fn save_macro_version(&mut self, id: u8, value: &MouseMacro) -> Result<()> {
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        save_new_json(
            &data_dir()
                .join("macros")
                .join(format!("{timestamp}-macro-{id}.json")),
            value,
        )?;
        self.state.macro_library = load_macro_library(&data_dir().join("macros"))?
            .into_keys()
            .collect();
        Ok(())
    }

    fn refresh_local_macros(&mut self) -> Result<()> {
        self.state.macro_library = load_macro_library(&data_dir().join("macros"))?
            .into_keys()
            .collect();
        Ok(())
    }

    fn apply_profile_cycle_all(&mut self, key_index: u8) -> Result<()> {
        ensure!(key_index < 6, "Неизвестный индекс кнопки");
        let original = self.mouse()?.read_settings()?.profile_index;
        for profile in 0..3 {
            let keys = self.mouse()?.read_keys(profile)?;
            ensure!(
                keys.get(key_index as usize)
                    .is_some_and(|key| key.kind != 0xff),
                "Кнопка отсутствует в профиле {}",
                profile + 1
            );
            ensure!(
                keys.iter()
                    .enumerate()
                    .any(|(index, key)| index != key_index as usize
                        && key.kind == 0
                        && key.data.first() == Some(&1)),
                "В профиле {} должна остаться хотя бы одна кнопка левого щелчка",
                profile + 1
            );
        }
        self.backup_profiles(&[0, 1, 2])?;
        let result = (|| -> Result<()> {
            for profile in 0..3 {
                let mut expected = self.mouse()?.read_keys(profile)?;
                let binding = KeyBinding::profile(0xff);
                expected[key_index as usize] = binding.clone();
                self.mouse()?.write_keys(profile, key_index, &[binding])?;
                ensure!(
                    self.mouse()?.read_keys(profile)? == expected,
                    "Назначение переключения профилей не подтверждено в профиле {}",
                    profile + 1
                );
            }
            Ok(())
        })();
        let restore = (|| -> Result<()> {
            if self.mouse()?.read_settings()?.profile_index != original {
                self.mouse()?.select_profile(original)?;
            }
            ensure!(
                self.mouse()?.read_settings()?.profile_index == original,
                "Не удалось вернуть исходный активный профиль"
            );
            Ok(())
        })();
        match (result, restore) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => {
                Err(error
                    .context("Изменение всех профилей не завершено; резервные копии сохранены"))
            }
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(restore)) => Err(error.context(format!(
                "Также не удалось вернуть активный профиль: {restore:#}"
            ))),
        }
    }

    fn execute(&mut self, command: Command) -> Result<()> {
        ensure!(
            !self.read_only || !command.writes_hardware(),
            "Включён режим только чтения"
        );
        if matches!(&command, Command::ApplyDock(_) | Command::StartMusic
            | Command::StopMusic | Command::ResetDock | Command::FlashDock(_)
            | Command::FlashMouse { .. }) {
            self.stop_charging_indicator()?;
        }
        match command {
            Command::Ace75(command) => {
                let progress = self.keyboard.state.progress_channel();
                let callback_progress = progress.clone();
                let result = self.keyboard.execute_with_progress(command, move |done, total| {
                    if let Ok(mut value) = callback_progress.lock() { *value = Some((done, total)); }
                });
                if let Ok(mut value) = progress.lock() { *value = None; }
                self.state.ace75 = self.keyboard.state.clone();
                if !self.state.ace75.message.is_empty() {
                    self.state.message = self.state.ace75.message.clone();
                }
                return result;
            }
            Command::Refresh => return self.refresh(),
            Command::SelectMouse(path) => {
                let info = self
                    .state
                    .devices
                    .iter()
                    .find(|d| d.path == path && d.kind == DeviceKind::Mouse)
                    .context("Устройство не найдено")?;
                self.mouse = Some(Mouse::new(Box::new(HidTransport::open(info)?)));
                self.mouse_path = Some(path);
                self.state.active_mouse_path = self.mouse_path.clone();
            }
            Command::ApplySettings(settings) => {
                self.backup_profiles(&[settings.profile_index])?;
                write_settings_verified(self.mouse()?, &settings)?;
            }
            Command::ApplyPerformance { settings, lod } => {
                settings.encode()?;
                ensure!(lod < 5, "Недопустимый LOD");
                self.backup_profiles(&[settings.profile_index])?;
                write_settings_verified(self.mouse()?, &settings)?;
                write_lod_verified(self.mouse()?, settings.profile_index, lod)?;
            }
            Command::ApplyDpi(dpi) => {
                self.backup_profiles(&[dpi.profile_index])?;
                write_dpi_verified(self.mouse()?, &dpi)?;
            }
            Command::ApplyKeys { profile, keys } => {
                validated_keys(&self.mouse()?.read_keys(profile)?, &keys)?;
                self.backup_profiles(&[profile])?;
                write_keys_verified(self.mouse()?, profile, &keys)?;
            }
            Command::ApplyProfileCycleAcrossAllSlots { key_index } => {
                self.apply_profile_cycle_all(key_index)?;
            }
            Command::SetLod { profile, lod } => {
                self.backup_profiles(&[profile])?;
                write_lod_verified(self.mouse()?, profile, lod)?;
            }
            Command::SelectProfile(profile) => {
                self.mouse()?.select_profile(profile)?;
                ensure!(
                    self.mouse()?.read_settings()?.profile_index == profile,
                    "Мышь не подтвердила выбор профиля"
                );
            }
            Command::RenameProfile { profile, name } => {
                self.backup_profiles(&[profile])?;
                write_name_verified(self.mouse()?, profile, &name)?;
            }
            Command::ApplyAdaptive { .. } => {
                bail!("Адаптивное энергосбережение не подтверждено для A7 V3 Ultra+")
            }
            Command::ApplyDock(mut settings) => {
                settings.lighting.music_sync = settings.lighting.mode == DockMode::Music;
                if settings.lighting.mode == DockMode::Music && settings.lighting.enabled {
                    return self.start_music(settings.lighting);
                }
                self.stop_music()?;
                self.backup_dock()?;
                self.state.dock = Some(self.dock()?.set_lighting(&settings.lighting)?);
            }
            Command::ReadMacro(id) => {
                self.state.macro_value = Some((id, self.mouse()?.read_macro(id)?));
            }
            Command::SaveLocalMacro(value) => {
                save_local_macro(&data_dir().join("macros"), &value, None)?;
                self.state.local_macro_value = Some(value);
                self.refresh_local_macros()?;
                self.state.message = "Макрос сохранён в локальную библиотеку".into();
                return Ok(());
            }
            Command::OpenLocalMacro(name) => {
                self.state.local_macro_value = Some(
                    load_macro_library(&data_dir().join("macros"))?
                        .remove(&name)
                        .context("Макрос отсутствует в библиотеке")?,
                );
                self.state.message = format!("Открыт локальный макрос «{name}»");
                return Ok(());
            }
            Command::UpdateLocalMacro { name, value } => {
                save_local_macro(&data_dir().join("macros"), &value, Some(&name))?;
                self.state.local_macro_value = Some(value);
                self.refresh_local_macros()?;
                self.state.message =
                    "Новая версия макроса сохранена; предыдущая сохранена в библиотеке".into();
                return Ok(());
            }
            Command::DeleteLocalMacro(name) => {
                let archive = archive_local_macro(
                    &data_dir().join("macros"),
                    &data_dir().join("backups"),
                    &name,
                    None,
                )?;
                self.state.local_macro_value = None;
                self.refresh_local_macros()?;
                self.state.message = format!(
                    "Макрос убран из библиотеки. Все версии: {}",
                    archive.display()
                );
                return Ok(());
            }
            Command::RenameLocalMacro { from, to } => {
                let archive = archive_local_macro(
                    &data_dir().join("macros"),
                    &data_dir().join("backups"),
                    &from,
                    Some(&to),
                )?;
                self.state.local_macro_value =
                    load_macro_library(&data_dir().join("macros"))?.remove(&to);
                self.refresh_local_macros()?;
                self.state.message = format!(
                    "Макрос переименован. Исходные версии: {}",
                    archive.display()
                );
                return Ok(());
            }
            Command::WriteMacro { id, value } => {
                self.backup()?;
                self.backup_macro_slots(&[id])?;
                self.mouse()?.write_macro(id, &value)?;
                let actual = self.mouse()?.read_macro(id)?;
                ensure!(
                    actual == value,
                    "Макрос после записи отличается от отправленного"
                );
                self.state.macro_value = Some((id, actual));
                self.save_macro_version(id, &value).context(
                    "Макрос записан и проверен, но не удалось сохранить его в локальной библиотеке",
                )?;
            }
            Command::ExportProfile(path) => {
                let value = self.profile_file()?;
                let mut exported = hub_format::profile_to_hub_json(&value.snapshot);
                exported[hub_format::HUB_MODEL]["macroInfo"] = serde_json::to_value(
                    value
                        .macros
                        .iter()
                        .map(|(id, value)| (format!("{id:04x}"), value.name.clone()))
                        .collect::<BTreeMap<_, _>>(),
                )?;
                save_new_json(&path, &exported)?;
                self.state.message = format!("Профиль сохранён: {}", path.display());
                return Ok(());
            }
            Command::SavePreset(name) => {
                let path = preset_path(&name)?;
                let value = self.profile_file()?;
                save_new_json(&path, &value)?;
                self.state.presets = list_presets()?;
                self.state.message = format!("Локальный профиль «{name}» сохранён");
                return Ok(());
            }
            Command::PreviewPreset(path) => {
                existing_preset(&path)?;
                let value = serde_json::from_value(read_json(&path)?)?;
                validate_local_profile(&value)?;
                self.state.preview = Some(value);
                self.state.message =
                    "Предпросмотр локального профиля. Настройки мыши не изменены.".into();
                return Ok(());
            }
            Command::SaveEditedPreset { path, value } => {
                existing_preset(&path)?;
                let backup = save_edited_preset(&path, &data_dir().join("backups"), &value)?;
                self.state.preview = Some(value);
                self.state.presets = list_presets()?;
                self.state.message = format!(
                    "Локальный профиль сохранён. Предыдущая версия: {}",
                    backup.display()
                );
                return Ok(());
            }
            Command::RenamePreset { path, name } => {
                existing_preset(&path)?;
                let target = preset_path(&name)?;
                if target != path {
                    fs::hard_link(&path, &target)
                        .context("Новое имя уже существует или недоступно")?;
                    fs::remove_file(path)?;
                }
                self.state.presets = list_presets()?;
                return Ok(());
            }
            Command::DeletePreset(path) => {
                existing_preset(&path)?;
                fs::remove_file(path)?;
                self.state.presets = list_presets()?;
                self.state.preview = None;
                return Ok(());
            }
            Command::ImportProfile(path) => {
                let value = read_json(&path)?;
                let has_receiver = self.has_receiver();
                let base = self.mouse()?.read_snapshot_with_receiver(has_receiver)?;
                let library = load_macro_library(&data_dir().join("macros"))?;
                let mut file = imported_profile(&value, &base, &library)?;
                let target = base.settings.profile_index;
                let name = remap_profile(&mut file, target)?;
                validated_keys(&base.keys, &file.snapshot.keys)?;
                for (index, key) in file
                    .snapshot
                    .keys
                    .iter()
                    .enumerate()
                    .filter(|(_, key)| key.kind != 0xff)
                {
                    ensure!(
                        base.keys
                            .get(index)
                            .is_some_and(|current| current.kind != 0xff),
                        "Импорт содержит назначение для отсутствующей кнопки {}",
                        index + 1
                    );
                    key.encode()?;
                }
                self.backup_profiles(&[target])?;
                self.backup_macro_slots(&file.macros.keys().copied().collect::<Vec<_>>())?;
                let mouse = self.mouse()?;
                for (id, value) in &file.macros {
                    mouse.write_macro(*id, value)?;
                    ensure!(
                        mouse.read_macro(*id)? == *value,
                        "Импорт макроса «{}» не подтверждён",
                        value.name
                    );
                }
                write_settings_verified(mouse, &file.snapshot.settings)?;
                write_dpi_verified(mouse, &file.snapshot.dpi)?;
                write_keys_verified(mouse, target, &file.snapshot.keys)?;
                write_lod_verified(mouse, target, file.snapshot.lod_index)?;
                if !name.is_empty() {
                    write_name_verified(mouse, target, &name)?;
                }
            }
            Command::ResetProfile {
                profile,
                buttons_only,
            } => {
                self.backup_profiles(&[profile])?;
                self.mouse()?.reset_profiles(&[profile], buttons_only)?;
            }
            Command::ResetAllProfiles => {
                self.backup_profiles(&[0, 1, 2])?;
                self.backup_macro_slots(&(0..18).collect::<Vec<_>>())?;
                self.mouse()?.reset_profiles(&[0, 1, 2], false)?;
            }
            Command::ResetDock => {
                self.stop_music()?;
                self.backup_dock()?;
                self.state.dock = Some(self.dock()?.reset_lighting()?);
            }
            Command::CheckFirmware => {
                self.state.firmware = Some(firmware::fetch_catalog()?);
                self.state.message = "Каталог прошивок получен с CDN MCHOSE".into();
                return Ok(());
            }
            Command::DownloadFirmware { index, directory } => {
                let image = self
                    .state
                    .firmware
                    .as_ref()
                    .and_then(|c| c.images.get(index))
                    .context("Сначала проверьте обновления")?;
                let files = firmware::download_pair(image, &directory)?;
                self.state.message =
                    format!("Скачано {} файла в {}", files.len(), directory.display());
                return Ok(());
            }
            Command::DownloadDockFirmware(directory) => {
                let file = firmware::download_dock(&directory)?;
                self.state.message = format!(
                    "MagDock: {} байт, SHA-256 {}, файл {}",
                    file.bytes,
                    file.sha256,
                    directory.join(file.filename).display()
                );
                return Ok(());
            }
            Command::FlashMouse {
                device_path,
                main,
                sub,
            } => {
                let expected = self
                    .state
                    .devices
                    .iter()
                    .find(|d| d.path == device_path && d.kind == DeviceKind::Mouse)
                    .cloned()
                    .context("Выберите подключённое устройство")?;
                let info = fresh_device(&expected)?;
                let catalog = match &self.state.firmware {
                    Some(catalog) => catalog.clone(),
                    None => firmware::fetch_catalog()?,
                };
                validate_firmware_files(&info, &main, &sub, &catalog)?;
                let main = fs::read(main)?;
                let sub = fs::read(sub)?;
                let index = match info.product_id {
                    0x4033 => 0,
                    0x1014 => 1,
                    0x1018 => 2,
                    _ => bail!("Неизвестная цель прошивки"),
                };
                firmware::verify_official_pair(&catalog.images[index], &main, &sub)?;
                let info = fresh_device(&expected)?;
                self.stop_music()?;
                let mut device = Mouse::new(Box::new(HidTransport::open(&info)?));
                let snapshot = device.read_snapshot_with_receiver(info.product_id != 0x4033)?;
                ensure!(
                    snapshot.status.vendor_id == info.vendor_id,
                    "Ответ пришёл от другого производителя; прошивка отменена"
                );
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
                save_new_json(
                    &data_dir()
                        .join("backups")
                        .join(format!("before-firmware-{timestamp}.json")),
                    &snapshot,
                )?;
                let events = self.events.clone();
                let mut progress_state = self.state.clone();
                progress_state.busy = true;
                let mut last_progress = Instant::now();
                device.update_firmware(&main, &sub, move |sent, total| {
                    if sent == 0
                        || sent == total
                        || last_progress.elapsed() >= Duration::from_millis(50)
                    {
                        progress_state.firmware_progress = Some((sent, total));
                        let _ = events.send(progress_state.clone());
                        last_progress = Instant::now();
                    }
                })?;
                self.mouse = None;
                self.mouse_path = None;
                self.state.active_mouse_path = None;
                self.state.mouse = None;
                self.state.message =
                    "Передача прошивки завершена. Переподключите устройство для проверки версии."
                        .into();
                return Ok(());
            }
            Command::FlashDock(path) => {
                ensure!(
                    fs::metadata(&path)?.len() <= 8_000_000,
                    "Файл прошивки слишком большой"
                );
                let image = fs::read(path)?;
                let plan = FirmwarePlan::from_full_image(&image)?;
                firmware::verify_official_dock(&image)?;
                let expected = self
                    .state
                    .devices
                    .iter()
                    .find(|device| Some(&device.path) == self.dock_path.as_ref())
                    .cloned()
                    .context("MagDock не подключена")?;
                let info = fresh_device(&expected)?;
                self.stop_music()?;
                self.dock = Some(Dock::new(Box::new(HidTransport::open(&info)?)));
                self.backup_dock()?;
                let events = self.events.clone();
                let mut progress_state = self.state.clone();
                progress_state.busy = true;
                let mut last_progress = Instant::now();
                self.dock()?.flash_firmware(&plan, move |sent, total| {
                    if sent == 0
                        || sent == total
                        || last_progress.elapsed() >= Duration::from_millis(50)
                    {
                        progress_state.firmware_progress = Some((sent, total));
                        let _ = events.send(progress_state.clone());
                        last_progress = Instant::now();
                    }
                })?;
                self.dock = None;
                self.dock_path = None;
                self.state.dock = None;
                self.state.message =
                    "Передача прошивки MagDock завершена. Переподключите док для проверки версии."
                        .into();
                return Ok(());
            }
            Command::StartMusic => {
                let mut lighting = self.dock()?.read_settings()?.lighting;
                lighting.mode = DockMode::Music;
                lighting.music_sync = true;
                lighting.enabled = true;
                return self.start_music(lighting);
            }
            Command::StopMusic => {
                self.stop_music()?;
                self.state.last_update = Some(Instant::now());
                self.state.message =
                    "Музыкальная подсветка остановлена; прежний эффект восстановлен".into();
                return Ok(());
            }
            Command::SetChargingIndicator(enabled) => {
                let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
                let directory = data_dir();
                let pending = directory.join(format!(".charging-indicator-{timestamp}.json"));
                save_new_json(&pending, &enabled)?;
                fs::rename(pending, directory.join("charging-indicator.json"))?;
                self.state.charging_indicator_enabled = enabled;
                self.state.message = if enabled {
                    "Во время зарядки кольцо показывает уровень батареи"
                } else {
                    "Индикация зарядки на кольце отключена"
                }.into();
                return Ok(());
            }
            Command::Stop => return Ok(()),
        }
        self.refresh()?;
        self.state.message = "Операция выполнена, параметры перечитаны с устройства".into();
        Ok(())
    }

    fn has_receiver(&self) -> bool {
        self.state
            .devices
            .iter()
            .any(|d| Some(&d.path) == self.mouse_path.as_ref() && d.product_id != 0x4033)
    }

    fn update_charging_indicator(&mut self) -> Result<()> {
        let percent = self.state.mouse.as_ref()
            .filter(|mouse| matches!(mouse.status.charge_status, 1 | 2))
            .map(|mouse| mouse.status.battery_percent.min(100))
            .filter(|_| self.state.charging_indicator_enabled && !self.read_only && self.dock.is_some());
        let Some(percent) = percent else {
            return self.stop_charging_indicator();
        };
        if !self.state.charging_indicator_active {
            if self.charging_restore.is_none() {
                self.backup_dock()?;
                self.charging_restore = Some(self.dock()?.read_settings()?.lighting);
            }
            let mut lighting = self.charging_restore.as_ref().unwrap().clone();
            lighting.enabled = true;
            lighting.mode = DockMode::Music;
            lighting.music_sync = true;
            if let Err(error) = self.dock()?.set_lighting(&lighting) {
                return match self.stop_charging_indicator() {
                    Ok(()) => Err(error.context("Не удалось включить индикацию зарядки; прежний эффект восстановлен")),
                    Err(restore) => Err(error.context(format!("Не удалось восстановить подсветку: {restore:#}"))),
                };
            }
        }
        self.state.charging_indicator_active = true;
        self.state.charging_percent = Some(percent);
        self.charging_frame()
    }

    fn charging_frame(&mut self) -> Result<()> {
        let Some(configured) = &self.charging_restore else { return Ok(()); };
        let Some(percent) = self.state.charging_percent else { return Ok(()); };
        let colors = crate::charging::charging_colors(percent, configured.brightness, configured.direction);
        self.dock()?.stream_leds(&colors)
    }

    fn stop_charging_indicator(&mut self) -> Result<()> {
        self.state.charging_indicator_active = false;
        self.state.charging_percent = None;
        if let Some(configured) = self.charging_restore.clone() {
            self.state.dock = Some(self.dock()?.set_lighting(&configured)?);
            self.charging_restore = None;
        }
        Ok(())
    }

    fn start_music(&mut self, lighting: DockLighting) -> Result<()> {
        lighting.validate()?;
        self.backup_dock()?;
        if self.music.is_none() {
            let previous = self.dock()?.read_settings()?.lighting;
            let capture = crate::audio::MusicCapture::start()?;
            self.music_restore = Some(previous);
            self.music = Some(capture);
        }
        match self.dock()?.set_lighting(&lighting) {
            Ok(settings) => self.state.dock = Some(settings),
            Err(error) => {
                let rollback = self.stop_music();
                return match rollback {
                    Ok(()) => Err(error.context("Не удалось включить музыкальную подсветку; прежние параметры восстановлены")),
                    Err(restore) => Err(error.context(format!("Не удалось восстановить подсветку: {restore:#}"))),
                };
            }
        }
        self.state.message = format!(
            "Подсветка следует за звуком: {}",
            self.music.as_ref().unwrap().sink_name()
        );
        self.state.music_active = true;
        self.state.last_update = Some(Instant::now());
        Ok(())
    }

    fn music_frame(&mut self) -> Result<()> {
        let Some(music) = &mut self.music else {
            return Ok(());
        };
        if let Some(error) = music.take_error() {
            bail!("{error}");
        }
        if let Some(settings) = &self.state.dock {
            if let Some(colors) =
                music.latest_colors(settings.lighting.color, settings.lighting.brightness)
            {
                self.dock()?.stream_leds(&colors)?;
            }
        }
        Ok(())
    }

    fn stop_music(&mut self) -> Result<()> {
        self.music = None;
        self.state.music_active = false;
        if let Some(lighting) = self.music_restore.clone() {
            self.state.dock = Some(self.dock()?.set_lighting(&lighting)?);
            self.music_restore = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mouse::encode_frame;
    use crate::transport::HidIo;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let unique = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mchose-macro-test-{}-{timestamp}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn sample_macro(name: &str) -> MouseMacro {
        MouseMacro {
            name: name.into(),
            repeat_mode: 0,
            reserved: [0; 5],
            events: vec![crate::mouse::MacroEvent {
                delay_ms: 0,
                kind: 17,
                pressed: true,
                value: 4,
            }],
        }
    }

    #[test]
    fn local_macro_updates_preserve_old_version_and_reject_duplicate_creation() {
        let root = TestDirectory::new();
        let directory = root.0.join("macros");
        let original = sample_macro("Reload");
        save_local_macro(&directory, &original, None).unwrap();
        let original_path = fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let original_bytes = fs::read(&original_path).unwrap();
        assert!(save_local_macro(&directory, &original, None).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        let mut changed = original.clone();
        changed.events[0].value = 5;
        save_local_macro(&directory, &changed, Some("Reload")).unwrap();
        assert_eq!(load_macro_library(&directory).unwrap()["Reload"], changed);
        assert_eq!(fs::read(&original_path).unwrap(), original_bytes);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        changed.name = "Other".into();
        assert!(save_local_macro(&directory, &changed, Some("Reload")).is_err());
    }

    #[test]
    fn rename_and_delete_archive_all_versions_and_keep_other_hub_bundle_macros() {
        let root = TestDirectory::new();
        let directory = root.0.join("macros");
        let backups = root.0.join("backups");
        let original = sample_macro("Reload");
        let other = sample_macro("Other");
        let bundle = serde_json::json!([
            hub_format::macro_to_hub_json(&original).unwrap(),
            hub_format::macro_to_hub_json(&other).unwrap()
        ]);
        let bundle_path = directory.join("source-hub.json");
        save_new_json(&bundle_path, &bundle).unwrap();
        let bundle_bytes = fs::read(&bundle_path).unwrap();
        let mut latest = original.clone();
        latest.events[0].value = 5;
        save_local_macro(&directory, &latest, Some("Reload")).unwrap();
        assert!(archive_local_macro(&directory, &backups, "Reload", Some("Other")).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
        let archive = archive_local_macro(&directory, &backups, "Reload", Some("Renamed")).unwrap();
        let library = load_macro_library(&directory).unwrap();
        assert!(!library.contains_key("Reload"));
        assert_eq!(library["Other"], other);
        latest.name = "Renamed".into();
        assert_eq!(library["Renamed"], latest);
        assert_eq!(
            fs::read(archive.join("originals/source-hub.json")).unwrap(),
            bundle_bytes
        );
        assert_eq!(fs::read_dir(archive.join("originals")).unwrap().count(), 2);
        save_local_macro(&directory, &latest, Some("Renamed")).unwrap();
        let deleted = archive_local_macro(&directory, &backups, "Renamed", None).unwrap();
        assert_eq!(fs::read_dir(deleted.join("originals")).unwrap().count(), 2);
        assert_eq!(
            load_macro_library(&directory).unwrap(),
            BTreeMap::from([("Other".into(), other)])
        );
    }

    #[test]
    fn local_commands_are_allowed_read_only_and_offline_discovery_opens_no_hid() {
        for command in [
            Command::SaveLocalMacro(sample_macro("New")),
            Command::OpenLocalMacro("New".into()),
            Command::UpdateLocalMacro {
                name: "New".into(),
                value: sample_macro("New"),
            },
            Command::DeleteLocalMacro("New".into()),
            Command::RenameLocalMacro {
                from: "New".into(),
                to: "Old".into(),
            },
        ] {
            assert!(!command.writes_hardware());
        }
        let (events, _) = mpsc::channel();
        let mut worker = Worker::new(events, true);
        worker.offline = true;
        assert!(!worker.discover().unwrap());
        assert!(worker.state.devices.is_empty() && worker.mouse.is_none() && worker.dock.is_none());
    }

    #[test]
    fn key_validation_preserves_last_left_click_even_for_partial_updates() {
        let original = vec![
            KeyBinding::mouse(1),
            KeyBinding::mouse(2),
            KeyBinding::disabled(),
        ];
        assert!(validated_keys(&original, &[KeyBinding::disabled()]).is_err());
        let remaining_left = vec![
            KeyBinding::mouse(2),
            KeyBinding::mouse(1),
            KeyBinding::disabled(),
        ];
        assert_eq!(
            validated_keys(&remaining_left, &[KeyBinding::disabled()]).unwrap(),
            [KeyBinding::disabled()]
        );
        let absent = KeyBinding {
            kind: 0xff,
            data: vec![0xff, 0xff],
        };
        let before = vec![KeyBinding::mouse(1), absent];
        assert!(validated_keys(&before, &[KeyBinding::mouse(1), KeyBinding::mouse(2)]).is_err());
    }

    #[test]
    fn edited_preset_validates_offline_and_keeps_exact_previous_bytes() {
        let root = TestDirectory::new();
        let path = root.0.join("preset.json");
        let backups = root.0.join("backups");
        let original = ProfileFile {
            schema_version: 1,
            model: firmware::MODEL.into(),
            snapshot: snapshot(),
            macros: BTreeMap::new(),
            adaptive: None,
        };
        let original_bytes = serde_json::to_vec(&original).unwrap();
        fs::write(&path, &original_bytes).unwrap();
        let mut changed = original.clone();
        changed.snapshot.settings.left_debounce_ms += 1;
        let backup = save_edited_preset(&path, &backups, &changed).unwrap();
        assert_eq!(fs::read(backup).unwrap(), original_bytes);
        let actual: ProfileFile = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(actual.snapshot.settings, changed.snapshot.settings);
        assert_eq!(actual.snapshot.dpi, original.snapshot.dpi);
        assert_eq!(actual.snapshot.keys, original.snapshot.keys);
        assert!(
            !Command::SaveEditedPreset {
                path,
                value: changed
            }
            .writes_hardware()
        );
    }

    #[test]
    fn invalid_edited_preset_and_missing_macro_leave_file_and_backups_untouched() {
        let root = TestDirectory::new();
        let path = root.0.join("preset.json");
        let backups = root.0.join("backups");
        let original = ProfileFile {
            schema_version: 1,
            model: firmware::MODEL.into(),
            snapshot: snapshot(),
            macros: BTreeMap::new(),
            adaptive: None,
        };
        let original_bytes = serde_json::to_vec(&original).unwrap();
        fs::write(&path, &original_bytes).unwrap();
        let mut invalid = original.clone();
        invalid.snapshot.dpi.x[0] = 150;
        assert!(save_edited_preset(&path, &backups, &invalid).is_err());
        invalid = original.clone();
        invalid.snapshot.keys[1] = KeyBinding::macro_id(4);
        let error = save_edited_preset(&path, &backups, &invalid)
            .unwrap_err()
            .to_string();
        assert!(error.contains("слот 4"));
        invalid = original;
        invalid.schema_version = 2;
        assert!(save_edited_preset(&path, &backups, &invalid).is_err());
        assert_eq!(fs::read(path).unwrap(), original_bytes);
        assert!(!backups.exists());
    }

    fn snapshot() -> MouseSnapshot {
        let evidence: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/device-snapshot.json")).unwrap();
        serde_json::from_value(evidence["mouse"].clone()).unwrap()
    }

    struct DockHid {
        responses: Mutex<VecDeque<Vec<u8>>>,
        writes: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl HidIo for DockHid {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> { bail!("Unexpected feature write") }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> { bail!("Unexpected feature read") }
        fn write_output(&self, id: u8, payload: &[u8]) -> Result<()> {
            ensure!(id == 0, "Expected unnumbered MagDock report");
            self.writes.lock().unwrap().push(payload.to_vec());
            Ok(())
        }
        fn read_input(&self, timeout: i32) -> Result<Vec<u8>> {
            if timeout == 0 { return Ok(Vec::new()); }
            Ok(self.responses.lock().unwrap().pop_front().unwrap_or_default())
        }
    }

    fn charging_worker(responses: Vec<Vec<u8>>, directory: &Path) -> (Worker, Arc<Mutex<Vec<Vec<u8>>>>) {
        let (events, _) = mpsc::channel();
        let mut worker = Worker::new(events, false);
        worker.dock_backup_directory = directory.join("backups");
        let writes = Arc::new(Mutex::new(Vec::new()));
        worker.dock = Some(Dock::new(Box::new(DockHid {
            responses: Mutex::new(responses.into()), writes: writes.clone(),
        })));
        worker.state.mouse = Some(snapshot());
        worker.state.mouse.as_mut().unwrap().status.charge_status = 1;
        worker.state.mouse.as_mut().unwrap().status.battery_percent = 50;
        (worker, writes)
    }

    fn dock_capture() -> Vec<u8> {
        let capture: serde_json::Value = serde_json::from_str(include_str!(
            "../research/fixtures/dock-music-0003.json"
        )).unwrap();
        let mut raw: Vec<u8> = serde_json::from_value(capture["started"].clone()).unwrap();
        raw[11] = 0;
        raw
    }

    #[test]
    fn charging_indicator_tracks_battery_and_restores_effect_after_removal() {
        let directory = TestDirectory::new();
        let original = dock_capture();
        let mut music = original.clone();
        music[11] = 1;
        let (mut worker, writes) = charging_worker(vec![
            original.clone(), original.clone(), music, original.clone(),
        ], &directory.0);
        let configured = crate::dock::decode_settings(&original).unwrap();
        worker.state.dock = Some(configured.clone());
        worker.update_charging_indicator().unwrap();
        assert!(worker.state.charging_indicator_active);
        assert_eq!(worker.state.charging_percent, Some(50));
        assert_eq!(worker.state.dock.as_ref().unwrap().lighting, configured.lighting);
        assert_eq!(fs::read_dir(directory.0.join("backups")).unwrap().count(), 1);
        let first = writes.lock().unwrap().last().unwrap().clone();
        assert_eq!(first[1], 0x29);
        assert_eq!(first[6..42].chunks_exact(3).filter(|rgb| *rgb == [255, 255, 0]).count(), 6);
        assert_eq!(first[6..42].chunks_exact(3).filter(|rgb| *rgb == [0, 0, 0]).count(), 6);

        worker.state.mouse.as_mut().unwrap().status.battery_percent = 100;
        worker.state.mouse.as_mut().unwrap().status.charge_status = 2;
        worker.update_charging_indicator().unwrap();
        assert_eq!(&writes.lock().unwrap().last().unwrap()[6..42], [0, 255, 0].repeat(12));
        worker.state.mouse.as_mut().unwrap().status.charge_status = 0;
        worker.update_charging_indicator().unwrap();
        assert!(!worker.state.charging_indicator_active);
        assert_eq!(worker.state.charging_percent, None);
        assert_eq!(worker.state.dock.unwrap().lighting, configured.lighting);
        assert!(worker.charging_restore.is_none());
    }

    #[test]
    fn charging_indicator_respects_disabled_and_read_only_modes() {
        for read_only in [false, true] {
            let directory = TestDirectory::new();
            let (mut worker, writes) = charging_worker(vec![], &directory.0);
            worker.read_only = read_only;
            worker.state.charging_indicator_enabled = read_only;
            worker.update_charging_indicator().unwrap();
            assert!(writes.lock().unwrap().is_empty());
            assert!(!worker.state.charging_indicator_active);
            assert!(!directory.0.join("backups").exists());
        }
    }

    #[test]
    fn charging_status_does_not_require_settings_reads_while_mouse_is_on_dock() {
        let directory = TestDirectory::new();
        let (mut worker, _) = charging_worker(vec![dock_capture()], &directory.0);
        worker.offline = true;
        let mut status = snapshot().status.raw;
        status[11] = 1;
        status[12] = 50;
        let writes = Arc::new(Mutex::new(Vec::new()));
        worker.mouse = Some(Mouse::new(Box::new(MockHid {
            input: Mutex::new(VecDeque::from([encode_frame(0x0900, &status, 64).unwrap()])),
            output: writes.clone(),
        })));
        worker.poll_status().unwrap();
        assert_eq!(worker.state.mouse.as_ref().unwrap().status.charge_status, 1);
        assert_eq!(worker.state.mouse.as_ref().unwrap().status.battery_percent, 50);
        let writes = writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(&writes[0][4..6], &[0, 9]);
    }

    #[test]
    fn rejected_charging_override_rolls_back_without_streaming() {
        let directory = TestDirectory::new();
        let original = dock_capture();
        let (mut worker, writes) = charging_worker(vec![original.clone(); 4], &directory.0);
        let error = combine_results(
            Err(anyhow::anyhow!("Тайм-аут состояния мыши")),
            worker.update_charging_indicator(),
        ).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("Тайм-аут состояния мыши"));
        assert!(message.contains("MagDock не подтвердил"));
        assert!(worker.charging_restore.is_none());
        assert!(!worker.state.charging_indicator_active);
        assert_eq!(worker.state.dock.unwrap().lighting, crate::dock::decode_settings(&original).unwrap().lighting);
        assert!(!writes.lock().unwrap().iter().any(|packet| packet[1] == 0x29));
    }

    #[test]
    fn failed_charging_restore_stops_frames_and_keeps_original_for_retry() {
        let directory = TestDirectory::new();
        let original = dock_capture();
        let mut music = original.clone();
        music[11] = 1;
        let (mut worker, writes) = charging_worker(vec![
            original.clone(), original.clone(), music.clone(), music, original.clone(),
        ], &directory.0);
        worker.update_charging_indicator().unwrap();
        worker.state.charging_indicator_enabled = false;
        assert!(worker.update_charging_indicator().is_err());
        assert!(!worker.state.charging_indicator_active);
        assert_eq!(worker.state.charging_percent, None);
        assert!(worker.charging_restore.is_some());
        let count = writes.lock().unwrap().len();
        worker.charging_frame().unwrap();
        assert_eq!(writes.lock().unwrap().len(), count);
        worker.update_charging_indicator().unwrap();
        assert!(worker.charging_restore.is_none());
        assert_eq!(worker.state.dock.unwrap().lighting, crate::dock::decode_settings(&original).unwrap().lighting);
    }

    struct MockHid {
        input: Mutex<VecDeque<Vec<u8>>>,
        output: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl HidIo for MockHid {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            bail!("Unexpected feature write")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            bail!("Unexpected feature read")
        }
        fn write_output(&self, id: u8, payload: &[u8]) -> Result<()> {
            let mut packet = vec![id];
            packet.extend_from_slice(payload);
            self.output.lock().unwrap().push(packet);
            Ok(())
        }
        fn read_input(&self, _: i32) -> Result<Vec<u8>> {
            Ok(self.input.lock().unwrap().pop_front().unwrap_or_default())
        }
    }

    #[test]
    fn failed_device_handle_is_retired_without_closing_other_device() {
        let mock = || {
            Box::new(MockHid {
                input: Mutex::new(VecDeque::new()),
                output: Arc::new(Mutex::new(Vec::new())),
            })
        };
        let (events, _) = mpsc::channel();
        let mut worker = Worker::new(events, true);
        worker.mouse = Some(Mouse::new(mock()));
        worker.mouse_path = Some("/dev/mock-mouse".into());
        worker.state.active_mouse_path = worker.mouse_path.clone();
        worker.state.mouse = Some(snapshot());
        worker.dock = Some(Dock::new(mock()));
        worker.dock_path = Some("/dev/mock-dock".into());
        worker.retire_mouse();
        assert!(worker.mouse.is_none() && worker.mouse_path.is_none());
        assert!(worker.state.mouse.is_none() && worker.state.active_mouse_path.is_none());
        assert!(worker.dock.is_some() && worker.dock_path.as_deref() == Some("/dev/mock-dock"));
        worker.mouse = Some(Mouse::new(mock()));
        worker.retire_dock();
        assert!(worker.dock.is_none() && worker.dock_path.is_none());
        assert!(worker.mouse.is_some());
    }

    #[test]
    fn write_ack_is_not_success_when_readback_differs() {
        let actual = snapshot().settings;
        let mut desired = actual.clone();
        desired.left_debounce_ms += 1;
        let output = Arc::new(Mutex::new(Vec::new()));
        let io = MockHid {
            input: Mutex::new(VecDeque::from([
                encode_frame(0x0102, &[0], 64).unwrap(),
                encode_frame(0x0002, &actual.encode().unwrap(), 64).unwrap(),
            ])),
            output: Arc::clone(&output),
        };
        let mut mouse = Mouse::new(Box::new(io));
        let error = write_settings_verified(&mut mouse, &desired)
            .unwrap_err()
            .to_string();
        assert!(error.contains("не подтвердила"));
        let output = output.lock().unwrap();
        assert_eq!(output.len(), 2);
        assert_eq!(&output[0][4..6], &[2, 1]);
        assert_eq!(&output[1][4..6], &[2, 0]);
    }

    #[test]
    fn native_and_hub_import_keep_source_name_when_remapped() {
        let base = snapshot();
        let mut source = base.clone();
        source.settings.profile_index = 2;
        source.dpi.profile_index = 2;
        source.profile_names = vec!["Target name".into(), "Other".into(), "Source name".into()];
        let native = ProfileFile {
            schema_version: 1,
            model: firmware::MODEL.into(),
            snapshot: source.clone(),
            macros: BTreeMap::new(),
            adaptive: None,
        };
        for value in [
            serde_json::to_value(native).unwrap(),
            hub_format::profile_to_hub_json(&source),
        ] {
            let mut imported = imported_profile(&value, &base, &BTreeMap::new()).unwrap();
            assert_eq!(remap_profile(&mut imported, 0).unwrap(), "Source name");
            assert_eq!(imported.snapshot.settings.profile_index, 0);
            assert_eq!(imported.snapshot.dpi.profile_index, 0);
        }
    }

    #[test]
    fn hub_import_reports_exact_missing_macro_before_writes() {
        let base = snapshot();
        let mut source = base.clone();
        source.keys[1] = KeyBinding::macro_id(10);
        let mut value = hub_format::profile_to_hub_json(&source);
        value[hub_format::HUB_MODEL]["macroInfo"] = serde_json::json!({"000a":"Reload"});
        let error = imported_profile(&value, &base, &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("Reload") && error.contains("слот 10") && error.contains("до записи")
        );
        let macro_value = MouseMacro {
            name: "Reload".into(),
            repeat_mode: 0,
            events: vec![],
            reserved: [0; 5],
        };
        let library = BTreeMap::from([("Reload".into(), macro_value.clone())]);
        assert_eq!(
            imported_profile(&value, &base, &library).unwrap().macros[&10],
            macro_value
        );
    }

    #[test]
    fn polling_does_not_erase_failed_operation() {
        let (sender, receiver) = mpsc::channel();
        let mut worker = Worker::new(sender, true);
        worker.finish_poll(Err(anyhow::anyhow!("Mouse is temporarily unavailable")));
        assert_eq!(
            receiver.recv().unwrap().error.as_deref(),
            Some("Mouse is temporarily unavailable")
        );
        assert!(worker.operation_error.is_none());
        worker.finish_poll(Ok(()));
        let cleared = receiver.recv().unwrap();
        assert!(cleared.error.is_none());
        assert!(cleared.error_chain.is_empty());
        worker.finish(Err(anyhow::anyhow!("Write was not confirmed")
            .context("Saved profile: C:\\Нет данных.json")));
        worker.finish_poll(Err(anyhow::anyhow!("Temporary polling error")));
        let events: Vec<_> = receiver.try_iter().collect();
        assert_eq!(
            events.last().unwrap().error.as_deref(),
            Some("Saved profile: C:\\Нет данных.json: Write was not confirmed")
        );
        assert_eq!(events.last().unwrap().error_chain, [
            "Saved profile: C:\\Нет данных.json", "Write was not confirmed"
        ]);
        worker.finish(Ok(()));
        let cleared = receiver.recv().unwrap();
        assert!(cleared.error.is_none());
        assert!(cleared.error_chain.is_empty());
    }

    #[test]
    fn reused_hidraw_path_is_not_same_device() {
        let expected = DeviceInfo {
            path: "/dev/hidraw6".into(),
            name: "MCHOSE A7 V3 Ultra+".into(),
            vendor_id: 0x3837,
            product_id: 0x1014,
            interface: 2,
            usage_page: 0xff01,
            usage: 1,
            kind: DeviceKind::Mouse,
        };
        assert!(same_device(&expected, &expected));
        let mut changed = expected.clone();
        changed.product_id = 0x1012;
        assert!(!same_device(&expected, &changed));
        changed = expected.clone();
        changed.usage_page = 1;
        assert!(!same_device(&expected, &changed));
    }
}
