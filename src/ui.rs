use crate::controller::{Command, Controller, ProfileFile};
use crate::dock::{DockMode, DockSettings};
use crate::mouse::{DpiSettings, KeyBinding, MacroEvent, MouseMacro, MouseSettings};
use crate::i18n::{self, Language, t};
use crate::tf;
use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use std::path::PathBuf;
use std::time::{Duration, Instant};

mod ace75;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeviceView {
    Mouse,
    Ace75,
}

impl DeviceView {
    fn code(self) -> &'static str {
        match self { Self::Mouse => "a7", Self::Ace75 => "ace75" }
    }

    fn parse(value: &str) -> Option<Self> {
        match value { "a7" | "mouse" => Some(Self::Mouse), "ace75" | "keyboard" => Some(Self::Ace75), _ => None }
    }

    fn label(self) -> &'static str {
        match self { Self::Mouse => "A7 V3 Ultra+", Self::Ace75 => "ACE75" }
    }
}

fn preferred_device_view() -> Option<DeviceView> {
    let path = crate::controller::data_dir().join("ui-preferences.json");
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    DeviceView::parse(value.get("device")?.as_str()?)
}

const RATES: [u32; 6] = [125, 500, 1000, 2000, 4000, 8000];
const LODS: [&str; 5] = ["0,7 мм", "0,9 мм", "1,2 мм", "1,4 мм", "1,7 мм"];
const BUTTONS: [&str; 6] = [
    "Левая кнопка",
    "Правая кнопка",
    "Колесо",
    "Вперёд",
    "Назад",
    "Кнопка DPI",
];
const STAGE_COLORS: [Color32; 6] = [
    Color32::from_rgb(234, 94, 86),
    Color32::from_rgb(0, 83, 226),
    Color32::from_rgb(50, 170, 112),
    Color32::from_rgb(245, 224, 37),
    Color32::from_rgb(0, 221, 228),
    Color32::from_rgb(181, 0, 236),
];

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Keys,
    Dpi,
    Performance,
    Dock,
    Macros,
    Profiles,
    System,
}
impl Page {
    fn title(self) -> &'static str {
        t(match self {
            Self::Keys => "Кнопки",
            Self::Dpi => "DPI",
            Self::Performance => "Производительность",
            Self::Dock => "Подсветка",
            Self::Macros => "Макросы",
            Self::Profiles => "Профили",
            Self::System => "Устройство",
        })
    }
}
#[derive(Clone, Copy)]
struct Palette {
    outer: Color32,
    panel: Color32,
    field: Color32,
    text: Color32,
    muted: Color32,
    border: Color32,
    accent: Color32,
    accent_text: Color32,
    error_text: Color32,
}
impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                outer: Color32::from_rgb(12, 12, 12),
                panel: Color32::from_rgb(20, 20, 20),
                field: Color32::from_rgb(26, 26, 27),
                text: Color32::WHITE,
                muted: Color32::from_gray(158),
                border: Color32::from_gray(43),
                accent: Color32::from_rgb(34, 107, 238),
                accent_text: Color32::from_rgb(117, 167, 255),
                error_text: Color32::from_rgb(234, 94, 86),
            }
        } else {
            Self {
                outer: Color32::from_rgb(230, 231, 237),
                panel: Color32::WHITE,
                field: Color32::from_rgb(242, 243, 247),
                text: Color32::from_rgb(21, 21, 21),
                muted: Color32::from_rgb(100, 103, 111),
                border: Color32::from_rgb(215, 222, 227),
                accent: Color32::from_rgb(0, 83, 226),
                accent_text: Color32::from_rgb(0, 83, 226),
                error_text: Color32::from_rgb(180, 35, 35),
            }
        }
    }
}

pub struct NativeApp {
    controller: Controller,
    device_view: DeviceView,
    auto_device_view: bool,
    ace75: ace75::Ace75Ui,
    page: Page,
    dark: bool,
    theme_pending: bool,
    read_only: bool,
    offline: bool,
    textures: Vec<egui::TextureHandle>,
    dock_texture: egui::TextureHandle,
    brand_texture: egui::TextureHandle,
    rust_texture: egui::TextureHandle,
    skin: usize,
    staged_settings: Option<MouseSettings>,
    staged_dpi: Option<DpiSettings>,
    staged_keys: Vec<KeyBinding>,
    staged_dock: Option<DockSettings>,
    staged_lod: u8,
    synced_at: Option<Instant>,
    dirty_settings: bool,
    dirty_dpi: bool,
    dirty_keys: bool,
    dirty_dock: bool,
    selected_button: usize,
    allow_left: bool,
    key_category: usize,
    key_search: String,
    key_usage: u8,
    key_modifiers: u8,
    macro_value: MouseMacro,
    macro_slot: u8,
    macro_path: String,
    macro_seen: Option<(u8, MouseMacro)>,
    local_macro_seen: Option<MouseMacro>,
    local_macro_name: String,
    local_macro_rename: String,
    imported_macros: Vec<MouseMacro>,
    recording: bool,
    recorder: Option<crate::recorder::WindowKeyboardRecorder>,
    recorded_at: Option<Instant>,
    record_area: egui::Rect,
    record_modifiers: u8,
    record_held: std::collections::BTreeSet<(u8, u8)>,
    fixed_delay: bool,
    delay_ms: u16,
    macro_dirty: bool,
    profile_name: String,
    profile_path: String,
    preset_name: String,
    preview_path: Option<PathBuf>,
    preview_pending: Option<PathBuf>,
    preset_edit: Option<(PathBuf, ProfileFile)>,
    preset_saving: Option<ProfileFile>,
    dirty_preset: bool,
    firmware_path: String,
    firmware_sub_path: String,
    firmware_device: String,
    firmware_target: usize,
    firmware_dir: String,
    firmware_info: String,
    local_message: String,
    pending: Option<(String, Command)>,
    applying: Option<(&'static str, Option<Instant>)>,
    capture_path: Option<PathBuf>,
    capture_frames: u32,
    capture_requested: bool,
}

impl NativeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, read_only: bool, offline: bool) -> Self {
        let preferred_view = preferred_device_view();
        let textures = [
            (
                "white",
                include_bytes!("../assets/mouse-white.png").as_slice(),
            ),
            (
                "black",
                include_bytes!("../assets/mouse-black.png").as_slice(),
            ),
            (
                "blue",
                include_bytes!("../assets/mouse-blue.png").as_slice(),
            ),
            ("red", include_bytes!("../assets/mouse-red.png").as_slice()),
            (
                "pink",
                include_bytes!("../assets/mouse-pink.png").as_slice(),
            ),
        ]
        .into_iter()
        .map(|(name, data)| load_texture(&cc.egui_ctx, name, data))
        .collect();
        let mut app = Self {
            controller: if offline {
                Controller::new_offline()
            } else {
                Controller::new(read_only)
            },
            device_view: preferred_view.unwrap_or(DeviceView::Mouse),
            auto_device_view: preferred_view.is_none(),
            ace75: ace75::Ace75Ui::default(),
            page: Page::Keys,
            dark: true,
            theme_pending: false,
            read_only,
            offline,
            textures,
            dock_texture: load_texture(
                &cc.egui_ctx,
                "magdock",
                include_bytes!("../assets/magdock.png"),
            ),
            brand_texture: load_texture(&cc.egui_ctx, "mchose-brand", include_bytes!("../assets/mchose-icon.png")),
            rust_texture: load_texture(&cc.egui_ctx, "rust-brand", include_bytes!("../assets/rust-logo-128.png")),
            skin: 0,
            staged_settings: None,
            staged_dpi: None,
            staged_keys: Vec::new(),
            staged_dock: None,
            staged_lod: 0,
            synced_at: None,
            dirty_settings: false,
            dirty_dpi: false,
            dirty_keys: false,
            dirty_dock: false,
            selected_button: 0,
            allow_left: false,
            key_category: 0,
            key_search: String::new(),
            key_usage: 4,
            key_modifiers: 0,
            macro_value: MouseMacro {
                name: t("Новый макрос").into(),
                repeat_mode: 0,
                events: Vec::new(),
                reserved: [0; 5],
            },
            macro_slot: 0,
            macro_path: default_path("macro.json"),
            macro_seen: None,
            local_macro_seen: None,
            local_macro_name: String::new(),
            local_macro_rename: String::new(),
            imported_macros: Vec::new(),
            recording: false,
            recorder: None,
            recorded_at: None,
            record_area: egui::Rect::NOTHING,
            record_modifiers: 0,
            record_held: Default::default(),
            fixed_delay: false,
            delay_ms: 50,
            macro_dirty: false,
            profile_name: String::new(),
            profile_path: default_path("profile.json"),
            preset_name: String::new(),
            preview_path: None,
            preview_pending: None,
            preset_edit: None,
            preset_saving: None,
            dirty_preset: false,
            firmware_path: String::new(),
            firmware_sub_path: String::new(),
            firmware_device: String::new(),
            firmware_target: 0,
            firmware_dir: default_path("firmware"),
            firmware_info: String::new(),
            local_message: String::new(),
            pending: None,
            applying: None,
            capture_path: None,
            capture_frames: 0,
            capture_requested: false,
        };
        app.style(&cc.egui_ctx);
        app.sync();
        app
    }
    fn style(&self, ctx: &egui::Context) {
        ctx.set_theme(if self.dark { egui::ThemePreference::Dark } else { egui::ThemePreference::Light });
        let p = Palette::new(self.dark);
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.panel_fill = p.outer;
        visuals.window_fill = p.panel;
        visuals.extreme_bg_color = p.field;
        visuals.faint_bg_color = p.field;
        visuals.override_text_color = None;
        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
        visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, p.text);
        visuals.slider_trailing_fill = true;
        visuals.handle_shape = egui::style::HandleShape::Circle;
        visuals.widgets.noninteractive.bg_fill = p.field;
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
        visuals.widgets.inactive.bg_fill = if self.dark {
            Color32::from_rgb(118, 123, 135)
        } else {
            Color32::from_rgb(116, 123, 138)
        };
        visuals.widgets.inactive.weak_bg_fill = p.field;
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
        visuals.widgets.hovered.bg_fill = p.accent.gamma_multiply(0.2);
        visuals.widgets.hovered.weak_bg_fill = p.accent.gamma_multiply(0.2);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
        visuals.widgets.active.bg_fill = p.accent;
        visuals.widgets.active.weak_bg_fill = p.accent.gamma_multiply(0.25);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
        visuals.selection.bg_fill = p.accent;
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
        for widgets in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widgets.corner_radius = egui::CornerRadius::same(8);
        }
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(10.0, 10.0);
            s.spacing.button_padding = Vec2::new(14.0, 9.0);
            s.spacing.interact_size = Vec2::new(36.0, 36.0);
            s.spacing.slider_width = 220.0;
            s.spacing.slider_rail_height = 6.0;
            s.text_styles
                .insert(egui::TextStyle::Body, FontId::proportional(14.0));
            s.text_styles
                .insert(egui::TextStyle::Button, FontId::proportional(14.0));
            s.text_styles
                .insert(egui::TextStyle::Small, FontId::proportional(12.0));
            s.text_styles
                .insert(egui::TextStyle::Heading, FontId::proportional(22.0));
        });
    }
    fn sync(&mut self) {
        self.ace75.sync(&self.controller.snapshot.ace75, self.controller.snapshot.busy);
        if self.auto_device_view && !self.controller.snapshot.busy {
            self.device_view = if self.controller.snapshot.mouse.is_some() { DeviceView::Mouse } else { DeviceView::Ace75 };
            self.auto_device_view = false;
        }
        if self.recording
            && (self.local_macro_seen != self.controller.snapshot.local_macro_value
                || !self.macro_dirty && self.macro_seen != self.controller.snapshot.macro_value)
        {
            self.stop_recording(None);
        }
        if self.preview_pending.is_some() && !self.controller.snapshot.busy {
            let path = self.preview_pending.take();
            if self.controller.snapshot.error.is_none() {
                self.preview_path = path;
            }
        }
        if self.preset_saving.is_some() && !self.controller.snapshot.busy {
            let saved = self.preset_saving.take().unwrap();
            if self.controller.snapshot.error.is_none()
                && serde_json::to_value(&self.controller.snapshot.preview).ok()
                    == serde_json::to_value(Some(&saved)).ok()
            {
                if let Some((_, value)) = &mut self.preset_edit {
                    *value = saved;
                }
                self.dirty_keys = false;
                self.dirty_dpi = false;
                self.dirty_settings = false;
                self.dirty_preset = false;
            }
        }
        let snap = &self.controller.snapshot;
        if let Some((kind, started)) = self.applying {
            if !snap.busy && (snap.error.is_some() || snap.last_update != started) {
                if snap.error.is_none() {
                    match kind {
                        "settings" => {
                            if let (Some(expected), Some(actual)) =
                                (&self.staged_settings, &snap.mouse)
                            {
                                if expected.encode().ok() == actual.settings.encode().ok()
                                    && self.staged_lod == actual.lod_index
                                {
                                    self.dirty_settings = false;
                                }
                            }
                        }
                        "dpi" => {
                            if let (Some(expected), Some(actual)) = (&self.staged_dpi, &snap.mouse)
                            {
                                if expected.encode(0).ok() == actual.dpi.encode(0).ok()
                                    && expected.encode(1).ok() == actual.dpi.encode(1).ok()
                                {
                                    self.dirty_dpi = false;
                                }
                            }
                        }
                        "keys" => {
                            if let Some(actual) = &snap.mouse {
                                if self.staged_keys == actual.keys {
                                    self.dirty_keys = false;
                                }
                            }
                        }
                        "dock" => {
                            if let (Some(expected), Some(actual)) = (&self.staged_dock, &snap.dock)
                            {
                                if expected.lighting == actual.lighting {
                                    self.dirty_dock = false;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                self.applying = None;
            }
        }
        if snap.last_update != self.synced_at {
            self.synced_at = snap.last_update;
            if let Some(mouse) = snap.mouse.as_ref().filter(|_| self.preset_edit.is_none()) {
                if !self.dirty_settings {
                    self.staged_settings = Some(mouse.settings.clone());
                    self.staged_lod = mouse.lod_index;
                }
                if !self.dirty_dpi {
                    self.staged_dpi = Some(mouse.dpi.clone());
                }
                if !self.dirty_keys {
                    self.staged_keys = mouse.keys.clone();
                }
                if self.profile_name.is_empty() {
                    self.profile_name = mouse
                        .profile_names
                        .get(mouse.settings.profile_index as usize)
                        .cloned()
                        .unwrap_or_default();
                }
            }
            if !self.dirty_dock {
                self.staged_dock = snap.dock.clone();
            }
        }
        if !self.macro_dirty && self.macro_seen != snap.macro_value {
            self.macro_seen = snap.macro_value.clone();
            if let Some((slot, value)) = &snap.macro_value {
                self.macro_slot = *slot;
                self.macro_value = value.clone();
            }
        }
        if self.local_macro_seen != snap.local_macro_value {
            self.local_macro_seen = snap.local_macro_value.clone();
            if let Some(value) = &snap.local_macro_value {
                self.macro_value = value.clone();
                self.local_macro_name = value.name.clone();
                self.local_macro_rename = value.name.clone();
                self.macro_dirty = true;
            } else {
                self.local_macro_name.clear();
                self.local_macro_rename.clear();
            }
        }
    }
    pub fn select_page(&mut self, name: &str) {
        self.ace75.select_page(name);
        self.page = match name {
            "dpi" => Page::Dpi,
            "performance" => Page::Performance,
            "dock" | "lighting" => Page::Dock,
            "macros" => Page::Macros,
            "profiles" => Page::Profiles,
            "system" | "firmware" => Page::System,
            _ => Page::Keys,
        };
    }
    pub fn select_device(&mut self, name: &str) {
        let Some(view) = DeviceView::parse(name) else { return; };
        if self.device_view != view {
            if view != DeviceView::Ace75 {
                if let Some(command) = self.ace75.hide(&self.controller.snapshot.ace75) { self.command(Command::Ace75(command)); }
            } else { self.ace75.stop_recording(None); }
        }
        if self.device_view != view && self.recording {
            self.stop_recording(None);
        }
        self.device_view = view;
        self.auto_device_view = false;
        if let Err(error) = i18n::save_ui_preference("device", serde_json::json!(view.code())) {
            self.local_message = format!("Не удалось сохранить выбор устройства: {error}");
        }
    }
    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        self.theme_pending = true;
    }
    pub fn set_language(&mut self, code: &str) {
        let Some(language) = Language::parse(code) else { return; };
        i18n::set_language(language);
        if let Err(error) = i18n::save_preference(language) {
            self.local_message = format!("Не удалось сохранить язык: {error}");
        }
    }
    pub fn capture_to(&mut self, path: PathBuf) {
        self.capture_path = Some(path);
        self.capture_frames = 0;
    }
    fn writable(&self) -> bool {
        !self.read_only
            && !self.offline
            && !self.controller.snapshot.busy
            && self.preset_edit.is_none()
    }
    fn command(&mut self, cmd: Command) {
        if let Command::Ace75(command) = &cmd { self.ace75.command_started(command); }
        if let Command::PreviewPreset(path) = &cmd {
            self.preview_pending = Some(path.clone());
        }
        if let Command::SaveEditedPreset { value, .. } = &cmd {
            self.preset_saving = Some(value.clone());
        }
        self.local_message.clear();
        self.controller.send(cmd);
    }
    fn begin_preset_edit(&mut self, path: PathBuf, value: ProfileFile) {
        if self.has_dirty() {
            self.local_message = "Сначала примените или отмените текущие изменения.".into();
            return;
        }
        self.staged_settings = Some(value.snapshot.settings.clone());
        self.staged_dpi = Some(value.snapshot.dpi.clone());
        self.staged_keys = value.snapshot.keys.clone();
        self.staged_lod = value.snapshot.lod_index;
        self.preset_edit = Some((path, value));
        self.page = Page::Keys;
    }
    fn edited_preset_value(&self) -> Option<ProfileFile> {
        let mut value = self.preset_edit.as_ref()?.1.clone();
        value.snapshot.settings = self.staged_settings.clone()?;
        value.snapshot.dpi = self.staged_dpi.clone()?;
        value.snapshot.settings.dpi_index = value.snapshot.dpi.selected;
        value.snapshot.keys = self.staged_keys.clone();
        value.snapshot.lod_index = self.staged_lod;
        Some(value)
    }
    fn save_edited_preset(&mut self) {
        if let (Some((path, _)), Some(value)) = (&self.preset_edit, self.edited_preset_value()) {
            self.pending = Some((
                format!(
                    "Сохранить изменения в локальный профиль «{}»? Исходная версия сохранится в резервной папке.",
                    path.file_stem().unwrap_or_default().to_string_lossy()
                ),
                Command::SaveEditedPreset {
                    path: path.clone(),
                    value,
                },
            ));
        }
    }
    fn close_preset_edit(&mut self) {
        self.preset_edit = None;
        self.dirty_keys = false;
        self.dirty_dpi = false;
        self.dirty_settings = false;
        self.dirty_preset = false;
        self.staged_settings = None;
        self.staged_dpi = None;
        self.staged_keys.clear();
        self.synced_at = None;
        self.sync();
        self.page = Page::Profiles;
    }
    fn apply(&mut self, kind: &'static str, cmd: Command) {
        self.applying = Some((kind, self.controller.snapshot.last_update));
        self.command(cmd);
    }
    fn top(&mut self, ctx: &egui::Context) {
        let p = Palette::new(self.dark);
        egui::TopBottomPanel::top("hub-header")
            .exact_height(60.0)
            .frame(
                egui::Frame::new()
                    .fill(p.outer)
                    .inner_margin(egui::Margin::symmetric(24, 10)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Image::new(&self.brand_texture).fit_to_exact_size(Vec2::splat(28.0)));
                    ui.label(RichText::new("MCHOSE Hub Rust").size(if ctx.content_rect().width() < 1000.0 { 16.0 } else { 18.0 }).strong());
                    ui.add(egui::Image::new(&self.rust_texture).fit_to_exact_size(Vec2::splat(26.0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = t(if self.dark {
                            "Светлая тема"
                        } else {
                            "Тёмная тема"
                        });
                        if ui.button(label).clicked() {
                            self.dark = !self.dark;
                            self.style(ctx);
                        }
                        if ui
                            .add_enabled(
                                !self.offline && !self.controller.snapshot.busy,
                                egui::Button::new(t("Обновить")),
                            )
                            .clicked()
                        {
                            self.command(Command::Refresh);
                        }
                        ui.horizontal(|ui| {
                            for (code, label) in [("ru", "RU"), ("en", "EN")] {
                                if ui.selectable_label(i18n::language().code() == code, label)
                                    .on_hover_text(t("Язык интерфейса")).clicked() {
                                    self.set_language(code);
                                }
                            }
                        });
                        if self.read_only {
                            ui.label(RichText::new(t("Только чтение")).color(p.muted));
                        }
                        if self.offline {
                            ui.label(RichText::new(t("Без подключения")).color(p.muted));
                        }
                    });
                });
            });
    }
    fn ace75_action(&mut self) {
        match self.ace75.take_action() {
            Some(ace75::Action::Send(command)) => self.command(Command::Ace75(command)),
            Some(ace75::Action::Confirm(message, command)) => self.pending = Some((message, Command::Ace75(command))),
            None => {}
        }
    }
    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = Palette::new(self.dark);
        egui::SidePanel::left("device-profiles")
            .exact_width(if ctx.content_rect().width() < 1000.0 { 188.0 } else { 224.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.outer)
                    .inner_margin(egui::Margin::symmetric(20, 20)),
            )
            .show(ctx, |ui| {
                let mut view = self.device_view;
                ui.add_enabled_ui(!self.controller.snapshot.busy, |ui| { egui::ComboBox::from_id_salt("device-view")
                    .width(ui.available_width())
                    .selected_text(view.label())
                    .show_ui(ui, |ui| {
                        for item in [DeviceView::Mouse, DeviceView::Ace75] {
                            ui.selectable_value(&mut view, item, item.label());
                        }
                    }); });
                if view != self.device_view {
                    self.select_device(view.code());
                }
                ui.add_space(16.0);
                if self.device_view == DeviceView::Ace75 {
                    let writable = !self.read_only && !self.offline && !self.controller.snapshot.busy;
                    egui::ScrollArea::vertical().id_salt("ace75-sidebar").show(ui, |ui| self.ace75.sidebar(ui, p, &self.controller.snapshot.ace75, self.controller.snapshot.busy, writable));
                    self.ace75_action();
                    return;
                }
                egui::ScrollArea::vertical()
                    .id_salt("device-sidebar-scroll")
                    .show(ui, |ui| {
                        ui.label(RichText::new("A7 V3 Ultra+").size(23.0).strong());
                        ui.label(RichText::new("MCHOSE").small().color(p.muted));
                        ui.add_space(16.0);
                        if let Some(mouse) = &self.controller.snapshot.mouse {
                            let status = &mouse.status;
                            let wired = self.controller.snapshot.devices.iter().any(|device| {
                                device.product_id == 0x4033
                                    && Some(&device.path)
                                        == self.controller.snapshot.active_mouse_path.as_ref()
                            });
                            let connected = wired || status.battery_percent > 0;
                            ui.horizontal(|ui| {
                                status_dot(
                                    ui,
                                    if connected {
                                        Color32::from_rgb(50, 170, 112)
                                    } else {
                                        p.muted
                                    },
                                );
                                ui.label(t(if wired {
                                    "USB-кабель"
                                } else if connected {
                                    "Радиоканал 2,4 ГГц"
                                } else {
                                    "Мышь спит / отключена"
                                }));
                            });
                            ui.add_space(4.0);
                            let charge = if status.battery_percent < 100
                                && (wired || status.charge_status == 1)
                            {
                                t(" · зарядка")
                            } else {
                                ""
                            };
                            ui.label(
                                RichText::new(format!("{}%{}", status.battery_percent, charge))
                                    .size(22.0),
                            );
                            ui.add(
                                egui::ProgressBar::new(
                                    (status.battery_percent as f32 / 100.0).clamp(0.0, 1.0),
                                )
                                .fill(p.accent)
                                .desired_height(4.0),
                            );
                            ui.add_space(20.0);
                            ui.label(RichText::new(t("ПАМЯТЬ МЫШИ")).size(11.0).color(p.muted));
                            ui.add_space(4.0);
                            let profiles = mouse.profile_names.clone();
                            let current = mouse.settings.profile_index;
                            for (index, name) in profiles.iter().enumerate() {
                                let title = if name.trim().is_empty() {
                                    tf!("Профиль {}", index + 1)
                                } else {
                                    name.clone()
                                };
                                let response = ui.add_enabled(
                                    self.writable(),
                                    egui::Button::new(title)
                                        .selected(current as usize == index)
                                        .min_size(Vec2::new(ui.available_width(), 40.0)),
                                );
                                if response.clicked()
                                    && current as usize != index
                                    && self.writable()
                                {
                                    if self.has_dirty() {
                                        self.local_message =
                                    "Сначала примените или отмените изменения текущего профиля."
                                        .into();
                                    } else {
                                        self.profile_name.clear();
                                        self.command(Command::SelectProfile(index as u8));
                                    }
                                }
                            }
                        } else {
                            ui.horizontal(|ui| {
                                status_dot(ui, p.muted);
                                ui.label(t("Нет данных мыши"));
                            });
                            ui.add_space(10.0);
                            ui.label(
                                RichText::new(t("Подключите мышь кабелем или её USB-приёмник."))
                                    .color(p.muted),
                            );
                        }
                        ui.add_space(24.0);
                        if !self.controller.snapshot.presets.is_empty() {
                            ui.label(RichText::new(t("ЛОКАЛЬНЫЕ ПРОФИЛИ")).size(11.0).color(p.muted));
                            for preset in self.controller.snapshot.presets.clone() {
                                if ui
                                    .add_sized(
                                        [ui.available_width(), 32.0],
                                        egui::Button::new(&preset.name),
                                    )
                                    .clicked()
                                {
                                    self.profile_path = preset.path.to_string_lossy().into_owned();
                                    self.preset_name = preset.name;
                                    self.page = Page::Profiles;
                                    self.command(Command::PreviewPreset(preset.path));
                                }
                            }
                            ui.add_space(12.0);
                        }
                        ui.separator();
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            status_dot(
                                ui,
                                if self.controller.snapshot.dock.is_some() {
                                    Color32::from_rgb(50, 170, 112)
                                } else {
                                    p.muted
                                },
                            );
                            ui.label("MagDock");
                        });
                        ui.label(
                            RichText::new(t(if self.controller.snapshot.dock.is_some() {
                                "Подключена"
                            } else {
                                "Нет данных станции"
                            }))
                            .small()
                            .color(p.muted),
                        );
                        if ui
                            .add_sized(
                                [ui.available_width(), 36.0],
                                egui::Button::new(t("Настроить подсветку")),
                            )
                            .clicked()
                        {
                            self.page = Page::Dock;
                        }
                        ui.add_space(24.0);
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("A7 V3 Ultra+ / MagDock")
                                    .small()
                                    .color(p.muted),
                            );
                            ui.add_space(12.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = ((ui.available_width() - 120.0) / 4.0).clamp(0.0, 10.0);
                                for (i, color) in [
                                    Color32::from_gray(238),
                                    Color32::from_gray(40),
                                    Color32::from_rgb(150, 195, 225),
                                    Color32::from_rgb(190, 45, 40),
                                    Color32::from_rgb(232, 167, 184),
                                ]
                                .into_iter()
                                .enumerate()
                                {
                                    let (r, response) = ui.allocate_exact_size(
                                        Vec2::splat(24.0),
                                        egui::Sense::click(),
                                    );
                                    ui.painter().circle_filled(r.center(), 9.0, color);
                                    if self.skin == i {
                                        ui.painter().circle_stroke(
                                            r.center(),
                                            11.0,
                                            Stroke::new(1.5_f32, p.accent),
                                        );
                                    }
                                    if response.clicked() {
                                        self.skin = i;
                                    }
                                }
                            });
                            ui.label(RichText::new(t("Цвет корпуса")).small().color(p.muted));
                        });
                    });
            });
    }
    fn has_dirty(&self) -> bool {
        self.dirty_settings
            || self.dirty_dpi
            || self.dirty_keys
            || self.dirty_dock
            || self.dirty_preset
    }
    fn footer(&mut self, ctx: &egui::Context) {
        let p = Palette::new(self.dark);
        egui::TopBottomPanel::bottom("status-footer")
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.outer)
                    .inner_margin(egui::Margin::symmetric(24, 9)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if self.controller.snapshot.busy {
                        ui.label(RichText::new(t("Ожидание…")).color(p.muted));
                    }
                    if let Some((sent, total)) = self.controller.snapshot.firmware_progress {
                        if total > 0 {
                            ui.add(
                                egui::ProgressBar::new(sent as f32 / total as f32)
                                    .desired_width(180.0)
                                    .show_percentage(),
                            );
                        }
                    }
                    if let Some(error) = &self.controller.snapshot.error {
                        let message = if self.controller.snapshot.error_chain.is_empty() {
                            i18n::message(error)
                        } else {
                            i18n::error_chain(&self.controller.snapshot.error_chain)
                        };
                        ui.colored_label(p.error_text, message);
                    } else if !self.local_message.is_empty() {
                        ui.label(i18n::message(&self.local_message));
                    } else {
                        ui.label(RichText::new(i18n::message(&self.controller.snapshot.message)).color(p.muted));
                    }
                    if self.has_dirty() {
                        ui.label(
                            RichText::new(t("Есть неприменённые изменения")).color(p.accent_text),
                        );
                    }
                });
            });
    }
    fn content(&mut self, ctx: &egui::Context) {
        let p = Palette::new(self.dark);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.outer).inner_margin(egui::Margin {
                left: 0,
                right: if ctx.content_rect().width() < 1000.0 { 14 } else { 24 },
                top: 0,
                bottom: 20,
            }))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.panel)
                    .corner_radius(12)
                    .inner_margin(if ctx.content_rect().width() < 1000.0 { 18 } else { 24 })
                    .show(ui, |ui| {
                        ui.set_min_height(ui.available_height());
                        if self.device_view == DeviceView::Ace75 {
                            let writable = !self.read_only && !self.offline && !self.controller.snapshot.busy;
                            self.ace75.show(ui, p, &self.controller.snapshot.ace75, self.controller.snapshot.busy, writable);
                            self.ace75_action();
                            return;
                        }
                        ui.horizontal_wrapped(|ui| {
                            for page in [
                                Page::Keys,
                                Page::Dpi,
                                Page::Performance,
                                Page::Dock,
                                Page::Macros,
                                Page::Profiles,
                                Page::System,
                            ] {
                                if ui
                                    .add_enabled(
                                        self.preset_edit.is_none()
                                            || !matches!(page, Page::Dock | Page::System),
                                        egui::Button::new(RichText::new(page.title()).color(
                                            if self.page == page {
                                                Color32::WHITE
                                            } else {
                                                p.text
                                            },
                                        ))
                                        .selected(self.page == page)
                                        .frame(self.page == page),
                                    )
                                    .clicked()
                                {
                                    self.page = page;
                                }
                            }
                        });
                        ui.add_space(14.0);
                        ui.separator();
                        ui.add_space(18.0);
                        if let Some((path, _)) = &self.preset_edit {
                            let name = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            ui.label(
                                RichText::new(tf!("Редактируется локальный профиль: {name}"))
                                    .strong()
                                    .color(p.accent_text),
                            );
                            ui.horizontal_wrapped(|ui| {
                                if ui
                                    .add_enabled(
                                        self.has_dirty() && !self.controller.snapshot.busy,
                                        egui::Button::new(t("Сохранить локальный профиль")),
                                    )
                                    .clicked()
                                {
                                    self.save_edited_preset();
                                }
                                if ui
                                    .add_enabled(
                                        !self.controller.snapshot.busy,
                                        egui::Button::new(t(if self.has_dirty() {
                                            "Закрыть без сохранения"
                                        } else {
                                            "Закрыть редактор"
                                        })),
                                    )
                                    .clicked()
                                {
                                    self.close_preset_edit();
                                }
                            });
                            ui.add_space(12.0);
                        }
                        if self.page == Page::Dock && self.preset_edit.is_none() {
                            self.dock_apply_row(ui);
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("page-scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add_enabled_ui(self.preset_saving.is_none(), |ui| {
                                    match self.page {
                                        Page::Keys => self.keys_page(ui),
                                        Page::Dpi => self.dpi_page(ui),
                                        Page::Performance => self.performance_page(ui),
                                        Page::Dock if self.preset_edit.is_some() => { ui.label(t("Закройте редактор локального профиля, чтобы настроить станцию.")); }
                                        Page::Dock => self.dock_page(ui),
                                        Page::Macros => self.macros_page(ui),
                                        Page::Profiles => self.profiles_page(ui),
                                        Page::System => self.system_page(ui),
                                    }
                                });
                            });
                    });
            });
    }
    fn no_mouse(&self, ui: &mut egui::Ui) -> bool {
        if self.controller.snapshot.mouse.is_some() || self.preset_edit.is_some() {
            return false;
        }
        let p = Palette::new(self.dark);
        ui.add_space(16.0);
        ui.horizontal(|ui|{ui.add(egui::Image::new(&self.textures[self.skin]).fit_to_exact_size(Vec2::new(151.2,285.6)));ui.add_space(24.0);ui.vertical(|ui|{ui.heading(t("Подключите A7 V3 Ultra+"));ui.add_space(10.0);ui.label(t("Настройки появятся после чтения памяти мыши."));ui.label(RichText::new(t("Если приёмник уже подключён, нажмите «Обновить» и проверьте сообщение внизу.")).color(p.muted));});});
        true
    }
    fn apply_row(
        &mut self,
        ui: &mut egui::Ui,
        dirty: bool,
        cmd: Option<Command>,
        kind: &'static str,
    ) {
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    dirty
                        && (self.writable()
                            || self.preset_edit.is_some() && !self.controller.snapshot.busy),
                    egui::Button::new(
                        RichText::new(t(if self.preset_edit.is_some() {
                            "Сохранить профиль"
                        } else {
                            "Применить"
                        }))
                        .color(Color32::WHITE),
                    )
                    .fill(Palette::new(self.dark).accent),
                )
                .clicked()
            {
                if self.preset_edit.is_some() {
                    self.save_edited_preset();
                } else if let Some(cmd) = cmd {
                    self.apply(kind, cmd);
                }
            }
            if ui
                .add_enabled(dirty, egui::Button::new(t("Отменить изменения")))
                .clicked()
            {
                match kind {
                    "settings" => self.dirty_settings = false,
                    "dpi" => self.dirty_dpi = false,
                    "keys" => self.dirty_keys = false,
                    "dock" => self.dirty_dock = false,
                    _ => {}
                }
                if let Some((_, value)) = &self.preset_edit {
                    match kind {
                        "settings" => {
                            self.staged_settings = Some(value.snapshot.settings.clone());
                            self.staged_lod = value.snapshot.lod_index;
                        }
                        "dpi" => self.staged_dpi = Some(value.snapshot.dpi.clone()),
                        "keys" => self.staged_keys = value.snapshot.keys.clone(),
                        _ => {}
                    }
                } else {
                    self.synced_at = None;
                    self.sync();
                }
            }
            if self.read_only && self.preset_edit.is_none() {
                ui.label(t("Запись отключена в режиме чтения."));
            }
        });
    }
    fn keys_page(&mut self, ui: &mut egui::Ui) {
        if self.no_mouse(ui) {
            return;
        }
        let p = Palette::new(self.dark);
        let left_width = 290.0_f32.min(ui.available_width() * 0.40);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(left_width, 540.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    egui::Frame::new()
                        .fill(p.field)
                        .corner_radius(12)
                        .inner_margin(18)
                        .show(ui, |ui| {
                            ui.set_min_width(left_width - 36.0);
                            ui.heading(t("Назначение"));
                            egui::ComboBox::from_id_salt("physical-button")
                                .width(ui.available_width())
                                .selected_text(t(BUTTONS[self.selected_button.min(5)]))
                                .show_ui(ui, |ui| {
                                    for (i, name) in
                                        BUTTONS.iter().enumerate().take(self.staged_keys.len())
                                    {
                                        ui.selectable_value(&mut self.selected_button, i, t(name));
                                    }
                                });
                            if self.selected_button == 0 {
                                ui.checkbox(&mut self.allow_left, t("Разрешить замену левой кнопки"));
                            }
                            ui.add(
                                egui::TextEdit::singleline(&mut self.key_search)
                                    .hint_text(t("Поиск действия"))
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal_wrapped(|ui| {
                                for (i, name) in ["Мышь", "Клавиатура", "Система", "Макрос"]
                                    .into_iter()
                                    .enumerate()
                                {
                                    ui.selectable_value(&mut self.key_category, i, t(name));
                                }
                            });
                            ui.separator();
                            let enabled = (self.selected_button != 0 || self.allow_left)
                                && self
                                    .staged_keys
                                    .get(self.selected_button)
                                    .is_some_and(|key| key.kind != 0xff);
                            ui.add_enabled_ui(enabled, |ui| {
                                let mut options: Vec<(String, KeyBinding)> = Vec::new();
                                match self.key_category {
                                    0 => {}
                                    1 => {
                                        ui.label(t("Клавиша (USB HID)"));
                                        egui::ComboBox::from_id_salt("keyboard-usage")
                                            .width(205.0)
                                            .selected_text(usage_name(self.key_usage))
                                            .show_ui(ui, |ui| {
                                                for code in (4..=115).chain(224..=231) {
                                                    ui.selectable_value(
                                                        &mut self.key_usage,
                                                        code,
                                                        usage_name(code),
                                                    );
                                                }
                                            });
                                        ui.horizontal_wrapped(|ui| {
                                            for (label, mask) in [
                                                ("Ctrl", 1),
                                                ("Shift", 2),
                                                ("Alt", 4),
                                                ("Super", 8),
                                            ] {
                                                let mut value = self.key_modifiers & mask != 0;
                                                if ui.checkbox(&mut value, label).changed() {
                                                    if value {
                                                        self.key_modifiers |= mask
                                                    } else {
                                                        self.key_modifiers &= !mask
                                                    }
                                                }
                                            }
                                        });
                                        let binding = if self.key_modifiers == 0 {
                                            KeyBinding::keyboard(self.key_usage)
                                        } else {
                                            KeyBinding::chord(self.key_modifiers, self.key_usage)
                                        };
                                        options.push((t("Назначить клавишу").into(), binding));
                                    }
                                    2 => {}
                                    _ => {
                                        ui.label(t("Номер макроса в памяти"));
                                        ui.add(
                                            egui::DragValue::new(&mut self.macro_slot)
                                                .range(0..=17),
                                        );
                                        options.push((
                                            tf!("Назначить макрос {}", self.macro_slot),
                                            KeyBinding::macro_id(self.macro_slot as u16),
                                        ));
                                        if ui.button(t("Открыть редактор")).clicked() {
                                            self.page = Page::Macros;
                                        }
                                    }
                                }
                                for action in hub_actions().iter().filter(|action| match self.key_category {0=>matches!(action.category.as_str(),"mouse"|"dpi"|"profile"|"disabled"),1=>matches!(action.category.as_str(),"keyboard"|"modifiers"),2=>matches!(action.category.as_str(),"system"|"media"),_=>false}) {
                                    options.push((action.name().to_owned(),KeyBinding {kind:action.kind,data:action.data.clone()}));
                                }
                                egui::ScrollArea::vertical().id_salt("key-action-list").max_height(360.0).auto_shrink([false,false]).show(ui,|ui|{for (label, binding) in options {
                                    if !label
                                        .to_lowercase()
                                        .contains(&self.key_search.to_lowercase())
                                    {
                                        continue;
                                    }
                                    if ui
                                        .add_sized(
                                            [ui.available_width(), 32.0],
                                            egui::Button::new(label),
                                        )
                                        .clicked()
                                    {
                                        let removes_left = removes_last_left(&self.staged_keys, self.selected_button, &binding);
                                        if removes_left{self.local_message="Сначала назначьте левый клик другой кнопке.".into();continue;}
                                        if let Some(key) =
                                            self.staged_keys.get_mut(self.selected_button)
                                        {
                                            *key = binding;
                                            self.dirty_keys = true;
                                        }
                                    }
                                }});
                            });
                        });
                },
            );
            ui.add_space(18.0);
            ui.vertical(|ui| {
                let available = ui.available_width();
                if available>=540.0 {
                    let (area,_)=ui.allocate_exact_size(Vec2::new(available,530.0),egui::Sense::hover());
                    let image_rect=egui::Rect::from_center_size(area.center(),Vec2::new(252.0,476.0));
                    ui.painter().image(self.textures[self.skin].id(),image_rect,egui::Rect::from_min_max(egui::pos2(0.0,0.0),egui::pos2(1.0,1.0)),Color32::WHITE);
                    let anchors=[(0.28,0.18,-1.0,0.12),(0.72,0.18,1.0,0.12),(0.50,0.25,1.0,0.28),(0.03,0.38,-1.0,0.38),(0.04,0.53,-1.0,0.55),(0.73,0.48,1.0,0.52)];
                    for (index,key) in self.staged_keys.iter().enumerate().take(6){
                        let (ax,ay,side,ly)=anchors[index];
                        let label_width=((available-252.0)/2.0-22.0).clamp(110.0,170.0);
                        let x=if side<0.0 {image_rect.left()-label_width-20.0}else{image_rect.right()+20.0};
                        let y=image_rect.top()+ly*image_rect.height();
                        let label_rect=egui::Rect::from_min_size(egui::pos2(x,y),Vec2::new(label_width,48.0));
                        let anchor=egui::pos2(image_rect.left()+ax*image_rect.width(),image_rect.top()+ay*image_rect.height());
                        let line_end=if side<0.0 {label_rect.right_center()}else{label_rect.left_center()};
                        let color=if self.selected_button==index {p.accent}else{p.border};
                        ui.painter().line_segment([line_end,anchor],Stroke::new(1.0_f32,color));
                        ui.painter().circle_filled(anchor,4.0,color);
                        let label=format!("{}\n{}",t(BUTTONS[index]),binding_name(key));
                        if ui.put(label_rect,egui::Button::new(RichText::new(label).size(12.0)).selected(self.selected_button==index)).clicked(){self.selected_button=index;}
                    }
                } else {
                let image_height = (ui.available_height() - 100.0).clamp(285.6, 476.0);
                let image_width = image_height * 504.0 / 952.0;
                ui.horizontal(|ui| {
                    ui.add_space(((available - image_width) / 2.0).max(0.0));
                    ui.add(
                        egui::Image::new(&self.textures[self.skin])
                            .fit_to_exact_size(Vec2::new(image_width, image_height)),
                    );
                });
                ui.add_space(12.0);
                egui::Grid::new("button-mappings")
                    .num_columns(2)
                    .spacing([14.0, 8.0])
                    .show(ui, |ui| {
                        for (i, key) in self.staged_keys.iter().enumerate() {
                            if ui
                                .selectable_label(
                                    self.selected_button == i,
                                    t(BUTTONS.get(i).unwrap_or(&"Кнопка")),
                                )
                                .clicked()
                            {
                                self.selected_button = i;
                            }
                            ui.label(RichText::new(binding_name(key)).color(p.muted));
                            ui.end_row();
                        }
                    });
                }
            });
        });
        let profile = self.staged_settings.as_ref().map_or(0, |s| s.profile_index);
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Восстановить выбранную кнопку")).clicked() {
                let binding = default_binding(self.selected_button);
                if removes_last_left(&self.staged_keys, self.selected_button, &binding) {
                    self.local_message = "Сначала назначьте левый клик другой кнопке.".into();
                } else if let Some(key) = self.staged_keys.get_mut(self.selected_button) {
                    if key.kind != 0xff {
                        *key = binding;
                        self.dirty_keys = true;
                    }
                }
            }
            if ui.button(t("Восстановить все кнопки")).clicked() {
                for (index, key) in self.staged_keys.iter_mut().enumerate() {
                    if key.kind != 0xff {
                        *key = default_binding(index);
                    }
                }
                self.dirty_keys = true;
            }
            let cycle = self.staged_keys.get(self.selected_button)
                .is_some_and(|key| key.kind == 0x33 && key.data.first() == Some(&255));
            if cycle && ui.add_enabled(self.writable(), egui::Button::new(t("Цикл профилей во всех ячейках"))).clicked() {
                self.pending = Some((
                    format!("Назначить переключение профилей кнопке «{}» во всех трёх ячейках памяти? Остальные изменения здесь сначала примените отдельно.", t(BUTTONS[self.selected_button])),
                    Command::ApplyProfileCycleAcrossAllSlots { key_index: self.selected_button as u8 },
                ));
            }
        });
        self.apply_row(
            ui,
            self.dirty_keys,
            Some(Command::ApplyKeys {
                profile,
                keys: self.staged_keys.clone(),
            }),
            "keys",
        );
    }
    fn dpi_page(&mut self, ui: &mut egui::Ui) {
        if self.no_mouse(ui) {
            return;
        }
        let p = Palette::new(self.dark);
        let Some(mut dpi) = self.staged_dpi.clone() else {
            return;
        };
        let before = dpi.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Уровни DPI"));
            for count in 1..=6 {
                if ui
                    .selectable_value(&mut dpi.count, count, count.to_string())
                    .changed()
                {
                    dpi.selected = dpi.selected.min(count - 1);
                }
            }
            ui.add_space(20.0);
            ui.checkbox(&mut dpi.separate_y, t("Раздельные X / Y"));
        });
        ui.add_space(16.0);
        for index in 0..dpi.count.min(6) as usize {
            egui::Frame::new()
                .fill(p.field)
                .corner_radius(12)
                .inner_margin(20)
                .stroke(Stroke::new(
                    if dpi.selected as usize == index {
                        1.0
                    } else {
                        0.0_f32
                    },
                    p.accent,
                ))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(8.0, 26.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 4, STAGE_COLORS[index]);
                        ui.selectable_value(
                            &mut dpi.selected,
                            index as u8,
                            tf!("Уровень {}", index + 1),
                        );
                        ui.add_space(18.0);
                        ui.label("X");
                        let width = (ui.available_width() - 110.0).max(120.0);
                        ui.spacing_mut().slider_width = width;
                        ui.add(
                            egui::Slider::new(&mut dpi.x[index], 200..=50000)
                                .logarithmic(true)
                                .show_value(false),
                        );
                        ui.add(
                            egui::DragValue::new(&mut dpi.x[index])
                                .range(200..=50000)
                                .speed(50.0),
                        );
                    });
                    if dpi.separate_y {
                        ui.horizontal(|ui| {
                            ui.add_space(136.0);
                            ui.label("Y");
                            ui.spacing_mut().slider_width =
                                (ui.available_width() - 110.0).max(120.0);
                            ui.add(
                                egui::Slider::new(&mut dpi.y[index], 200..=50000)
                                    .logarithmic(true)
                                    .show_value(false),
                            );
                            ui.add(
                                egui::DragValue::new(&mut dpi.y[index])
                                    .range(200..=50000)
                                    .speed(50.0),
                            );
                        });
                    } else if dpi.x[index] != before.x[index] || before.separate_y {
                        dpi.y[index] = dpi.x[index];
                    }
                });
            ui.add_space(6.0);
        }
        ui.label(
            RichText::new(t("200–50 000 DPI. Выбранный уровень сохраняется в активный профиль мыши."))
                .small()
                .color(p.muted),
        );
        if ui.button(t("Восстановить DPI по умолчанию")).clicked() {
            dpi.count = 6;
            dpi.selected = 2;
            dpi.separate_y = false;
            dpi.x = [400, 800, 1600, 3200, 6400, 50000];
            dpi.y = dpi.x;
        }
        self.dirty_dpi |= dpi != before;
        self.staged_dpi = Some(dpi.clone());
        self.apply_row(ui, self.dirty_dpi, Some(Command::ApplyDpi(dpi)), "dpi");
    }
    fn performance_page(&mut self, ui: &mut egui::Ui) {
        if self.no_mouse(ui) {
            return;
        }
        let p = Palette::new(self.dark);
        let Some(mut settings) = self.staged_settings.clone() else {
            return;
        };
        let before = settings.clone();
        let old_lod = self.staged_lod;
        let columns = if ui.available_width() < 850.0 { 1 } else { 2 };
        setting_box(
            ui,
            p,
            "Режим сенсора",
            "Баланс энергопотребления и производительности.",
            |ui| {
                let mut mode = settings.game_mode();
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut mode, 0, t("Офисный"));
                    ui.selectable_value(&mut mode, 1, t("Производительный"));
                    ui.selectable_value(&mut mode, 2, t("Соревновательный"));
                });
                let _ = settings.set_game_mode(mode);
            },
        );
        ui.add_space(8.0);
        let mut draw_rate = |ui: &mut egui::Ui| {
            setting_box(
                ui,
                p,
                "Частота опроса",
                "Настройки USB и радиоканала хранятся отдельно.",
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label("USB");
                        rate_combo(ui, "usb-rate", &mut settings.usb_rate_index);
                        ui.add_space(16.0);
                        ui.label(t("2,4 ГГц"));
                        rate_combo(ui, "wireless-rate", &mut settings.wireless_rate_index);
                    });
                },
            );
        };
        draw_rate(ui);
        ui.add_space(8.0);
        ui.columns(columns, |cols| {
            setting_box(
                &mut cols[0],
                p,
                "Высота отрыва (LOD)",
                "Когда сенсор перестаёт отслеживать поверхность.",
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (i, lod) in LODS.iter().enumerate() {
                            ui.selectable_value(&mut self.staged_lod, i as u8, t(lod));
                        }
                    });
                },
            );
            if columns == 1 {
                cols[0].add_space(8.0);
            }
            setting_box(
                &mut cols[columns - 1],
                p,
                "Сон мыши",
                "Таймер бездействия.",
                |ui| {
                    let mut disabled = settings.sleep_disabled || settings.sleep_minutes == 0;
                    ui.spacing_mut().slider_width =
                        (ui.available_width() - 90.0).clamp(80.0, 220.0);
                    ui.add_enabled(
                        !disabled,
                        egui::Slider::new(&mut settings.sleep_minutes, 1..=100)
                            .clamping(egui::SliderClamping::Edits)
                            .suffix(t(" мин")),
                    );
                    if ui.checkbox(&mut disabled, t("Никогда не засыпать")).changed()
                    {
                        settings.sleep_disabled = disabled;
                        if !disabled && settings.sleep_minutes == 0 {
                            settings.sleep_minutes = 3;
                        }
                    }
                },
            );
        });
        ui.add_space(8.0);
        ui.columns(columns, |cols| {
            setting_box(
                &mut cols[0],
                p,
                "Задержка подавления дребезга",
                "Независимо для левой и правой кнопки.",
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(t("Левая"));
                        ui.spacing_mut().slider_width =
                            (ui.available_width() - 80.0).clamp(80.0, 220.0);
                        ui.add(
                            egui::Slider::new(&mut settings.left_debounce_ms, 1..=20)
                                .clamping(egui::SliderClamping::Edits)
                                .suffix(t(" мс")),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Правая"));
                        ui.spacing_mut().slider_width =
                            (ui.available_width() - 80.0).clamp(80.0, 220.0);
                        ui.add(
                            egui::Slider::new(&mut settings.right_debounce_ms, 1..=20)
                                .clamping(egui::SliderClamping::Edits)
                                .suffix(t(" мс")),
                        );
                    });
                },
            );
            if columns == 1 {
                cols[0].add_space(8.0);
            }
            setting_box(
                &mut cols[columns - 1],
                p,
                "Поворот сенсора",
                "Подстройка направления движения под хват.",
                |ui| {
                    ui.spacing_mut().slider_width =
                        (ui.available_width() - 80.0).clamp(80.0, 220.0);
                    ui.add(egui::Slider::new(&mut settings.angle, -30..=30).suffix("°"));
                    ui.horizontal(|ui| {
                        if ui.button("0°").clicked() {
                            settings.angle = 0;
                        }
                        ui.label(RichText::new("−30° … +30°").color(p.muted));
                    });
                },
            );
        });
        ui.add_space(8.0);
        setting_box(
            ui,
            p,
            "Обработка движения",
            "Параметры записываются в активный профиль.",
            |ui| {
                ui.columns(columns, |cols| {
                    for (column, bit, label, tip) in [
                        (
                            0,
                            2,
                            "Ripple Control",
                            "Подавление колебаний на высоком DPI.",
                        ),
                        (
                            0,
                            3,
                            "Angle Snapping",
                            "Коррекция движения до прямых линий.",
                        ),
                        (1, 4, "Motion Sync", "Синхронизация отчётов сенсора."),
                        (
                            1,
                            7,
                            "Режим стекла",
                            "При включении LOD меняется на 1,7 мм.",
                        ),
                    ] {
                        let mut value = settings.sensor & (1 << bit) != 0;
                        if cols[column.min(columns - 1)]
                            .checkbox(&mut value, t(label))
                            .on_hover_text(t(tip))
                            .changed()
                        {
                            let _ = settings.set_sensor_bit(bit, value);
                            if bit == 7 && value {
                                self.staged_lod = 4;
                            }
                        }
                    }
                });
            },
        );
        self.dirty_settings |= settings != before || old_lod != self.staged_lod;
        self.staged_settings = Some(settings.clone());
        self.apply_row(
            ui,
            self.dirty_settings,
            Some(Command::ApplyPerformance {
                settings,
                lod: self.staged_lod,
            }),
            "settings",
        );
    }
    fn dock_page(&mut self, ui: &mut egui::Ui) {
        let p = Palette::new(self.dark);
        let Some(mut dock) = self.staged_dock.clone() else {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Image::new(&self.dock_texture).fit_to_exact_size(Vec2::new(230.0, 333.4)),
                );
                ui.add_space(30.0);
                ui.vertical(|ui| {
                    ui.heading("MagDock");
                    ui.label(t("Подключите док-станцию по USB."));
                    ui.label(
                        RichText::new(t("Станция определяется отдельно от приёмника мыши."))
                            .color(p.muted),
                    );
                });
            });
            return;
        };
        let before = dock.clone();
        let compact = ui.available_width() < 760.0;
        ui.horizontal_top(|ui| {
            let width = if compact { 88.0 } else { 120.0 };
            ui.add(egui::Image::new(&self.dock_texture).fit_to_exact_size(Vec2::new(width, width * 890.0 / 614.0)));
            ui.add_space(16.0);
            ui.vertical(|ui| {
                ui.label(RichText::new("MCHOSE MagDock").size(19.0).strong());
                ui.label(RichText::new(tf!("Прошивка {}", dock.firmware_version)).small().color(p.muted));
                ui.add_space(8.0);
                ui.checkbox(&mut dock.lighting.enabled, t("Подсветка включена"));
                let mut enabled = self.controller.snapshot.charging_indicator_enabled;
                if ui.add_enabled(self.writable(), egui::Checkbox::new(&mut enabled, t("Показывать заряд мыши на кольце")))
                    .on_hover_text(t("Работает сразу, без кнопки «Применить»." )).changed() {
                    self.command(Command::SetChargingIndicator(enabled));
                }
                ui.add(egui::Label::new(RichText::new(t("50%: половина жёлтого кольца. 100%: полное зелёное кольцо.")).small().color(p.muted)).wrap())
                    .on_hover_text(t("Во время зарядки: 50% заполняет половину кольца жёлтым, 100% заполняет всё кольцо зелёным. После снятия мыши возвращается выбранный эффект. Приложение должно оставаться открытым."));
                if self.controller.snapshot.charging_indicator_active {
                    let status = match self.controller.snapshot.charging_percent {
                        Some(percent) => tf!("Зарядка: {percent}% — выбранный эффект временно приостановлен."),
                        None => t("Зарядка: уровень пока неизвестен — выбранный эффект временно приостановлен.").into(),
                    };
                    ui.add(egui::Label::new(RichText::new(status).color(p.accent_text)).wrap());
                }
            });
        });
        ui.add_space(12.0);
        setting_box(ui, p, "Эффект", "Настройка светового кольца станции.", |ui| {
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [(DockMode::Static, "Статичный"), (DockMode::Breathing, "Дыхание"),
                    (DockMode::Shining, "Мерцание"), (DockMode::Cycle, "Смена цветов"),
                    (DockMode::Flow, "Поток"), (DockMode::Music, "Музыка")] {
                    ui.selectable_value(&mut dock.lighting.mode, mode, t(label));
                }
            });
        });
        ui.add_space(8.0);
        ui.columns(if compact { 1 } else { 2 }, |columns| {
            setting_box(&mut columns[0], p, "Яркость и скорость", "Пять уровней яркости и скорости эффекта.", |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 65.0).clamp(80.0, 250.0);
                ui.label(t("Яркость"));
                let mut brightness = dock.lighting.brightness.saturating_add(1);
                ui.add(egui::Slider::new(&mut brightness, 1..=5));
                dock.lighting.brightness = brightness - 1;
                ui.label(t("Скорость"));
                let mut speed = 5 - dock.lighting.speed.min(4);
                ui.add_enabled(dock.lighting.mode != DockMode::Flow, egui::Slider::new(&mut speed, 1..=5));
                dock.lighting.speed = if dock.lighting.mode == DockMode::Flow { 1 } else { 5 - speed };
            });
            if compact { columns[0].add_space(8.0); }
            setting_box(&mut columns[usize::from(!compact)], p, "Цвет и направление", "Цвет используется эффектами, поддерживающими один цвет.", |ui| {
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(&mut dock.lighting.color);
                    ui.label(format!("#{:02X}{:02X}{:02X}", dock.lighting.color[0], dock.lighting.color[1], dock.lighting.color[2]));
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(&mut dock.lighting.direction, 0, t("По часовой"));
                    ui.selectable_value(&mut dock.lighting.direction, 1, t("Против часовой"));
                });
            });
        });
        if dock.lighting.mode == DockMode::Music {
            ui.add_space(8.0);
            ui.label(RichText::new(t("Нажмите «Применить», чтобы подсветка реагировала на системный звук. Приложение должно оставаться открытым.")).small().color(p.muted));
        }
        dock.lighting.music_sync = dock.lighting.mode == DockMode::Music;
        self.dirty_dock |= dock != before;
        self.staged_dock = Some(dock.clone());
        ui.add_space(14.0);
        ui.horizontal_wrapped(|ui| {
            if self.controller.snapshot.music_active {
                if ui
                    .add_enabled(
                        self.writable(),
                        egui::Button::new(t("Остановить музыкальную подсветку")),
                    )
                    .clicked()
                {
                    self.command(Command::StopMusic);
                }
                ui.label(t(if self.controller.snapshot.charging_indicator_active {
                    "Музыкальная подсветка продолжится после зарядки."
                } else {
                    "Станция реагирует на системный звук."
                }));
            }
            if ui
                .add_enabled(self.writable(), egui::Button::new(t("Сбросить подсветку")))
                .clicked()
            {
                self.pending = Some((
                    "Сбросить настройки подсветки MagDock?".into(),
                    Command::ResetDock,
                ));
            }
        });
    }
    fn dock_apply_row(&mut self, ui: &mut egui::Ui) {
        let Some(dock) = self.staged_dock.clone() else { return; };
        let needs_music_start = dock.lighting.enabled && dock.lighting.mode == DockMode::Music
            && !self.controller.snapshot.music_active;
        self.apply_row(ui, self.dirty_dock || needs_music_start, Some(Command::ApplyDock(dock)), "dock");
        ui.add_space(12.0);
    }
    fn macros_page(&mut self, ui: &mut egui::Ui) {
        let before = self.macro_value.clone();
        let p = Palette::new(self.dark);
        egui::CollapsingHeader::new(t("Библиотека макросов"))
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    egui::ComboBox::from_id_salt("local-macro-library")
                        .width(220.0)
                        .selected_text(if self.local_macro_name.is_empty() { t("Выберите макрос") } else { &self.local_macro_name })
                        .show_ui(ui, |ui| {
                            for name in &self.controller.snapshot.macro_library {
                                if ui.selectable_value(&mut self.local_macro_name, name.clone(), name).changed() {
                                    self.local_macro_rename = name.clone();
                                }
                            }
                        });
                    if ui.add_enabled(!self.local_macro_name.is_empty() && !self.controller.snapshot.busy, egui::Button::new(t("Открыть"))).clicked() {
                        self.stop_recording(None);
                        self.local_macro_seen = None;
                        self.command(Command::OpenLocalMacro(self.local_macro_name.clone()));
                    }
                    if ui.button(t("Новый макрос")).clicked() {
                        self.stop_recording(None);
                        self.local_macro_name.clear();
                        self.local_macro_rename.clear();
                        self.macro_value = MouseMacro { name: t("Новый макрос").into(), repeat_mode: 0, events: Vec::new(), reserved: [0; 5] };
                        self.macro_dirty = true;
                    }
                    if ui.add_enabled(!self.controller.snapshot.busy && !self.macro_value.name.trim().is_empty(), egui::Button::new(t("Сохранить в библиотеку"))).clicked() {
                        self.stop_recording(None);
                        let command = if self.controller.snapshot.macro_library.contains(&self.macro_value.name) && self.local_macro_name == self.macro_value.name {
                            Command::UpdateLocalMacro { name: self.local_macro_name.clone(), value: self.macro_value.clone() }
                        } else {
                            Command::SaveLocalMacro(self.macro_value.clone())
                        };
                        self.command(command);
                    }
                });
                if !self.local_macro_name.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.local_macro_rename).desired_width(220.0));
                        if ui.add_enabled(!self.controller.snapshot.busy && !self.local_macro_rename.trim().is_empty() && self.local_macro_name != self.local_macro_rename, egui::Button::new(t("Переименовать"))).clicked() {
                            self.pending = Some((format!("Переименовать локальный макрос «{}» в «{}»?", self.local_macro_name, self.local_macro_rename), Command::RenameLocalMacro { from: self.local_macro_name.clone(), to: self.local_macro_rename.clone() }));
                        }
                        if ui.add_enabled(!self.controller.snapshot.busy, egui::Button::new(t("Удалить из библиотеки"))).clicked() {
                            self.pending = Some((format!("Убрать «{}» из библиотеки? Сохранённые версии переместятся в резервную папку.", self.local_macro_name), Command::DeleteLocalMacro(self.local_macro_name.clone())));
                        }
                    });
                }
            });
        ui.add_space(12.0);
        if !self.imported_macros.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(t("Импортированные макросы"));
                for value in self.imported_macros.clone() {
                    if ui.button(&value.name).clicked() {
                        self.stop_recording(None);
                        self.macro_value = value;
                        self.macro_dirty = true;
                    }
                }
            });
            ui.add_space(10.0);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Название"));
            self.macro_dirty |= ui
                .add(egui::TextEdit::singleline(&mut self.macro_value.name).desired_width(190.0))
                .changed();
            ui.label(t("Ячейка"));
            ui.add(egui::DragValue::new(&mut self.macro_slot).range(0..=17));
            ui.label(tf!("{} событий", self.macro_value.events.len()));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Повтор"));
            egui::ComboBox::from_id_salt("macro-repeat")
                .selected_text(t(match self.macro_value.repeat_mode {
                    0 => "Пока удерживается",
                    1 => "До повторного нажатия",
                    2 => "Выполнить один раз",
                    _ => "До другой кнопки",
                }))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.macro_value.repeat_mode, 0, t("Пока удерживается"));
                    ui.selectable_value(
                        &mut self.macro_value.repeat_mode,
                        1,
                        t("До повторного нажатия"),
                    );
                    ui.selectable_value(&mut self.macro_value.repeat_mode, 3, t("До другой кнопки"));
                    ui.selectable_value(&mut self.macro_value.repeat_mode, 2, t("Выполнить один раз"));
                });
            ui.checkbox(&mut self.fixed_delay, t("Фиксированная задержка"));
            ui.add_enabled(
                self.fixed_delay,
                egui::DragValue::new(&mut self.delay_ms)
                    .range(1..=60000)
                    .suffix(t(" мс")),
            );
        });
        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(t(if self.recording {
                            "Остановить запись (Esc)"
                        } else {
                            "Начать запись"
                        }))
                        .color(if self.recording {
                            Color32::WHITE
                        } else {
                            p.text
                        }),
                    )
                    .fill(if self.recording {
                        Color32::from_rgb(180, 35, 35)
                    } else {
                        p.field
                    }),
                )
                .clicked()
            {
                if self.recording {
                    self.stop_recording(None);
                } else {
                    self.recorder = match crate::recorder::WindowKeyboardRecorder::start(ui.ctx()) {
                        Ok(recorder) => Some(recorder),
                        Err(error) => {
                            self.local_message =
                                format!("Точная запись клавиш недоступна: {error}");
                            None
                        }
                    };
                    self.recording = true;
                    self.recorded_at = None;
                    self.record_modifiers = 0;
                    self.record_held.clear();
                }
            }
            if ui.button(t("Добавить событие")).clicked() {
                self.macro_value.events.push(MacroEvent {
                    delay_ms: 0,
                    kind: 17,
                    pressed: true,
                    value: 4,
                });
                self.macro_dirty = true;
            }
            if ui.button(t("Копировать JSON")).clicked() {
                match serde_json::to_string_pretty(&self.macro_value) {
                    Ok(json) => ui.ctx().copy_text(json),
                    Err(error) => self.local_message = error.to_string(),
                }
            }
        });
        if self.recording {
            ui.label(
                RichText::new(t("Записываются только события в этом окне. Esc завершает запись."))
                    .color(p.accent_text),
            );
            if self.recorder.is_none() {
                ui.label(RichText::new(t("Ctrl / Shift / Alt записываются как левые. Super и правые модификаторы можно добавить вручную.")).small().color(p.muted));
            }
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 90.0), egui::Sense::hover());
            self.record_area = rect;
            if let Some(recorder) = &mut self.recorder {
                if let Err(error) = recorder.set_pointer_area(rect, ui.ctx().pixels_per_point()) {
                    self.local_message = format!("Запись событий остановлена: {error}");
                    self.stop_recording(None);
                }
            }
            ui.painter().rect_filled(rect, 8, p.field);
            ui.painter().rect_stroke(
                rect,
                8,
                Stroke::new(1.0_f32, p.accent),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                t("Нажимайте кнопки мыши и вращайте колесо в этой области"),
                FontId::proportional(14.0),
                p.text,
            );
        }
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(p.field)
            .corner_radius(12)
            .inner_margin(16)
            .show(ui, |ui| {
                if self.macro_value.events.is_empty() {
                    ui.add_space(30.0);
                    ui.label(t("Событий пока нет. Начните запись или добавьте событие вручную."));
                    ui.add_space(30.0);
                }
                let mut delete = None;
                egui::Grid::new("macro-events")
                    .num_columns(6)
                    .striped(true)
                    .spacing([14.0, 9.0])
                    .show(ui, |ui| {
                        for label in ["№", "Задержка", "Тип", "Состояние", "Код", " "]
                        {
                            ui.label(RichText::new(t(label)).color(p.muted));
                        }
                        ui.end_row();
                        for (index, event) in self.macro_value.events.iter_mut().enumerate() {
                            ui.label((index + 1).to_string());
                            self.macro_dirty |= ui
                                .add_enabled(
                                    event.kind == 63,
                                    egui::DragValue::new(&mut event.delay_ms)
                                        .range(0..=60000)
                                        .suffix(t(" мс")),
                                )
                                .changed();
                            egui::ComboBox::from_id_salt(("event-kind", index))
                                .selected_text(event_kind(event.kind))
                                .show_ui(ui, |ui| {
                                    for (kind, label) in [
                                        (17, "Клавиша"),
                                        (0, "Кнопка мыши"),
                                        (5, "Колесо"),
                                        (63, "Задержка"),
                                    ] {
                                        self.macro_dirty |= ui
                                            .selectable_value(&mut event.kind, kind, t(label))
                                            .changed();
                                    }
                                });
                            self.macro_dirty |= ui.checkbox(&mut event.pressed, t("Нажата")).changed();
                            self.macro_dirty |= ui
                                .add(egui::DragValue::new(&mut event.value).range(0..=255))
                                .on_hover_text(usage_name(event.value))
                                .changed();
                            if ui.small_button(t("Удалить")).clicked() {
                                delete = Some(index);
                            }
                            ui.end_row();
                        }
                    });
                if let Some(index) = delete {
                    self.macro_value.events.remove(index);
                    self.macro_dirty = true;
                }
            });
        ui.add_space(16.0);
        if self.preset_edit.is_some() {
            ui.horizontal_wrapped(|ui| {
                if ui.button(t("Открыть макрос профиля")).clicked() {
                    self.stop_recording(None);
                    if let Some(value) = self
                        .preset_edit
                        .as_ref()
                        .and_then(|(_, profile)| profile.macros.get(&self.macro_slot))
                        .cloned()
                    {
                        self.macro_value = value;
                        self.macro_dirty = true;
                    } else {
                        self.local_message =
                            "В этой ячейке профиля нет сохранённого макроса.".into();
                    }
                }
                if ui
                    .add_enabled(
                        !self.macro_value.events.is_empty(),
                        egui::Button::new(t("Добавить в локальный профиль")),
                    )
                    .clicked()
                {
                    if let Some((_, profile)) = &mut self.preset_edit {
                        profile
                            .macros
                            .insert(self.macro_slot, self.macro_value.clone());
                        self.dirty_preset = true;
                    }
                }
                if ui.button(t("Убрать из локального профиля")).clicked() {
                    if let Some((_, profile)) = &mut self.preset_edit {
                        let referenced = self.staged_keys.iter().any(|key| {
                            key.kind == 0x21 && key.data.first() == Some(&self.macro_slot)
                        });
                        if referenced {
                            self.local_message =
                                "Сначала уберите назначение этого макроса во вкладке «Кнопки»."
                                    .into();
                        } else {
                            self.dirty_preset |= profile.macros.remove(&self.macro_slot).is_some();
                        }
                    }
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        self.controller.snapshot.mouse.is_some() && !self.controller.snapshot.busy,
                        egui::Button::new(t("Прочитать из мыши")),
                    )
                    .clicked()
                {
                    self.macro_dirty = false;
                    self.macro_seen = None;
                    self.command(Command::ReadMacro(self.macro_slot));
                }
                if ui
                    .add_enabled(
                        self.writable()
                            && self.controller.snapshot.mouse.is_some()
                            && !self.macro_value.events.is_empty(),
                        egui::Button::new(RichText::new(t("Записать в мышь")).color(Color32::WHITE))
                            .fill(p.accent),
                    )
                    .clicked()
                {
                    self.stop_recording(None);
                    self.command(Command::WriteMacro {
                        id: self.macro_slot,
                        value: self.macro_value.clone(),
                    });
                }
            });
        }
        ui.add_space(14.0);
        ui.label(RichText::new(t("Локальный файл макроса")).strong());
        ui.add(egui::TextEdit::singleline(&mut self.macro_path).desired_width(f32::INFINITY));
        ui.horizontal_wrapped(|ui| {
            if ui.button(t("Импортировать JSON")).clicked() {
                match std::fs::read_to_string(&self.macro_path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| {
                        let value: serde_json::Value =
                            serde_json::from_str(&s).map_err(|e| e.to_string())?;
                        if let Ok(native) = serde_json::from_value::<MouseMacro>(value.clone()) {
                            return Ok(vec![native]);
                        }
                        crate::hub_format::macros_from_hub_json(&value).map_err(|e| e.to_string())
                    }) {
                    Ok(values) => {
                        self.stop_recording(None);
                        if let Some(value) = values.first() {
                            self.macro_value = value.clone();
                        }
                        self.imported_macros = values;
                        self.macro_dirty = true;
                        self.local_message = "Макрос импортирован в редактор.".into();
                    }
                    Err(e) => self.local_message = e,
                }
            }
            if ui.button(t("Сохранить новый файл")).clicked() {
                self.stop_recording(None);
                match serde_json::to_vec_pretty(&self.macro_value)
                    .map_err(|e| e.to_string())
                    .and_then(|data| write_new_file(&self.macro_path, &data))
                {
                    Ok(()) => self.local_message = format!("Макрос сохранён: {}", self.macro_path),
                    Err(e) => self.local_message = e,
                }
            }
            if ui.button(t("Экспорт для M HUB")).clicked() {
                self.stop_recording(None);
                let data = crate::hub_format::macro_to_hub_json(&self.macro_value)
                    .map_err(|e| e.to_string())
                    .and_then(|value| serde_json::to_vec_pretty(&value).map_err(|e| e.to_string()));
                match data.and_then(|data| write_new_file(&self.macro_path, &data)) {
                    Ok(()) => {
                        self.local_message = format!("Макрос M HUB сохранён: {}", self.macro_path)
                    }
                    Err(error) => self.local_message = error,
                }
            }
        });
        self.macro_dirty |= before != self.macro_value;
    }
    fn profiles_page(&mut self, ui: &mut egui::Ui) {
        if self.preset_edit.is_some() {
            ui.label(t("Измените кнопки, DPI, производительность и макросы во вкладках. Сохранение доступно над вкладкой."));
            return;
        }
        let p = Palette::new(self.dark);
        let profile = self.staged_settings.as_ref().map_or(0, |s| s.profile_index);
        setting_box(
            ui,
            p,
            "Библиотека профилей",
            "Локальные копии для этой мыши. Сохранение не меняет её настройки.",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.preset_name)
                            .hint_text(t("Имя локального профиля"))
                            .desired_width(250.0),
                    );
                    if ui
                        .add_enabled(
                            !self.preset_name.trim().is_empty()
                                && !self.controller.snapshot.busy
                                && self.controller.snapshot.mouse.is_some()
                                && self.preset_edit.is_none(),
                            egui::Button::new(t("Сохранить текущий")),
                        )
                        .clicked()
                    {
                        self.command(Command::SavePreset(self.preset_name.clone()));
                    }
                });
                for preset in self.controller.snapshot.presets.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&preset.name).strong());
                        if ui.button(t("Просмотр")).clicked() {
                            self.profile_path = preset.path.to_string_lossy().into_owned();
                            self.command(Command::PreviewPreset(preset.path.clone()));
                        }
                        if ui
                            .add_enabled(self.writable(), egui::Button::new(t("Применить")))
                            .clicked()
                        {
                            self.pending = Some((
                                format!("Заменить текущий профиль настройками «{}»?", preset.name),
                                Command::ImportProfile(preset.path.clone()),
                            ));
                        }
                        if ui
                            .add_enabled(
                                !self.preset_name.trim().is_empty(),
                                egui::Button::new(t("Переименовать")),
                            )
                            .clicked()
                        {
                            self.pending = Some((
                                format!(
                                    "Переименовать «{}» в «{}»?",
                                    preset.name, self.preset_name
                                ),
                                Command::RenamePreset {
                                    path: preset.path.clone(),
                                    name: self.preset_name.clone(),
                                },
                            ));
                        }
                        if ui.button(t("Удалить")).clicked() {
                            self.pending = Some((
                                format!("Удалить локальный профиль «{}»?", preset.name),
                                Command::DeletePreset(preset.path),
                            ));
                        }
                    });
                }
                if let Some(preview) = self.controller.snapshot.preview.clone() {
                    egui::CollapsingHeader::new(t("Содержимое выбранного профиля"))
                        .default_open(true)
                        .show(ui, |ui| {
                            let snapshot = &preview.snapshot;
                            ui.label(tf!(
                                "DPI: {:?} · уровней {} · LOD {}",
                                &snapshot.dpi.x[..snapshot.dpi.count.min(6) as usize],
                                snapshot.dpi.count,
                                t(LODS.get(snapshot.lod_index as usize)
                                    .unwrap_or(&"неизвестно"))
                            ));
                            ui.label(tf!(
                                "Частота USB: {} Гц · радиоканал: {} Гц · макросов: {}",
                                RATES
                                    .get(snapshot.settings.usb_rate_index as usize)
                                    .unwrap_or(&0),
                                RATES
                                    .get(snapshot.settings.wireless_rate_index as usize)
                                    .unwrap_or(&0),
                                preview.macros.len()
                            ));
                            if self.preset_edit.is_none()
                                && self.preview_path.is_some()
                                && ui
                                    .add_enabled(
                                        !self.controller.snapshot.busy,
                                        egui::Button::new(t("Редактировать локальную копию")),
                                    )
                                    .clicked()
                            {
                                self.begin_preset_edit(
                                    self.preview_path.clone().unwrap(),
                                    preview.clone(),
                                );
                            }
                            if let Ok(json) = serde_json::to_string_pretty(&preview) {
                                egui::CollapsingHeader::new(t("Полный JSON")).show(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(json).monospace().size(11.0),
                                        )
                                        .wrap(),
                                    );
                                });
                            }
                        });
                }
            },
        );
        if self.controller.snapshot.mouse.is_none() || self.preset_edit.is_some() {
            return;
        }
        ui.add_space(12.0);
        setting_box(
            ui,
            p,
            "Название профиля",
            "Название сохраняется в выбранную ячейку памяти.",
            |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.profile_name).desired_width(300.0));
                    if ui
                        .add_enabled(
                            self.writable() && !self.profile_name.trim().is_empty(),
                            egui::Button::new(t("Переименовать")),
                        )
                        .clicked()
                    {
                        self.command(Command::RenameProfile {
                            profile,
                            name: self.profile_name.clone(),
                        });
                    }
                });
            },
        );
        ui.add_space(12.0);
        setting_box(
            ui,
            p,
            "Резервная копия",
            "Экспорт и импорт локального JSON с настройками текущего профиля.",
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.profile_path).desired_width(f32::INFINITY),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t("Экспортировать в новый файл")).clicked() {
                        let path = PathBuf::from(&self.profile_path);
                        self.command(Command::ExportProfile(path));
                    }
                    if ui
                        .add_enabled(self.writable(), egui::Button::new(t("Импортировать профиль")))
                        .clicked()
                    {
                        self.pending = Some((
                            "Заменить настройки текущего профиля данными из файла?".into(),
                            Command::ImportProfile(PathBuf::from(&self.profile_path)),
                        ));
                    }
                });
            },
        );
        ui.add_space(16.0);
        ui.label(RichText::new(t("Сброс")).strong());
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.writable(),
                    egui::Button::new(t("Сбросить назначения кнопок")),
                )
                .clicked()
            {
                self.pending = Some((
                    "Восстановить назначения кнопок активного профиля?".into(),
                    Command::ResetProfile {
                        profile,
                        buttons_only: true,
                    },
                ));
            }
            if ui
                .add_enabled(self.writable(), egui::Button::new(t("Сбросить этот профиль")))
                .clicked()
            {
                self.pending = Some((
                    "Сбросить настройки активного профиля мыши?".into(),
                    Command::ResetProfile {
                        profile,
                        buttons_only: false,
                    },
                ));
            }
            if ui
                .add_enabled(self.writable(), egui::Button::new(t("Сбросить все профили")))
                .clicked()
            {
                self.pending = Some((
                    "Сбросить все профили в памяти мыши? Сначала экспортируйте нужные настройки."
                        .into(),
                    Command::ResetAllProfiles,
                ));
            }
        });
    }
    fn system_page(&mut self, ui: &mut egui::Ui) {
        let p = Palette::new(self.dark);
        ui.heading(t("Устройства и прошивки"));
        ui.add_space(12.0);
        setting_box(
            ui,
            p,
            "Подключение",
            "Устройства, обнаруженные через USB HID.",
            |ui| {
                if self.controller.snapshot.devices.is_empty() {
                    ui.label(t("Совместимые HID-интерфейсы пока не найдены."));
                }
                for dev in self.controller.snapshot.devices.clone() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&dev.name).strong());
                        ui.label(format!(
                            "{:04X}:{:04X} · {}",
                            dev.vendor_id, dev.product_id, dev.path
                        ));
                        if dev.kind == crate::transport::DeviceKind::Mouse {
                            let active = self.controller.snapshot.active_mouse_path.as_ref() == Some(&dev.path);
                            if active {
                                ui.label(RichText::new(t("Активно")).color(p.accent_text));
                            } else if ui.add_enabled(!self.controller.snapshot.busy, egui::Button::new(t("Подключить"))).clicked() {
                                if self.has_dirty() {
                                    self.local_message = "Сначала примените или отмените изменения текущего устройства.".into();
                                } else {
                                    self.command(Command::SelectMouse(dev.path.clone()));
                                }
                            }
                        }
                    });
                }
            },
        );
        egui::CollapsingHeader::new(t("Сопряжение приёмника")).show(ui, |ui| {
            ui.label(t("1. Отключите кабель от мыши и включите режим 2,4 ГГц."));
            ui.label(t("2. Удерживайте левую, среднюю и правую кнопки 3–5 секунд."));
            ui.label(t("3. Вставьте приёмник в USB и поднесите мышь к нему."));
        });
        ui.add_space(10.0);
        setting_box(
            ui,
            p,
            "Версии прошивок",
            "Данные, прочитанные непосредственно из устройств.",
            |ui| {
                egui::Grid::new("firmware-versions")
                    .num_columns(2)
                    .spacing([32.0, 12.0])
                    .show(ui, |ui| {
                        ui.label(t("Мышь"));
                        ui.label(
                            self.controller
                                .snapshot
                                .mouse
                                .as_ref()
                                .map_or(t("Нет данных"), |m| m.mouse_firmware.as_str()),
                        );
                        ui.end_row();
                        ui.label(t("Приёмник"));
                        ui.label(
                            self.controller
                                .snapshot
                                .mouse
                                .as_ref()
                                .map_or(t("Нет данных"), |m| m.receiver_firmware.as_str()),
                        );
                        ui.end_row();
                        ui.label("MagDock");
                        ui.label(
                            self.controller
                                .snapshot
                                .dock
                                .as_ref()
                                .map_or(t("Нет данных"), |d| d.firmware_version.as_str()),
                        );
                        ui.end_row();
                    });
            },
        );
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !self.controller.snapshot.busy,
                    egui::Button::new(t("Проверить обновления")),
                )
                .clicked()
            {
                self.command(Command::CheckFirmware);
            }
            ui.label(RichText::new(t("Официальный каталог MCHOSE")).color(p.muted));
        });
        if let Some(catalog) = self.controller.snapshot.firmware.clone() {
            ui.add(
                egui::TextEdit::singleline(&mut self.firmware_dir)
                    .desired_width(f32::INFINITY)
                    .hint_text(t("Папка для файлов прошивки")),
            );
            for (index, entry) in catalog.images.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(tf!("{} · версия {:04}", i18n::message(&entry.role), entry.version));
                    if ui
                        .add_enabled(
                            !self.controller.snapshot.busy,
                            egui::Button::new(t("Скачать файлы")),
                        )
                        .clicked()
                    {
                        self.command(Command::DownloadFirmware {
                            index,
                            directory: PathBuf::from(&self.firmware_dir),
                        });
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(tf!(
                    "MagDock · официальный файл {:04}",
                    catalog.dock.version
                ));
                let legacy = self
                    .controller
                    .snapshot
                    .dock
                    .as_ref()
                    .and_then(|dock| dock.firmware_version.parse::<u32>().ok())
                    .is_some_and(|version| version < 3);
                if legacy {
                    ui.hyperlink_to(t("Утилита обновления для Windows"), &catalog.dock.legacy_url);
                } else if ui
                    .add_enabled(
                        !self.controller.snapshot.busy,
                        egui::Button::new(t("Скачать MagDock")),
                    )
                    .clicked()
                {
                    self.command(Command::DownloadDockFirmware(PathBuf::from(
                        &self.firmware_dir,
                    )));
                }
            });
        }
        ui.add_space(14.0);
        setting_box(
            ui,
            p,
            "Файл прошивки",
            "Проверка файла не запускает обновление устройства.",
            |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.firmware_path)
                        .desired_width(f32::INFINITY)
                        .hint_text(t("Путь к файлу .bin")),
                );
                if ui.button(t("Проверить файл")).clicked() {
                    use sha2::{Digest, Sha256};
                    self.firmware_info = match std::fs::read(&self.firmware_path) {
                        Ok(data) => format!(
                            "Размер: {} байт\nSHA-256: {:x}\nСовместимость с моделью по одному бинарному файлу не подтверждена.",
                            data.len(),
                            Sha256::digest(&data)
                        ),
                        Err(error) => error.to_string(),
                    };
                }
                if !self.firmware_info.is_empty() {
                    ui.label(i18n::message(&self.firmware_info));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.firmware_target, 0, t("Мышь / приёмник"));
                    ui.selectable_value(&mut self.firmware_target, 1, "MagDock");
                });
                if self.firmware_target == 0 {
                    egui::ComboBox::from_id_salt("flash-device")
                        .selected_text(if self.firmware_device.is_empty() {
                            t("Выберите USB-устройство")
                        } else {
                            self.firmware_device.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for device in self
                                .controller
                                .snapshot
                                .devices
                                .iter()
                                .filter(|d| d.kind == crate::transport::DeviceKind::Mouse)
                            {
                                ui.selectable_value(
                                    &mut self.firmware_device,
                                    device.path.clone(),
                                    format!(
                                        "{} · {:04X} · {}",
                                        device.name, device.product_id, device.path
                                    ),
                                );
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.firmware_sub_path)
                            .desired_width(f32::INFINITY)
                            .hint_text(t("Парный файл Sub .bin (выше укажите Main)")),
                    );
                }
                let ready = self.writable()
                    && !self.firmware_path.is_empty()
                    && (self.firmware_target == 1
                        || (!self.firmware_device.is_empty()
                            && !self.firmware_sub_path.is_empty()));
                if ui
                    .add_enabled(ready, egui::Button::new(t("Подготовить обновление")))
                    .clicked()
                {
                    let paths = if self.firmware_target == 0 {
                        vec![self.firmware_path.as_str(), self.firmware_sub_path.as_str()]
                    } else {
                        vec![self.firmware_path.as_str()]
                    };
                    let summaries: Result<Vec<_>, _> =
                        paths.into_iter().map(firmware_summary).collect();
                    match summaries {
                        Ok(summaries) => {
                            let target = if self.firmware_target == 0 {
                                self.firmware_device.clone()
                            } else {
                                "MagDock 3837:1012".into()
                            };
                            let message = format!(
                                "Обновить прошивку {target}?\n\n{}\n\nФайлы должны соответствовать этому устройству. Не отключайте USB до завершения.",
                                summaries.join("\n\n")
                            );
                            let command = if self.firmware_target == 0 {
                                Command::FlashMouse {
                                    device_path: self.firmware_device.clone(),
                                    main: PathBuf::from(&self.firmware_path),
                                    sub: PathBuf::from(&self.firmware_sub_path),
                                }
                            } else {
                                Command::FlashDock(PathBuf::from(&self.firmware_path))
                            };
                            self.pending = Some((message, command));
                        }
                        Err(error) => self.local_message = error,
                    }
                }
            },
        );
        ui.add_space(14.0);
        egui::CollapsingHeader::new(t("Диагностика USB и исходные данные")).show(ui, |ui| {
            if let Some(mouse) = &self.controller.snapshot.mouse {
                if let Ok(json) = serde_json::to_string_pretty(mouse) {
                    if ui.button(t("Копировать снимок JSON")).clicked() {
                        ui.ctx().copy_text(json.clone());
                    }
                    ui.add(egui::Label::new(RichText::new(json).monospace().size(11.0)).wrap());
                }
            }
        });
    }
    fn record_events(&mut self, ctx: &egui::Context) {
        if !self.recording {
            return;
        }
        if let Some(recorder) = &mut self.recorder {
            let batch = recorder
                .set_pointer_area(self.record_area, ctx.pixels_per_point())
                .and_then(|()| recorder.poll());
            match batch {
                Ok(batch) => {
                    for event in batch.events {
                        if event.kind == 17 && event.value == 41 && event.pressed {
                            self.stop_recording(Some(event.delay_ms));
                            break;
                        }
                        let delay = Some(event.delay_ms);
                        self.append_recorded_timed(event.kind, event.pressed, event.value, delay);
                        if !self.recording {
                            break;
                        }
                    }
                    if batch.focus_lost {
                        self.stop_recording(None);
                    }
                }
                Err(error) => {
                    self.local_message = format!("Запись клавиш остановлена: {error}");
                    self.stop_recording(None);
                }
            }
        }
        if !self.recording || self.recorder.is_some() {
            return;
        }
        if self.recorder.is_none() {
            let modifiers = ctx.input(|i| i.modifiers);
            let current = u8::from(modifiers.ctrl)
                | u8::from(modifiers.shift) << 1
                | u8::from(modifiers.alt) << 2
                | u8::from(modifiers.mac_cmd) << 3;
            for bit in 0..4 {
                let mask = 1 << bit;
                if (current ^ self.record_modifiers) & mask != 0 {
                    self.append_recorded(17, current & mask != 0, 224 + bit);
                }
            }
            self.record_modifiers = current;
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let data = match event {
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => {
                    self.stop_recording(None);
                    None
                }
                egui::Event::Key {
                    key,
                    pressed,
                    repeat: false,
                    ..
                } if self.recorder.is_none() => {
                    egui_key_usage(key).map(|value| (17, pressed, value))
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } if self.record_area.contains(pos)
                    || (!pressed
                        && self.record_held.contains(&(
                            0,
                            match button {
                                egui::PointerButton::Primary => 1,
                                egui::PointerButton::Secondary => 2,
                                egui::PointerButton::Middle => 4,
                                egui::PointerButton::Extra1 => 8,
                                egui::PointerButton::Extra2 => 16,
                            },
                        ))) =>
                {
                    Some((
                        0,
                        pressed,
                        match button {
                            egui::PointerButton::Primary => 1,
                            egui::PointerButton::Secondary => 2,
                            egui::PointerButton::Middle => 4,
                            egui::PointerButton::Extra1 => 8,
                            egui::PointerButton::Extra2 => 16,
                        },
                    ))
                }
                egui::Event::MouseWheel { delta, .. }
                    if ctx.input(|i| {
                        i.pointer
                            .hover_pos()
                            .is_some_and(|pos| self.record_area.contains(pos))
                    }) =>
                {
                    Some((5, false, u8::from(delta.y < 0.0)))
                }
                _ => None,
            };
            if let Some((kind, pressed, value)) = data {
                if self.recording {
                    self.append_recorded(kind, pressed, value);
                }
            }
        }
    }
    fn append_recorded(&mut self, kind: u8, pressed: bool, value: u8) {
        self.append_recorded_timed(kind, pressed, value, None);
    }
    fn append_recorded_timed(
        &mut self,
        kind: u8,
        pressed: bool,
        value: u8,
        source_delay: Option<u16>,
    ) {
        let now = Instant::now();
        let delay = if self.fixed_delay {
            self.delay_ms
        } else {
            source_delay
                .map(|delay| delay.min(60000))
                .unwrap_or_else(|| {
                    self.recorded_at.map_or(0, |last| {
                        now.duration_since(last).as_millis().min(60000) as u16
                    })
                })
        };
        let mut held_after = self.record_held.len();
        if matches!(kind, 0 | 17) {
            if pressed && !self.record_held.contains(&(kind, value)) {
                held_after += 1;
            } else if !pressed && self.record_held.contains(&(kind, value)) {
                held_after -= 1;
            }
        }
        if self.macro_value.events.len()
            + usize::from(delay > 0)
            + 1
            + held_after
            + usize::from(held_after > 0)
            > 1008
        {
            self.stop_recording(None);
            self.local_message = "Достигнут предел размера макроса.".into();
            return;
        }
        if delay > 0 {
            self.macro_value.events.push(MacroEvent {
                delay_ms: delay,
                kind: 63,
                pressed: false,
                value: 0,
            });
        }
        self.macro_value.events.push(MacroEvent {
            delay_ms: 0,
            kind,
            pressed,
            value,
        });
        if matches!(kind, 0 | 17) {
            if pressed {
                self.record_held.insert((kind, value));
            } else {
                self.record_held.remove(&(kind, value));
            }
        }
        self.recorded_at = Some(now);
        self.macro_dirty = true;
    }
    fn stop_recording(&mut self, source_delay: Option<u16>) {
        self.recording = false;
        let source_delay = source_delay.or_else(|| {
            self.recorder
                .as_ref()
                .and_then(|recorder| recorder.elapsed_since_last_event())
        });
        self.recorder = None;
        if !self.record_held.is_empty() {
            let delay = if self.fixed_delay {
                self.delay_ms
            } else {
                source_delay
                    .map(|delay| delay.min(60000))
                    .unwrap_or_else(|| {
                        self.recorded_at
                            .map_or(0, |last| last.elapsed().as_millis().min(60000) as u16)
                    })
            };
            if delay > 0 {
                self.macro_value.events.push(MacroEvent {
                    delay_ms: delay,
                    kind: 63,
                    pressed: false,
                    value: 0,
                });
            }
        }
        for (kind, value) in std::mem::take(&mut self.record_held) {
            self.macro_value.events.push(MacroEvent {
                delay_ms: 0,
                kind,
                pressed: false,
                value,
            });
        }
        self.record_modifiers = 0;
        self.recorded_at = None;
    }
    fn confirm(&mut self, ctx: &egui::Context) {
        let Some((message, _)) = &self.pending else {
            return;
        };
        let message = i18n::message(message);
        let mut yes = false;
        let mut no = false;
        egui::Window::new(t("Подтвердите действие"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(message);
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    yes = ui.button(t("Подтвердить")).clicked();
                    no = ui.button(t("Отмена")).clicked();
                });
            });
        if yes {
            if let Some((_, cmd)) = self.pending.take() {
                self.command(cmd);
            }
        } else if no {
            self.pending = None;
        }
    }
}
impl eframe::App for NativeApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        self.ace75.filter_raw_input(ctx, input);
        if self.recording && self.recorder.is_some() {
            let mut pointer = ctx.input(|input| input.pointer.hover_pos());
            input.events.retain(|event| {
                if let egui::Event::PointerMoved(position) = event {
                    pointer = Some(*position);
                }
                !is_keyboard_input(event)
                    && !(matches!(event, egui::Event::MouseWheel { .. })
                        && pointer.is_some_and(|position| self.record_area.contains(position)))
            });
            input.modifiers = egui::Modifiers::NONE;
        }
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.theme_pending {
            self.style(ctx);
            self.theme_pending = false;
        }
        if self.recording && (self.device_view != DeviceView::Mouse || self.page != Page::Macros) {
            self.stop_recording(None);
        }
        if self.device_view != DeviceView::Ace75 { self.ace75.stop_recording(None); }
        self.ace75.poll_recording(ctx);
        self.controller.poll();
        self.sync();
        let was_recording = self.recording;
        self.record_events(ctx);
        if was_recording {
            ctx.input_mut(|input| {
                input.events.retain(|event| !is_keyboard_input(event));
                input.modifiers = egui::Modifiers::NONE;
            });
        }
        self.top(ctx);
        self.footer(ctx);
        self.sidebar(ctx);
        self.content(ctx);
        if self.recording && (self.device_view != DeviceView::Mouse || self.page != Page::Macros) {
            self.stop_recording(None);
        }
        self.confirm(ctx);
        if self.capture_path.is_some() {
            self.capture_frames += 1;
            if self.capture_frames >= 15
                && !self.controller.snapshot.busy
                && !self.capture_requested
            {
                self.capture_requested = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    if let Some(path) = self.capture_path.take() {
                        let bytes: Vec<u8> =
                            image.pixels.iter().flat_map(|p| p.to_array()).collect();
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if let Err(error) = image::save_buffer(
                            &path,
                            &bytes,
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        ) {
                            eprintln!("Screenshot: {error}");
                        }
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(100));
    }
}
fn is_keyboard_input(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_)
            | egui::Event::Ime(_)
    )
}
fn load_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> egui::TextureHandle {
    let image = image::load_from_memory(bytes)
        .expect("Embedded product image")
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    ctx.load_texture(
        name,
        egui::ColorImage::from_rgba_unmultiplied(size, &image),
        egui::TextureOptions::LINEAR,
    )
}
fn firmware_summary(path: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if bytes.is_empty() {
        return Err(format!("Пустой файл: {path}"));
    }
    Ok(tf!(
        "{path}\n{} байт\nSHA-256: {:x}",
        bytes.len(),
        Sha256::digest(&bytes)
    ))
}
fn setting_box(
    ui: &mut egui::Ui,
    p: Palette,
    title: &'static str,
    desc: &'static str,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(p.field)
        .corner_radius(12)
        .inner_margin(20)
        .show(ui, |ui| {
            ui.set_min_width((ui.available_width() - 2.0).max(0.0));
            ui.label(RichText::new(t(title)).strong());
            ui.label(RichText::new(t(desc)).size(13.0).color(p.muted));
            ui.add_space(8.0);
            body(ui);
        });
}
fn status_dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(9.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}
fn rate_combo(ui: &mut egui::Ui, id: &str, index: &mut u8) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(tf!(
            "{} Гц",
            RATES.get(*index as usize).copied().unwrap_or(0)
        ))
        .show_ui(ui, |ui| {
            for (i, rate) in RATES.iter().enumerate() {
                ui.selectable_value(index, i as u8, tf!("{} Гц", rate));
            }
        });
}
fn default_path(name: &str) -> String {
    std::env::var("HOME")
        .map(|base| format!("{base}/Downloads/mchose-{name}"))
        .unwrap_or_else(|_| format!("mchose-{name}"))
}
fn write_new_file(path: &str, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("Не удалось создать файл: {e}. Для сохранения выберите новое имя."))?;
    file.write_all(data).map_err(|e| e.to_string())
}
fn event_kind(kind: u8) -> &'static str {
    t(match kind {
        17 => "Клавиша",
        0 => "Кнопка мыши",
        5 => "Колесо",
        63 => "Задержка",
        _ => "Другой",
    })
}
#[derive(serde::Deserialize)]
struct HubAction {
    category: String,
    name_ru: String,
    name_en: String,
    kind: u8,
    data: Vec<u8>,
}
impl HubAction {
    fn name(&self) -> &str {
        if i18n::language() == Language::En { &self.name_en } else { &self.name_ru }
    }
}
fn hub_actions() -> &'static Vec<HubAction> {
    static ACTIONS: std::sync::OnceLock<Vec<HubAction>> = std::sync::OnceLock::new();
    ACTIONS.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/hub-actions.json"))
            .expect("Embedded Hub actions")
    })
}
fn removes_last_left(keys: &[KeyBinding], selected: usize, replacement: &KeyBinding) -> bool {
    let left = |key: &KeyBinding| key.kind == 0 && key.data.first() == Some(&1);
    keys.get(selected).is_some_and(left)
        && !left(replacement)
        && !keys
            .iter()
            .enumerate()
            .any(|(index, key)| index != selected && left(key))
}
fn default_binding(index: usize) -> KeyBinding {
    match index {
        0 => KeyBinding::mouse(1),
        1 => KeyBinding::mouse(2),
        2 => KeyBinding::mouse(4),
        3 => KeyBinding::mouse(16),
        4 => KeyBinding::mouse(8),
        _ => KeyBinding::dpi(0),
    }
}
fn binding_name(binding: &KeyBinding) -> String {
    if let Some(action) = hub_actions()
        .iter()
        .find(|action| action.kind == binding.kind && action.data == binding.data)
    {
        return action.name().to_owned();
    }
    match binding.kind {
        0 => match binding.data.first().copied().unwrap_or(0) {
            1 => t("Левая кнопка").into(),
            2 => t("Правая кнопка").into(),
            4 => t("Колесо").into(),
            8 => t("Назад").into(),
            16 => t("Вперёд").into(),
            v => tf!("Кнопка {v}"),
        },
        1 => t("Переключение DPI").into(),
        0x11 => usage_name(binding.data.first().copied().unwrap_or(0)),
        0x13 => tf!("Сочетание {:02X?}", binding.data),
        0x14 => tf!("Мультимедиа {:02X?}", binding.data),
        0x21 => tf!("Макрос {}", binding.data.first().copied().unwrap_or(0)),
        0x33 => {
            if binding.data.first() == Some(&255) {
                t("Следующий профиль").into()
            } else {
                tf!("Профиль {}", binding.data.first().copied().unwrap_or(0) + 1)
            }
        }
        0xfe => t("Отключена").into(),
        0xff => t("Переназначение недоступно").into(),
        _ => tf!("Тип 0x{:02X} / {:02X?}", binding.kind, binding.data),
    }
}
fn usage_name(code: u8) -> String {
    match code {
        4..=29 => ((b'A' + code - 4) as char).to_string(),
        30..=38 => (code - 29).to_string(),
        39 => "0".into(),
        40 => "Enter".into(),
        41 => "Esc".into(),
        42 => "Backspace".into(),
        43 => "Tab".into(),
        44 => t("Пробел").into(),
        45 => "-".into(),
        46 => "=".into(),
        47 => "[".into(),
        48 => "]".into(),
        49 => "\\".into(),
        51 => ";".into(),
        52 => "'".into(),
        53 => "`".into(),
        54 => ",".into(),
        55 => ".".into(),
        56 => "/".into(),
        57 => "Caps Lock".into(),
        58..=69 => format!("F{}", code - 57),
        70 => "Print Screen".into(),
        71 => "Scroll Lock".into(),
        72 => "Pause".into(),
        73 => "Insert".into(),
        74 => "Home".into(),
        75 => "Page Up".into(),
        76 => "Delete".into(),
        77 => "End".into(),
        78 => "Page Down".into(),
        79 => t("Вправо").into(),
        80 => t("Влево").into(),
        81 => t("Вниз").into(),
        82 => t("Вверх").into(),
        224 => t("Левый Ctrl").into(),
        225 => t("Левый Shift").into(),
        226 => t("Левый Alt").into(),
        227 => t("Левый Super").into(),
        228 => t("Правый Ctrl").into(),
        229 => t("Правый Shift").into(),
        230 => t("Правый Alt").into(),
        231 => t("Правый Super").into(),
        _ => format!("HID {code}"),
    }
}
fn egui_key_usage(key: egui::Key) -> Option<u8> {
    use egui::Key;
    Some(match key {
        Key::A => 4,
        Key::B => 5,
        Key::C => 6,
        Key::D => 7,
        Key::E => 8,
        Key::F => 9,
        Key::G => 10,
        Key::H => 11,
        Key::I => 12,
        Key::J => 13,
        Key::K => 14,
        Key::L => 15,
        Key::M => 16,
        Key::N => 17,
        Key::O => 18,
        Key::P => 19,
        Key::Q => 20,
        Key::R => 21,
        Key::S => 22,
        Key::T => 23,
        Key::U => 24,
        Key::V => 25,
        Key::W => 26,
        Key::X => 27,
        Key::Y => 28,
        Key::Z => 29,
        Key::Num1 => 30,
        Key::Num2 => 31,
        Key::Num3 => 32,
        Key::Num4 => 33,
        Key::Num5 => 34,
        Key::Num6 => 35,
        Key::Num7 => 36,
        Key::Num8 => 37,
        Key::Num9 => 38,
        Key::Num0 => 39,
        Key::Enter => 40,
        Key::Escape => 41,
        Key::Backspace => 42,
        Key::Tab => 43,
        Key::Space => 44,
        Key::F1 => 58,
        Key::F2 => 59,
        Key::F3 => 60,
        Key::F4 => 61,
        Key::F5 => 62,
        Key::F6 => 63,
        Key::F7 => 64,
        Key::F8 => 65,
        Key::F9 => 66,
        Key::F10 => 67,
        Key::F11 => 68,
        Key::F12 => 69,
        Key::Insert => 73,
        Key::Home => 74,
        Key::PageUp => 75,
        Key::Delete => 76,
        Key::End => 77,
        Key::PageDown => 78,
        Key::ArrowRight => 79,
        Key::ArrowLeft => 80,
        Key::ArrowDown => 81,
        Key::ArrowUp => 82,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoring_a_remapped_button_preserves_the_last_left_click() {
        let keys = vec![
            KeyBinding::keyboard(4),
            KeyBinding::mouse(2),
            KeyBinding::mouse(4),
            KeyBinding::mouse(1),
        ];
        assert!(removes_last_left(&keys, 3, &default_binding(3)));
        assert!(!removes_last_left(&keys, 0, &default_binding(0)));
        let mut safe = keys;
        safe[0] = default_binding(0);
        assert!(!removes_last_left(&safe, 3, &default_binding(3)));
    }

    #[test]
    fn official_palette_includes_profile_cycle_and_all_direct_profiles() {
        let actions = hub_actions();
        for profile in [0, 1, 2, 255] {
            assert!(
                actions
                    .iter()
                    .any(|action| action.kind == 0x33 && action.data == [profile, 0])
            );
        }
    }
}
