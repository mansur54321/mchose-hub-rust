use super::Palette;
use crate::ace75::host_lighting::{LightingAnimation, LightingFrame};
use crate::ace75::{
    FunctionConfig, KeyAssignment, KeyTrigger, KeyboardProfile, LightScope, MacroAction, MacroBank,
    MacroKeyAction,
};
use crate::i18n::{self, Language, t};
use crate::keyboard_controller::{KeyboardCommand, KeyboardPreset, KeyboardState};
use crate::tf;
use eframe::egui::{self, Color32, RichText, Stroke, Vec2};
use serde::Deserialize;
use std::{collections::BTreeSet, path::PathBuf, sync::OnceLock};

#[derive(Debug, Deserialize)]
pub(crate) struct Layout {
    pub canvas: Canvas,
    pub keys: Vec<Cell>,
    pub controls: Vec<Cell>,
    pub lighting_regions: Vec<Cell>,
    pub hidden_actions: Vec<HiddenAction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Canvas {
    pub width: f32,
    pub height: f32,
    pub top_inset: f32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Cell {
    pub source_index: u16,
    pub matrix_index: u16,
    pub code: u16,
    pub label: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub available: Vec<String>,
    pub hidden: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct HiddenAction {
    pub matrix_index: u16,
    pub parent_matrix_index: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum GridPurpose {
    #[default]
    Assignment,
    Trigger,
    Performance,
    Advanced,
    Lighting,
}

impl GridPurpose {
    fn field(self) -> &'static str {
        match self {
            Self::Assignment => "keyCode",
            Self::Trigger => "trigger",
            Self::Performance => "performance",
            Self::Advanced => "advancedKey",
            Self::Lighting => "light",
        }
    }
}

impl Cell {
    pub fn supports(&self, purpose: GridPurpose) -> bool {
        let field = purpose.field();
        let matches = |value: &str| value.split(':').next() == Some(field);
        !self.hidden.iter().any(|value| matches(value))
            && (self.available.is_empty() || self.available.iter().any(|value| matches(value)))
    }

    fn legend(&self) -> &str {
        match self.label.as_str() {
            "gunlun" => "↕",
            "Delete" => "Del",
            "LShift" | "RShift" => "Shift",
            "LCtrl" | "RCtrl" => "Ctrl",
            "LAlt" => "Alt",
            "Space" | "sideLight" | "side2Light" | "Light1" | "Light2" | "Light3" | "Light4" => "",
            _ if self.code == 49 => "\\",
            _ => &self.label,
        }
    }

    fn description(&self) -> String {
        match self.label.as_str() {
            "gunlun" => t("Колесо").into(),
            "LShift" => t("Левый Shift").into(),
            "RShift" => t("Правый Shift").into(),
            "LCtrl" => t("Левый Ctrl").into(),
            "RCtrl" => t("Правый Ctrl").into(),
            "LAlt" => t("Левый Alt").into(),
            "sideLight" => tf!("Сегмент полосы {}", self.matrix_index - 94),
            "side2Light" => t("Подсветка логотипа").into(),
            "Light1" | "Light2" | "Light3" | "Light4" => t("Подсветка пробела").into(),
            _ => self.label.clone(),
        }
    }
}

pub(crate) fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/ace75-layout.json"))
            .expect("Embedded ACE75 layout")
    })
}

fn cells(purpose: GridPurpose) -> impl Iterator<Item = &'static Cell> {
    let layout = layout();
    layout
        .keys
        .iter()
        .chain(&layout.controls)
        .chain(
            layout
                .lighting_regions
                .iter()
                .filter(move |_| purpose == GridPurpose::Lighting),
        )
        .filter(move |cell| cell.supports(purpose))
}

fn onboard_profile_name(value: &str, profile: u8) -> String {
    let default_name = value
        .strip_prefix("i18n<defaultOnboard>")
        .is_some_and(|suffix| suffix.bytes().all(|byte| byte.is_ascii_digit()));
    if value.is_empty() || default_name {
        tf!("Профиль {}", profile + 1)
    } else {
        value.into()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Page {
    #[default]
    Keys,
    Trigger,
    Advanced,
    Performance,
    Lighting,
    Macros,
    Profiles,
    Device,
}

impl Page {
    fn title(self) -> &'static str {
        t(match self {
            Self::Keys => "Клавиши",
            Self::Trigger => "Срабатывание",
            Self::Advanced => "Дополнительно",
            Self::Macros => "Макросы",
            Self::Performance => "Параметры",
            Self::Lighting => "Подсветка",
            Self::Profiles => "Профили",
            Self::Device => "Устройство",
        })
    }
}

pub(crate) enum Action {
    Send(KeyboardCommand),
    Confirm(String, KeyboardCommand),
}

#[derive(Default)]
pub(crate) struct Ace75Ui {
    pub layer: u8,
    page: Page,
    purpose: GridPurpose,
    pub selected_keys: BTreeSet<u16>,
    pub selected_lights: BTreeSet<u16>,
    multi_select: bool,
    rt_double: bool,
    paint_color: [u8; 3],
    macro_slot: usize,
    macro_local_mode: bool,
    local_macro_bank: Option<MacroBank>,
    local_macro_original: Option<MacroBank>,
    local_macro_path: Option<PathBuf>,
    local_macro_pending: Option<PathBuf>,
    local_macro_saving: Option<MacroBank>,
    local_macro_renaming: Option<(PathBuf, String)>,
    local_macro_deleting: Option<PathBuf>,
    macro_library_name: String,
    macro_library_file: String,
    macro_target_slot: u8,
    new_macro_confirm: bool,
    macro_draft: Option<MacroBank>,
    macro_baseline: Option<MacroBank>,
    local_macro_baseline: Option<MacroBank>,
    macro_event_kind: u8,
    macro_event_code: u16,
    macro_event_delay: u16,
    macro_event_pressed: bool,
    onboard_name: String,
    onboard_name_baseline: Option<String>,
    onboard_layout: Option<(u8, Vec<u8>)>,
    onboard_layout_baseline: Option<(u8, Vec<u8>)>,
    firmware_path: String,
    firmware_download_directory: String,
    music_scopes: Option<[bool; 3]>,
    animation_draft: Option<LightingAnimation>,
    animation_source: Option<LightingAnimation>,
    animation_loading: bool,
    animation_saving: Option<LightingAnimation>,
    animation_path: String,
    animation_frame: usize,
    frame_preview: Option<Vec<[u8; 3]>>,
    advanced_editor: Option<AdvancedEditor>,
    calibrated: BTreeSet<u16>,
    calibrating: bool,
    recorder: Option<crate::recorder::WindowKeyboardRecorder>,
    record_area: Option<egui::Rect>,
    record_held: BTreeSet<(u8, u16)>,
    recorded_at: Option<std::time::Instant>,
    record_skipped_ms: u32,
    revision: Option<u64>,
    device_path: Option<String>,
    baseline: Option<KeyboardProfile>,
    draft: Option<KeyboardProfile>,
    local_edit: Option<(PathBuf, KeyboardPreset)>,
    local_baseline: Option<KeyboardProfile>,
    preview_path: Option<PathBuf>,
    preview_pending: Option<PathBuf>,
    saving: Option<KeyboardPreset>,
    action: Option<Action>,
    message: String,
    preset_name: String,
    preset_path: String,
    light_scope: u8,
    action_search: String,
    action_category: String,
    chord_modifiers: u8,
    chord_usage: u16,
}

impl Ace75Ui {
    pub fn take_action(&mut self) -> Option<Action> {
        self.action.take()
    }
    pub fn command_started(&mut self, command: &KeyboardCommand) {
        if let KeyboardCommand::SaveEditedPreset { value, .. } = command {
            self.saving = Some(value.clone());
        }
        if let KeyboardCommand::OpenLocalMacro(path) = command {
            self.local_macro_pending = Some(path.clone());
        }
        if matches!(
            command,
            KeyboardCommand::SaveLocalMacro { .. } | KeyboardCommand::UpdateLocalMacro { .. }
        ) && self.macro_local_mode
        {
            self.local_macro_saving = self.local_macro_bank.clone();
        }
        if let KeyboardCommand::RenameLocalMacro { path, name } = command {
            self.local_macro_renaming = Some((path.clone(), name.trim().into()));
        }
        if let KeyboardCommand::DeleteLocalMacro(path) = command {
            self.local_macro_deleting = Some(path.clone());
        }
        if matches!(command, KeyboardCommand::LoadAnimation(_)) {
            self.animation_loading = true;
        }
        if let KeyboardCommand::SaveAnimation { value, .. } = command {
            self.animation_saving = Some(value.clone());
        }
    }

    pub fn select_page(&mut self, name: &str) {
        if name != "macros" {
            self.stop_recording(None);
        }
        self.page = match name {
            "trigger" | "dpi" => Page::Trigger,
            "advanced" => Page::Advanced,
            "macros" => Page::Macros,
            "performance" => Page::Performance,
            "lighting" | "dock" => Page::Lighting,
            "profiles" => Page::Profiles,
            "system" | "device" => Page::Device,
            _ => Page::Keys,
        };
    }

    pub fn sync(&mut self, state: &KeyboardState, busy: bool) {
        self.calibrated = state
            .calibration_keys
            .iter()
            .filter_map(|(index, status)| (*status == 255).then_some(*index))
            .collect();
        self.calibrating = state.calibration_active;
        if !busy {
            if let Some((old_path, name)) = self.local_macro_renaming.take() {
                if state.error.is_none() && self.local_macro_path.as_ref() == Some(&old_path) {
                    if let Some(entry) = state.macros.iter().find(|entry| entry.name == name) {
                        self.local_macro_path = Some(entry.path.clone());
                        if let Some(original) = self
                            .local_macro_original
                            .as_mut()
                            .and_then(|bank| bank.macros.first_mut())
                        {
                            if let Some(draft) = self
                                .local_macro_bank
                                .as_mut()
                                .and_then(|bank| bank.macros.first_mut())
                            {
                                if draft.name == original.name {
                                    draft.name = name.clone();
                                }
                            }
                            original.name = name;
                        }
                    }
                }
            }
            if let Some(path) = self.local_macro_deleting.take() {
                if state.error.is_none() && self.local_macro_path.as_ref() == Some(&path) {
                    self.local_macro_path = None;
                    self.local_macro_original = None;
                }
            }
            if let Some(value) = self.animation_saving.take() {
                if state.error.is_none() {
                    self.animation_source = Some(value);
                }
            }
            if self.animation_loading {
                self.animation_loading = false;
                if state.error.is_none() {
                    self.animation_draft = state.animation.clone();
                    self.animation_source = state.animation.clone();
                    self.animation_frame = 0;
                }
            }
            if let Some(path) = self.local_macro_pending.take() {
                if state.error.is_none() {
                    if let Some(value) = &state.local_macro {
                        self.new_local_macro();
                        let mut value = value.clone();
                        value.macro_index = 0;
                        self.local_macro_bank.as_mut().unwrap().macros[0] = value;
                        self.local_macro_original = self.local_macro_bank.clone();
                        self.local_macro_path = Some(path);
                    }
                }
            }
            if let Some(bank) = self.local_macro_saving.take() {
                if state.error.is_none() {
                    self.local_macro_path = state
                        .macros
                        .iter()
                        .find(|item| item.name == bank.macros[0].name)
                        .map(|item| item.path.clone())
                        .or(self.local_macro_path.clone());
                    self.local_macro_original = Some(bank);
                }
            }
            if let Some(path) = self.preview_pending.take() {
                if state.error.is_none() && state.preview.is_some() {
                    self.preview_path = Some(path);
                }
            }
            if let Some(saved) = self.saving.take() {
                if state.error.is_none()
                    && state.preview.as_ref().is_some_and(|actual| {
                        actual.profile == saved.profile && actual.macros == saved.macros
                    })
                {
                    self.local_baseline = Some(saved.profile.clone());
                    self.local_macro_baseline = saved.macros.clone();
                    if let Some((_, value)) = &mut self.local_edit {
                        *value = saved;
                    }
                }
            }
        }
        if self.revision == Some(state.revision) {
            return;
        }
        self.revision = Some(state.revision);
        let path = state.device.as_ref().map(|device| device.path.clone());
        let Some(snapshot) = &state.snapshot else {
            return;
        };
        let fresh_name = onboard_profile_name(
            snapshot
                .profile_names
                .names
                .get(snapshot.profile.index as usize)
                .map_or("", String::as_str),
            snapshot.profile.index,
        );
        if self.onboard_name.is_empty()
            || self.onboard_name_baseline.as_ref() == Some(&self.onboard_name)
        {
            self.onboard_name = fresh_name.clone();
        }
        self.onboard_name_baseline = Some(fresh_name);
        let fresh_layout = Some((
            snapshot.base.profile_count,
            snapshot.base.profile_order.clone(),
        ));
        if self.onboard_layout == self.onboard_layout_baseline {
            self.onboard_layout = fresh_layout.clone();
        }
        self.onboard_layout_baseline = fresh_layout;
        let same_scope = path == self.device_path
            && self
                .baseline
                .as_ref()
                .is_some_and(|profile| profile.index == snapshot.profile.index);
        if !same_scope {
            self.stop_recording(None);
        }
        if !same_scope && self.hardware_dirty() {
            self.message = t("Активный профиль или устройство изменились. Черновик сохранён; отмените изменения, чтобы прочитать новый профиль.").into();
            return;
        }
        if same_scope {
            if let (Some(previous), Some(draft)) = (&self.baseline, &mut self.draft) {
                merge_profile(draft, previous, &snapshot.profile);
            }
        } else {
            self.draft = Some(snapshot.profile.clone());
        }
        if same_scope {
            if let (Some(draft), Some(previous)) = (&mut self.macro_draft, &self.macro_baseline) {
                for (index, value) in draft.macros.iter_mut().enumerate() {
                    if previous.macros.get(index) == Some(value) {
                        *value = snapshot.macros.macros[index].clone();
                    }
                }
                draft.raw = snapshot.macros.raw.clone();
            }
        } else {
            self.macro_draft = Some(snapshot.macros.clone());
        }
        self.macro_baseline = Some(snapshot.macros.clone());
        self.baseline = Some(snapshot.profile.clone());
        self.device_path = path;
    }

    fn working(&self) -> Option<&KeyboardProfile> {
        self.local_edit
            .as_ref()
            .map(|(_, value)| &value.profile)
            .or(self.draft.as_ref())
    }
    fn working_mut(&mut self) -> Option<&mut KeyboardProfile> {
        if let Some((_, value)) = &mut self.local_edit {
            Some(&mut value.profile)
        } else {
            self.draft.as_mut()
        }
    }
    fn original(&self) -> Option<&KeyboardProfile> {
        if self.local_edit.is_some() {
            self.local_baseline.as_ref()
        } else {
            self.baseline.as_ref()
        }
    }
    fn macro_working(&self) -> Option<&MacroBank> {
        if self.macro_local_mode {
            self.local_macro_bank.as_ref()
        } else if let Some((_, value)) = &self.local_edit {
            value.macros.as_ref()
        } else {
            self.macro_draft.as_ref()
        }
    }
    fn macro_working_mut(&mut self) -> Option<&mut MacroBank> {
        if self.macro_local_mode {
            self.local_macro_bank.as_mut()
        } else if let Some((_, value)) = &mut self.local_edit {
            value.macros.as_mut()
        } else {
            self.macro_draft.as_mut()
        }
    }
    fn profile_macros_dirty(&self) -> bool {
        if let Some((_, value)) = &self.local_edit {
            value.macros != self.local_macro_baseline
        } else {
            self.macro_draft != self.macro_baseline
        }
    }
    fn macros_dirty(&self) -> bool {
        if self.macro_local_mode {
            self.local_macro_bank != self.local_macro_original
        } else {
            self.profile_macros_dirty()
        }
    }
    fn hardware_dirty(&self) -> bool {
        self.draft != self.baseline
            || self.macro_draft != self.macro_baseline
            || self.onboard_layout != self.onboard_layout_baseline
    }
    fn part_dirty(&self, part: Page) -> bool {
        if self.local_edit.is_some() {
            return self.dirty();
        }
        let (Some(value), Some(original)) = (self.working(), self.original()) else {
            return false;
        };
        match part {
            Page::Keys => value.layers[self.layer as usize]
                .iter()
                .zip(&original.layers[self.layer as usize])
                .enumerate()
                .any(|(index, (left, right))| ![93, 94].contains(&index) && left != right),
            Page::Trigger => {
                value.triggers != original.triggers
                    || value.functions.performance.tachyon_mode
                        != original.functions.performance.tachyon_mode
            }
            Page::Lighting => value.functions.lighting != original.functions.lighting,
            Page::Macros => self.macros_dirty(),
            Page::Advanced => {
                value.advanced != original.advanced || value.layers != original.layers
            }
            Page::Performance => {
                let mut parameters = value.functions.performance.clone();
                parameters.report_rate = original.functions.performance.report_rate;
                parameters.tachyon_mode = original.functions.performance.tachyon_mode;
                parameters != original.functions.performance
            }
            _ => self.dirty(),
        }
    }
    fn dirty(&self) -> bool {
        self.working() != self.original() || self.profile_macros_dirty()
    }
    fn target(&self, state: &KeyboardState) -> Option<(String, u8)> {
        let path = self.device_path.as_ref()?;
        let profile = self.draft.as_ref()?.index;
        (state
            .device
            .as_ref()
            .is_some_and(|device| &device.path == path)
            && state
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.profile.index == profile))
        .then(|| (path.clone(), profile))
    }

    pub fn sidebar(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.label(RichText::new("ACE75").size(23.0).strong());
        ui.label(RichText::new("MCHOSE · USB").small().color(palette.muted));
        ui.add_space(16.0);
        if let Some(snapshot) = &state.snapshot {
            ui.label(tf!("Прошивка: {}", snapshot.info.firmware_version));
            ui.add_space(20.0);
            ui.label(RichText::new(t("Профили устройства")).strong());
            for profile in snapshot
                .base
                .profile_order
                .iter()
                .take(snapshot.base.profile_count as usize)
            {
                let active = snapshot.profile.index == *profile;
                let name = onboard_profile_name(
                    snapshot
                        .profile_names
                        .names
                        .get(*profile as usize)
                        .map_or("", String::as_str),
                    *profile,
                );
                let enabled =
                    writable && !busy && !self.hardware_dirty() && self.local_edit.is_none();
                if ui
                    .add_enabled(
                        enabled,
                        egui::Button::new(RichText::new(name).color(if active {
                            Color32::WHITE
                        } else {
                            palette.text
                        }))
                        .fill(if active {
                            palette.accent
                        } else {
                            palette.field
                        })
                        .min_size(Vec2::new(ui.available_width(), 34.0)),
                    )
                    .clicked()
                {
                    if let Some(device) = &state.device {
                        self.action = Some(Action::Send(KeyboardCommand::SwitchProfile {
                            device_path: device.path.clone(),
                            profile: *profile,
                        }));
                    }
                }
                ui.add_space(5.0);
            }
            if self.hardware_dirty() {
                ui.label(
                    RichText::new(t("Сначала примените или отмените изменения."))
                        .small()
                        .color(palette.muted),
                );
            }
        } else {
            ui.label(
                RichText::new(t(if state.bootloader.is_some() {
                    "ACE75 в режиме обновления"
                } else {
                    "Нет данных клавиатуры"
                }))
                .color(palette.muted),
            );
            ui.label(RichText::new(t("Подключите ACE75 по USB.")).color(palette.muted));
        }
        ui.add_space(18.0);
        if ui
            .add_enabled(!busy, egui::Button::new(t("Перечитать")))
            .clicked()
        {
            self.action = Some(Action::Send(KeyboardCommand::Refresh));
        }
        ui.add_space(22.0);
        ui.label(RichText::new(t("Локальные профили")).strong());
        for preset in &state.presets {
            if ui
                .add_enabled(
                    !busy && self.local_edit.is_none(),
                    egui::Button::new(&preset.name).min_size(Vec2::new(ui.available_width(), 30.0)),
                )
                .clicked()
            {
                self.preview_pending = Some(preset.path.clone());
                self.action = Some(Action::Send(KeyboardCommand::PreviewPreset(
                    preset.path.clone(),
                )));
                self.page = Page::Profiles;
            }
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            for page in [
                Page::Keys,
                Page::Trigger,
                Page::Advanced,
                Page::Performance,
                Page::Lighting,
                Page::Macros,
                Page::Profiles,
                Page::Device,
            ] {
                if ui
                    .add_enabled(
                        !(busy && (state.calibration_active || state.monitor_active)),
                        egui::Button::new(RichText::new(page.title()).color(
                            if self.page == page {
                                Color32::WHITE
                            } else {
                                palette.text
                            },
                        ))
                        .fill(if self.page == page {
                            palette.accent
                        } else {
                            Color32::TRANSPARENT
                        })
                        .min_size(Vec2::new(0.0, 34.0)),
                    )
                    .clicked()
                {
                    if page != Page::Macros {
                        self.stop_recording(None);
                    }
                    if self.page == Page::Device
                        && page != Page::Device
                        && (state.calibration_active || state.monitor_active)
                    {
                        self.action = Some(Action::Send(KeyboardCommand::StopCalibration));
                    }
                    self.page = page;
                }
            }
        });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(12.0);
        if let Some((path, _)) = &self.local_edit {
            ui.label(
                RichText::new(tf!(
                    "Редактируется локальный профиль: {}",
                    path.file_stem().unwrap_or_default().to_string_lossy()
                ))
                .color(palette.accent_text),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !busy && self.dirty(),
                        egui::Button::new(t("Сохранить в файл")),
                    )
                    .clicked()
                {
                    self.save_local();
                }
                if ui
                    .add_enabled(!self.dirty(), egui::Button::new(t("Закрыть редактор")))
                    .clicked()
                {
                    self.local_edit = None;
                    self.local_baseline = None;
                }
                if self.dirty() && ui.button(t("Отменить изменения")).clicked() {
                    if let (Some((_, value)), Some(original)) =
                        (&mut self.local_edit, &self.local_baseline)
                    {
                        value.profile = original.clone();
                        value.macros = self.local_macro_baseline.clone();
                    }
                }
            });
            ui.add_space(8.0);
        }
        if !self.message.is_empty() {
            ui.label(RichText::new(i18n::message(&self.message)).color(palette.error_text));
        }
        if let Some(error) = &state.error {
            ui.label(RichText::new(i18n::message(error)).color(palette.error_text));
        } else if !state.message.is_empty() {
            ui.label(
                RichText::new(i18n::message(&state.message))
                    .small()
                    .color(palette.muted),
            );
        }
        if let Some(warning) = &state.cleanup_warning {
            ui.label(
                RichText::new(i18n::message(warning)).color(if ui.visuals().dark_mode {
                    Color32::from_rgb(245, 156, 37)
                } else {
                    Color32::from_rgb(138, 79, 0)
                }),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt(("ace75-page", self.page as u8))
            .auto_shrink([false, false])
            .show(ui, |ui| match self.page {
                Page::Keys => self.keys_page(ui, palette, state, busy, writable),
                Page::Trigger => self.trigger_page(ui, palette, state, busy, writable),
                Page::Advanced => self.advanced_page(ui, palette, state, busy, writable),
                Page::Macros => self.macros_page(ui, palette, state, busy, writable),
                Page::Performance => self.performance_page(ui, palette, state, busy, writable),
                Page::Lighting => self.lighting_page(ui, palette, state, busy, writable),
                Page::Profiles => self.profiles_page(ui, palette, state, busy, writable),
                Page::Device => self.device_page(ui, palette, state, busy, writable),
            });
    }

    fn save_local(&mut self) {
        let Some((path, value)) = &self.local_edit else {
            return;
        };
        let mut value = value.clone();
        match normalize_functions(&mut value.profile) {
            Ok(()) => {
                self.action = Some(Action::Confirm(
                    tf!(
                        "Сохранить изменения локального профиля «{}»? Предыдущая версия останется в резервной копии.",
                        path.file_stem().unwrap_or_default().to_string_lossy()
                    ),
                    KeyboardCommand::SaveEditedPreset {
                        path: path.clone(),
                        value,
                    },
                ));
            }
            Err(error) => self.message = format!("{error:#}"),
        }
    }

    fn apply_row(
        &mut self,
        ui: &mut egui::Ui,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
        part: Page,
    ) {
        ui.horizontal_wrapped(|ui| {
            let enabled = !busy && self.part_dirty(part) && (self.local_edit.is_some() || writable && self.target(state).is_some());
            if ui.add_enabled(enabled, egui::Button::new(t(if self.local_edit.is_some() { "Сохранить в файл" } else { "Применить" }))).clicked() {
                if self.local_edit.is_some() { self.save_local(); }
                else if let (Some((device_path, profile)), Some(value)) = (self.target(state), self.working()) {
                    let command = match part {
                        Page::Keys => {
                            let mut values = value.layers[self.layer as usize].clone();
                            if let Some(original) = self.original() { for index in [93,94] { values[index] = original.layers[self.layer as usize][index]; } }
                            if self.needs_advanced_commit() {
                                self.advanced_command(device_path, profile, Some(self.layer)).unwrap()
                            } else { KeyboardCommand::ApplyKeys { device_path, profile, layer:self.layer, values } }
                        },
                        Page::Trigger => KeyboardCommand::ApplyTriggers { device_path, profile, values: value.triggers.clone(), tachyon_mode: value.functions.performance.tachyon_mode },
                        Page::Lighting => KeyboardCommand::ApplyLighting { device_path, profile, value: value.functions.lighting.clone() },
                        _ => {
                            let mut performance = value.functions.performance.clone();
                            if let Some(original) = self.original() { performance.tachyon_mode = original.functions.performance.tachyon_mode; }
                            KeyboardCommand::ApplyPerformance { device_path, profile, value: performance }
                        },
                    };
                    let stability_off = part == Page::Performance && self.baseline.as_ref().is_some_and(|baseline| baseline.functions.performance.bottom_rapid_trigger) && !value.functions.performance.bottom_rapid_trigger;
                    self.action = Some(if stability_off { Action::Confirm(t("Отключить защиту от случайных срабатываний? Вибрации стола и небольшое давление на клавиши могут вызвать повторные нажатия.").into(), command) } else { Action::Send(command) });
                }
            }
            if ui.add_enabled(!busy && self.dirty(), egui::Button::new(t("Отменить изменения"))).clicked() {
                if let Some((_, local)) = &mut self.local_edit {
                    if let Some(original) = &self.local_baseline { local.profile = original.clone(); local.macros = self.local_macro_baseline.clone(); }
                } else if let Some(snapshot) = &state.snapshot {
                    self.draft = Some(snapshot.profile.clone()); self.baseline = self.draft.clone();
                    self.macro_draft = Some(snapshot.macros.clone()); self.macro_baseline = self.macro_draft.clone();
                    self.device_path = state.device.as_ref().map(|device| device.path.clone()); self.message.clear();
                }
            }
            if self.dirty() { ui.label(t("Есть несохранённые изменения")); }
        });
        ui.add_space(12.0);
    }

    fn grid_section(&mut self, ui: &mut egui::Ui, palette: Palette, purpose: GridPurpose) {
        self.purpose = purpose;
        self.selected_keys
            .retain(|index| cells(purpose).any(|cell| cell.matrix_index == *index));
        self.selection_controls(ui);
        ui.add_space(8.0);
        self.show_grid(ui, palette, purpose);
        ui.add_space(10.0);
    }

    fn no_data(&self, ui: &mut egui::Ui, palette: Palette) -> bool {
        if self.working().is_none() {
            ui.label(
                RichText::new(t("Настройки появятся после чтения клавиатуры."))
                    .color(palette.muted),
            );
            true
        } else {
            false
        }
    }

    pub fn layer_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Слой"));
            for (index, label) in ["Win: основной", "Win: Fn", "Mac: основной", "Mac: Fn"]
                .into_iter()
                .enumerate()
            {
                ui.selectable_value(&mut self.layer, index as u8, t(label));
            }
        });
    }

    fn selection_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(
                &mut self.multi_select,
                t(if self.purpose == GridPurpose::Lighting {
                    "Выбор нескольких зон"
                } else {
                    "Выбор нескольких клавиш"
                }),
            );
            let selected = if self.purpose == GridPurpose::Lighting {
                &mut self.selected_lights
            } else {
                &mut self.selected_keys
            };
            if ui.button(t("Выбрать все")).clicked() {
                *selected = cells(self.purpose).map(|cell| cell.matrix_index).collect();
            }
            if ui
                .add_enabled(
                    !selected.is_empty(),
                    egui::Button::new(t("Снять выделение")),
                )
                .clicked()
            {
                selected.clear();
            }
            ui.label(tf!("Выбрано: {}", selected.len()));
        });
    }

    pub fn show_grid(&mut self, ui: &mut egui::Ui, palette: Palette, purpose: GridPurpose) {
        let layout = layout();
        let width = ui.available_width().min(layout.canvas.width);
        let scale = width / layout.canvas.width;
        let height = (layout.canvas.height + layout.canvas.top_inset) * scale;
        let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let origin = area.min + Vec2::new(0.0, layout.canvas.top_inset * scale);
        let case =
            egui::Rect::from_min_size(origin, Vec2::new(width, layout.canvas.height * scale));
        ui.painter().rect_filled(case, 10.0, palette.field);
        let colors = (purpose == GridPurpose::Lighting)
            .then(|| {
                self.frame_preview
                    .clone()
                    .or_else(|| self.working().map(|profile| profile.colors.clone()))
            })
            .flatten();
        let selected = if purpose == GridPurpose::Lighting {
            &mut self.selected_lights
        } else {
            &mut self.selected_keys
        };
        let additive =
            self.multi_select || ui.input(|input| input.modifiers.ctrl || input.modifiers.command);
        let edge = if ui.visuals().dark_mode {
            Color32::from_rgb(118, 123, 135)
        } else {
            Color32::from_rgb(116, 123, 138)
        };
        {
            let mut grid =
                ui.new_child(egui::UiBuilder::new().id_salt("ace75-grid").max_rect(area));
            let ui = &mut grid;
            ui.spacing_mut().button_padding = Vec2::new(2.0, 0.0);
            for cell in cells(purpose) {
                let mut rect = egui::Rect::from_min_size(
                    origin + Vec2::new(cell.x, cell.y) * scale,
                    Vec2::new(cell.width, cell.height) * scale,
                );
                if purpose == GridPurpose::Lighting && cell.label == "sideLight" {
                    rect.set_height((12.0 * scale).max(6.0));
                    rect.set_center(egui::pos2(rect.center().x, origin.y + 73.0 * scale));
                }
                if cell.label == "gunlun" {
                    rect.max.x = rect.max.x.max(rect.min.x + 22.0);
                }
                let active = selected.contains(&cell.matrix_index);
                let light_color = if self.calibrating && purpose == GridPurpose::Trigger {
                    self.calibrated
                        .contains(&cell.matrix_index)
                        .then_some(Color32::from_rgb(35, 170, 92))
                } else {
                    colors
                        .as_ref()
                        .and_then(|colors| colors.get(cell.matrix_index as usize))
                        .map(|rgb| Color32::from_rgb(rgb[0], rgb[1], rgb[2]))
                };
                let fill = light_color.unwrap_or(if active {
                    palette.accent
                } else {
                    palette.panel
                });
                let text_color = light_color.map(contrast_text).unwrap_or(if active {
                    Color32::WHITE
                } else {
                    palette.text
                });
                let response = if cell.label == "sideLight" {
                    let response = ui.interact(
                        rect,
                        ui.id().with(("ace75-key", cell.source_index)),
                        egui::Sense::click(),
                    );
                    ui.painter().rect_filled(rect, 2.0, fill);
                    ui.painter().rect_stroke(
                        rect,
                        2.0,
                        Stroke::new(
                            if active { 2.0_f32 } else { 1.0_f32 },
                            if active { palette.accent_text } else { edge },
                        ),
                        egui::StrokeKind::Inside,
                    );
                    response
                } else {
                    ui.push_id(("ace75-key", cell.source_index), |ui| {
                        ui.put(
                            rect,
                            egui::Button::new(
                                RichText::new(cell.legend())
                                    .size((13.0 * scale).max(10.5))
                                    .color(text_color),
                            )
                            .fill(fill)
                            .stroke(Stroke::new(
                                if active { 2.0_f32 } else { 1.0_f32 },
                                if active { palette.accent_text } else { edge },
                            ))
                            .corner_radius(4.0),
                        )
                    })
                    .inner
                };
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        active,
                        cell.description(),
                    )
                });
                if response.clicked() && !self.calibrating {
                    toggle_selection(selected, cell.matrix_index, additive);
                }
                if response.has_focus() {
                    ui.painter().rect_stroke(
                        rect.expand(1.0),
                        4.0,
                        Stroke::new(2.0_f32, palette.accent_text),
                        egui::StrokeKind::Outside,
                    );
                }
                response.on_hover_text(cell.description());
            }
        }
    }
}

impl Ace75Ui {
    fn needs_advanced_commit(&self) -> bool {
        let (Some(value), Some(original)) = (self.working(), self.original()) else {
            return false;
        };
        value.layers[self.layer as usize]
            .iter()
            .zip(&original.layers[self.layer as usize])
            .any(|(key, old)| {
                key != old && ((144..=149).contains(&key.kind) || (144..=149).contains(&old.kind))
            })
    }

    fn advanced_command(
        &self,
        device_path: String,
        profile: u8,
        ordinary_layer: Option<u8>,
    ) -> Option<KeyboardCommand> {
        let (value, original) = self.working().zip(self.original())?;
        let mut layers = original.layers.clone();
        for (layer, keys) in layers.iter_mut().enumerate() {
            for (index, key) in keys.iter_mut().enumerate() {
                let edited = value.layers[layer][index];
                if ![93, 94].contains(&index)
                    && (ordinary_layer == Some(layer as u8)
                        || (144..=149).contains(&key.kind)
                        || (144..=149).contains(&edited.kind))
                {
                    *key = edited;
                }
            }
        }
        let mut triggers = original.triggers.clone();
        for (trigger, edited) in triggers.iter_mut().zip(&value.triggers) {
            trigger.priority = edited.priority;
        }
        Some(KeyboardCommand::ApplyAdvanced {
            device_path,
            profile,
            advanced: value.advanced.clone(),
            layers,
            triggers,
            custom_params: value.custom_params.clone(),
        })
    }

    fn global_changes(&self) -> Vec<(u8, KeyAssignment)> {
        let (Some(value), Some(original)) = (self.working(), self.original()) else {
            return Vec::new();
        };
        [93, 94]
            .into_iter()
            .filter_map(|index| {
                value
                    .layers
                    .iter()
                    .zip(&original.layers)
                    .any(|(left, right)| left[index] != right[index])
                    .then_some((index as u8, value.layers[0][index]))
            })
            .collect()
    }

    fn keys_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        self.apply_row(ui, state, busy, writable, Page::Keys);
        self.layer_selector(ui);
        ui.add_space(10.0);
        self.grid_section(ui, palette, GridPurpose::Assignment);
        if self.no_data(ui, palette) {
            return;
        }
        let mut selected = self.selected_keys.iter().copied().collect::<Vec<_>>();
        if selected.is_empty() {
            ui.label(t("Выберите клавишу на схеме."));
            return;
        }
        let global = selected.contains(&93);
        if global {
            if selected.len() > 1 {
                ui.label(t("Для настройки RT выберите только эту кнопку."));
                return;
            }
            ui.label(t("RT — общая кнопка для всех слоёв."));
            ui.horizontal_wrapped(|ui| {
                ui.selectable_value(&mut self.rt_double, false, t("Одиночное нажатие"));
                ui.selectable_value(&mut self.rt_double, true, t("Двойное нажатие"));
            });
            let hidden = &layout().hidden_actions[0];
            selected = vec![if self.rt_double {
                hidden.matrix_index
            } else {
                hidden.parent_matrix_index
            }];
            let changes = self.global_changes();
            if ui
                .add_enabled(
                    !changes.is_empty()
                        && !busy
                        && (self.local_edit.is_some() || writable && self.target(state).is_some()),
                    egui::Button::new(t("Применить RT ко всем слоям")),
                )
                .clicked()
            {
                if self.local_edit.is_some() {
                    self.save_local();
                } else if let Some((device_path, profile)) = self.target(state) {
                    self.action = Some(Action::Send(KeyboardCommand::ApplyGlobalKeys {
                        device_path,
                        profile,
                        indices: changes,
                    }));
                }
            }
        }
        if let Some(value) = self
            .working()
            .and_then(|profile| profile.layers.get(self.layer as usize))
            .and_then(|keys| keys.get(selected[0] as usize))
        {
            ui.label(tf!("Текущее назначение: {}", key_label(*value, self.layer)));
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Поиск"));
            ui.text_edit_singleline(&mut self.action_search);
            if ui.button(t("Восстановить выбранные клавиши")).clicked()
            {
                let layer = self.layer;
                if let Some(profile) = self.working_mut() {
                    if !global {
                        remove_advanced_assignments(
                            profile,
                            layer,
                            &selected
                                .iter()
                                .map(|index| *index as u8)
                                .collect::<Vec<_>>(),
                        );
                    }
                    for index in &selected {
                        if let Some(value) = default_assignment(*index, profile.index, layer) {
                            if global {
                                for keys in &mut profile.layers {
                                    keys[*index as usize] = value;
                                }
                            } else {
                                profile.layers[layer as usize][*index as usize] = value;
                            }
                        }
                    }
                }
            }
        });
        ui.add_space(8.0);
        let mut chosen = self.action_picker(ui, None);
        egui::CollapsingHeader::new(t("Сочетание клавиш")).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (bit, label) in [
                    (0, "Левый Ctrl"),
                    (1, "Левый Shift"),
                    (2, "Левый Alt"),
                    (3, "Левый Win"),
                    (4, "Правый Ctrl"),
                    (5, "Правый Shift"),
                    (6, "Правый Alt"),
                    (7, "Правый Win"),
                ] {
                    let mut selected = self.chord_modifiers & (1 << bit) != 0;
                    if ui.checkbox(&mut selected, t(label)).changed() {
                        if selected {
                            self.chord_modifiers |= 1 << bit;
                        } else {
                            self.chord_modifiers &= !(1 << bit);
                        }
                    }
                }
            });
            macro_code_picker(
                ui,
                "ace75-chord-key",
                &mut self.chord_usage,
                false,
                self.layer,
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Без обычной клавиши")).clicked() {
                    self.chord_usage = 0;
                }
                if ui
                    .add_enabled(
                        self.chord_modifiers != 0 && self.chord_usage < 224,
                        egui::Button::new(t("Назначить сочетание")),
                    )
                    .clicked()
                {
                    chosen = Some(KeyAssignment {
                        kind: 16,
                        code1: self.chord_modifiers,
                        code2: self.chord_usage as u8,
                    });
                }
            });
        });
        egui::CollapsingHeader::new(t("Назначить макрос")).show(ui, |ui| {
            let bank = if let Some((_, value)) = &self.local_edit {
                value.macros.as_ref()
            } else {
                self.macro_draft.as_ref()
            };
            if let Some(bank) = bank {
                ui.horizontal_wrapped(|ui| {
                    for entry in &bank.macros {
                        let label = if entry.name.is_empty() {
                            tf!("Макрос {}", entry.macro_index + 1)
                        } else {
                            entry.name.clone()
                        };
                        if ui.button(label).clicked() {
                            chosen = Some(KeyAssignment {
                                kind: 112,
                                code1: entry.macro_index,
                                code2: entry.mode,
                            });
                        }
                    }
                });
            }
        });
        if let Some(value) = chosen {
            let layer = self.layer;
            if let Some(profile) = self.working_mut() {
                if !global {
                    remove_advanced_assignments(
                        profile,
                        layer,
                        &selected
                            .iter()
                            .map(|index| *index as u8)
                            .collect::<Vec<_>>(),
                    );
                }
                for index in selected {
                    if global {
                        for keys in &mut profile.layers {
                            keys[index as usize] = value;
                        }
                    } else {
                        profile.layers[layer as usize][index as usize] = value;
                    }
                }
            }
        }
    }

    fn action_picker(
        &mut self,
        ui: &mut egui::Ui,
        advanced_type: Option<&str>,
    ) -> Option<KeyAssignment> {
        if self.action_category.is_empty() {
            self.action_category = "basic".into();
        }
        ui.horizontal_wrapped(|ui| {
            for (id, label) in [
                ("basic", "Клавиатура"),
                ("extra", "Система и сочетания"),
                ("mouse", "Мышь"),
                ("media", "Мультимедиа"),
                ("light", "Подсветка"),
            ] {
                ui.selectable_value(&mut self.action_category, id.into(), t(label));
            }
        });
        let query = self.action_search.to_lowercase();
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            for action in key_actions().iter().filter(|action| {
                action.layers.contains(&self.layer)
                    && (query.is_empty() && action.category == self.action_category
                        || !query.is_empty() && action.label().to_lowercase().contains(&query))
            }) {
                let allowed = advanced_type.is_none_or(|kind| {
                    !action
                        .disabled_advanced_types
                        .iter()
                        .any(|value| value == kind)
                });
                if ui
                    .push_id(&action.id, |ui| {
                        ui.add_enabled(allowed, egui::Button::new(action.label()))
                    })
                    .inner
                    .clicked()
                {
                    chosen = Some(action.assignment());
                }
            }
        });
        chosen
    }

    fn trigger_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        self.apply_row(ui, state, busy, writable, Page::Trigger);
        self.grid_section(ui, palette, GridPurpose::Trigger);
        if self.no_data(ui, palette) {
            return;
        }
        let mut tachyon = self.working().unwrap().functions.performance.tachyon_mode;
        if ui
            .checkbox(&mut tachyon, t("Режим высокой точности RT (Tachyon 2.0)"))
            .changed()
        {
            let selected = self.selected_keys.clone();
            if let Some(profile) = self.working_mut() {
                profile.functions.performance.tachyon_mode = tachyon;
                if !tachyon {
                    for index in selected {
                        let trigger = &mut profile.triggers[index as usize];
                        let max = if trigger.switch_type == 3 { 350 } else { 340 };
                        trigger.rt_press = ((f64::from(trigger.rt_press)
                            / precision_base(trigger.press_precision)
                            * 100.0)
                            .floor() as u16)
                            .clamp(2, max);
                        trigger.rt_release = ((f64::from(trigger.rt_release)
                            / precision_base(trigger.release_precision)
                            * 100.0)
                            .floor() as u16)
                            .clamp(2, max);
                        trigger.press_precision = 0;
                        trigger.release_precision = 0;
                    }
                }
            }
        }
        ui.label(RichText::new(t("Высокая точность позволяет выбрать шаг RT 0,005 или 0,001 мм. Отключение возвращает выбранные клавиши к шагу 0,01 мм.")).small().color(palette.muted));
        let Some(index) = self.selected_keys.first().copied() else {
            ui.label(t("Выберите клавишу на схеме."));
            return;
        };
        let Some(mut value) = self
            .working()
            .and_then(|profile| profile.triggers.get(index as usize))
            .cloned()
        else {
            return;
        };
        let before = value.clone();
        let firmware = firmware_number(state);
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Магнитный переключатель"));
            egui::ComboBox::from_id_salt("ace75-switch")
                .selected_text(switch_name(value.switch_type))
                .show_ui(ui, |ui| {
                    for id in 0..=if firmware >= 117 { 5 } else { 4 } {
                        ui.selectable_value(&mut value.switch_type, id, switch_name(id));
                    }
                });
        });
        let max_actuation = if value.switch_type == 3 { 350 } else { 340 };
        mm_slider(
            ui,
            &mut value.actuation,
            100.0,
            10..=max_actuation,
            t("Точка срабатывания"),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label("Rapid Trigger");
            for (mode, label) in [(0, "Отключён"), (1, "Включён"), (2, "Непрерывный")]
            {
                ui.selectable_value(&mut value.key_mode, mode, t(label));
            }
        });
        if value.key_mode != 0 {
            let tachyon = self
                .working()
                .is_some_and(|profile| profile.functions.performance.tachyon_mode);
            rt_control(
                ui,
                "ace75-rt-press",
                t("RT: нажатие"),
                &mut value.press_precision,
                &mut value.rt_press,
                max_actuation,
                tachyon,
            );
            rt_control(
                ui,
                "ace75-rt-release",
                t("RT: отпускание"),
                &mut value.release_precision,
                &mut value.rt_release,
                max_actuation,
                tachyon,
            );
        }
        egui::CollapsingHeader::new(t("Мёртвые зоны")).show(ui, |ui| {
            deadzone_control(ui, &mut value.press_deadzone, t("Верхняя мёртвая зона"));
            deadzone_control(ui, &mut value.release_deadzone, t("Нижняя мёртвая зона"));
        });
        if value != before {
            let selected = self.selected_keys.clone();
            if let Some(profile) = self.working_mut() {
                for index in selected {
                    let target = &mut profile.triggers[index as usize];
                    merge_trigger(target, &before, &value);
                    if before.switch_type != value.switch_type {
                        let travel = if target.switch_type == 3 { 350 } else { 340 };
                        target.actuation = target.actuation.min(travel);
                        if target.press_precision == 0 {
                            target.rt_press = target.rt_press.min(travel);
                        }
                        if target.release_precision == 0 {
                            target.rt_release = target.rt_release.min(travel);
                        }
                    }
                }
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new(t("Изменяются только выбранные клавиши. Остальные значения каждой клавиши сохраняются.")).small().color(palette.muted));
    }

    fn performance_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        self.apply_row(ui, state, busy, writable, Page::Performance);
        if self.no_data(ui, palette) {
            return;
        }
        let mut value = self.working().unwrap().functions.performance.clone();
        let before = value.clone();
        ui.label(RichText::new(t("Частота опроса")).strong());
        ui.horizontal_wrapped(|ui| {
            for (code, label) in [
                (4, "1000 Hz"),
                (3, "2000 Hz"),
                (2, "4000 Hz"),
                (1, "8000 Hz"),
            ] {
                ui.selectable_value(&mut value.report_rate, code, label);
            }
            if ui
                .add_enabled(
                    !busy && writable && self.local_edit.is_none() && self.target(state).is_some(),
                    egui::Button::new(t("Применить частоту")),
                )
                .clicked()
            {
                if let Some((device_path, profile)) = self.target(state) {
                    self.action = Some(Action::Send(KeyboardCommand::ApplyReportRate {
                        device_path,
                        profile,
                        report_rate: value.report_rate,
                    }));
                }
            }
        });
        ui.add_space(16.0);
        ui.checkbox(
            &mut value.bottom_rapid_trigger,
            t("Защита от случайных срабатываний"),
        )
        .on_hover_text(t(
            "Снижает ложные нажатия из-за вибраций стола и небольшого давления на клавиши.",
        ));
        let mut debounce = value.debounce_level > 0;
        if ui
            .checkbox(&mut debounce, t("Оптимизация дребезга клавиш"))
            .changed()
        {
            value.debounce_level = if debounce { 7 } else { 0 };
        }
        let mut mac = value.mac_mode % 4 >= 2;
        if ui.checkbox(&mut mac, t("Режим Mac")).changed() {
            value.mac_mode = self.working().unwrap().index * 4 + if mac { 2 } else { 0 };
            if mac {
                value.lock_win = false;
            }
        }
        ui.add_enabled(
            !mac,
            egui::Checkbox::new(&mut value.lock_win, t("Блокировать Win")),
        );
        ui.checkbox(
            &mut value.travel_check,
            t("Адаптивная динамическая калибровка (бета)"),
        );
        ui.add_space(16.0);
        let mut smart = value.rt_smart > 0;
        if ui.checkbox(&mut smart, "RT Smart").changed() {
            value.rt_smart = if smart { 8 } else { 0 };
        }
        if smart {
            ui.add(
                egui::Slider::new(&mut value.rt_smart, 1..=15)
                    .text(t("Стабильность"))
                    .clamping(egui::SliderClamping::Edits),
            );
        }
        ui.add_space(16.0);
        ui.label(RichText::new(t("Сон подсветки")).strong());
        let mut never = value.sleep_mode == 1;
        if ui
            .checkbox(&mut never, t("Не выключать автоматически"))
            .changed()
        {
            value.sleep_mode = u8::from(never);
        }
        let mut minutes = f32::from(value.sleep_time) / 2.0;
        if ui
            .add_enabled(
                !never,
                egui::Slider::new(&mut minutes, 1.0..=30.0)
                    .step_by(1.0)
                    .suffix(t(" мин"))
                    .clamping(egui::SliderClamping::Edits),
            )
            .changed()
        {
            value.sleep_time = (minutes * 2.0) as u8;
        }
        if value != before {
            self.working_mut().unwrap().functions.performance = value;
        }
        ui.add_space(12.0);
        ui.label(RichText::new(t("Частота опроса применяется отдельной кнопкой; остальные параметры — кнопкой «Применить».")).small().color(palette.muted));
    }

    fn lighting_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        self.apply_row(ui, state, busy, writable, Page::Lighting);
        ui.horizontal_wrapped(|ui| {
            for (id, label) in [(0, "Основная"), (1, "Полоса"), (2, "Логотип")]
            {
                ui.selectable_value(&mut self.light_scope, id, t(label));
            }
        });
        ui.add_space(12.0);
        if self.no_data(ui, palette) {
            self.grid_section(ui, palette, GridPurpose::Lighting);
            ui.add_space(18.0);
            self.animation_editor(ui, palette, state, busy, writable);
            return;
        }
        let mut lighting = self.working().unwrap().functions.lighting.clone();
        let before = lighting.clone();
        let zone = match self.light_scope {
            1 => &mut lighting.side,
            2 => &mut lighting.side2,
            _ => &mut lighting.main,
        };
        let effects = effect_options(self.light_scope, firmware_number(state));
        let current = effects
            .iter()
            .find(|effect| effect.value == zone.effect)
            .copied();
        egui::ComboBox::from_id_salt("ace75-effect")
            .width(ui.available_width().min(300.0))
            .selected_text(
                current
                    .map(LightEffect::label)
                    .unwrap_or_else(|| t("Неизвестный эффект")),
            )
            .show_ui(ui, |ui| {
                for effect in &effects {
                    ui.selectable_value(&mut zone.effect, effect.value, effect.label());
                }
            });
        ui.add_space(10.0);
        let current = effects
            .iter()
            .find(|effect| effect.value == zone.effect)
            .copied();
        if let Some(effect) = current {
            if effect.brightness {
                ui.add(
                    egui::Slider::new(&mut zone.brightness, 0..=100)
                        .text(t("Яркость"))
                        .suffix("%")
                        .clamping(egui::SliderClamping::Edits),
                );
            }
            if effect.speed {
                ui.add(
                    egui::Slider::new(&mut zone.speed, 0..=4)
                        .text(t("Скорость"))
                        .clamping(egui::SliderClamping::Edits),
                );
            }
            if effect.direction {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Направление"));
                    for (id, label) in effect.direction_labels().into_iter().enumerate() {
                        ui.selectable_value(&mut zone.direction, id as u8, t(label));
                    }
                });
            }
            if effect.color {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Цвет"));
                    ui.color_edit_button_srgb(&mut zone.color);
                    ui.checkbox(&mut zone.rainbow, t("Радуга"));
                });
            }
        }
        if lighting != before {
            lighting.scope = match self.light_scope {
                1 => LightScope::Side,
                2 => LightScope::Side2,
                _ => LightScope::Main,
            };
            self.working_mut().unwrap().functions.lighting = lighting;
        }
        ui.add_space(18.0);
        self.grid_section(ui, palette, GridPurpose::Lighting);
        self.colors_editor(ui, state, busy, writable);
        egui::CollapsingHeader::new(t("Цвет калибровки")).show(ui, |ui| {
            if let Some(profile) = self.working_mut() {
                ui.color_edit_button_srgb(&mut profile.functions.lighting.calibration_color);
            }
        });
        ui.add_space(18.0);
        self.music_controls(ui, palette, state, busy, writable);
        ui.add_space(14.0);
        egui::CollapsingHeader::new(t("Статические и анимированные темы")).show(ui, |ui| {
            self.animation_editor(ui, palette, state, busy, writable)
        });
    }

    fn profiles_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.label(
            RichText::new(t("Локальная библиотека ACE75"))
                .size(20.0)
                .strong(),
        );
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Имя"));
            ui.add(
                egui::TextEdit::singleline(&mut self.preset_name)
                    .desired_width(ui.available_width().min(240.0)),
            );
            if ui
                .add_enabled(
                    !busy
                        && state.snapshot.is_some()
                        && self.local_edit.is_none()
                        && !self.preset_name.trim().is_empty(),
                    egui::Button::new(t("Сохранить текущий профиль")),
                )
                .clicked()
            {
                self.action = Some(Action::Send(KeyboardCommand::SavePreset(
                    self.preset_name.trim().into(),
                )));
            }
        });
        ui.add_space(12.0);
        for preset in &state.presets {
            ui.horizontal_wrapped(|ui| {
                ui.label(&preset.name);
                if ui
                    .add_enabled(
                        !busy && self.local_edit.is_none(),
                        egui::Button::new(t("Просмотр")),
                    )
                    .clicked()
                {
                    self.preview_pending = Some(preset.path.clone());
                    self.action = Some(Action::Send(KeyboardCommand::PreviewPreset(
                        preset.path.clone(),
                    )));
                }
                if ui
                    .add_enabled(
                        !busy && !self.preset_name.trim().is_empty(),
                        egui::Button::new(t("Переименовать")),
                    )
                    .clicked()
                {
                    self.action = Some(Action::Confirm(
                        tf!(
                            "Переименовать профиль «{}» в «{}»?",
                            preset.name,
                            self.preset_name.trim()
                        ),
                        KeyboardCommand::RenamePreset {
                            path: preset.path.clone(),
                            name: self.preset_name.trim().into(),
                        },
                    ));
                }
                if ui
                    .add_enabled(!busy, egui::Button::new(t("Удалить")))
                    .clicked()
                {
                    self.action = Some(Action::Confirm(
                        tf!(
                            "Удалить локальный профиль «{}»? Сохранится резервная копия.",
                            preset.name
                        ),
                        KeyboardCommand::DeletePreset(preset.path.clone()),
                    ));
                }
            });
        }
        if let (Some(path), Some(preview)) = (&self.preview_path, &state.preview) {
            ui.add_space(16.0);
            ui.separator();
            ui.label(tf!(
                "Просмотр: {}",
                path.file_stem().unwrap_or_default().to_string_lossy()
            ));
            ui.label(tf!(
                "Слоёв: {} · Клавиш: {}",
                preview.profile.layers.len(),
                preview.profile.layers.first().map_or(0, Vec::len)
            ));
            let path = path.clone();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy && self.local_edit.is_none(), egui::Button::new(t("Редактировать локально"))).clicked() {
                    self.local_baseline = Some(preview.profile.clone()); self.local_macro_baseline = preview.macros.clone(); self.local_edit = Some((path.clone(), preview.clone())); self.page = Page::Keys;
                }
                if ui.add_enabled(writable && !busy && self.local_edit.is_none() && !self.hardware_dirty() && self.target(state).is_some(), egui::Button::new(t("Применить к устройству"))).clicked() {
                    if let Some((device_path, profile)) = self.target(state) { self.action = Some(Action::Confirm(tf!("Заменить настройки профиля {} содержимым «{}»? Будет создана резервная копия.", profile + 1, path.file_stem().unwrap_or_default().to_string_lossy()), KeyboardCommand::ApplyPreset { device_path, profile, path: path.clone() })); }
                }
                if ui.add_enabled(!busy && self.local_edit.is_none(), egui::Button::new(t("Закрыть просмотр"))).clicked() { self.preview_path = None; self.action = Some(Action::Send(KeyboardCommand::ClosePreview)); }
            });
        }
        ui.add_space(20.0);
        ui.label(t("Путь к JSON-файлу"));
        ui.add(
            egui::TextEdit::singleline(&mut self.preset_path).desired_width(ui.available_width()),
        );
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && !self.preset_path.trim().is_empty(),
                    egui::Button::new(t("Импорт в библиотеку")),
                )
                .clicked()
            {
                self.action = Some(Action::Send(KeyboardCommand::ImportPreset(PathBuf::from(
                    self.preset_path.trim(),
                ))));
            }
            if ui
                .add_enabled(
                    !busy
                        && !self.preset_path.trim().is_empty()
                        && (state.snapshot.is_some() || state.preview.is_some()),
                    egui::Button::new(t("Экспорт")),
                )
                .clicked()
            {
                self.action = Some(Action::Send(KeyboardCommand::ExportProfile(PathBuf::from(
                    self.preset_path.trim(),
                ))));
            }
            if ui
                .add_enabled(
                    !busy
                        && !self.preset_path.trim().is_empty()
                        && (state.snapshot.is_some() || state.preview.is_some()),
                    egui::Button::new(t("Экспорт для MCHOSE Hub")),
                )
                .clicked()
            {
                self.action = Some(Action::Send(KeyboardCommand::ExportHubProfile(
                    PathBuf::from(self.preset_path.trim()),
                )));
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new(t("Импорт и локальное редактирование не меняют клавиатуру. Для записи используйте «Применить к устройству».")).small().color(palette.muted));
    }

    fn device_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.heading("MCHOSE ACE75 8K");
        ui.add_space(16.0);
        if let Some(device) = &state.device {
            ui.label(tf!("Устройство: {}", device.name));
            ui.label(tf!("Путь: {}", device.path));
        }
        if let Some(snapshot) = &state.snapshot {
            ui.label(tf!("Прошивка: {}", snapshot.info.firmware_version));
            ui.label(tf!("Активный профиль: {}", snapshot.profile.index + 1));
            ui.label(tf!("Аппаратных профилей: {}", snapshot.base.profile_count));
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Имя активного профиля"));
                ui.text_edit_singleline(&mut self.onboard_name);
                if ui
                    .add_enabled(
                        writable
                            && !busy
                            && self.local_edit.is_none()
                            && !self.onboard_name.trim().is_empty()
                            && self.onboard_name_baseline.as_deref()
                                != Some(self.onboard_name.trim()),
                        egui::Button::new(t("Переименовать")),
                    )
                    .clicked()
                {
                    if let Some((device_path, profile)) = self.target(state) {
                        self.action = Some(Action::Confirm(
                            tf!(
                                "Переименовать аппаратный профиль {} в «{}»?",
                                profile + 1,
                                self.onboard_name.trim()
                            ),
                            KeyboardCommand::RenameOnboardProfile {
                                device_path,
                                profile,
                                name: self.onboard_name.trim().into(),
                            },
                        ));
                    }
                }
            });
            ui.label(tf!(
                "Частота опроса: {} Hz",
                match snapshot.profile.functions.performance.report_rate {
                    1 => 8000,
                    2 => 4000,
                    3 => 2000,
                    4 => 1000,
                    _ => 0,
                }
            ));
        } else {
            ui.label(RichText::new(t("Подключите ACE75 по USB.")).color(palette.muted));
        }
        self.calibration_controls(ui, palette, state, busy, writable);
        if ui
            .add_enabled(
                !busy && state.snapshot.is_some(),
                egui::Button::new(t("Создать полную резервную копию ACE75")),
            )
            .clicked()
        {
            self.action = Some(Action::Send(KeyboardCommand::Backup));
        }
        ui.add_space(20.0);
        self.profile_layout_controls(ui, state, busy, writable);
        ui.add_space(20.0);
        self.firmware_controls(ui, palette, state, busy, writable);
        ui.add_space(20.0);
        ui.label(RichText::new(t("Заводские настройки")).strong());
        ui.horizontal_wrapped(|ui| {
            if let Some((device_path, profile)) = self.target(state) {
                let enabled = writable && !busy && self.local_edit.is_none();
                if ui.add_enabled(enabled, egui::Button::new(t("Сбросить текущий профиль"))).clicked() {
                    self.action = Some(Action::Confirm(tf!("Сбросить аппаратный профиль {}? Настройки будут сохранены в резервной копии.", profile + 1), KeyboardCommand::Reset { device_path: device_path.clone(), profile: Some(profile) }));
                }
                if ui.add_enabled(enabled, egui::Button::new(t("Сбросить все профили"))).clicked() {
                    self.action = Some(Action::Confirm(t("Сбросить все аппаратные профили ACE75? Настройки будут сохранены в резервной копии.").into(), KeyboardCommand::Reset { device_path, profile: None }));
                }
            }
        });
    }
}

impl Ace75Ui {
    fn profile_layout_controls(
        &mut self,
        ui: &mut egui::Ui,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        let Some(snapshot) = &state.snapshot else {
            return;
        };
        let Some((mut count, mut order)) = self.onboard_layout.clone() else {
            return;
        };
        ui.label(RichText::new(t("Порядок аппаратных профилей")).strong());
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Количество"));
            for value in 1..=snapshot.base.max_profiles.min(3) {
                ui.selectable_value(&mut count, value, value.to_string());
            }
        });
        let mut move_profile = None;
        for (position, profile) in order.iter().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let name = onboard_profile_name(
                    snapshot
                        .profile_names
                        .names
                        .get(*profile as usize)
                        .map_or("", String::as_str),
                    *profile,
                );
                ui.label(format!("{}. {}", position + 1, name));
                if position >= count as usize {
                    ui.label(t("Не участвует в переключении"));
                }
                if ui
                    .add_enabled(
                        snapshot.base.supports_order && position > 0,
                        egui::Button::new(t("Выше")),
                    )
                    .clicked()
                {
                    move_profile = Some((position, position - 1));
                }
                if ui
                    .add_enabled(
                        snapshot.base.supports_order && position + 1 < order.len(),
                        egui::Button::new(t("Ниже")),
                    )
                    .clicked()
                {
                    move_profile = Some((position, position + 1));
                }
            });
        }
        if let Some((left, right)) = move_profile {
            order.swap(left, right);
        }
        self.onboard_layout = Some((count, order.clone()));
        ui.horizontal_wrapped(|ui| {
            let changed = self.onboard_layout != self.onboard_layout_baseline;
            if ui.add_enabled(changed && writable && !busy && self.local_edit.is_none(), egui::Button::new(t("Применить порядок профилей"))).clicked() {
                if let Some((device_path, profile)) = self.target(state) { self.action = Some(Action::Confirm(t("Изменить число и порядок доступных профилей? Будет создана резервная копия.").into(), KeyboardCommand::ApplyProfileLayout { device_path, profile, profile_count: count, profile_order: order })); }
            }
            if ui.add_enabled(changed, egui::Button::new(t("Отменить порядок"))).clicked() { self.onboard_layout = self.onboard_layout_baseline.clone(); }
        });
    }

    fn firmware_controls(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.label(RichText::new(t("Прошивка ACE75 8K")).strong());
        if state.bootloader.is_some() {
            ui.label(
                RichText::new(t(
                    "Клавиатура в режиме обновления. Выберите официальный BIN для Ace75 8K.",
                ))
                .color(palette.accent_text),
            );
        }
        if ui
            .add_enabled(
                !busy,
                egui::Button::new(t("Проверить официальную прошивку")),
            )
            .clicked()
        {
            self.action = Some(Action::Send(KeyboardCommand::CheckFirmware));
        }
        if let Some(firmware) = &state.firmware {
            ui.label(tf!("Последняя официальная версия: {}", firmware.version));
            ui.hyperlink_to(t("Файл на CDN MCHOSE"), &firmware.url);
            if let Some(bytes) = firmware.bytes {
                ui.label(tf!("Размер: {} байт", bytes));
            }
            if let Some(hash) = &firmware.sha256 {
                ui.add(egui::Label::new(format!("SHA-256: {hash}")).wrap());
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.firmware_download_directory)
                .desired_width(ui.available_width())
                .hint_text(t("Каталог для скачивания прошивки")),
        );
        if ui
            .add_enabled(
                !busy
                    && state.firmware.is_some()
                    && !self.firmware_download_directory.trim().is_empty(),
                egui::Button::new(t("Скачать официальный BIN")),
            )
            .clicked()
        {
            self.action = Some(Action::Send(KeyboardCommand::DownloadFirmware(
                PathBuf::from(self.firmware_download_directory.trim()),
            )));
        }
        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.firmware_path)
                .desired_width(ui.available_width())
                .hint_text(t("Путь к файлу Ace75_8K.bin")),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(writable && !busy && !self.firmware_path.trim().is_empty() && (state.device.is_some() || state.bootloader.is_some()), egui::Button::new(t("Подготовить обновление"))).clicked() {
                match super::firmware_summary(self.firmware_path.trim()) {
                    Ok(summary) => self.action = Some(Action::Confirm(tf!("Обновить прошивку ACE75 8K?\n\n{}\n\nНе отключайте USB до завершения. Образ будет проверен по официальной версии для этой модели.", summary), KeyboardCommand::FlashFirmware { path: PathBuf::from(self.firmware_path.trim()) })),
                    Err(error) => self.message = error,
                }
            }
        });
    }

    fn music_controls(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.label(RichText::new(t("Музыкальная подсветка")).strong());
        let selected = self.music_scopes.get_or_insert([true; 3]);
        ui.horizontal_wrapped(|ui| {
            for (index, label) in ["Основная", "Полоса", "Логотип"].iter().enumerate()
            {
                ui.add_enabled(
                    !state.music_active,
                    egui::Checkbox::new(&mut selected[index], t(label)),
                );
            }
        });
        let scopes = [LightScope::Main, LightScope::Side, LightScope::Side2]
            .into_iter()
            .zip(*selected)
            .filter_map(|(scope, enabled)| enabled.then_some(scope))
            .collect::<Vec<_>>();
        if ui
            .add_enabled(
                !busy
                    && (state.music_active
                        || writable
                            && self.local_edit.is_none()
                            && !self.hardware_dirty()
                            && !scopes.is_empty()
                            && self.target(state).is_some()),
                egui::Button::new(t(if state.music_active {
                    "Остановить музыку"
                } else {
                    "Включить музыку"
                })),
            )
            .clicked()
        {
            if let Some((device_path, profile)) = self.target(state) {
                self.action = Some(Action::Send(KeyboardCommand::SetMusic {
                    device_path,
                    profile,
                    enabled: !state.music_active,
                    scopes,
                }));
            }
        }
        ui.label(RichText::new(t("Реагирует на системный звук, пока программа открыта. После остановки возвращается прежний эффект.")).small().color(palette.muted));
    }

    fn animation_editor(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.label(t("Загрузите GIF, PNG, JPEG, WebP или JSON темы. Анимация воспроизводится программой через USB."));
        if ui
            .add_enabled(
                !busy && state.animation_active,
                egui::Button::new(t("Остановить тему")),
            )
            .clicked()
        {
            self.action = Some(Action::Send(KeyboardCommand::StopAnimation));
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.animation_path)
                .desired_width(ui.available_width())
                .hint_text(t("Путь к теме или изображению")),
        );
        let dirty = self.animation_draft != self.animation_source;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && !self.animation_path.trim().is_empty(),
                    egui::Button::new(t("Загрузить тему")),
                )
                .clicked()
            {
                let command =
                    KeyboardCommand::LoadAnimation(PathBuf::from(self.animation_path.trim()));
                self.action = Some(if dirty {
                    Action::Confirm(
                        t("Загрузить другую тему и отменить несохранённые изменения анимации?")
                            .into(),
                        command,
                    )
                } else {
                    Action::Send(command)
                });
            }
            if ui
                .add_enabled(
                    !busy && !dirty,
                    egui::Button::new(t("Новая тема из текущих цветов")),
                )
                .clicked()
            {
                let colors = self
                    .working()
                    .map_or_else(|| vec![[0, 0, 0]; 128], |profile| profile.colors.clone());
                self.animation_draft = Some(LightingAnimation {
                    schema_version: 1,
                    model: crate::ace75::MODEL.into(),
                    name: String::new(),
                    scopes: vec![LightScope::Main, LightScope::Side, LightScope::Side2],
                    frames: vec![LightingFrame {
                        duration_ms: 100,
                        colors,
                    }],
                });
                self.animation_frame = 0;
            }
            if ui
                .add_enabled(
                    !busy && dirty,
                    egui::Button::new(t("Отменить изменения темы")),
                )
                .clicked()
            {
                self.animation_draft = self.animation_source.clone();
                self.animation_frame = 0;
            }
        });
        let Some(mut animation) = self.animation_draft.take() else {
            return;
        };
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Имя темы"));
            ui.text_edit_singleline(&mut animation.name);
        });
        ui.horizontal_wrapped(|ui| {
            for (scope, label) in [
                (LightScope::Main, "Основная"),
                (LightScope::Side, "Полоса"),
                (LightScope::Side2, "Логотип"),
            ] {
                let mut selected = animation.scopes.contains(&scope);
                if ui.checkbox(&mut selected, t(label)).changed() {
                    if selected {
                        animation.scopes.push(scope);
                    } else {
                        animation.scopes.retain(|value| *value != scope);
                    }
                }
            }
        });
        self.animation_frame = self
            .animation_frame
            .min(animation.frames.len().saturating_sub(1));
        ui.horizontal_wrapped(|ui| {
            let current = self.animation_frame;
            egui::ComboBox::from_id_salt("ace75-animation-frame")
                .selected_text(tf!("Кадр {} из {}", current + 1, animation.frames.len()))
                .show_ui(ui, |ui| {
                    for index in 0..animation.frames.len() {
                        ui.selectable_value(
                            &mut self.animation_frame,
                            index,
                            tf!("Кадр {}", index + 1),
                        );
                    }
                });
            if ui
                .add_enabled(
                    !animation.frames.is_empty() && animation.frames.len() < 512,
                    egui::Button::new(t("Дублировать кадр")),
                )
                .clicked()
            {
                animation.frames.insert(
                    self.animation_frame + 1,
                    animation.frames[self.animation_frame].clone(),
                );
                self.animation_frame += 1;
            }
            if ui
                .add_enabled(
                    animation.frames.len() > 1,
                    egui::Button::new(t("Удалить кадр")),
                )
                .clicked()
            {
                animation.frames.remove(self.animation_frame);
                self.animation_frame = self.animation_frame.min(animation.frames.len() - 1);
            }
            if ui
                .add_enabled(
                    self.animation_frame > 0,
                    egui::Button::new(t("Кадр раньше")),
                )
                .clicked()
            {
                animation
                    .frames
                    .swap(self.animation_frame, self.animation_frame - 1);
                self.animation_frame -= 1;
            }
            if ui
                .add_enabled(
                    self.animation_frame + 1 < animation.frames.len(),
                    egui::Button::new(t("Кадр позже")),
                )
                .clicked()
            {
                animation
                    .frames
                    .swap(self.animation_frame, self.animation_frame + 1);
                self.animation_frame += 1;
            }
        });
        if let Some(frame) = animation.frames.get_mut(self.animation_frame) {
            ui.add(
                egui::Slider::new(&mut frame.duration_ms, 5..=60_000)
                    .logarithmic(true)
                    .suffix(t(" мс"))
                    .text(t("Длительность кадра"))
                    .clamping(egui::SliderClamping::Edits),
            );
            self.frame_preview = Some(frame.colors.clone());
            self.grid_section(ui, palette, GridPurpose::Lighting);
            self.frame_preview = None;
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Цвет выбранных зон"));
                ui.color_edit_button_srgb(&mut self.paint_color);
                if ui
                    .add_enabled(
                        !self.selected_lights.is_empty(),
                        egui::Button::new(t("Окрасить кадр")),
                    )
                    .clicked()
                {
                    for index in &self.selected_lights {
                        if let Some(color) = frame.colors.get_mut(*index as usize) {
                            *color = self.paint_color;
                        }
                    }
                }
                if ui.button(t("Взять цвета из профиля")).clicked() {
                    if let Some(profile) = self.working() {
                        frame.colors = profile.colors.clone();
                    }
                }
            });
        }
        let validation = animation.validate();
        if let Err(error) = &validation {
            ui.label(RichText::new(i18n::message(&format!("{error:#}"))).color(palette.error_text));
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !busy && validation.is_ok() && !self.animation_path.trim().is_empty(),
                    egui::Button::new(t("Сохранить тему JSON")),
                )
                .clicked()
            {
                self.action = Some(Action::Send(KeyboardCommand::SaveAnimation {
                    path: PathBuf::from(self.animation_path.trim()),
                    value: animation.clone(),
                }));
            }
            if ui
                .add_enabled(
                    !busy
                        && writable
                        && self.local_edit.is_none()
                        && !self.hardware_dirty()
                        && validation.is_ok()
                        && self.target(state).is_some(),
                    egui::Button::new(t("Воспроизвести тему")),
                )
                .clicked()
            {
                if let Some((device_path, profile)) = self.target(state) {
                    self.action = Some(Action::Send(KeyboardCommand::PlayAnimation {
                        device_path,
                        profile,
                        value: animation.clone(),
                    }));
                }
            }
        });
        ui.label(RichText::new(t("Предпросмотр не изменяет клавиатуру. Воспроизведение требует открытой программы; остановка возвращает прежнюю подсветку.")).small().color(palette.muted));
        self.animation_draft = Some(animation);
    }
}

impl Ace75Ui {
    fn macros_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        if self.macro_working().is_none() {
            self.new_local_macro();
        }
        self.macro_library(ui, state, busy);
        let Some(bank) = self.macro_working() else {
            return;
        };
        let bank_size = bank.raw.len();
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Макрос"));
            ui.add_enabled_ui(!self.is_recording() && !self.macro_local_mode, |ui| { egui::ComboBox::from_id_salt("ace75-macro-slot").selected_text(format!("{}", self.macro_slot + 1)).show_ui(ui, |ui| { for index in 0..16 { ui.selectable_value(&mut self.macro_slot, index, format!("{}", index + 1)); } }); });
            if ui.add_enabled(!self.is_recording() && !busy && self.macros_dirty() && (self.macro_local_mode || self.local_edit.is_some() || writable && self.target(state).is_some()), egui::Button::new(t(if self.macro_local_mode {"Сохранить в библиотеку"} else if self.local_edit.is_some() { "Сохранить в файл" } else { "Записать макросы" }))).clicked() {
                if self.macro_local_mode {
                    if let Some(value)=self.macro_working().and_then(|bank|bank.macros.get(self.macro_slot)) { self.action=Some(if let Some(path)=&self.local_macro_path {Action::Confirm(t("Сохранить изменения макроса? Предыдущая версия останется в резервной копии.").into(),KeyboardCommand::UpdateLocalMacro {path:path.clone(),value:value.clone()})} else {Action::Send(KeyboardCommand::SaveLocalMacro {name:value.name.clone(),value:value.clone()})}); }
                } else if self.local_edit.is_some() { self.save_local(); }
                else if let (Some((device_path, profile)), Some(bank)) = (self.target(state), self.macro_working()) {
                    match bank.encode() {
                        Ok(_) => self.action = Some(Action::Send(KeyboardCommand::ApplyMacros { device_path, profile, bank: bank.clone() })),
                        Err(error) => self.message = format!("{error:#}"),
                    }
                }
            }
            if ui.add_enabled(!self.is_recording() && !busy && self.macros_dirty(), egui::Button::new(t("Отменить изменения"))).clicked() {
                if self.macro_local_mode {self.local_macro_bank=self.local_macro_original.clone();}
                else if let Some((_, value)) = &mut self.local_edit { value.macros = self.local_macro_baseline.clone(); }
                else { self.macro_draft = self.macro_baseline.clone(); }
            }
        });
        ui.add_space(10.0);
        let Some(mut value) = self
            .macro_working()
            .and_then(|bank| bank.macros.get(self.macro_slot))
            .cloned()
        else {
            return;
        };
        let original = value.clone();
        ui.add_enabled_ui(!self.is_recording(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Имя"));
                ui.text_edit_singleline(&mut value.name);
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Воспроизведение"));
                for (mode, label) in [
                    (0, "Пока удерживается клавиша"),
                    (255, "До повторного нажатия"),
                    (1, "Один раз"),
                ] {
                    ui.selectable_value(&mut value.mode, mode, t(label));
                }
            });
            if (1..=254).contains(&value.mode) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(t("Число повторений"));
                    ui.add(egui::DragValue::new(&mut value.mode).range(1..=254));
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(
                    &mut value.enable_default_delay,
                    t("Фиксированная задержка записи"),
                );
                ui.add_enabled(
                    value.enable_default_delay,
                    egui::DragValue::new(&mut value.default_delay)
                        .range(5..=65535)
                        .suffix(t(" мс")),
                );
            });
        });
        ui.add_space(14.0);
        let recording = self.is_recording();
        if ui
            .add(
                egui::Button::new(
                    RichText::new(t(if recording {
                        "Остановить запись"
                    } else {
                        "Начать запись"
                    }))
                    .color(if recording {
                        Color32::WHITE
                    } else {
                        palette.text
                    }),
                )
                .fill(if recording {
                    Color32::from_rgb(180, 35, 35)
                } else {
                    palette.field
                }),
            )
            .clicked()
        {
            if recording {
                self.stop_recording(None);
            } else {
                let slot = self.macro_slot;
                if let Some(bank) = self.macro_working_mut() {
                    bank.macros[slot] = value.clone();
                }
                self.start_recording(ui.ctx());
            }
            if let Some(current) = self
                .macro_working()
                .and_then(|bank| bank.macros.get(self.macro_slot))
            {
                value = current.clone();
            }
        }
        let (area, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.0), egui::Sense::hover());
        self.record_area = Some(area);
        ui.painter().rect_filled(area, 8.0, palette.field);
        ui.painter().text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            t("Область записи кнопок мыши"),
            egui::FontId::proportional(14.0),
            palette.text,
        );
        ui.label(RichText::new(t("Записываются клавиши и кнопки мыши в этой области. Esc или потеря фокуса останавливают запись. Колесо не поддерживается форматом макросов ACE75.")).small().color(palette.muted));
        ui.add_space(14.0);
        ui.add_enabled_ui(!self.is_recording(), |ui| {
            ui.label(RichText::new(t("Добавить событие")).strong());
            ui.horizontal_wrapped(|ui| {
                for (kind, label) in [(0, "Клавиша"), (1, "Кнопка мыши"), (2, "Задержка")]
                {
                    ui.selectable_value(&mut self.macro_event_kind, kind, t(label));
                }
            });
            ui.horizontal_wrapped(|ui| {
                if self.macro_event_kind == 2 {
                    if self.macro_event_delay < 5 {
                        self.macro_event_delay = 50;
                    }
                    ui.add(
                        egui::DragValue::new(&mut self.macro_event_delay)
                            .range(5..=65535)
                            .suffix(t(" мс")),
                    );
                } else {
                    macro_code_picker(
                        ui,
                        "ace75-add-event",
                        &mut self.macro_event_code,
                        self.macro_event_kind == 1,
                        self.layer,
                    );
                    ui.checkbox(&mut self.macro_event_pressed, t("Нажатие"));
                }
                if ui
                    .add_enabled(
                        self.macro_event_kind == 2 || self.macro_event_code > 0,
                        egui::Button::new(t("Добавить")),
                    )
                    .clicked()
                {
                    value.macro_actions.push(if self.macro_event_kind == 2 {
                        MacroAction::Time {
                            delay: self.macro_event_delay.max(5),
                        }
                    } else {
                        MacroAction::Action {
                            code: self.macro_event_code.max(1),
                            action: match (self.macro_event_kind, self.macro_event_pressed) {
                                (1, true) => MacroKeyAction::Mousedown,
                                (1, false) => MacroKeyAction::Mouseup,
                                (_, true) => MacroKeyAction::Keydown,
                                (_, false) => MacroKeyAction::Keyup,
                            },
                        }
                    });
                }
                if ui
                    .add_enabled(
                        !value.macro_actions.is_empty(),
                        egui::Button::new(t("Очистить события")),
                    )
                    .clicked()
                {
                    value.macro_actions.clear();
                }
            });
            ui.add_space(12.0);
            let mut operation = None;
            egui::ScrollArea::vertical()
                .id_salt("ace75-macro-events")
                .max_height(320.0)
                .show(ui, |ui| {
                    for (index, event) in value.macro_actions.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("{}", index + 1));
                                match event {
                                    MacroAction::Time { delay } => {
                                        ui.label(t("Задержка"));
                                        ui.add(
                                            egui::DragValue::new(delay)
                                                .range(5..=65535)
                                                .suffix(t(" мс")),
                                        );
                                    }
                                    MacroAction::Action { action, code } => {
                                        let mouse = matches!(
                                            action,
                                            MacroKeyAction::Mousedown | MacroKeyAction::Mouseup
                                        );
                                        let mut pressed = matches!(
                                            action,
                                            MacroKeyAction::Keydown | MacroKeyAction::Mousedown
                                        );
                                        if ui.checkbox(&mut pressed, t("Нажатие")).changed()
                                        {
                                            *action = match (mouse, pressed) {
                                                (true, true) => MacroKeyAction::Mousedown,
                                                (true, false) => MacroKeyAction::Mouseup,
                                                (false, true) => MacroKeyAction::Keydown,
                                                (false, false) => MacroKeyAction::Keyup,
                                            };
                                        }
                                        macro_code_picker(
                                            ui,
                                            "event-code",
                                            code,
                                            mouse,
                                            self.layer,
                                        );
                                    }
                                    MacroAction::Raw { bytes } => {
                                        ui.label(tf!("Неизвестное событие: {:02X?}", bytes));
                                    }
                                }
                                if ui.button("↑").on_hover_text(t("Выше")).clicked() {
                                    operation = Some((index, -1));
                                }
                                if ui.button("↓").on_hover_text(t("Ниже")).clicked() {
                                    operation = Some((index, 1));
                                }
                                if ui.button("×").on_hover_text(t("Удалить событие")).clicked()
                                {
                                    operation = Some((index, 0));
                                }
                            });
                        });
                    }
                });
            if let Some((index, direction)) = operation {
                match direction {
                    -1 if index > 0 => value.macro_actions.swap(index, index - 1),
                    1 if index + 1 < value.macro_actions.len() => {
                        value.macro_actions.swap(index, index + 1)
                    }
                    0 => {
                        value.macro_actions.remove(index);
                    }
                    _ => {}
                }
            }
        });
        if value != original {
            let slot = self.macro_slot;
            if let Some(bank) = self.macro_working_mut() {
                bank.macros[slot] = value;
            }
        }
        ui.add_space(12.0);
        ui.label(
            RichText::new(tf!("Общий банк: {} байт · 16 макросов", bank_size))
                .small()
                .color(palette.muted),
        );
        if let Some(bank) = self.macro_working() {
            if let Err(error) = bank.encode() {
                ui.label(
                    RichText::new(i18n::message(&format!("{error:#}"))).color(palette.error_text),
                );
            }
        }
    }

    fn colors_editor(
        &mut self,
        ui: &mut egui::Ui,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Цвет выбранных зон"));
            ui.color_edit_button_srgb(&mut self.paint_color);
            let selected = self
                .selected_lights
                .iter()
                .copied()
                .filter(|index| light_index_in_scope(*index, self.light_scope))
                .collect::<Vec<_>>();
            let supported = self.light_scope != 2 || firmware_number(state) >= 115;
            if ui
                .add_enabled(
                    supported && !selected.is_empty(),
                    egui::Button::new(t("Окрасить выбранные")),
                )
                .clicked()
            {
                let color = self.paint_color;
                if let Some(profile) = self.working_mut() {
                    for index in selected {
                        profile.colors[index as usize] = color;
                    }
                }
            }
            if ui.button(t("Выбрать всю зону")).clicked() {
                self.selected_lights = cells(GridPurpose::Lighting)
                    .filter(|cell| light_index_in_scope(cell.matrix_index, self.light_scope))
                    .map(|cell| cell.matrix_index)
                    .collect();
            }
        });
        let changed = self
            .working()
            .zip(self.original())
            .is_some_and(|(value, before)| value.colors != before.colors);
        if ui
            .add_enabled(
                changed
                    && !busy
                    && (self.local_edit.is_some() || writable && self.target(state).is_some()),
                egui::Button::new(t("Применить цвета")),
            )
            .clicked()
        {
            if self.local_edit.is_some() {
                self.save_local();
            } else if let (Some((device_path, profile)), Some(value)) =
                (self.target(state), self.working())
            {
                self.action = Some(Action::Send(KeyboardCommand::ApplyColors {
                    device_path,
                    profile,
                    colors: value.colors.clone(),
                }));
            }
        }
        ui.label(t("Индивидуальные цвета видны в пользовательском эффекте. Выберите его в настройках соответствующей зоны."));
    }

    fn calibration_controls(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.add_space(20.0);
        ui.label(RichText::new(t("Калибровка переключателей")).strong());
        ui.label(t("Поставьте клавиатуру на ровную неметаллическую поверхность и не двигайте её. Перед началом отпустите все клавиши."));
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(writable && !busy && self.local_edit.is_none() && !self.hardware_dirty() && self.target(state).is_some() && !state.calibration_active, egui::Button::new(t("Начать калибровку"))).clicked() {
                if let Some((device_path, profile)) = self.target(state) { self.action = Some(Action::Confirm(t("Все клавиши отпущены, клавиатура лежит неподвижно? После начала нажмите каждую клавишу до упора несколько раз.").into(), KeyboardCommand::StartCalibration { device_path, profile })); }
            }
            if ui.add_enabled(!busy && state.calibration_active, egui::Button::new(t("Остановить"))).clicked() { self.action = Some(Action::Send(KeyboardCommand::StopCalibration)); }
            if ui.add_enabled(!busy && writable && state.calibration_active, egui::Button::new(t("Начать заново"))).clicked() { self.action = Some(Action::Send(KeyboardCommand::RestartCalibration)); }
        });
        if state.calibration_active {
            ui.label(RichText::new(t("Нажмите каждую клавишу до упора несколько раз. Подтверждённые клавиши отмечаются зелёным.")).color(palette.accent_text));
            ui.label(tf!("Подтверждено клавиш: {}", self.calibrated.len()));
            self.show_grid(ui, palette, GridPurpose::Trigger);
            if let Some(travel) = &state.travel {
                ui.label(tf!(
                    "Клавиша: {} · Ход: {:.2} мм",
                    key_label(travel.key, 0),
                    if travel.travel_raw <= 10 {
                        0.0
                    } else {
                        f64::from(travel.travel_raw) / 100.0
                    }
                ));
                if state.calibration_state == Some(255) {
                    ui.label(t("Калибровка этой клавиши подтверждена"));
                }
            }
        }
        let mut monitor = state.monitor_active;
        if ui
            .add_enabled(
                writable
                    && !busy
                    && !state.calibration_active
                    && self.local_edit.is_none()
                    && self.target(state).is_some(),
                egui::Checkbox::new(&mut monitor, t("Показывать ход клавиш в реальном времени")),
            )
            .changed()
        {
            if let Some((device_path, profile)) = self.target(state) {
                self.action = Some(Action::Send(KeyboardCommand::Telemetry {
                    device_path,
                    profile,
                    enabled: monitor,
                }));
            }
        }
        if state.monitor_active {
            if let Some(travel) = &state.travel {
                ui.label(tf!(
                    "Клавиша: {} · Ход: {:.2} мм",
                    key_label(travel.key, 0),
                    if travel.travel_raw <= 10 {
                        0.0
                    } else {
                        f64::from(travel.travel_raw) / 100.0
                    }
                ));
            }
        }
    }
}

impl Ace75Ui {
    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }
    pub fn hide(&mut self, state: &KeyboardState) -> Option<KeyboardCommand> {
        self.stop_recording(None);
        (state.calibration_active || state.monitor_active)
            .then_some(KeyboardCommand::StopCalibration)
    }

    fn start_recording(&mut self, ctx: &egui::Context) {
        if self.macro_working().is_none() {
            return;
        }
        match crate::recorder::WindowKeyboardRecorder::start(ctx) {
            Ok(mut recorder) => {
                if let Some(area) = self.record_area {
                    if let Err(error) = recorder.set_pointer_area(area, ctx.pixels_per_point()) {
                        self.message = format!("{error:#}");
                        return;
                    }
                }
                self.record_held.clear();
                self.record_skipped_ms = 0;
                self.recorded_at = None;
                self.recorder = Some(recorder);
            }
            Err(error) => self.message = format!("{error:#}"),
        }
    }

    pub fn stop_recording(&mut self, delay_override: Option<u16>) {
        if self.recorder.is_none() {
            return;
        }
        let elapsed = delay_override
            .or_else(|| {
                self.recorder
                    .as_ref()
                    .and_then(|recorder| recorder.elapsed_since_last_event())
            })
            .unwrap_or_else(|| {
                self.recorded_at
                    .map_or(0, |time| time.elapsed().as_millis().min(65535) as u16)
            });
        let delay = u32::from(elapsed)
            .saturating_add(self.record_skipped_ms)
            .min(65535) as u16;
        self.recorder = None;
        let held = std::mem::take(&mut self.record_held);
        let slot = self.macro_slot;
        if let Some(value) = self
            .macro_working_mut()
            .and_then(|bank| bank.macros.get_mut(slot))
        {
            for (index, (kind, code)) in held.into_iter().enumerate() {
                if index == 0 {
                    value.macro_actions.push(MacroAction::Time {
                        delay: if value.enable_default_delay {
                            value.default_delay.max(5)
                        } else {
                            delay.max(5)
                        },
                    });
                }
                value.macro_actions.push(MacroAction::Action {
                    action: if kind == 0 {
                        MacroKeyAction::Mouseup
                    } else {
                        MacroKeyAction::Keyup
                    },
                    code,
                });
            }
        }
        self.recorded_at = None;
        self.record_skipped_ms = 0;
    }

    pub fn poll_recording(&mut self, ctx: &egui::Context) {
        if self.page != Page::Macros {
            self.stop_recording(None);
            return;
        }
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        if let Some(area) = self.record_area {
            if let Err(error) = recorder.set_pointer_area(area, ctx.pixels_per_point()) {
                self.message = format!("{error:#}");
                self.stop_recording(None);
                return;
            }
        }
        match recorder.poll() {
            Ok(batch) => {
                for event in batch.events {
                    if event.kind == 17 && event.value == 41 && event.pressed {
                        self.stop_recording(Some(event.delay_ms));
                        break;
                    }
                    if !matches!(event.kind, 0 | 17) {
                        self.record_skipped_ms = self
                            .record_skipped_ms
                            .saturating_add(u32::from(event.delay_ms));
                        continue;
                    }
                    let delay = u32::from(event.delay_ms)
                        .saturating_add(self.record_skipped_ms)
                        .min(65535) as u16;
                    self.record_skipped_ms = 0;
                    if !self.append_recorded(
                        event.kind,
                        u16::from(event.value),
                        event.pressed,
                        delay,
                    ) {
                        self.stop_recording(Some(delay));
                        break;
                    }
                }
                if batch.focus_lost {
                    self.stop_recording(None);
                }
            }
            Err(error) => {
                self.message = format!("{error:#}");
                self.stop_recording(None);
            }
        }
    }

    fn append_recorded(&mut self, kind: u8, code: u16, pressed: bool, delay: u16) -> bool {
        let Some(bank) = self.macro_working() else {
            return false;
        };
        let mut bank = bank.clone();
        let Some(value) = bank.macros.get_mut(self.macro_slot) else {
            return false;
        };
        let mut held = self.record_held.clone();
        if pressed {
            held.insert((kind, code));
        } else {
            held.remove(&(kind, code));
        }
        if self.recorded_at.is_some() {
            value.macro_actions.push(MacroAction::Time {
                delay: if value.enable_default_delay {
                    value.default_delay.max(5)
                } else {
                    delay.max(5)
                },
            });
        }
        value.macro_actions.push(MacroAction::Action {
            action: match (kind, pressed) {
                (0, true) => MacroKeyAction::Mousedown,
                (0, false) => MacroKeyAction::Mouseup,
                (_, true) => MacroKeyAction::Keydown,
                (_, false) => MacroKeyAction::Keyup,
            },
            code,
        });
        let candidate = value.clone();
        for (index, (kind, code)) in held.iter().enumerate() {
            if index == 0 {
                value.macro_actions.push(MacroAction::Time { delay: 65535 });
            }
            value.macro_actions.push(MacroAction::Action {
                action: if *kind == 0 {
                    MacroKeyAction::Mouseup
                } else {
                    MacroKeyAction::Keyup
                },
                code: *code,
            });
        }
        if let Err(error) = bank.encode() {
            self.message = format!("{error:#}");
            return false;
        }
        let slot = self.macro_slot;
        if let Some(bank) = self.macro_working_mut() {
            bank.macros[slot] = candidate;
        }
        self.record_held = held;
        self.recorded_at = Some(std::time::Instant::now());
        true
    }

    pub fn filter_raw_input(&self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if !self.is_recording() {
            return;
        }
        let mut pointer = ctx.input(|state| state.pointer.hover_pos());
        input.events.retain(|event| {
            if let egui::Event::PointerMoved(position) = event {
                pointer = Some(*position);
            }
            !super::is_keyboard_input(event)
                && !(matches!(event, egui::Event::MouseWheel { .. })
                    && self
                        .record_area
                        .zip(pointer)
                        .is_some_and(|(area, position)| area.contains(position)))
        });
        input.modifiers = egui::Modifiers::NONE;
    }
}

impl Ace75Ui {
    fn new_local_macro(&mut self) {
        self.stop_recording(None);
        let bank = MacroBank::decode(&vec![0; 8192]).expect("Empty macro bank");
        self.local_macro_original = Some(bank.clone());
        self.local_macro_bank = Some(bank);
        self.macro_local_mode = true;
        self.macro_slot = 0;
        self.local_macro_path = None;
    }

    fn macro_library(&mut self, ui: &mut egui::Ui, state: &KeyboardState, busy: bool) {
        ui.horizontal_wrapped(|ui| {
            let has_bank = self
                .local_edit
                .as_ref()
                .map_or(self.macro_draft.is_some(), |(_, value)| {
                    value.macros.is_some()
                });
            let mut mode = self.macro_local_mode;
            ui.add_enabled_ui(!self.is_recording(), |ui| {
                if has_bank {
                    ui.selectable_value(&mut mode, false, t("Банк профиля"));
                }
                ui.selectable_value(&mut mode, true, t("Локальный макрос"));
                if ui.button(t("Новый макрос")).clicked() {
                    if self.macro_local_mode && self.macros_dirty() {
                        self.new_macro_confirm = true;
                    } else {
                        self.new_local_macro();
                        mode = true;
                    }
                }
            });
            if mode != self.macro_local_mode {
                self.macro_local_mode = mode;
                self.macro_slot = 0;
            }
        });
        if self.macro_local_mode && self.local_macro_bank.is_none() {
            self.new_local_macro();
        }
        egui::CollapsingHeader::new(t("Библиотека макросов ACE75")).default_open(self.macro_local_mode).show(ui,|ui| {
            ui.horizontal_wrapped(|ui| { ui.label(t("Новое имя")); ui.text_edit_singleline(&mut self.macro_library_name); });
            for item in &state.macros {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&item.name);
                    if ui.add_enabled(!busy && !self.is_recording(),egui::Button::new(t("Открыть"))).clicked() { let command=KeyboardCommand::OpenLocalMacro(item.path.clone()); self.action=Some(if self.macro_local_mode && self.macros_dirty() {Action::Confirm(t("Открыть другой макрос и отменить несохранённые изменения редактора?").into(),command)} else {Action::Send(command)}); }
                    if ui.add_enabled(!busy && !self.macro_library_name.trim().is_empty(),egui::Button::new(t("Переименовать"))).clicked() { self.action=Some(Action::Confirm(tf!("Переименовать макрос «{}» в «{}»?",item.name,self.macro_library_name.trim()),KeyboardCommand::RenameLocalMacro {path:item.path.clone(),name:self.macro_library_name.trim().into()})); }
                    if ui.add_enabled(!busy,egui::Button::new(t("Удалить"))).clicked() { self.action=Some(Action::Confirm(tf!("Удалить локальный макрос «{}»? Сохранится резервная копия.",item.name),KeyboardCommand::DeleteLocalMacro(item.path.clone()))); }
                });
            }
            ui.label(t("Путь к JSON-файлу")); ui.add(egui::TextEdit::singleline(&mut self.macro_library_file).desired_width(ui.available_width()));
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy && !self.macro_library_file.trim().is_empty(),egui::Button::new(t("Импорт в библиотеку"))).clicked() { self.action=Some(Action::Send(KeyboardCommand::ImportLocalMacro(PathBuf::from(self.macro_library_file.trim())))); }
                if ui.add_enabled(!busy && !self.macro_library_file.trim().is_empty() && self.macro_working().is_some(),egui::Button::new(t("Экспорт"))).clicked() { if let Some(value)=self.macro_working().and_then(|bank|bank.macros.get(self.macro_slot)) { self.action=Some(Action::Send(KeyboardCommand::ExportLocalMacro {path:PathBuf::from(self.macro_library_file.trim()),value:value.clone()})); } }
            });
        });
        if self.macro_local_mode {
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Слот профиля"));
                ui.add(egui::DragValue::new(&mut self.macro_target_slot).range(1..=16));
                let destination = if let Some((_, value)) = &self.local_edit {
                    value.macros.is_some()
                } else {
                    self.macro_draft.is_some()
                };
                if ui
                    .add_enabled(
                        destination && !self.is_recording(),
                        egui::Button::new(t("Вставить в слот профиля")),
                    )
                    .clicked()
                {
                    if let Some(mut value) = self
                        .macro_working()
                        .and_then(|bank| bank.macros.get(self.macro_slot))
                        .cloned()
                    {
                        let slot = self.macro_target_slot.clamp(1, 16) - 1;
                        value.macro_index = slot;
                        let bank = if let Some((_, value)) = &mut self.local_edit {
                            value.macros.as_mut()
                        } else {
                            self.macro_draft.as_mut()
                        };
                        if let Some(bank) = bank {
                            bank.macros[slot as usize] = value;
                            self.macro_slot = slot as usize;
                            self.macro_local_mode = false;
                        }
                    }
                }
            });
        }
        if self.new_macro_confirm {
            egui::Window::new(t("Новый макрос"))
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(t(
                        "Создать новый макрос и отменить несохранённые изменения редактора?",
                    ));
                    ui.horizontal(|ui| {
                        if ui.button(t("Подтвердить")).clicked() {
                            self.new_local_macro();
                            self.new_macro_confirm = false;
                        }
                        if ui.button(t("Отмена")).clicked() {
                            self.new_macro_confirm = false;
                        }
                    });
                });
        }
        ui.add_space(12.0);
    }
}

fn light_index_in_scope(index: u16, scope: u8) -> bool {
    match scope {
        1 => (95..123).contains(&index),
        2 => index == 123,
        _ => !(95..124).contains(&index),
    }
}
fn macro_key_label(code: u16, layer: u8) -> String {
    key_label(
        if (224..=231).contains(&code) {
            KeyAssignment {
                kind: 16,
                code1: 1 << (code - 224),
                code2: 0,
            }
        } else {
            KeyAssignment {
                kind: 16,
                code1: 0,
                code2: code as u8,
            }
        },
        layer,
    )
}
fn macro_code_picker(ui: &mut egui::Ui, id: &str, code: &mut u16, mouse: bool, layer: u8) {
    let label = if mouse {
        t(match *code {
            1 => "Левая кнопка",
            2 => "Правая кнопка",
            4 => "Средняя кнопка",
            8 => "Назад",
            16 => "Вперёд",
            _ => "Неизвестный",
        })
        .to_owned()
    } else {
        macro_key_label(*code, layer)
    };
    egui::ComboBox::from_id_salt(id)
        .selected_text(label)
        .show_ui(ui, |ui| {
            if mouse {
                for (value, label) in [
                    (1, "Левая кнопка"),
                    (2, "Правая кнопка"),
                    (4, "Средняя кнопка"),
                    (8, "Назад"),
                    (16, "Вперёд"),
                ] {
                    ui.selectable_value(code, value, t(label));
                }
            } else {
                let mut seen = BTreeSet::new();
                for action in key_actions().iter().filter(|action| {
                    action.kind == 16
                        && action.code1 == 0
                        && action.code2 != 0
                        && action.layers.contains(&layer)
                }) {
                    if seen.insert(action.code2) {
                        ui.selectable_value(code, u16::from(action.code2), action.label());
                    }
                }
                for value in 224..=231 {
                    ui.selectable_value(code, value, macro_key_label(value, layer));
                }
            }
        });
}

struct AdvancedEditor {
    layer: u8,
    indices: Vec<u8>,
    kind: u8,
    actions: [KeyAssignment; 4],
    dks: crate::ace75::DksKey,
    timeout: u16,
    priority: u8,
    pick: usize,
    dirty: bool,
}

impl AdvancedEditor {
    fn load(profile: &KeyboardProfile, layer: u8, indices: &[u16]) -> Self {
        let empty = KeyAssignment {
            kind: 16,
            code1: 0,
            code2: 0,
        };
        let mut value = Self {
            layer,
            indices: indices.iter().map(|index| *index as u8).collect(),
            kind: 144,
            actions: [empty; 4],
            dks: crate::ace75::DksKey {
                points: [10, 30, 30, 10],
                actions: [empty; 4],
                masks: [[0; 4]; 4],
                reserved_masks: [0; 4],
            },
            timeout: 150,
            priority: 0,
            pick: 0,
            dirty: false,
        };
        let Some(index) = value.indices.first().copied() else {
            return value;
        };
        let binding = profile.layers[layer as usize][index as usize];
        if (144..=149).contains(&binding.kind) {
            value.kind = binding.kind;
            let slot = binding.code1 as usize;
            match binding.kind {
                144 => {
                    if let Some(record) = profile.advanced.dks.get(slot) {
                        value.dks = record.clone();
                        value.actions = record.actions;
                    }
                }
                145 => {
                    if let Some(action) = profile.advanced.tgl.get(slot) {
                        value.actions[0] = *action;
                    }
                }
                _ => {
                    if let Some(record) = profile.advanced.mt.get(slot) {
                        value.actions[0] = record.click;
                        value.actions[1] = record.hold;
                    }
                }
            }
            if binding.kind == 146 {
                value.timeout = u16::from(binding.code2) * 10;
            }
            if binding.kind >= 147 {
                value.indices = vec![index, binding.code2];
                value.priority = profile.triggers[index as usize].priority;
            }
        } else {
            for (slot, index) in value.indices.iter().take(2).enumerate() {
                value.actions[slot] = profile.layers[layer as usize][*index as usize];
            }
        }
        value
    }

    fn kind_name(&self) -> &'static str {
        advanced_name(self.kind)
    }

    fn stage(&self, original: &KeyboardProfile) -> anyhow::Result<KeyboardProfile> {
        anyhow::ensure!(
            self.layer == 0 || self.layer == 2,
            "Расширенные назначения доступны только на основных слоях."
        );
        let pair = self.kind >= 147;
        anyhow::ensure!(
            self.indices.len() == if pair { 2 } else { 1 },
            "Выберите подходящее количество клавиш для этого назначения."
        );
        let mut profile = original.clone();
        remove_advanced_assignments(&mut profile, self.layer, &self.indices);
        let used = profile
            .layers
            .iter()
            .flatten()
            .filter(|key| match self.kind {
                144 => key.kind == 144,
                145 => key.kind == 145,
                _ => (146..=149).contains(&key.kind),
            })
            .map(|key| key.code1)
            .collect::<BTreeSet<_>>();
        let free = (0..32)
            .filter(|slot| !used.contains(slot))
            .take(if pair { 2 } else { 1 })
            .collect::<Vec<u8>>();
        anyhow::ensure!(
            free.len() == if pair { 2 } else { 1 },
            "Нет свободных ячеек для расширенного назначения."
        );
        let slot = free[0];
        let first = self.indices[0] as usize;
        match self.kind {
            144 => {
                let mut record = self.dks.clone();
                record.actions = self.actions;
                profile.advanced.dks[slot as usize] = record;
                profile.layers[self.layer as usize][first] =
                    crate::ace75::AdvancedBinding::Dks { slot, reserved: 0 }.encode()?;
            }
            145 => {
                profile.advanced.tgl[slot as usize] = self.actions[0];
                profile.layers[self.layer as usize][first] =
                    crate::ace75::AdvancedBinding::Tgl { slot, reserved: 0 }.encode()?;
            }
            146 => {
                profile.advanced.mt[slot as usize] = crate::ace75::MtKey {
                    click: self.actions[0],
                    hold: self.actions[1],
                };
                profile.layers[self.layer as usize][first] = crate::ace75::AdvancedBinding::Mt {
                    slot,
                    delay_ms: self.timeout,
                }
                .encode()?;
            }
            kind => {
                let assignments = crate::ace75::paired_assignments(
                    kind,
                    [self.indices[0], self.indices[1]],
                    [free[0], free[1]],
                )?;
                let records = crate::ace75::paired_mt_records([self.actions[0], self.actions[1]]);
                for index in 0..2 {
                    profile.advanced.mt[free[index] as usize] = records[index].clone();
                    profile.layers[self.layer as usize][self.indices[index] as usize] =
                        assignments[index];
                }
                if kind == 148 {
                    profile.triggers[first].priority = self.priority;
                    profile.triggers[self.indices[1] as usize].priority = match self.priority {
                        1 => 2,
                        2 => 1,
                        other => other,
                    };
                }
            }
        }
        profile.advanced.validate_references(&profile.layers)?;
        Ok(profile)
    }
}

impl Ace75Ui {
    fn advanced_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            let changed = self
                .working()
                .zip(self.original())
                .is_some_and(|(value, original)| {
                    value.advanced != original.advanced
                        || value.layers != original.layers
                        || value.triggers != original.triggers
                        || value.custom_params != original.custom_params
                });
            if ui
                .add_enabled(
                    changed
                        && !busy
                        && (self.local_edit.is_some() || writable && self.target(state).is_some()),
                    egui::Button::new(t("Применить")),
                )
                .clicked()
            {
                if self.local_edit.is_some() {
                    self.save_local();
                } else if let Some((device_path, profile)) = self.target(state) {
                    if let Some(command) = self.advanced_command(device_path, profile, None) {
                        self.action = Some(Action::Send(command));
                    }
                }
            }
            if ui.button(t("Отменить настройки редактора")).clicked() {
                self.advanced_editor = None;
            }
        });
        ui.add_space(12.0);
        let editing = self
            .advanced_editor
            .as_ref()
            .is_some_and(|editor| editor.dirty);
        ui.add_enabled_ui(!editing, |ui| {
            self.layer_selector(ui);
            self.grid_section(ui, palette, GridPurpose::Advanced);
        });
        if self.layer % 2 != 0 {
            ui.label(t(
                "Расширенные назначения доступны только на основных слоях.",
            ));
            return;
        }
        if self.no_data(ui, palette) {
            return;
        }
        if self.selected_keys.is_empty() {
            ui.label(t("Выберите одну клавишу или пару клавиш на схеме."));
            return;
        }
        let selected = self.selected_keys.iter().copied().collect::<Vec<_>>();
        if self.advanced_editor.as_ref().is_none_or(|editor| {
            !editor.dirty
                && (editor.layer != self.layer
                    || editor
                        .indices
                        .iter()
                        .copied()
                        .map(u16::from)
                        .collect::<Vec<_>>()
                        != selected)
        }) {
            self.advanced_editor = Some(AdvancedEditor::load(
                self.working().unwrap(),
                self.layer,
                &selected,
            ));
        }
        let mut editor = self.advanced_editor.take().unwrap();
        let before_kind = editor.kind;
        ui.horizontal_wrapped(|ui| {
            for kind in [147, 149, 148, 144, 146, 145] {
                ui.selectable_value(&mut editor.kind, kind, advanced_name(kind));
            }
        });
        if editor.kind != before_kind {
            editor.dirty = true;
            editor.pick = 0;
        }
        ui.label(
            RichText::new(t(match editor.kind {
                147 => "Приоритет получает глубже нажатая клавиша.",
                148 => "Разрешение одновременного нажатия противоположных клавиш.",
                149 => "Действия пары клавиш связаны между собой.",
                144 => "До четырёх действий на разных этапах нажатия и отпускания.",
                146 => "Разные действия при коротком нажатии и удержании.",
                _ => "Первое нажатие включает удержание, повторное выключает.",
            }))
            .color(palette.muted),
        );
        let count = match editor.kind {
            144 => 4,
            145 => 1,
            _ => 2,
        };
        ui.horizontal_wrapped(|ui| {
            for index in 0..count {
                if ui
                    .selectable_label(
                        editor.pick == index,
                        tf!(
                            "Действие {}: {}",
                            index + 1,
                            key_label(editor.actions[index], self.layer)
                        ),
                    )
                    .clicked()
                {
                    editor.pick = index;
                }
            }
        });
        if editor.kind == 146 {
            editor.dirty |= ui
                .add(
                    egui::Slider::new(&mut editor.timeout, 10..=2550)
                        .step_by(10.0)
                        .suffix(t(" мс"))
                        .text(t("Порог удержания")),
                )
                .changed();
        }
        if editor.kind == 148 {
            ui.horizontal_wrapped(|ui| {
                for (priority, label) in [
                    (0, "Последнее нажатие"),
                    (1, "Приоритет первой клавиши"),
                    (2, "Приоритет второй клавиши"),
                    (3, "Оба нажатия отменяются"),
                ] {
                    editor.dirty |= ui
                        .selectable_value(&mut editor.priority, priority, t(label))
                        .changed();
                }
            });
        }
        if editor.kind == 144 {
            dks_editor(ui, &mut editor);
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Поиск"));
            ui.text_edit_singleline(&mut self.action_search);
        });
        if let Some(action) =
            self.action_picker(ui, Some(editor.kind_name().to_ascii_lowercase().as_str()))
        {
            editor.actions[editor.pick] = action;
            editor.dirty = true;
        }
        egui::CollapsingHeader::new(t("Макрос для действия")).show(ui, |ui| {
            let bank = if let Some((_, value)) = &self.local_edit {
                value.macros.as_ref()
            } else {
                self.macro_draft.as_ref()
            };
            if let Some(bank) = bank {
                ui.horizontal_wrapped(|ui| {
                    for value in bank
                        .macros
                        .iter()
                        .filter(|value| !value.macro_actions.is_empty())
                    {
                        let label = if value.name.is_empty() {
                            tf!("Макрос {}", value.macro_index + 1)
                        } else {
                            value.name.clone()
                        };
                        if ui.button(label).clicked() {
                            editor.actions[editor.pick] = KeyAssignment {
                                kind: 112,
                                code1: value.macro_index,
                                code2: value.mode,
                            };
                            editor.dirty = true;
                        }
                    }
                });
            }
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Добавить назначение в профиль")).clicked() {
                match editor.stage(self.working().unwrap()) {
                    Ok(profile) => {
                        *self.working_mut().unwrap() = profile;
                        editor.dirty = false;
                        self.message.clear();
                    }
                    Err(error) => self.message = format!("{error:#}"),
                }
            }
            if ui.button(t("Удалить расширенное назначение")).clicked()
            {
                if let Some(profile) = self.working_mut() {
                    remove_advanced_assignments(profile, editor.layer, &editor.indices);
                }
                editor.dirty = false;
            }
        });
        if editor.dirty {
            ui.label(t(
                "Завершите или отмените настройку редактора, чтобы выбрать другие клавиши.",
            ));
        }
        self.advanced_editor = Some(editor);
    }
}

fn dks_editor(ui: &mut egui::Ui, editor: &mut AdvancedEditor) {
    ui.label(t(
        "Этапы DKS: нажатие 1,0 → 3,0 мм; отпускание 3,0 → 1,0 мм.",
    ));
    let points = ["↓ 1.0 mm", "↓ 3.0 mm", "↑ 3.0 mm", "↑ 1.0 mm"];
    for action in 0..4 {
        let Ok(mut phases) = editor.dks.ui_phases(action) else {
            continue;
        };
        let original = phases;
        ui.push_id(("dks-phases", action), |ui| {
            ui.label(tf!("Действие {}", action + 1));
            ui.horizontal_wrapped(|ui| {
                for phase in 0..4 {
                    let covered =
                        (0..phase).any(|previous| usize::from(phases[previous]) > phase - previous);
                    ui.add_enabled_ui(!covered, |ui| {
                        ui.vertical(|ui| {
                            ui.label(points[phase]);
                            egui::ComboBox::from_id_salt(phase)
                                .width(112.0)
                                .selected_text(dks_phase_label(phase, phases[phase], &points))
                                .show_ui(ui, |ui| {
                                    for value in 0..=(4 - phase) as u8 {
                                        ui.selectable_value(
                                            &mut phases[phase],
                                            value,
                                            dks_phase_label(phase, value, &points),
                                        );
                                    }
                                });
                        });
                    });
                }
            });
        });
        if phases != original {
            if editor.dks.set_ui_phases(action, phases).is_ok() {
                editor.dirty = true;
            }
        }
    }
}
fn dks_phase_label(phase: usize, value: u8, points: &[&str; 4]) -> String {
    match value {
        0 => t("Нет действия").into(),
        1 => t("Нажать и отпустить").into(),
        _ => tf!(
            "Удерживать до {}",
            points[(phase + usize::from(value) - 1).min(3)]
        ),
    }
}

fn advanced_name(kind: u8) -> &'static str {
    match kind {
        144 => "DKS",
        145 => "TGL",
        146 => "MT",
        147 => "RS",
        148 => "SOCD",
        149 => "LT",
        _ => "?",
    }
}
fn remove_advanced_assignments(profile: &mut KeyboardProfile, layer: u8, indices: &[u8]) {
    let mut reset = indices.iter().copied().collect::<BTreeSet<_>>();
    for index in indices {
        let binding = profile.layers[layer as usize][*index as usize];
        if (147..=149).contains(&binding.kind) && binding.code2 < 128 {
            let other = profile.layers[layer as usize][binding.code2 as usize];
            if other.kind == binding.kind && other.code2 == *index {
                reset.insert(binding.code2);
            }
        }
    }
    for index in reset {
        if profile.layers[layer as usize][index as usize].kind == 148 {
            profile.triggers[index as usize].priority = 0;
        }
        if let Some(key) = default_assignment(u16::from(index), profile.index, layer) {
            profile.layers[layer as usize][index as usize] = key;
        }
        profile.custom_params.combo_indices[layer as usize].retain(|value| *value != index);
    }
}

#[derive(Deserialize)]
struct LightEffect {
    id: String,
    value: u8,
    label_en: String,
    label_ru: String,
    brightness: bool,
    speed: bool,
    direction: bool,
    color: bool,
    direction_name: Option<String>,
}
impl LightEffect {
    fn direction_labels(&self) -> [&'static str; 2] {
        match self.direction_name.as_deref() {
            Some("row") => ["Слева направо", "Справа налево"],
            Some("col") => ["Сверху вниз", "Снизу вверх"],
            Some("spiral") => ["Изнутри наружу", "Снаружи внутрь"],
            _ => ["Прямое", "Обратное"],
        }
    }
    fn label(&self) -> &str {
        if i18n::language() == Language::En {
            &self.label_en
        } else {
            &self.label_ru
        }
    }
}
fn effect_options(scope: u8, firmware: u16) -> Vec<&'static LightEffect> {
    static EFFECTS: OnceLock<Vec<LightEffect>> = OnceLock::new();
    let all = EFFECTS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/ace75-light-effects.json"))
            .expect("Embedded ACE75 effects")
    });
    let keys: Vec<String> = match scope {
        1 => [
            "side-light-follow-light-mode-0",
            "side-light-flowing-light-mode-1",
            "side-light-neon-2",
            "side-light-breathing-3",
            "side-light-constant-on-4",
            "side-light-custom-5",
        ]
        .iter()
        .map(|key| (*key).into())
        .collect(),
        2 if firmware >= 115 => [
            "side-light2-follow-light-mode-0",
            "side-light-neon-1",
            "side-light-breathing-2",
            "side-light-constant-on-3",
        ]
        .iter()
        .map(|key| (*key).into())
        .collect(),
        2 => [
            "side-light-neon-0",
            "side-light-breathing-1",
            "side-light-constant-on-2",
        ]
        .iter()
        .map(|key| (*key).into())
        .collect(),
        _ => [
            0, 6, 7, 8, 9, 4, 1, 5, 3, 2, 11, 12, 10, 18, 19, 20, 21, 22, 14, 13, 15, 16, 17,
        ]
        .iter()
        .map(|id| format!("light-{id}"))
        .collect(),
    };
    keys.iter()
        .filter_map(|key| all.iter().find(|effect| &effect.id == key))
        .collect()
}
fn firmware_number(state: &KeyboardState) -> u16 {
    state
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.info.firmware_version.parse().ok())
        .unwrap_or(0)
}
fn switch_name(value: u8) -> &'static str {
    match value {
        0 => t("По умолчанию"),
        1 => "Taishan GT",
        2 => "Rage",
        3 => "Naruto V2",
        4 => "ztBm",
        5 => "Star Ring V2",
        _ => t("Неизвестный"),
    }
}
fn mm_slider(
    ui: &mut egui::Ui,
    raw: &mut u16,
    base: f64,
    range: std::ops::RangeInclusive<u16>,
    label: &str,
) {
    let mut value = f64::from(*raw) / base;
    if ui
        .add(
            egui::Slider::new(
                &mut value,
                f64::from(*range.start()) / base..=f64::from(*range.end()) / base,
            )
            .step_by(1.0 / base)
            .max_decimals(3)
            .suffix(t(" мм"))
            .text(label)
            .clamping(egui::SliderClamping::Edits),
        )
        .changed()
    {
        *raw = (value * base).round() as u16;
    }
}
fn rt_control(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    precision: &mut u8,
    value: &mut u16,
    max_travel: u16,
    tachyon: bool,
) {
    ui.push_id(id, |ui| {
        ui.add_enabled_ui(tachyon, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(label);
                for (code, name) in [(0, "0.01 mm"), (1, "0.005 mm"), (2, "0.001 mm")] {
                    let previous = *precision;
                    if ui.selectable_value(precision, code, name).changed() {
                        let old_base = precision_base(previous);
                        let new_base = precision_base(code);
                        *value = ((f64::from(*value) / old_base * new_base).round() as u16).clamp(
                            if tachyon { 1 } else { 2 },
                            if code == 0 { max_travel } else { 500 },
                        );
                    }
                }
            });
        });
        mm_slider(
            ui,
            value,
            precision_base(*precision),
            if tachyon { 1 } else { 2 }..=if *precision == 0 { max_travel } else { 500 },
            label,
        );
    });
}
fn precision_base(value: u8) -> f64 {
    match value {
        1 => 200.0,
        2 => 1000.0,
        _ => 100.0,
    }
}
fn deadzone_control(ui: &mut egui::Ui, value: &mut u8, label: &str) {
    ui.horizontal_wrapped(|ui| {
        let mut enabled = *value > 0;
        if ui.checkbox(&mut enabled, label).changed() {
            *value = if enabled { 15 } else { 0 };
        }
        if enabled {
            let mut raw = u16::from(*value);
            mm_slider(ui, &mut raw, 100.0, 1..=50, "");
            *value = raw as u8;
        }
    });
}
fn merge_trigger(target: &mut KeyTrigger, before: &KeyTrigger, after: &KeyTrigger) {
    macro_rules! field { ($($field:ident),* $(,)?) => { $(if before.$field != after.$field { target.$field = after.$field; })* }; }
    field!(
        switch_type,
        key_mode,
        priority,
        actuation,
        press_precision,
        release_precision,
        rt_press,
        rt_release,
        press_deadzone,
        release_deadzone
    );
}
fn normalize_functions(profile: &mut KeyboardProfile) -> anyhow::Result<()> {
    let raw = profile
        .functions
        .with_performance(&profile.functions.performance, true)?;
    let functions = FunctionConfig::decode(&raw)?;
    let raw = functions.with_lighting(&profile.functions.lighting)?;
    profile.functions = FunctionConfig::decode(&raw)?;
    Ok(())
}
fn merge_profile(draft: &mut KeyboardProfile, previous: &KeyboardProfile, fresh: &KeyboardProfile) {
    macro_rules! untouched { ($target:expr, $old:expr, $new:expr; $($field:ident),* $(,)?) => { $(if $target.$field == $old.$field { $target.$field = $new.$field.clone(); })* }; }
    for (index, layer) in draft.layers.iter_mut().enumerate() {
        for (key_index, key) in layer.iter_mut().enumerate() {
            if *key == previous.layers[index][key_index] {
                *key = fresh.layers[index][key_index];
            }
        }
    }
    for (index, target) in draft.triggers.iter_mut().enumerate() {
        let old = &previous.triggers[index];
        let new = &fresh.triggers[index];
        untouched!(target, old, new; switch_type, key_mode, priority, actuation, press_precision, release_precision, rt_press, rt_release, press_deadzone, release_deadzone);
    }
    for (target, old, new) in [
        (
            &mut draft.functions.lighting.main,
            &previous.functions.lighting.main,
            &fresh.functions.lighting.main,
        ),
        (
            &mut draft.functions.lighting.side,
            &previous.functions.lighting.side,
            &fresh.functions.lighting.side,
        ),
        (
            &mut draft.functions.lighting.side2,
            &previous.functions.lighting.side2,
            &fresh.functions.lighting.side2,
        ),
    ] {
        untouched!(target, old, new; effect, brightness, speed, direction, rainbow, color_index, color);
    }
    untouched!(draft.functions.lighting, previous.functions.lighting, fresh.functions.lighting; scope, calibration_color);
    untouched!(draft.functions.performance, previous.functions.performance, fresh.functions.performance; mac_mode, report_rate, scan_rate, dead_zone, lock_win, lock_alt_tab, lock_alt_f4, tachyon_mode, bottom_rapid_trigger, travel_check, debug_mode, debounce_level, rt_smart, battery_percent, charging, roller_type, work_mode, sleep_time, sleep_mode, bluetooth_channel, wireless_report_rate);
    draft.functions.raw = fresh.functions.raw.clone();
    draft.trigger_raw = fresh.trigger_raw.clone();
    for (index, color) in draft.colors.iter_mut().enumerate() {
        if *color == previous.colors[index] {
            *color = fresh.colors[index];
        }
    }
    untouched!(draft.custom_params, previous.custom_params, fresh.custom_params; press_precision, release_precision, rt_smart_cache, combo_indices);
    draft.custom_params.raw = fresh.custom_params.raw.clone();
    if draft.advanced == previous.advanced {
        draft.advanced = fresh.advanced.clone();
    }
    draft.custom_raw = fresh.custom_raw.clone();
}
fn default_assignment(index: u16, profile: u8, layer: u8) -> Option<KeyAssignment> {
    static DEFAULTS: OnceLock<Vec<Vec<[u8; 3]>>> = OnceLock::new();
    let keys = DEFAULTS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/ace75-default-key-layers.json"))
            .expect("Embedded ACE75 defaults")
    });
    let value = keys
        .get(usize::from(profile) * 4 + usize::from(layer))?
        .get(usize::from(index))?;
    Some(KeyAssignment {
        kind: value[0],
        code1: value[1],
        code2: value[2],
    })
}
#[derive(Deserialize)]
struct KeyAction {
    id: String,
    category: String,
    name_en: String,
    name_ru: String,
    kind: u8,
    code1: u8,
    code2: u8,
    layers: Vec<u8>,
    disabled_advanced_types: Vec<String>,
}
impl KeyAction {
    fn assignment(&self) -> KeyAssignment {
        KeyAssignment {
            kind: self.kind,
            code1: self.code1,
            code2: self.code2,
        }
    }
    fn label(&self) -> &str {
        if i18n::language() == Language::En {
            &self.name_en
        } else {
            &self.name_ru
        }
    }
}
fn key_actions() -> &'static [KeyAction] {
    static ACTIONS: OnceLock<Vec<KeyAction>> = OnceLock::new();
    ACTIONS.get_or_init(|| {
        serde_json::from_str(include_str!("../../assets/ace75-actions.json"))
            .expect("Embedded ACE75 actions")
    })
}
fn key_label(value: KeyAssignment, layer: u8) -> String {
    if let Some(action) = key_actions()
        .iter()
        .find(|action| action.layers.contains(&layer) && action.assignment() == value)
    {
        return action.label().into();
    }
    if value.kind == 16 {
        let mut parts: Vec<String> = [
            "LCtrl", "LShift", "LAlt", "LWin", "RCtrl", "RShift", "RAlt", "RWin",
        ]
        .iter()
        .enumerate()
        .filter(|(bit, _)| value.code1 & (1 << bit) != 0)
        .map(|(_, name)| (*name).into())
        .collect();
        if value.code2 != 0 {
            parts.push(
                key_actions()
                    .iter()
                    .find(|action| {
                        action.kind == 16
                            && action.code1 == 0
                            && action.code2 == value.code2
                            && action.layers.contains(&layer)
                    })
                    .map(|action| action.label().to_owned())
                    .unwrap_or_else(|| format!("HID {:02X}", value.code2)),
            );
        }
        if !parts.is_empty() {
            return parts.join(" + ");
        }
    }
    format!(
        "{:02X} · {:02X} · {:02X}",
        value.kind, value.code1, value.code2
    )
}

fn contrast_text(color: Color32) -> Color32 {
    let channel = |value: u8| {
        let value = f32::from(value) / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance =
        0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b());
    if luminance > 0.179 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

fn toggle_selection(selected: &mut BTreeSet<u16>, index: u16, additive: bool) {
    if !additive {
        selected.clear();
        selected.insert(index);
    } else if !selected.remove(&index) {
        selected.insert(index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_fixture() -> KeyboardProfile {
        let mut raw = vec![0; 64];
        raw[4] = 1;
        raw[9] = 100;
        raw[25] = 100;
        raw[45] = 100;
        KeyboardProfile {
            index: 0,
            functions: FunctionConfig::decode(&raw).unwrap(),
            layers: (0..4)
                .map(|layer| {
                    (0..128)
                        .map(|index| default_assignment(index, 0, layer).unwrap())
                        .collect()
                })
                .collect(),
            triggers: KeyTrigger::decode_all(&vec![0; 1024]).unwrap(),
            trigger_raw: vec![0; 1024],
            colors: vec![[0, 0, 0]; 128],
            custom_params: crate::ace75::CustomParams::decode(&[0; 56]).unwrap(),
            advanced: crate::ace75::AdvancedState::decode(&[0; 768], &[0; 256], &[0; 128]).unwrap(),
            custom_raw: vec![255; 1024],
        }
    }

    #[test]
    fn merely_rendering_settings_preserves_out_of_slider_range_device_values() {
        let context = egui::Context::default();
        let state = KeyboardState::default();
        let profile = profile_fixture();
        let mut editor = Ace75Ui {
            draft: Some(profile.clone()),
            baseline: Some(profile.clone()),
            selected_keys: [88].into_iter().collect(),
            ..Default::default()
        };
        for page in [Page::Keys, Page::Trigger, Page::Performance, Page::Lighting] {
            editor.page = page;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(615.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = context.run(input, |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    editor.show(ui, Palette::new(true), &state, false, false)
                });
            });
            assert_eq!(editor.draft.as_ref(), Some(&profile));
            assert!(!editor.dirty());
            assert!(editor.take_action().is_none());
        }
    }

    #[test]
    fn device_refresh_preserves_draft_and_updates_untouched_fields() {
        let previous = profile_fixture();
        let mut draft = previous.clone();
        draft.triggers[88].actuation = 180;
        draft.functions.performance.report_rate = 3;
        let mut fresh = previous.clone();
        fresh.functions.lighting.main.brightness = 31;
        fresh.functions.performance.sleep_time = 14;
        fresh.triggers[88].rt_press = 42;
        fresh.colors[88] = [20, 30, 40];
        merge_profile(&mut draft, &previous, &fresh);
        assert_eq!(draft.triggers[88].actuation, 180);
        assert_eq!(draft.triggers[88].rt_press, 42);
        assert_eq!(draft.functions.performance.report_rate, 3);
        assert_eq!(draft.functions.performance.sleep_time, 14);
        assert_eq!(draft.functions.lighting.main.brightness, 31);
        assert_eq!(draft.colors[88], [20, 30, 40]);
    }

    #[test]
    fn rendering_disconnected_pages_emits_no_device_commands() {
        let context = egui::Context::default();
        let state = KeyboardState::default();
        let mut editor = Ace75Ui::default();
        for page in [
            Page::Keys,
            Page::Trigger,
            Page::Advanced,
            Page::Performance,
            Page::Lighting,
            Page::Macros,
            Page::Profiles,
            Page::Device,
        ] {
            editor.page = page;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(615.0, 600.0),
                )),
                ..Default::default()
            };
            let _ = context.run(input, |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    editor.show(ui, Palette::new(true), &state, false, false)
                });
            });
            assert!(editor.take_action().is_none());
        }
    }

    #[test]
    fn effect_lists_match_model_and_firmware() {
        assert_eq!(
            effect_options(0, 117)
                .iter()
                .map(|effect| effect.value)
                .collect::<Vec<_>>(),
            [
                0, 6, 7, 8, 9, 4, 1, 5, 3, 2, 11, 12, 10, 18, 19, 20, 21, 22, 14, 13, 15, 16, 17
            ]
        );
        assert_eq!(effect_options(1, 117).len(), 6);
        assert_eq!(effect_options(2, 114).len(), 3);
        assert_eq!(effect_options(2, 115).len(), 4);
        assert!(!effect_options(2, 115)[0].brightness);
    }

    #[test]
    fn onboard_default_sentinels_are_localized_only_for_display() {
        for (profile, stored) in [
            (0, "i18n<defaultOnboard>"),
            (1, "i18n<defaultOnboard>2"),
            (2, "i18n<defaultOnboard>3"),
        ] {
            assert_eq!(
                onboard_profile_name(stored, profile),
                tf!("Профиль {}", profile + 1)
            );
        }
        assert_eq!(onboard_profile_name("Gaming", 0), "Gaming");
        assert_eq!(
            onboard_profile_name("i18n<defaultOnboard>custom", 0),
            "i18n<defaultOnboard>custom"
        );
    }

    #[test]
    fn renamed_or_deleted_open_macro_updates_its_file_binding() {
        let old_path = PathBuf::from("old.json");
        let new_path = PathBuf::from("new.json");
        let mut editor = Ace75Ui::default();
        editor.new_local_macro();
        editor.local_macro_bank.as_mut().unwrap().macros[0].name = "old".into();
        editor.local_macro_original = editor.local_macro_bank.clone();
        editor.local_macro_path = Some(old_path.clone());
        let mut state = KeyboardState::default();
        state.macros = vec![crate::controller::PresetInfo {
            name: "new".into(),
            path: new_path.clone(),
        }];
        editor.command_started(&KeyboardCommand::RenameLocalMacro {
            path: old_path,
            name: "new".into(),
        });
        editor.sync(&state, false);
        assert_eq!(editor.local_macro_path.as_ref(), Some(&new_path));
        assert_eq!(
            editor.local_macro_bank.as_ref().unwrap().macros[0].name,
            "new"
        );
        assert!(!editor.macros_dirty());
        editor.command_started(&KeyboardCommand::DeleteLocalMacro(new_path));
        editor.sync(&state, false);
        assert!(editor.local_macro_path.is_none());
        assert!(editor.macros_dirty());
    }

    #[test]
    fn restore_defaults_keep_profile_and_os_layers() {
        assert_eq!(default_assignment(22, 0, 0).unwrap().code2, 1);
        assert_eq!(default_assignment(22, 0, 2).unwrap().code2, 3);
        assert!(default_assignment(0, 4, 0).is_none());
        assert!(default_assignment(128, 0, 0).is_none());
        for profile in 0..4 {
            for layer in 0..4 {
                assert!(default_assignment(127, profile, layer).is_some());
            }
        }
    }

    #[test]
    fn trigger_bulk_edit_changes_only_edited_fields() {
        let before = KeyTrigger {
            switch_type: 0,
            key_mode: 1,
            priority: 0,
            actuation: 150,
            press_precision: 0,
            release_precision: 1,
            rt_press: 10,
            rt_release: 12,
            press_deadzone: 15,
            release_deadzone: 20,
        };
        let mut after = before.clone();
        after.actuation = 250;
        let mut other = before.clone();
        other.rt_press = 27;
        other.press_precision = 2;
        merge_trigger(&mut other, &before, &after);
        assert_eq!(other.actuation, 250);
        assert_eq!((other.rt_press, other.press_precision), (27, 2));
        assert_eq!(precision_base(0), 100.0);
        assert_eq!(precision_base(1), 200.0);
        assert_eq!(precision_base(2), 1000.0);
    }

    #[test]
    fn advanced_apply_preserves_unapplied_trigger_key_and_global_edits() {
        let mut original = profile_fixture();
        original.layers[0][89] = KeyAssignment {
            kind: 144,
            code1: 0,
            code2: 0,
        };
        let mut draft = original.clone();
        let normal = KeyAssignment {
            kind: 16,
            code1: 0,
            code2: 5,
        };
        draft.layers[0][88] = normal;
        draft.layers[0][89] = normal;
        draft.layers[1][90] = normal;
        for keys in &mut draft.layers {
            keys[93] = normal;
        }
        draft.triggers[88].actuation = 250;
        draft.triggers[89].priority = 2;
        let editor = Ace75Ui {
            draft: Some(draft),
            baseline: Some(original.clone()),
            ..Default::default()
        };
        let Some(KeyboardCommand::ApplyAdvanced {
            layers, triggers, ..
        }) = editor.advanced_command("test".into(), 0, None)
        else {
            panic!("Advanced command expected");
        };
        assert_eq!(layers[0][88], original.layers[0][88]);
        assert_eq!(layers[0][89], normal);
        assert_eq!(layers[1][90], original.layers[1][90]);
        assert_eq!(layers[0][93], original.layers[0][93]);
        assert_eq!(triggers[88].actuation, original.triggers[88].actuation);
        assert_eq!(triggers[89].priority, 2);
        let Some(KeyboardCommand::ApplyAdvanced { layers, .. }) =
            editor.advanced_command("test".into(), 0, Some(0))
        else {
            panic!("Advanced command expected");
        };
        assert_eq!(layers[0][88], normal);
        assert_eq!(layers[1][90], original.layers[1][90]);
    }

    #[test]
    fn tachyon_change_belongs_to_trigger_apply() {
        let original = profile_fixture();
        let mut draft = original.clone();
        draft.functions.performance.tachyon_mode = !original.functions.performance.tachyon_mode;
        let editor = Ace75Ui {
            draft: Some(draft),
            baseline: Some(original),
            ..Default::default()
        };
        assert!(editor.part_dirty(Page::Trigger));
        assert!(!editor.part_dirty(Page::Performance));
    }

    #[test]
    fn official_layout_separates_keys_controls_and_led_regions() {
        let layout = layout();
        assert_eq!(
            (
                layout.keys.len(),
                layout.controls.len(),
                layout.hidden_actions.len(),
                layout.lighting_regions.len()
            ),
            (81, 2, 1, 34)
        );
        assert_eq!(cells(GridPurpose::Assignment).count(), 83);
        assert_eq!(cells(GridPurpose::Trigger).count(), 81);
        assert_eq!(cells(GridPurpose::Performance).count(), 81);
        assert_eq!(cells(GridPurpose::Advanced).count(), 81);
        assert_eq!(cells(GridPurpose::Lighting).count(), 114);
    }

    #[test]
    fn geometry_uses_matrix_addresses_not_placeholder_indices() {
        let layout = layout();
        for (code, index) in [(4, 88), (44, 13), (255, 22), (252, 93), (3809, 92)] {
            assert_eq!(
                layout
                    .keys
                    .iter()
                    .chain(&layout.controls)
                    .find(|cell| cell.code == code)
                    .unwrap()
                    .matrix_index,
                index
            );
        }
        assert_eq!(layout.hidden_actions[0].matrix_index, 94);
        assert_eq!(layout.hidden_actions[0].parent_matrix_index, 93);
        assert!(!cells(GridPurpose::Assignment).any(|cell| cell.matrix_index == 94));
    }

    #[test]
    fn each_visible_lighting_cell_has_one_matrix_address() {
        let indices = cells(GridPurpose::Lighting)
            .map(|cell| cell.matrix_index)
            .collect::<BTreeSet<_>>();
        assert_eq!(indices.len(), 114);
        assert!(indices.is_superset(&(95..=123).collect()));
        assert!(indices.is_superset(&[3, 12, 13, 23, 25].into_iter().collect()));
    }

    #[test]
    fn additive_selection_preserves_unrelated_keys() {
        let mut selected = BTreeSet::new();
        toggle_selection(&mut selected, 88, false);
        toggle_selection(&mut selected, 91, true);
        assert_eq!(selected, [88, 91].into_iter().collect());
        toggle_selection(&mut selected, 88, true);
        assert_eq!(selected, [91].into_iter().collect());
        toggle_selection(&mut selected, 22, false);
        assert_eq!(selected, [22].into_iter().collect());
    }

    fn grid_frame(
        context: &egui::Context,
        editor: &mut Ace75Ui,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(855.0, 700.0),
            )),
            events,
            modifiers,
            ..Default::default()
        };
        let _ = context.run(input, |context| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(context, |ui| {
                    editor.show_grid(ui, Palette::new(true), GridPurpose::Assignment)
                });
        });
    }

    #[test]
    fn native_pointer_selection_keeps_matrix_addresses() {
        let context = egui::Context::default();
        let mut editor = Ace75Ui::default();
        grid_frame(&context, &mut editor, vec![], egui::Modifiers::NONE);
        for (code, additive) in [(4, false), (7, true)] {
            let cell = layout().keys.iter().find(|cell| cell.code == code).unwrap();
            let position = egui::pos2(
                cell.x + cell.width / 2.0,
                layout().canvas.top_inset + cell.y + cell.height / 2.0,
            );
            let modifiers = egui::Modifiers {
                ctrl: additive,
                ..Default::default()
            };
            for pressed in [true, false] {
                grid_frame(
                    &context,
                    &mut editor,
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers,
                        },
                    ],
                    modifiers,
                );
            }
        }
        assert_eq!(editor.selected_keys, [88, 91].into_iter().collect());
        assert!(editor.selected_lights.is_empty());
    }

    #[test]
    fn grid_reserves_its_height_once() {
        let context = egui::Context::default();
        let mut editor = Ace75Ui::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(855.0, 700.0),
            )),
            ..Default::default()
        };
        let _ = context.run(input, |context| {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(context, |ui| {
                    let before = ui.cursor().top();
                    let expected = layout().canvas.height
                        + layout().canvas.top_inset
                        + ui.spacing().item_spacing.y;
                    editor.show_grid(ui, Palette::new(true), GridPurpose::Assignment);
                    assert!((ui.cursor().top() - before - expected).abs() <= 1.0);
                });
        });
    }

    #[test]
    fn thin_strip_segments_do_not_intercept_number_row_clicks() {
        let context = egui::Context::default();
        context.style_mut(|style| style.spacing.interact_size.y = 36.0);
        let mut editor = Ace75Ui::default();
        let number = layout()
            .keys
            .iter()
            .find(|cell| cell.x == 35.0 && cell.y == 80.0)
            .unwrap();
        let position = egui::pos2(
            number.x + number.width / 2.0,
            layout().canvas.top_inset + number.y + 2.0,
        );
        for pressed in [None, Some(true), Some(false)] {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(855.0, 700.0),
                )),
                events: pressed.map_or_else(Vec::new, |pressed| {
                    vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]
                }),
                ..Default::default()
            };
            let _ = context.run(input, |context| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(context, |ui| {
                        editor.show_grid(ui, Palette::new(true), GridPurpose::Lighting)
                    });
            });
        }
        assert_eq!(
            editor.selected_lights,
            [number.matrix_index].into_iter().collect()
        );
    }

    #[test]
    fn key_grid_can_be_selected_with_tab_and_enter() {
        let context = egui::Context::default();
        let mut editor = Ace75Ui::default();
        grid_frame(&context, &mut editor, vec![], egui::Modifiers::NONE);
        for key in [egui::Key::Tab, egui::Key::Enter] {
            for pressed in [true, false] {
                grid_frame(
                    &context,
                    &mut editor,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    egui::Modifiers::NONE,
                );
            }
        }
        assert_eq!(editor.selected_keys, [31].into_iter().collect());
    }
}
