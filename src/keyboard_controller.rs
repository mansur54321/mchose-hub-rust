use crate::ace75::hub_profile::{self, HubProfileScope};
use crate::ace75::{
    firmware_catalog::{self, KeyboardFirmwareInfo},
    firmware_service::{self, PreparedKeyboardFirmware},
    host_lighting::{self, AnimationPlayer, KeyboardMusic, LightingAnimation},
};
use crate::{
    ace75::{
        self, Ace75, AdvancedState, CustomParams, HubProfileDocument, KeyAssignment, KeyTrigger,
        KeyboardBase, KeyboardInfo, KeyboardNotification, KeyboardProfile, KeyboardSnapshot,
        Lighting, MacroBank, MacroDefinition, MacroLayout, Performance, RawKeyboardProfile,
    },
    controller::{PresetInfo, data_dir, save_new_json},
    transport::{DeviceInfo, DeviceKind, HidTransport},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone)]
pub struct KeyboardTravel {
    pub key: KeyAssignment,
    pub travel_raw: u16,
}

#[derive(Debug, Clone, Default)]
struct KeyboardTelemetry {
    travel: Option<KeyboardTravel>,
    calibration_state: Option<u8>,
    calibration_keys: BTreeMap<u16, u8>,
}

#[derive(Debug, Clone, Default)]
pub struct KeyboardState {
    pub device: Option<DeviceInfo>,
    pub bootloader: Option<DeviceInfo>,
    pub snapshot: Option<KeyboardSnapshot>,
    pub revision: u64,
    pub presets: Vec<PresetInfo>,
    pub preview: Option<KeyboardPreset>,
    pub message: String,
    pub error: Option<String>,
    pub cleanup_warning: Option<String>,
    pub calibration_active: bool,
    pub monitor_active: bool,
    pub calibration_state: Option<u8>,
    pub calibration_keys: BTreeMap<u16, u8>,
    pub travel: Option<KeyboardTravel>,
    pub macros: Vec<PresetInfo>,
    pub local_macro: Option<MacroDefinition>,
    pub macro_revision: u64,
    pub music_active: bool,
    pub animation_active: bool,
    pub animation: Option<LightingAnimation>,
    pub firmware: Option<KeyboardFirmwareInfo>,
    telemetry: Arc<Mutex<KeyboardTelemetry>>,
    firmware_progress: Arc<Mutex<Option<(usize, usize)>>>,
}

impl KeyboardState {
    pub(crate) fn progress_channel(&self) -> Arc<Mutex<Option<(usize, usize)>>> {
        self.firmware_progress.clone()
    }

    pub(crate) fn firmware_progress(&self) -> Option<(usize, usize)> {
        self.firmware_progress.lock().ok().and_then(|value| *value)
    }

    pub(crate) fn pull_telemetry(&mut self) {
        if let Ok(value) = self.telemetry.lock() {
            self.travel = value.travel.clone();
            self.calibration_state = value.calibration_state;
            self.calibration_keys = value.calibration_keys.clone();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyboardPreset {
    pub schema_version: u32,
    pub model: String,
    pub profile: KeyboardProfile,
    #[serde(default)]
    pub macros: Option<MacroBank>,
    #[serde(default)]
    pub hub_document: Option<HubProfileDocument>,
    #[serde(default)]
    pub hub_scope: Option<HubProfileScope>,
}

#[derive(Debug, Clone)]
pub enum KeyboardCommand {
    Refresh,
    Backup,
    ApplyProfileLayout {
        device_path: String,
        profile: u8,
        profile_count: u8,
        profile_order: Vec<u8>,
    },
    Reset {
        device_path: String,
        profile: Option<u8>,
    },
    SetMusic {
        device_path: String,
        profile: u8,
        enabled: bool,
        scopes: Vec<ace75::LightScope>,
    },
    LoadAnimation(PathBuf),
    SaveAnimation {
        path: PathBuf,
        value: LightingAnimation,
    },
    PlayAnimation {
        device_path: String,
        profile: u8,
        value: LightingAnimation,
    },
    StopAnimation,
    CheckFirmware,
    DownloadFirmware(PathBuf),
    FlashFirmware {
        path: PathBuf,
    },
    SwitchProfile {
        device_path: String,
        profile: u8,
    },
    ApplyLighting {
        device_path: String,
        profile: u8,
        value: Lighting,
    },
    ApplyPerformance {
        device_path: String,
        profile: u8,
        value: Performance,
    },
    ApplyReportRate {
        device_path: String,
        profile: u8,
        report_rate: u8,
    },
    ApplyKeys {
        device_path: String,
        profile: u8,
        layer: u8,
        values: Vec<KeyAssignment>,
    },
    ApplyGlobalKeys {
        device_path: String,
        profile: u8,
        indices: Vec<(u8, KeyAssignment)>,
    },
    ApplyColors {
        device_path: String,
        profile: u8,
        colors: Vec<[u8; 3]>,
    },
    ApplyAdvanced {
        device_path: String,
        profile: u8,
        advanced: AdvancedState,
        layers: Vec<Vec<KeyAssignment>>,
        triggers: Vec<KeyTrigger>,
        custom_params: CustomParams,
    },
    ApplyCustomParams {
        device_path: String,
        profile: u8,
        value: CustomParams,
    },
    ApplyMacros {
        device_path: String,
        profile: u8,
        bank: MacroBank,
    },
    RenameOnboardProfile {
        device_path: String,
        profile: u8,
        name: String,
    },
    StartCalibration {
        device_path: String,
        profile: u8,
    },
    RestartCalibration,
    StopCalibration,
    Telemetry {
        device_path: String,
        profile: u8,
        enabled: bool,
    },
    SaveLocalMacro {
        name: String,
        value: MacroDefinition,
    },
    UpdateLocalMacro {
        path: PathBuf,
        value: MacroDefinition,
    },
    OpenLocalMacro(PathBuf),
    ImportLocalMacro(PathBuf),
    RenameLocalMacro {
        path: PathBuf,
        name: String,
    },
    DeleteLocalMacro(PathBuf),
    ExportLocalMacro {
        path: PathBuf,
        value: MacroDefinition,
    },
    ApplyTriggers {
        device_path: String,
        profile: u8,
        values: Vec<KeyTrigger>,
        tachyon_mode: bool,
    },
    SavePreset(String),
    ImportPreset(PathBuf),
    PreviewPreset(PathBuf),
    ClosePreview,
    SaveEditedPreset {
        path: PathBuf,
        value: KeyboardPreset,
    },
    ApplyPreset {
        device_path: String,
        profile: u8,
        path: PathBuf,
    },
    RenamePreset {
        path: PathBuf,
        name: String,
    },
    DeletePreset(PathBuf),
    ExportProfile(PathBuf),
    ExportHubProfile(PathBuf),
}

impl KeyboardCommand {
    pub fn writes_hardware(&self) -> bool {
        matches!(
            self,
            Self::SwitchProfile { .. }
                | Self::ApplyLighting { .. }
                | Self::ApplyPerformance { .. }
                | Self::ApplyReportRate { .. }
                | Self::ApplyKeys { .. }
                | Self::ApplyGlobalKeys { .. }
                | Self::ApplyTriggers { .. }
                | Self::ApplyPreset { .. }
                | Self::ApplyColors { .. }
                | Self::ApplyAdvanced { .. }
                | Self::ApplyCustomParams { .. }
                | Self::ApplyMacros { .. }
                | Self::RenameOnboardProfile { .. }
                | Self::StartCalibration { .. }
                | Self::RestartCalibration
                | Self::StopCalibration
                | Self::Telemetry { .. }
        ) || matches!(
            self,
            Self::ApplyProfileLayout { .. }
                | Self::Reset { .. }
                | Self::SetMusic { .. }
                | Self::PlayAnimation { .. }
                | Self::StopAnimation
                | Self::FlashFirmware { .. }
        )
    }
}

#[derive(Serialize)]
struct Backup {
    schema_version: u32,
    model: String,
    info: KeyboardInfo,
    base: KeyboardBase,
    profiles: Vec<RawKeyboardProfile>,
    macro_banks: Vec<(u8, MacroBank)>,
}

pub(crate) struct KeyboardSession {
    pub state: KeyboardState,
    driver: Option<Ace75>,
    root: PathBuf,
    read_only: bool,
    debug_restore: Option<(u8, bool)>,
    music: Option<KeyboardMusic>,
    animation: Option<AnimationPlayer>,
    lighting_restore: Option<(u8, Lighting)>,
}

impl KeyboardSession {
    pub fn new(read_only: bool) -> Self {
        Self {
            state: KeyboardState::default(),
            driver: None,
            root: data_dir().join("ace75"),
            read_only,
            debug_restore: None,
            music: None,
            animation: None,
            lighting_restore: None,
        }
    }

    pub fn discover(&mut self, devices: &[DeviceInfo]) -> Result<bool> {
        self.state.bootloader = devices
            .iter()
            .find(|device| device.kind == DeviceKind::KeyboardBootloader)
            .cloned();
        let selected = devices
            .iter()
            .find(|device| device.kind == DeviceKind::Keyboard);
        let changed = selected.map(|device| &device.path)
            != self.state.device.as_ref().map(|device| &device.path)
            || selected.is_some() && self.driver.is_none();
        if changed {
            if let Err(error) = self.stop_activity() {
                self.state.cleanup_warning = Some(format!(
                    "ACE75: восстановление при отключении не подтверждено: {error:#}"
                ));
            }
            self.debug_restore = None;
            self.lighting_restore = None;
            self.state.calibration_active = false;
            self.state.monitor_active = false;
            self.state.travel = None;
            self.state.calibration_keys.clear();
            self.publish_telemetry();
            self.driver = None;
            self.state.device = None;
            self.state.snapshot = None;
            self.state.error = None;
            self.state.revision = self.state.revision.wrapping_add(1);
            if let Some(device) = selected {
                self.state.device = Some(device.clone());
                match HidTransport::open(device) {
                    Ok(transport) => self.driver = Some(Ace75::new(Box::new(transport))),
                    Err(error) => self.state.error = Some(format!("{error:#}")),
                }
            }
        }
        Ok(changed)
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.state.presets = self.list_presets()?;
        self.state.macros = self.list_macros()?;
        if let Some(driver) = &mut self.driver {
            match driver.read_snapshot() {
                Ok(mut snapshot) => {
                    snapshot.profile.functions = self.persistent_functions(
                        snapshot.base.active_profile,
                        snapshot.profile.functions,
                    )?;
                    self.state.error = self
                        .load_macro_metadata(&mut snapshot.macros)
                        .err()
                        .map(|error| format!("{error:#}"));
                    self.state.snapshot = Some(snapshot);
                    self.state.revision = self.state.revision.wrapping_add(1);
                }
                Err(error) => {
                    self.state.error = Some(format!("{error:#}"));
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn poll(&mut self) -> Result<()> {
        self.tick()?;
        let Some(driver) = &mut self.driver else {
            return Ok(());
        };
        let base = KeyboardBase::decode(&driver.read_base()?)?;
        let Some(previous) = &self.state.snapshot else {
            return self.refresh();
        };
        if previous.base != base {
            self.stop_activity()?;
            self.refresh()?;
        } else {
            let functions = ace75::FunctionConfig::decode(
                &self.driver()?.read_function_config(base.active_profile)?,
            )?;
            let functions = self.persistent_functions(base.active_profile, functions)?;
            if let Some(snapshot) = &mut self.state.snapshot {
                if snapshot.profile.functions != functions {
                    snapshot.profile.functions = functions;
                    self.state.revision = self.state.revision.wrapping_add(1);
                }
            }
        }
        Ok(())
    }

    pub fn needs_fast_tick(&self) -> bool {
        self.state.monitor_active
            || self.state.calibration_active
            || self.music.is_some()
            || self.animation.is_some()
    }

    pub fn tick(&mut self) -> Result<()> {
        if self.driver.is_none() {
            return Ok(());
        }
        let frame = if let Some(music) = &mut self.music {
            if let Some(error) = music.take_error() {
                anyhow::bail!("ACE75: {error}");
            }
            music
                .next_frame()
                .map(|frame| (frame, music.scopes().to_vec()))
        } else if let Some(animation) = &mut self.animation {
            let scopes = animation.scopes().to_vec();
            animation
                .next_frame(Instant::now())
                .map(|frame| (frame.colors.clone(), scopes))
        } else {
            None
        };
        if let Some((colors, scopes)) = frame {
            for scope in scopes {
                self.driver()?.stream_colors(0, scope, &colors)?;
            }
        }
        for _ in 0..128 {
            let Some(notification) = self.driver()?.read_notification(0)? else {
                break;
            };
            match notification {
                KeyboardNotification::Calibration {
                    key,
                    travel,
                    calibration_state,
                } => {
                    self.state.travel = Some(KeyboardTravel {
                        key,
                        travel_raw: travel,
                    });
                    self.state.calibration_state = Some(calibration_state);
                    if let Some(index) =
                        calibration_assignments().get(&(key.kind, key.code1, key.code2))
                    {
                        self.state
                            .calibration_keys
                            .insert(*index, calibration_state);
                    }
                }
                KeyboardNotification::ActiveProfile(profile)
                    if self
                        .state
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.base.active_profile != profile) =>
                {
                    self.stop_activity()?;
                    self.refresh()?;
                    break;
                }
                KeyboardNotification::Reset => {
                    self.stop_activity()?;
                    self.refresh()?;
                    break;
                }
                _ => {}
            }
        }
        self.publish_telemetry();
        Ok(())
    }

    fn publish_telemetry(&self) {
        if let Ok(mut value) = self.state.telemetry.lock() {
            value.travel = self.state.travel.clone();
            value.calibration_state = self.state.calibration_state;
            value.calibration_keys = self.state.calibration_keys.clone();
        }
    }

    fn persistent_functions(
        &self,
        profile: u8,
        functions: ace75::FunctionConfig,
    ) -> Result<ace75::FunctionConfig> {
        if let Some((debug_profile, original_debug)) = self.debug_restore {
            if profile == debug_profile {
                let mut performance = functions.performance.clone();
                performance.debug_mode = original_debug;
                return ace75::FunctionConfig::decode(
                    &functions.with_performance(&performance, false)?,
                );
            }
        }
        Ok(functions)
    }

    pub fn stop_activity(&mut self) -> Result<()> {
        let lighting = self.stop_host_lighting();
        let debug = self.stop_debug();
        match (lighting, debug) {
            (Ok(()), result) | (result, Ok(())) => result,
            (Err(error), Err(cleanup)) => Err(with_cleanup(error, Err(cleanup))),
        }
    }

    fn stop_debug(&mut self) -> Result<()> {
        let calibration = if self.state.calibration_active {
            self.driver()
                .and_then(|driver| driver.end_calibration())
                .map(|_| self.state.calibration_active = false)
        } else {
            Ok(())
        };
        let debug = if let Some((profile, enabled)) = self.debug_restore {
            self.driver()
                .and_then(|driver| driver.set_debug_mode(profile, enabled))
                .map(|_| self.debug_restore = None)
        } else {
            Ok(())
        };
        self.state.monitor_active = false;
        self.state.travel = None;
        self.state.calibration_state = None;
        self.publish_telemetry();
        match (calibration, debug) {
            (Ok(()), result) | (result, Ok(())) => result,
            (Err(error), Err(cleanup)) => Err(with_cleanup(error, Err(cleanup))),
        }
    }

    fn stop_host_lighting(&mut self) -> Result<()> {
        self.music = None;
        self.animation = None;
        self.state.music_active = false;
        self.state.animation_active = false;
        if let Some((profile, lighting)) = self.lighting_restore.clone() {
            self.driver()?.apply_lighting(profile, &lighting)?;
            self.lighting_restore = None;
        }
        Ok(())
    }

    fn driver(&mut self) -> Result<&mut Ace75> {
        self.driver.as_mut().context("ACE75 не подключена")
    }

    fn target(&mut self, path: &str, profile: u8) -> Result<KeyboardBase> {
        ensure!(
            self.state
                .device
                .as_ref()
                .is_some_and(|device| device.path == path),
            "ACE75: выбранное устройство изменилось, перечитайте настройки"
        );
        let base = KeyboardBase::decode(&self.driver()?.read_base()?)?;
        ensure!(
            profile < base.max_profiles,
            "ACE75: неверный аппаратный профиль"
        );
        ensure!(
            base.active_profile == profile,
            "ACE75: активный профиль изменился, перечитайте настройки"
        );
        Ok(base)
    }

    fn backup(&mut self, operation: &str) -> Result<PathBuf> {
        let path = self.root.join("backups").join(format!(
            "{operation}-{}-{}.json",
            stamp()?,
            std::process::id()
        ));
        let root = self.root.clone();
        let backup = read_keyboard_backup(self.driver()?, &root)?;
        save_new_json(&path, &backup)?;
        Ok(path)
    }

    fn current_preset(&self) -> Result<KeyboardPreset> {
        if let Some(value) = &self.state.preview {
            return Ok(value.clone());
        }
        let profile = self
            .state
            .snapshot
            .as_ref()
            .context("ACE75: нет прочитанных настроек")?
            .profile
            .clone();
        let macros = self
            .state
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.macros.clone());
        Ok(KeyboardPreset {
            schema_version: 1,
            model: ace75::MODEL.into(),
            profile,
            macros,
            hub_document: None,
            hub_scope: None,
        })
    }

    #[cfg(test)]
    pub fn execute(&mut self, command: KeyboardCommand) -> Result<()> {
        self.execute_with_progress(command, |_, _| {})
    }

    pub fn execute_with_progress(
        &mut self,
        command: KeyboardCommand,
        progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        ensure!(
            !self.read_only || !command.writes_hardware(),
            "Включён режим только чтения"
        );
        let writes_hardware = command.writes_hardware();
        let preview_changed = matches!(
            &command,
            KeyboardCommand::PreviewPreset(_)
                | KeyboardCommand::ClosePreview
                | KeyboardCommand::SaveEditedPreset { .. }
        );
        self.state.message.clear();
        let result = self.execute_inner(command, progress);
        if let Err(error) = &result {
            if writes_hardware {
                let _ = self.refresh();
            }
            self.state.error = Some(format!("{error:#}"));
        } else {
            self.state.error = None;
            if preview_changed {
                self.state.revision = self.state.revision.wrapping_add(1);
            }
        }
        result
    }

    fn execute_inner(
        &mut self,
        command: KeyboardCommand,
        progress: impl FnMut(usize, usize),
    ) -> Result<()> {
        match command {
            KeyboardCommand::Refresh => self.refresh()?,
            KeyboardCommand::Backup => {
                let path = self.backup("manual")?;
                self.state.message =
                    format!("ACE75: резервная копия сохранена в {}", path.display());
            }
            KeyboardCommand::ApplyProfileLayout {
                device_path,
                profile,
                profile_count,
                profile_order,
            } => {
                self.target(&device_path, profile)?;
                self.stop_activity()?;
                let backup = self.backup("profile-order")?;
                self.driver()?
                    .apply_profile_selection(profile, profile_count, &profile_order)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message =
                    "ACE75: порядок аппаратных профилей применён и проверен".into();
            }
            KeyboardCommand::Reset {
                device_path,
                profile,
            } => {
                let active = self
                    .state
                    .snapshot
                    .as_ref()
                    .context("ACE75: нет прочитанных настроек")?
                    .base
                    .active_profile;
                self.target(&device_path, profile.unwrap_or(active))?;
                self.stop_activity()?;
                let backup = self.backup("reset")?;
                self.driver()?.reset(profile).with_context(|| {
                    format!("ACE75: резервная копия сохранена в {}", backup.display())
                })?;
                self.refresh()?;
                self.state.message = "ACE75: сброс выполнен, настройки перечитаны".into();
            }
            KeyboardCommand::SetMusic {
                device_path,
                profile,
                enabled,
                scopes,
            } => {
                self.target(&device_path, profile)?;
                self.stop_activity()?;
                if enabled {
                    let music = KeyboardMusic::start(scopes)?;
                    self.backup("music")?;
                    let lighting = self.driver()?.read_profile(profile)?.functions.lighting;
                    self.state.message =
                        format!("ACE75: музыка включена, источник: {}", music.sink_name());
                    self.lighting_restore = Some((profile, lighting));
                    self.music = Some(music);
                    self.state.music_active = true;
                } else {
                    self.state.message = "ACE75: музыка выключена, подсветка восстановлена".into();
                }
            }
            KeyboardCommand::LoadAnimation(path) => {
                self.state.animation = Some(host_lighting::load_animation(&path)?);
            }
            KeyboardCommand::SaveAnimation { path, value } => {
                host_lighting::save_animation(&path, &value)?;
                self.state.message = format!("ACE75: анимация сохранена: {}", path.display());
            }
            KeyboardCommand::PlayAnimation {
                device_path,
                profile,
                value,
            } => {
                self.target(&device_path, profile)?;
                let player = AnimationPlayer::new(value.clone())?;
                self.stop_activity()?;
                self.backup("animation")?;
                let lighting = self.driver()?.read_profile(profile)?.functions.lighting;
                self.lighting_restore = Some((profile, lighting));
                self.animation = Some(player);
                self.state.animation = Some(value);
                self.state.animation_active = true;
                self.state.message = "ACE75: анимация запущена".into();
            }
            KeyboardCommand::StopAnimation => {
                self.stop_host_lighting()?;
                self.state.message = "ACE75: анимация остановлена, подсветка восстановлена".into();
            }
            KeyboardCommand::CheckFirmware => {
                self.state.firmware = Some(firmware_catalog::fetch_keyboard_firmware()?);
                self.state.message = "ACE75: каталог прошивок обновлён".into();
            }
            KeyboardCommand::DownloadFirmware(directory) => {
                let info = firmware_catalog::fetch_keyboard_firmware()?;
                let downloaded = firmware_catalog::download_keyboard_firmware(&info, &directory)?;
                self.state.firmware = Some(info);
                self.state.message = format!(
                    "ACE75: прошивка скачана: {}",
                    directory.join(downloaded.filename).display()
                );
            }
            KeyboardCommand::FlashFirmware { path } => {
                let prepared = PreparedKeyboardFirmware::verify_file(&path)?;
                self.stop_activity()?;
                if self.driver.is_some() {
                    self.backup("firmware")?;
                }
                let device = self
                    .state
                    .device
                    .as_ref()
                    .or(self.state.bootloader.as_ref())
                    .context("ACE75 не подключена")?
                    .clone();
                self.driver = None;
                let returned = firmware_service::flash_keyboard(&device, &prepared, progress)?;
                self.driver = Some(Ace75::new(Box::new(HidTransport::open(&returned)?)));
                self.state.device = Some(returned);
                self.state.bootloader = None;
                self.refresh()?;
                self.state.message = "ACE75: прошивка записана, версия перечитана".into();
            }
            KeyboardCommand::SwitchProfile {
                device_path,
                profile,
            } => {
                let active = self
                    .state
                    .snapshot
                    .as_ref()
                    .context("ACE75: нет прочитанных настроек")?
                    .base
                    .active_profile;
                let base = self.target(&device_path, active)?;
                self.stop_activity()?;
                ensure!(
                    base.profile_order[..base.profile_count as usize].contains(&profile),
                    "ACE75: профиль не включён в аппаратный список"
                );
                let backup = self.backup("switch-profile")?;
                self.driver()?
                    .apply_profile_selection(profile, base.profile_count, &base.profile_order)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = format!("ACE75: выбран профиль {}", profile + 1);
            }
            KeyboardCommand::ApplyLighting {
                device_path,
                profile,
                value,
            } => {
                self.target(&device_path, profile)?;
                value.validate()?;
                self.stop_host_lighting()?;
                let backup = self.backup("lighting")?;
                self.driver()?
                    .apply_lighting(profile, &value)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: подсветка применена и проверена".into();
            }
            KeyboardCommand::ApplyPerformance {
                device_path,
                profile,
                value,
            } => {
                self.target(&device_path, profile)?;
                value.validate()?;
                self.stop_debug()?;
                let backup = self.backup("performance")?;
                self.driver()?
                    .apply_performance(profile, &value)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: параметры применены и проверены".into();
            }
            KeyboardCommand::ApplyReportRate {
                device_path,
                profile,
                report_rate,
            } => {
                self.target(&device_path, profile)?;
                ensure!(
                    (1..=4).contains(&report_rate),
                    "ACE75: неверная частота опроса"
                );
                let scan_rate = self
                    .driver()?
                    .read_profile(profile)?
                    .functions
                    .performance
                    .scan_rate;
                let backup = self.backup("report-rate")?;
                self.driver()?
                    .apply_report_rates(profile, report_rate, scan_rate)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: частота опроса применена и проверена".into();
            }
            KeyboardCommand::ApplyKeys {
                device_path,
                profile,
                layer,
                values,
            } => {
                self.target(&device_path, profile)?;
                ensure!(
                    layer < 4 && values.len() == 128,
                    "ACE75: неверный слой клавиш"
                );
                let backup = self.backup("keys")?;
                self.driver()?
                    .apply_keys(profile, layer, &values)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: назначения клавиш применены и проверены".into();
            }
            KeyboardCommand::ApplyGlobalKeys {
                device_path,
                profile,
                indices,
            } => {
                self.target(&device_path, profile)?;
                ensure!(
                    !indices.is_empty()
                        && indices.iter().all(|(index, _)| matches!(index, 93 | 94)),
                    "ACE75: неверный глобальный контрол"
                );
                let backup = self.backup("global-keys")?;
                let mut current = self.driver()?.read_profile(profile)?;
                let result = (|| -> Result<()> {
                    for layer in 0..4 {
                        for (index, value) in &indices {
                            current.layers[layer][*index as usize] = *value;
                        }
                        self.driver()?
                            .apply_keys(profile, layer as u8, &current.layers[layer])?;
                    }
                    Ok(())
                })();
                result.with_context(|| {
                    format!("ACE75: резервная копия сохранена в {}", backup.display())
                })?;
                self.refresh()?;
                self.state.message = "ACE75: глобальные назначения применены ко всем слоям".into();
            }
            KeyboardCommand::ApplyColors {
                device_path,
                profile,
                colors,
            } => {
                self.target(&device_path, profile)?;
                ensure!(colors.len() == 128, "ACE75: нужны цвета всех 128 позиций");
                let backup = self.backup("colors")?;
                self.driver()?
                    .apply_colors(profile, 0, &colors)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: цвета применены и проверены".into();
            }
            KeyboardCommand::ApplyAdvanced {
                device_path,
                profile,
                advanced,
                layers,
                triggers,
                custom_params,
            } => {
                self.target(&device_path, profile)?;
                ensure!(
                    layers.len() == 4
                        && layers.iter().all(|layer| layer.len() == 128)
                        && triggers.len() == 128,
                    "ACE75: неверная структура профиля"
                );
                advanced.validate_references(&layers)?;
                advanced.encode()?;
                custom_params.encode()?;
                for trigger in &triggers {
                    trigger.validate()?;
                }
                let current = self.driver()?.read_profile(profile)?;
                let firmware = self.driver()?.read_identity()?.firmware_version;
                validate_trigger_changes(
                    &current.triggers,
                    &triggers,
                    current.functions.performance.tachyon_mode,
                    &firmware,
                )?;
                let backup = self.backup("advanced")?;
                let result = (|| -> Result<()> {
                    self.driver()?.apply_advanced(profile, &advanced)?;
                    for (layer, keys) in layers.iter().enumerate() {
                        self.driver()?.apply_keys(profile, layer as u8, keys)?;
                    }
                    self.driver()?.apply_triggers(profile, &triggers)?;
                    self.driver()?
                        .apply_custom_params(profile, &custom_params)?;
                    Ok(())
                })();
                result.with_context(|| {
                    format!("ACE75: резервная копия сохранена в {}", backup.display())
                })?;
                self.refresh()?;
                self.state.message = "ACE75: продвинутые действия применены и проверены".into();
            }
            KeyboardCommand::ApplyCustomParams {
                device_path,
                profile,
                value,
            } => {
                self.target(&device_path, profile)?;
                value.encode()?;
                let backup = self.backup("custom-params")?;
                self.driver()?
                    .apply_custom_params(profile, &value)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: дополнительные параметры применены и проверены".into();
            }
            KeyboardCommand::ApplyMacros {
                device_path,
                profile,
                bank,
            } => {
                let base = self.target(&device_path, profile)?;
                validate_macro_bank(&bank)?;
                let layout = MacroLayout::for_base(base.supports_multiple, base.supports_order);
                ensure!(
                    bank.raw.len() == layout.size,
                    "ACE75: банк макросов не соответствует текущей раскладке памяти"
                );
                let profiles: Vec<u8> = if layout.shared {
                    (0..base.max_profiles).collect()
                } else {
                    vec![profile]
                };
                let mut plans = Vec::new();
                for index in profiles {
                    let before = self.driver()?.read_profile(index)?;
                    let mut factory = Vec::new();
                    for layer in 0..4 {
                        factory.push(KeyAssignment::decode_all(
                            &self
                                .driver()?
                                .read_key_matrix(index * 4 + layer, 128, true)?,
                        )?);
                    }
                    let after =
                        hub_profile::plan_macro_reference_updates(&before, &factory, &bank)?;
                    if after.advanced != before.advanced {
                        after.advanced.encode()?;
                    }
                    plans.push((before, after));
                }
                let backup = self.backup("macros")?;
                let result = (|| -> Result<MacroBank> {
                    let actual = self.driver()?.apply_macro_bank(profile, &bank)?;
                    for (before, after) in plans {
                        if before.advanced != after.advanced {
                            self.driver()?
                                .apply_advanced(after.index, &after.advanced)?;
                        }
                        for layer in 0..4 {
                            if before.layers[layer] != after.layers[layer] {
                                self.driver()?.apply_keys(
                                    after.index,
                                    layer as u8,
                                    &after.layers[layer],
                                )?;
                            }
                        }
                    }
                    Ok(actual)
                })();
                let actual = result.with_context(|| {
                    format!("ACE75: резервная копия сохранена в {}", backup.display())
                })?;
                self.store_macro_metadata(&actual)?;
                self.refresh()?;
                self.state.message = "ACE75: макросы применены и проверены".into();
            }
            KeyboardCommand::RenameOnboardProfile {
                device_path,
                profile,
                name,
            } => {
                let active = self
                    .state
                    .snapshot
                    .as_ref()
                    .context("ACE75: нет прочитанных настроек")?
                    .base
                    .active_profile;
                let base = self.target(&device_path, active)?;
                ensure!(
                    profile < base.max_profiles
                        && name.chars().count() <= 64
                        && !name.trim().is_empty(),
                    "ACE75: недопустимое имя аппаратного профиля"
                );
                let mut names = self.driver()?.read_profile_names()?;
                names
                    .names
                    .resize(base.max_profiles as usize, String::new());
                names.names[profile as usize] = name.trim().into();
                names.encode()?;
                let backup = self.backup("profile-name")?;
                self.driver()?
                    .apply_profile_names(&names)
                    .with_context(|| {
                        format!("ACE75: резервная копия сохранена в {}", backup.display())
                    })?;
                self.refresh()?;
                self.state.message = "ACE75: аппаратный профиль переименован".into();
            }
            KeyboardCommand::StartCalibration {
                device_path,
                profile,
            } => {
                self.target(&device_path, profile)?;
                self.stop_activity()?;
                self.backup("calibration")?;
                self.enable_debug(profile)?;
                self.state.calibration_active = true;
                if let Err(error) = self.driver()?.start_calibration() {
                    let cleanup = self.stop_activity();
                    return Err(with_cleanup(error, cleanup));
                }
                self.state.monitor_active = true;
                self.state.calibration_keys.clear();
                self.publish_telemetry();
                self.state.message = "ACE75: калибровка запущена".into();
            }
            KeyboardCommand::RestartCalibration => {
                ensure!(
                    self.state.calibration_active,
                    "ACE75: калибровка не запущена"
                );
                self.driver()?.restart_calibration()?;
                self.state.calibration_keys.clear();
                self.publish_telemetry();
            }
            KeyboardCommand::StopCalibration => {
                self.stop_debug()?;
                self.refresh()?;
                self.state.message = "ACE75: мониторинг и калибровка остановлены".into();
            }
            KeyboardCommand::Telemetry {
                device_path,
                profile,
                enabled,
            } => {
                self.target(&device_path, profile)?;
                self.stop_activity()?;
                if enabled {
                    self.backup("telemetry")?;
                    self.enable_debug(profile)?;
                    self.state.monitor_active = true;
                }
                self.state.message = if enabled {
                    "ACE75: мониторинг хода клавиш включён"
                } else {
                    "ACE75: мониторинг хода клавиш выключен"
                }
                .into();
            }
            KeyboardCommand::SaveLocalMacro { name, mut value } => {
                value.name = name.trim().into();
                validate_macro(&value)?;
                save_new_json(&self.macro_path(&name)?, &value)?;
                self.state.local_macro = Some(value);
                self.state.macro_revision = self.state.macro_revision.wrapping_add(1);
            }
            KeyboardCommand::UpdateLocalMacro { path, value } => {
                self.existing_macro(&path)?;
                validate_macro(&value)?;
                let previous = fs::read(&path)?;
                save_new_bytes(
                    &self
                        .root
                        .join("backups")
                        .join(format!("edit-macro-{}.json", stamp()?)),
                    &previous,
                )?;
                replace_bytes(&path, &previous, &serde_json::to_vec_pretty(&value)?)?;
                self.state.local_macro = Some(value);
                self.state.macro_revision = self.state.macro_revision.wrapping_add(1);
            }
            KeyboardCommand::OpenLocalMacro(path) => {
                self.existing_macro(&path)?;
                self.state.local_macro = Some(read_macro(&path)?);
                self.state.macro_revision = self.state.macro_revision.wrapping_add(1);
            }
            KeyboardCommand::ImportLocalMacro(path) => {
                let mut value = read_macro(&path)?;
                let name = if value.name.trim().is_empty() {
                    path.file_stem()
                        .context("ACE75: у файла нет имени")?
                        .to_string_lossy()
                        .into_owned()
                } else {
                    value.name.clone()
                };
                value.name = name.clone();
                save_new_json(&self.macro_path(&name)?, &value)?;
                self.state.local_macro = Some(value);
                self.state.macro_revision = self.state.macro_revision.wrapping_add(1);
            }
            KeyboardCommand::RenameLocalMacro { path, name } => {
                self.existing_macro(&path)?;
                let mut value = read_macro(&path)?;
                value.name = name.trim().into();
                let target = self.macro_path(&name)?;
                ensure!(target != path, "ACE75: имя не изменилось");
                let previous = fs::read(&path)?;
                save_new_bytes(
                    &self
                        .root
                        .join("backups")
                        .join(format!("rename-macro-{}.json", stamp()?)),
                    &previous,
                )?;
                save_new_json(&target, &value)?;
                ensure!(
                    fs::read(&path)? == previous,
                    "ACE75: файл изменился во время переименования"
                );
                fs::remove_file(path)?;
            }
            KeyboardCommand::DeleteLocalMacro(path) => {
                self.existing_macro(&path)?;
                let previous = fs::read(&path)?;
                save_new_bytes(
                    &self
                        .root
                        .join("backups")
                        .join(format!("deleted-macro-{}.json", stamp()?)),
                    &previous,
                )?;
                ensure!(
                    fs::read(&path)? == previous,
                    "ACE75: файл изменился во время удаления"
                );
                fs::remove_file(path)?;
            }
            KeyboardCommand::ExportLocalMacro { path, value } => {
                validate_macro(&value)?;
                save_new_json(&path, &value)?;
            }
            KeyboardCommand::ApplyTriggers {
                device_path,
                profile,
                values,
                tachyon_mode,
            } => {
                self.target(&device_path, profile)?;
                ensure!(
                    values.len() == 128,
                    "ACE75: неверное число магнитных настроек"
                );
                for value in &values {
                    value.validate()?;
                }
                let current = self.driver()?.read_profile(profile)?;
                let firmware = self.driver()?.read_identity()?.firmware_version;
                validate_trigger_changes(&current.triggers, &values, tachyon_mode, &firmware)?;
                let backup = self.backup("triggers")?;
                let result = (|| -> Result<()> {
                    let mut performance =
                        self.driver()?.read_profile(profile)?.functions.performance;
                    if performance.tachyon_mode != tachyon_mode {
                        performance.tachyon_mode = tachyon_mode;
                        self.driver()?.apply_performance(profile, &performance)?;
                    }
                    self.driver()?.apply_triggers(profile, &values)?;
                    Ok(())
                })();
                result.with_context(|| {
                    format!("ACE75: резервная копия сохранена в {}", backup.display())
                })?;
                self.refresh()?;
                self.state.message = "ACE75: магнитные настройки применены и проверены".into();
            }
            KeyboardCommand::SavePreset(name) => {
                let value = self.current_preset()?;
                validate_preset(&value)?;
                save_new_json(&self.preset_path(&name)?, &value)?;
                self.state.message = format!("ACE75: профиль сохранён: {name}");
            }
            KeyboardCommand::ImportPreset(path) => {
                let value = self.read_import(&path)?;
                let name = path
                    .file_stem()
                    .context("ACE75: у файла нет имени")?
                    .to_string_lossy();
                save_new_json(&self.preset_path(&name)?, &value)?;
                self.state.message = format!("ACE75: профиль импортирован: {name}");
            }
            KeyboardCommand::PreviewPreset(path) => {
                self.existing_preset(&path)?;
                self.state.preview = Some(read_preset(&path)?);
            }
            KeyboardCommand::ClosePreview => self.state.preview = None,
            KeyboardCommand::SaveEditedPreset { path, mut value } => {
                self.existing_preset(&path)?;
                validate_preset(&value)?;
                let previous = fs::read(&path)?;
                let original: KeyboardPreset = serde_json::from_slice(&previous)?;
                if let Some(scope) = &mut value.hub_scope {
                    scope.keys |= value.profile.layers != original.profile.layers;
                    scope.triggers |= value.profile.triggers != original.profile.triggers;
                    scope.advanced |= value.profile.advanced != original.profile.advanced;
                    scope.performance |= value.profile.functions.performance
                        != original.profile.functions.performance;
                    scope.lighting |=
                        value.profile.functions.lighting != original.profile.functions.lighting;
                    scope.custom_params |=
                        value.profile.custom_params != original.profile.custom_params;
                    for (index, (before, after)) in original
                        .profile
                        .colors
                        .iter()
                        .zip(&value.profile.colors)
                        .enumerate()
                    {
                        if before != after && !scope.color_indices.contains(&index) {
                            scope.color_indices.push(index);
                        }
                    }
                }
                let backup = self
                    .root
                    .join("backups")
                    .join(format!("edit-preset-{}.json", stamp()?));
                save_new_bytes(&backup, &previous)?;
                replace_bytes(&path, &previous, &serde_json::to_vec_pretty(&value)?)?;
                self.state.preview = Some(value);
                self.state.message = "ACE75: локальные изменения сохранены".into();
            }
            KeyboardCommand::ApplyPreset {
                device_path,
                profile,
                path,
            } => {
                self.existing_preset(&path)?;
                let value = read_preset(&path)?;
                let base = self.target(&device_path, profile)?;
                self.stop_activity()?;
                let current = self.driver()?.read_profile(profile)?;
                let mut wanted = if let Some(scope) = &value.hub_scope {
                    scope.merge_into(&value.profile, &current)?
                } else {
                    value.profile.clone()
                };
                hub_profile::rebase_profile(&mut wanted, profile)?;
                let mut current_macros = self.driver()?.read_macro_bank(profile)?;
                self.load_macro_metadata(&mut current_macros)?;
                let mut reserved = BTreeSet::new();
                if MacroLayout::for_base(base.supports_multiple, base.supports_order).shared {
                    for index in (0..base.max_profiles).filter(|index| *index != profile) {
                        let mut references = current.clone();
                        references.layers.clear();
                        for layer in 0..4 {
                            references.layers.push(KeyAssignment::decode_all(
                                &self
                                    .driver()?
                                    .read_key_matrix(index * 4 + layer, 128, false)?,
                            )?);
                        }
                        references.advanced = self.driver()?.read_advanced(index)?;
                        reserved.extend(hub_profile::macro_slots(&references));
                    }
                }
                let merged_macros = hub_profile::merge_scoped_profile_macros_reserved(
                    &mut wanted,
                    value.macros.as_ref(),
                    &current_macros,
                    value.hub_scope.as_ref(),
                    &reserved,
                )?;
                validate_macro_bank(&merged_macros)?;
                wanted.advanced.validate_references(&wanted.layers)?;
                let firmware = self.driver()?.read_identity()?.firmware_version;
                validate_trigger_changes(
                    &current.triggers,
                    &wanted.triggers,
                    wanted.functions.performance.tachyon_mode,
                    &firmware,
                )?;
                if wanted.functions.performance.report_rate
                    != current.functions.performance.report_rate
                {
                    ensure!(
                        (1..=4).contains(&wanted.functions.performance.report_rate),
                        "ACE75: неверная частота опроса"
                    );
                }
                let backup = self.backup("apply-preset")?;
                let result = (|| -> Result<()> {
                    let actual_macros = if merged_macros.encode()? != current_macros.raw {
                        self.driver()?.apply_macro_bank(profile, &merged_macros)?
                    } else {
                        merged_macros
                    };
                    self.store_macro_metadata(&actual_macros)?;
                    if wanted.advanced != current.advanced {
                        self.driver()?.apply_advanced(profile, &wanted.advanced)?;
                    }
                    for (layer, keys) in wanted.layers.iter().enumerate() {
                        if *keys != current.layers[layer] {
                            self.driver()?.apply_keys(profile, layer as u8, keys)?;
                        }
                    }
                    if wanted.triggers != current.triggers {
                        self.driver()?.apply_triggers(profile, &wanted.triggers)?;
                    }
                    if wanted.colors != current.colors {
                        self.driver()?.apply_colors(profile, 0, &wanted.colors)?;
                    }
                    if wanted.custom_params != current.custom_params {
                        self.driver()?
                            .apply_custom_params(profile, &wanted.custom_params)?;
                    }
                    if wanted.functions.performance.report_rate
                        != current.functions.performance.report_rate
                        || wanted.functions.performance.scan_rate
                            != current.functions.performance.scan_rate
                    {
                        self.driver()?.apply_report_rates(
                            profile,
                            wanted.functions.performance.report_rate,
                            wanted.functions.performance.scan_rate,
                        )?;
                    }
                    if wanted.functions.performance != current.functions.performance {
                        self.driver()?
                            .apply_performance(profile, &wanted.functions.performance)?;
                    }
                    if wanted.functions.lighting != current.functions.lighting {
                        self.driver()?
                            .apply_lighting(profile, &wanted.functions.lighting)?;
                    }
                    Ok(())
                })();
                result.with_context(|| {
                    format!("ACE75: резервная копия сохранена в {}", backup.display())
                })?;
                self.refresh()?;
                self.state.message = "ACE75: профиль применён и проверен".into();
            }
            KeyboardCommand::RenamePreset { path, name } => {
                self.existing_preset(&path)?;
                let value = fs::read(&path)?;
                let target = self.preset_path(&name)?;
                ensure!(target != path, "ACE75: имя не изменилось");
                save_new_bytes(&target, &value)?;
                let backup = self
                    .root
                    .join("backups")
                    .join(format!("rename-preset-{}.json", stamp()?));
                save_new_bytes(&backup, &value)?;
                ensure!(
                    fs::read(&path)? == value,
                    "ACE75: файл изменился во время переименования"
                );
                fs::remove_file(path)?;
            }
            KeyboardCommand::DeletePreset(path) => {
                self.existing_preset(&path)?;
                let previous = fs::read(&path)?;
                let backup = self
                    .root
                    .join("backups")
                    .join(format!("deleted-preset-{}.json", stamp()?));
                save_new_bytes(&backup, &previous)?;
                ensure!(
                    fs::read(&path)? == previous,
                    "ACE75: файл изменился во время удаления"
                );
                fs::remove_file(path)?;
            }
            KeyboardCommand::ExportProfile(path) => {
                let value = self.current_preset()?;
                validate_preset(&value)?;
                save_new_json(&path, &value)?;
                self.state.message = format!("ACE75: профиль экспортирован: {}", path.display());
            }
            KeyboardCommand::ExportHubProfile(path) => {
                let value = self.current_preset()?;
                validate_preset(&value)?;
                let document = hub_profile::export_hub_profile(
                    &value.profile,
                    value.macros.as_ref(),
                    value.hub_document.as_ref(),
                )?;
                save_new_bytes(&path, &document.to_json()?)?;
                self.state.message =
                    format!("ACE75: профиль Hub экспортирован: {}", path.display());
            }
        }
        self.state.presets = self.list_presets()?;
        self.state.macros = self.list_macros()?;
        Ok(())
    }

    fn enable_debug(&mut self, profile: u8) -> Result<()> {
        let original = self
            .driver()?
            .read_profile(profile)?
            .functions
            .performance
            .debug_mode;
        self.debug_restore = Some((profile, original));
        if let Err(error) = self.driver()?.set_debug_mode(profile, true) {
            let cleanup = self.stop_activity();
            return Err(with_cleanup(error, cleanup));
        }
        Ok(())
    }

    fn read_import(&self, path: &Path) -> Result<KeyboardPreset> {
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= 4_000_000,
            "ACE75: нужен обычный JSON-файл до 4 МБ"
        );
        let text = fs::read_to_string(path)?;
        let document: serde_json::Value = serde_json::from_str(&text)?;
        if document.get("schema_version").is_some() {
            let value: KeyboardPreset = serde_json::from_value(document)?;
            validate_preset(&value)?;
            return Ok(value);
        }
        let baseline = match &self.state.snapshot {
            Some(snapshot) => snapshot.profile.clone(),
            None => empty_import_baseline()?,
        };
        let imported = hub_profile::import_hub_profile(
            &HubProfileDocument::from_json(text.as_bytes())?,
            &baseline,
        )?;
        let value = KeyboardPreset {
            schema_version: 1,
            model: ace75::MODEL.into(),
            profile: imported.profile,
            macros: imported.macros,
            hub_document: Some(imported.document),
            hub_scope: Some(imported.scope),
        };
        validate_preset(&value)?;
        Ok(value)
    }

    fn preset_path(&self, name: &str) -> Result<PathBuf> {
        let name = name.trim();
        ensure!(
            !name.is_empty()
                && name.chars().count() <= 64
                && !name.contains(['/', '\\', '\0'])
                && name != "."
                && name != "..",
            "ACE75: недопустимое имя локального профиля"
        );
        Ok(self.root.join("presets").join(format!("{name}.json")))
    }

    fn macro_path(&self, name: &str) -> Result<PathBuf> {
        let name = name.trim();
        ensure!(
            !name.is_empty()
                && name.chars().count() <= 64
                && !name.contains(['/', '\\', '\0'])
                && name != "."
                && name != "..",
            "ACE75: недопустимое имя локального макроса"
        );
        Ok(self.root.join("macros").join(format!("{name}.json")))
    }

    fn existing_macro(&self, path: &Path) -> Result<()> {
        ensure!(
            path.parent() == Some(self.root.join("macros").as_path())
                && fs::symlink_metadata(path)?.is_file(),
            "ACE75: файл находится вне библиотеки макросов"
        );
        Ok(())
    }

    fn list_macros(&self) -> Result<Vec<PresetInfo>> {
        let root = self.root.join("macros");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
            {
                entries.push(PresetInfo {
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
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    fn load_macro_metadata(&self, bank: &mut MacroBank) -> Result<()> {
        load_macro_metadata(&self.root, bank)
    }

    fn store_macro_metadata(&self, bank: &MacroBank) -> Result<()> {
        let value = serde_json::json!({ "sha256": format!("{:x}", Sha256::digest(&bank.raw)), "metadata": bank.local_metadata() });
        let path = self.root.join("macro-metadata.json");
        let bytes = serde_json::to_vec_pretty(&value)?;
        if path.exists() {
            replace_bytes(&path, &fs::read(&path)?, &bytes)
        } else {
            save_new_bytes(&path, &bytes)
        }
    }

    fn existing_preset(&self, path: &Path) -> Result<()> {
        ensure!(
            path.parent() == Some(self.root.join("presets").as_path())
                && fs::symlink_metadata(path)?.is_file(),
            "ACE75: файл находится вне библиотеки профилей"
        );
        Ok(())
    }

    fn list_presets(&self) -> Result<Vec<PresetInfo>> {
        let root = self.root.join("presets");
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut entries = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|value| value == "json")
            {
                entries.push(PresetInfo {
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
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }
}

fn stamp() -> Result<u128> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
}

fn empty_import_baseline() -> Result<KeyboardProfile> {
    let mut functions = vec![0; 64];
    functions[4] = 1;
    RawKeyboardProfile {
        index: 0,
        functions,
        layers: vec![vec![0; 384]; 4],
        triggers: vec![0; 1024],
        colors: vec![0; 384],
        dks: vec![0; 768],
        mt: vec![0; 256],
        tgl: vec![0; 128],
        custom: vec![255; 1024],
    }
    .decode()
}

pub fn write_keyboard_backup(driver: &mut Ace75, path: &Path) -> Result<()> {
    save_new_json(
        path,
        &read_keyboard_backup(driver, &data_dir().join("ace75"))?,
    )?;
    Ok(())
}

fn read_keyboard_backup(driver: &mut Ace75, root: &Path) -> Result<Backup> {
    let info = driver.read_identity()?;
    let base = KeyboardBase::decode(&driver.read_base()?)?;
    let profiles = driver.read_all_profiles_raw()?;
    let shared = MacroLayout::for_base(base.supports_multiple, base.supports_order).shared;
    let mut macro_banks = Vec::new();
    for profile in 0..if shared { 1 } else { base.max_profiles } {
        let mut bank = driver.read_macro_bank(profile)?;
        load_macro_metadata(root, &mut bank)?;
        macro_banks.push((profile, bank));
    }
    Ok(Backup {
        schema_version: 1,
        model: ace75::MODEL.into(),
        info,
        base,
        profiles,
        macro_banks,
    })
}

fn load_macro_metadata(root: &Path, bank: &mut MacroBank) -> Result<()> {
    let path = root.join("macro-metadata.json");
    if !path.exists() {
        return Ok(());
    }
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
    if value["sha256"].as_str() == Some(format!("{:x}", Sha256::digest(&bank.raw)).as_str()) {
        bank.apply_local_metadata(&value["metadata"])?;
    }
    Ok(())
}

fn calibration_indices() -> &'static BTreeMap<u16, u16> {
    static INDICES: OnceLock<BTreeMap<u16, u16>> = OnceLock::new();
    INDICES.get_or_init(|| {
        let layout: serde_json::Value =
            serde_json::from_str(include_str!("../assets/ace75-layout.json"))
                .expect("Embedded Ace75 layout");
        layout["keys"]
            .as_array()
            .expect("Ace75 keys")
            .iter()
            .filter_map(|cell| {
                Some((
                    u16::try_from(cell["code"].as_u64()?).ok()?,
                    u16::try_from(cell["matrix_index"].as_u64()?).ok()?,
                ))
            })
            .collect()
    })
}

fn calibration_assignments() -> &'static BTreeMap<(u8, u8, u8), u16> {
    static ASSIGNMENTS: OnceLock<BTreeMap<(u8, u8, u8), u16>> = OnceLock::new();
    ASSIGNMENTS.get_or_init(|| {
        let layers: Vec<Vec<[u8; 3]>> =
            serde_json::from_str(include_str!("../assets/ace75-default-key-layers.json"))
                .expect("Embedded Ace75 factory layers");
        calibration_indices()
            .values()
            .map(|index| {
                let [kind, code1, code2] = layers[0][*index as usize];
                ((kind, code1, code2), *index)
            })
            .collect()
    })
}

fn with_cleanup(error: anyhow::Error, cleanup: Result<()>) -> anyhow::Error {
    match cleanup {
        Ok(()) => error,
        Err(cleanup) => error.context(format!(
            "ACE75: восстановление настроек не подтверждено: {cleanup:#}"
        )),
    }
}

fn validate_trigger_changes(
    current: &[KeyTrigger],
    wanted: &[KeyTrigger],
    tachyon: bool,
    firmware: &str,
) -> Result<()> {
    ensure!(
        current.len() == 128 && wanted.len() == 128,
        "ACE75: неверное число магнитных настроек"
    );
    let firmware = firmware.parse::<u32>().unwrap_or(0);
    for &index in calibration_indices().values() {
        let before = &current[index as usize];
        let after = &wanted[index as usize];
        after.validate()?;
        if before == after {
            continue;
        }
        if before.switch_type != after.switch_type {
            ensure!(
                after.switch_type <= 5 && (after.switch_type != 5 || firmware >= 117),
                "ACE75: переключатель не поддерживается этой прошивкой"
            );
        }
        let travel = if after.switch_type == 3 { 350 } else { 340 };
        if before.actuation != after.actuation || before.switch_type != after.switch_type {
            ensure!(
                (10..=travel).contains(&after.actuation),
                "ACE75: точка срабатывания за пределами хода переключателя"
            );
        }
        if before.key_mode != after.key_mode {
            ensure!(
                after.key_mode <= 2,
                "ACE75: неподдерживаемый режим Rapid Trigger"
            );
        }
        for (old_precision, precision, old_value, value) in [
            (
                before.press_precision,
                after.press_precision,
                before.rt_press,
                after.rt_press,
            ),
            (
                before.release_precision,
                after.release_precision,
                before.rt_release,
                after.rt_release,
            ),
        ] {
            if precision != old_precision {
                ensure!(
                    precision < 3 && (precision == 0 || tachyon),
                    "ACE75: высокая точность RT требует режима Tachyon"
                );
            }
            if value != old_value
                || precision != old_precision
                || before.switch_type != after.switch_type
            {
                let max = if precision == 0 { travel } else { 500 };
                ensure!(
                    (if tachyon { 1 } else { 2 }..=max).contains(&value),
                    "ACE75: значение Rapid Trigger вне допустимого диапазона"
                );
            }
        }
        if before.press_deadzone != after.press_deadzone {
            ensure!(
                after.press_deadzone <= 50,
                "ACE75: мёртвая зона должна быть не больше 0,50 мм"
            );
        }
        if before.release_deadzone != after.release_deadzone {
            ensure!(
                after.release_deadzone <= 50,
                "ACE75: мёртвая зона должна быть не больше 0,50 мм"
            );
        }
    }
    Ok(())
}

fn validate_macro(value: &MacroDefinition) -> Result<()> {
    ensure!(
        value.macro_index < 16 && value.default_delay >= 5 && value.name.chars().count() <= 256,
        "ACE75: неверные параметры макроса"
    );
    ensure!(
        ace75::encode_macro_actions(&value.macro_actions)?.len() <= 8128,
        "ACE75: макрос превышает вместимость банка"
    );
    Ok(())
}

fn validate_macro_bank(value: &MacroBank) -> Result<()> {
    value.encode()?;
    for item in &value.macros {
        validate_macro(item)?;
    }
    Ok(())
}

fn read_macro(path: &Path) -> Result<MacroDefinition> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 4_000_000,
        "ACE75: нужен обычный JSON-файл до 4 МБ"
    );
    let value = serde_json::from_slice(&fs::read(path)?)?;
    validate_macro(&value)?;
    Ok(value)
}

fn validate_preset(value: &KeyboardPreset) -> Result<()> {
    ensure!(
        value.schema_version == 1 && value.model == ace75::MODEL,
        "ACE75: профиль относится к другой модели или версии формата"
    );
    ensure!(
        value.profile.index < 4
            && value.profile.layers.len() == 4
            && value.profile.layers.iter().all(|layer| layer.len() == 128)
            && value.profile.triggers.len() == 128
            && value.profile.colors.len() == 128
            && value.profile.custom_raw.len() == 1024,
        "ACE75: неверная структура профиля"
    );
    value.profile.functions.performance.validate()?;
    value.profile.functions.lighting.validate()?;
    for trigger in &value.profile.triggers {
        trigger.validate()?;
    }
    value.profile.advanced.encode()?;
    value
        .profile
        .advanced
        .validate_references(&value.profile.layers)?;
    value.profile.custom_params.encode()?;
    if let Some(macros) = &value.macros {
        validate_macro_bank(macros)?;
    }
    Ok(())
}

fn read_preset(path: &Path) -> Result<KeyboardPreset> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && metadata.len() <= 4_000_000,
        "ACE75: нужен обычный JSON-файл до 4 МБ"
    );
    let value = serde_json::from_slice(&fs::read(path)?)?;
    validate_preset(&value)?;
    Ok(value)
}

fn save_new_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn replace_bytes(path: &Path, previous: &[u8], bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("pending-{}-{}", std::process::id(), stamp()?));
    let result = (|| -> Result<()> {
        save_new_bytes(&temporary, bytes)?;
        ensure!(
            fs::symlink_metadata(path)?.is_file() && fs::read(path)? == previous,
            "ACE75: файл изменился во время сохранения"
        );
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(path.parent().context("ACE75: у файла нет каталога")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HidIo;
    use std::collections::VecDeque;

    struct Memory {
        regions: BTreeMap<u8, Vec<u8>>,
        replies: VecDeque<Vec<u8>>,
        mutations: Vec<u8>,
    }
    struct Io(Arc<Mutex<Memory>>);
    impl HidIo for Io {
        fn send_feature(&self, _: u8, _: &[u8]) -> Result<()> {
            anyhow::bail!("Unexpected feature report")
        }
        fn get_feature(&self, _: u8, _: usize) -> Result<Vec<u8>> {
            anyhow::bail!("Unexpected feature report")
        }
        fn write_output(&self, id: u8, packet: &[u8]) -> Result<()> {
            assert_eq!(id, 0);
            let mut memory = self.0.lock().unwrap();
            let command = packet[1];
            let offset = u16::from_le_bytes([packet[5], packet[6]]) as usize;
            let length = packet[4] as usize;
            let data = match command {
                3 => include_bytes!("../tests/fixtures/ace75/info-response.bin")[8..39].to_vec(),
                4 => include_bytes!("../tests/fixtures/ace75/base-response.bin")[8..64].to_vec(),
                6 => {
                    memory.regions.get_mut(&5).unwrap()[offset..offset + length]
                        .copy_from_slice(&packet[8..8 + length]);
                    memory.mutations.push(command);
                    vec![]
                }
                0xa8 | 0xa9 | 0xb1 => {
                    memory.mutations.push(command);
                    vec![]
                }
                _ => memory
                    .regions
                    .get(&command)
                    .unwrap_or_else(|| panic!("Unexpected command {command:02x}"))
                    [offset..offset + length]
                    .to_vec(),
            };
            let mut reply = vec![0; 64];
            reply[..8].copy_from_slice(&[
                0xaa,
                command,
                0,
                0,
                data.len() as u8,
                offset as u8,
                (offset >> 8) as u8,
                0,
            ]);
            reply[8..8 + data.len()].copy_from_slice(&data);
            reply[3] = reply[4..8 + data.len()]
                .iter()
                .fold(0u8, |sum, value| sum.wrapping_add(*value));
            memory.replies.push_back(reply);
            Ok(())
        }
        fn read_input(&self, _: i32) -> Result<Vec<u8>> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .replies
                .pop_front()
                .unwrap_or_default())
        }
    }
    fn session() -> (KeyboardSession, Arc<Mutex<Memory>>) {
        let mut regions: BTreeMap<u8, Vec<u8>> = [
            (5, 256),
            (8, 8192),
            (0xa0, 4096),
            (0x0a, 2048),
            (0xa2, 3072),
            (0xa4, 1024),
            (0xa6, 512),
            (0xf1, 4096),
            (0x0c, 8192),
        ]
        .into_iter()
        .map(|(command, size)| (command, vec![0; size]))
        .collect();
        regions.get_mut(&0xf1).unwrap().fill(255);
        let memory = Arc::new(Mutex::new(Memory {
            regions,
            replies: VecDeque::new(),
            mutations: vec![],
        }));
        let mut session = KeyboardSession::new(false);
        session.root = std::env::temp_dir().join(format!(
            "mchose-ace75-controller-test-{}-{}",
            std::process::id(),
            stamp().unwrap()
        ));
        session.driver = Some(Ace75::new(Box::new(Io(memory.clone()))));
        (session, memory)
    }

    #[test]
    fn full_backup_reads_all_four_profiles_without_mutations_or_typed_decode() {
        let (mut session, memory) = session();
        memory.lock().unwrap().regions.get_mut(&0xf1).unwrap()[3 * 1024..4 * 1024].fill(0xa2);
        let path = session.backup("test").unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["profiles"].as_array().unwrap().len(), 4);
        assert_eq!(value["macro_banks"].as_array().unwrap().len(), 1);
        assert!(
            value["profiles"][3]["custom"]
                .as_array()
                .unwrap()
                .iter()
                .all(|byte| byte == 162)
        );
        assert!(memory.lock().unwrap().mutations.is_empty());
        assert!(write_keyboard_backup(session.driver().unwrap(), &path).is_err());
        fs::remove_dir_all(session.root).unwrap();
    }

    #[test]
    fn monitoring_restores_both_possible_original_debug_values() {
        for original in [false, true] {
            let (mut session, memory) = session();
            memory.lock().unwrap().regions.get_mut(&5).unwrap()[7] =
                if original { 0x18 } else { 0x10 };
            session.enable_debug(0).unwrap();
            assert_eq!(memory.lock().unwrap().regions[&5][7], 0x18);
            session.state.calibration_active = true;
            session.state.monitor_active = true;
            session.stop_debug().unwrap();
            let memory = memory.lock().unwrap();
            assert_eq!(memory.regions[&5][7], if original { 0x18 } else { 0x10 });
            assert!(memory.mutations.contains(&0xa9));
            assert!(!session.state.monitor_active);
            assert!(session.debug_restore.is_none());
        }
    }

    #[test]
    fn refresh_during_monitoring_does_not_export_the_temporary_debug_bit() {
        let (mut session, memory) = session();
        session.refresh().unwrap();
        session.enable_debug(0).unwrap();
        session.state.monitor_active = true;
        session.refresh().unwrap();
        assert!(memory.lock().unwrap().regions[&5][7] & 8 != 0);
        assert!(
            !session
                .current_preset()
                .unwrap()
                .profile
                .functions
                .performance
                .debug_mode
        );
        session.stop_debug().unwrap();
        assert_eq!(memory.lock().unwrap().regions[&5][7] & 8, 0);
    }

    #[test]
    fn partial_profile_apply_preserves_macro_metadata_and_writes_only_changed_group() {
        let (mut session, memory) = session();
        memory.lock().unwrap().regions.get_mut(&5).unwrap()[4] = 1;
        session.state.device = Some(DeviceInfo {
            path: "test-keyboard".into(),
            name: ace75::USB_PRODUCT.into(),
            vendor_id: 0x3837,
            product_id: 0x303c,
            interface: 1,
            usage_page: 1,
            usage: 0,
            kind: DeviceKind::Keyboard,
        });
        session.refresh().unwrap();
        let mut bank = session.state.snapshot.as_ref().unwrap().macros.clone();
        bank.macros[0].name = "Saved name".into();
        bank.macros[0].default_delay = 123;
        session.store_macro_metadata(&bank).unwrap();
        let mut value = session.current_preset().unwrap();
        value.profile.functions.performance.lock_alt_tab = true;
        value.macros = None;
        value.hub_scope = Some(HubProfileScope {
            performance: true,
            ..Default::default()
        });
        let path = session.preset_path("partial").unwrap();
        save_new_json(&path, &value).unwrap();
        session
            .execute(KeyboardCommand::ApplyPreset {
                device_path: "test-keyboard".into(),
                profile: 0,
                path,
            })
            .unwrap();
        let actual = &session.state.snapshot.as_ref().unwrap().macros.macros[0];
        assert_eq!(actual.name, "Saved name");
        assert_eq!(actual.default_delay, 123);
        let memory = memory.lock().unwrap();
        assert!(!memory.mutations.is_empty());
        assert!(memory.mutations.iter().all(|command| *command == 6));
        fs::remove_dir_all(session.root).unwrap();
    }

    #[test]
    fn read_only_blocks_device_commands_but_keeps_local_macro_library() {
        let (mut session, memory) = session();
        session.read_only = true;
        assert!(
            session
                .execute(KeyboardCommand::StartCalibration {
                    device_path: "unused".into(),
                    profile: 0
                })
                .is_err()
        );
        assert!(memory.lock().unwrap().mutations.is_empty());
        let value = MacroBank::decode(&vec![0; 8192]).unwrap().macros[0].clone();
        session
            .execute(KeyboardCommand::SaveLocalMacro {
                name: "sample".into(),
                value,
            })
            .unwrap();
        let path = session.macro_path("sample").unwrap();
        session
            .execute(KeyboardCommand::OpenLocalMacro(path.clone()))
            .unwrap();
        assert_eq!(session.state.local_macro.as_ref().unwrap().name, "sample");
        assert!(
            session
                .execute(KeyboardCommand::RenameLocalMacro {
                    path: path.clone(),
                    name: "../outside".into()
                })
                .is_err()
        );
        assert!(path.exists());
        session
            .execute(KeyboardCommand::DeleteLocalMacro(path.clone()))
            .unwrap();
        assert!(!path.exists());
        assert_eq!(
            fs::read_dir(session.root.join("backups")).unwrap().count(),
            1
        );
        fs::remove_dir_all(session.root).unwrap();
    }

    #[test]
    fn telemetry_is_coalesced_without_full_snapshot_messages() {
        let (session, _) = session();
        let mut view = session.state.clone();
        session
            .state
            .telemetry
            .lock()
            .unwrap()
            .calibration_keys
            .insert(88, 255);
        view.pull_telemetry();
        assert_eq!(view.calibration_keys.get(&88), Some(&255));
        assert_eq!(calibration_indices().get(&4), Some(&88));
        assert_eq!(calibration_assignments().get(&(16, 0, 4)), Some(&88));
        assert_eq!(calibration_assignments().get(&(240, 255, 1)), Some(&22));
        let progress = session.state.progress_channel();
        for done in 0..15_000 {
            *progress.lock().unwrap() = Some((done, 15_000));
        }
        assert_eq!(view.firmware_progress(), Some((14_999, 15_000)));
        *progress.lock().unwrap() = None;
        assert!(view.firmware_progress().is_none());
    }

    #[test]
    fn physical_trigger_preflight_checks_edits_and_preserves_opaque_slots() {
        let profile = empty_import_baseline().unwrap();
        let before = profile.triggers;
        let mut after = before.clone();
        after[127].actuation = 512;
        validate_trigger_changes(&before, &after, false, "117").unwrap();
        after[88].actuation = 400;
        assert!(validate_trigger_changes(&before, &after, false, "117").is_err());
        after[88].actuation = 100;
        after[88].switch_type = 5;
        after[88].rt_press = 5;
        after[88].rt_release = 5;
        assert!(validate_trigger_changes(&before, &after, false, "116").is_err());
        validate_trigger_changes(&before, &after, false, "117").unwrap();
        after[88].press_precision = 1;
        assert!(validate_trigger_changes(&before, &after, false, "117").is_err());
        validate_trigger_changes(&before, &after, true, "117").unwrap();
        after[88].press_deadzone = 51;
        assert!(validate_trigger_changes(&before, &after, true, "117").is_err());
    }
}
