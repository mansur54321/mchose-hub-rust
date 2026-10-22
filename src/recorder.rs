use crate::mouse::MacroEvent;
#[cfg(all(test, target_os = "linux"))]
use anyhow::Context;
use anyhow::{Result, ensure};
use std::collections::BTreeSet;
#[cfg(all(test, target_os = "linux"))]
use std::collections::HashMap;
#[cfg(all(test, target_os = "linux"))]
use x11rb::connection::Connection;
#[cfg(all(test, target_os = "linux"))]
use x11rb::protocol::Event;
#[cfg(all(test, target_os = "linux"))]
use x11rb::protocol::xkb::{self, BoolCtrl, NameDetail, PerClientFlag};
#[cfg(all(test, target_os = "linux"))]
use x11rb::protocol::xproto::{ChangeWindowAttributesAux, ConnectionExt, EventMask};
#[cfg(all(test, target_os = "linux"))]
use x11rb::rust_connection::RustConnection;

#[derive(Debug, Default)]
pub struct KeyboardBatch {
    /// Capture delay preceding each event; the editor saves it as a kind-63 event.
    pub events: Vec<MacroEvent>,
    pub focus_lost: bool,
}

/// Records physical key, mouse-button and wheel input from the root window.
/// The hook preserves arrival order on Windows, Wayland and X11.
pub struct WindowKeyboardRecorder {
    context: eframe::egui::Context,
    hook: egui_winit::WindowEventHook,
    capture: std::sync::Arc<std::sync::Mutex<WindowCapture>>,
}

impl WindowKeyboardRecorder {
    pub fn start(context: &eframe::egui::Context) -> Result<Self> {
        let capture = std::sync::Arc::new(std::sync::Mutex::new(WindowCapture {
            active: true,
            ..WindowCapture::default()
        }));
        let shared = capture.clone();
        let hook: egui_winit::WindowEventHook = std::sync::Arc::new(move |viewport, event| {
            if viewport != eframe::egui::ViewportId::ROOT {
                return;
            }
            if let Ok(mut capture) = shared.lock() {
                capture.window_event(event, std::time::Instant::now());
            }
        });
        egui_winit::set_window_event_hook(context, Some(hook.clone()));
        Ok(Self {
            context: context.clone(),
            hook,
            capture,
        })
    }

    pub fn set_pointer_area(
        &mut self,
        area: eframe::egui::Rect,
        pixels_per_point: f32,
    ) -> Result<()> {
        ensure!(
            pixels_per_point.is_finite() && pixels_per_point > 0.0,
            "Неверный масштаб области записи"
        );
        let mut capture = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("Ошибка очереди записи ввода"))?;
        capture.pointer_area = Some((area, pixels_per_point));
        Ok(())
    }

    pub fn elapsed_since_last_event(&self) -> Option<u16> {
        let capture = self.capture.lock().ok()?;
        capture
            .last_time
            .map(|last| last.elapsed().as_millis().min(60000) as u16)
    }

    pub fn poll(&mut self) -> Result<KeyboardBatch> {
        let mut capture = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("Ошибка очереди записи клавиш"))?;
        ensure!(
            !capture.overflow,
            "Очередь записи клавиш переполнена; запись остановлена"
        );
        Ok(KeyboardBatch {
            events: std::mem::take(&mut capture.events),
            focus_lost: std::mem::take(&mut capture.focus_lost),
        })
    }
}

impl Drop for WindowKeyboardRecorder {
    fn drop(&mut self) {
        egui_winit::clear_window_event_hook(&self.context, &self.hook);
        if let Ok(mut capture) = self.capture.lock() {
            capture.active = false;
            capture.events.clear();
            capture.held.clear();
        }
    }
}

#[derive(Default)]
struct WindowCapture {
    events: Vec<MacroEvent>,
    held: BTreeSet<(u8, u8)>,
    last_time: Option<std::time::Instant>,
    pointer_position: Option<winit::dpi::PhysicalPosition<f64>>,
    pointer_area: Option<(eframe::egui::Rect, f32)>,
    active: bool,
    focus_lost: bool,
    overflow: bool,
}

impl WindowCapture {
    fn window_event(&mut self, event: &winit::event::WindowEvent, now: std::time::Instant) {
        use winit::event::{ElementState, WindowEvent};
        use winit::keyboard::PhysicalKey;
        if !self.active {
            return;
        }
        match event {
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.key(
                        code,
                        event.state == ElementState::Pressed,
                        event.repeat,
                        *is_synthetic,
                        now,
                    );
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.pointer_position = Some(*position),
            WindowEvent::CursorLeft { .. } => self.pointer_position = None,
            WindowEvent::MouseInput { state, button, .. } => {
                self.mouse_button(*button, *state == ElementState::Pressed, now)
            }
            WindowEvent::MouseWheel { delta, .. } => self.wheel(*delta, now),
            WindowEvent::Focused(false) | WindowEvent::Destroyed => self.lose_focus(now),
            _ => {}
        }
    }

    fn pointer_inside(&self) -> bool {
        match (self.pointer_area, self.pointer_position) {
            (Some((area, scale)), Some(position)) => area.contains(eframe::egui::pos2(
                position.x as f32 / scale,
                position.y as f32 / scale,
            )),
            _ => false,
        }
    }

    fn mouse_button(
        &mut self,
        button: winit::event::MouseButton,
        pressed: bool,
        now: std::time::Instant,
    ) {
        use winit::event::MouseButton;
        let value = match button {
            MouseButton::Left => 1,
            MouseButton::Right => 2,
            MouseButton::Middle => 4,
            MouseButton::Back => 8,
            MouseButton::Forward => 16,
            _ => return,
        };
        if self.pointer_inside() || (!pressed && self.held.contains(&(0, value))) {
            self.record(0, pressed, value, now);
        }
    }

    fn wheel(&mut self, delta: winit::event::MouseScrollDelta, now: std::time::Instant) {
        if !self.pointer_inside() {
            return;
        }
        let y = match delta {
            winit::event::MouseScrollDelta::LineDelta(_, y) => f64::from(y),
            winit::event::MouseScrollDelta::PixelDelta(position) => position.y,
        };
        if y != 0.0 && y.is_finite() {
            self.record(5, false, u8::from(y < 0.0), now);
        }
    }

    fn key(
        &mut self,
        code: winit::keyboard::KeyCode,
        pressed: bool,
        repeat: bool,
        synthetic: bool,
        now: std::time::Instant,
    ) {
        if pressed && (repeat || synthetic) {
            return;
        }
        if let Some(usage) = key_code_to_hid(code) {
            self.record(17, pressed, usage, now);
        }
    }

    fn record(&mut self, kind: u8, pressed: bool, value: u8, now: std::time::Instant) {
        if !self.active {
            return;
        }
        if matches!(kind, 0 | 17) {
            let changed = if pressed {
                self.held.insert((kind, value))
            } else {
                self.held.remove(&(kind, value))
            };
            if !changed {
                return;
            }
        }
        self.append(kind, pressed, value, now);
    }

    fn append(&mut self, kind: u8, pressed: bool, value: u8, now: std::time::Instant) {
        // Reserve room for releasing every held key/button on focus loss.
        if self.events.len() + self.held.len() + 1 > 2048 {
            self.active = false;
            self.overflow = true;
            return;
        }
        let delay_ms = self.last_time.map_or(0, |previous| {
            now.saturating_duration_since(previous)
                .as_millis()
                .min(60000) as u16
        });
        self.last_time = Some(now);
        self.events.push(MacroEvent {
            delay_ms,
            kind,
            pressed,
            value,
        });
    }

    fn lose_focus(&mut self, now: std::time::Instant) {
        for (kind, value) in std::mem::take(&mut self.held) {
            self.append(kind, false, value, now);
        }
        self.active = false;
        self.focus_lost = true;
    }
}

fn key_code_to_hid(code: winit::keyboard::KeyCode) -> Option<u8> {
    use winit::keyboard::KeyCode::*;
    const LETTERS: [winit::keyboard::KeyCode; 26] = [
        KeyA, KeyB, KeyC, KeyD, KeyE, KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK, KeyL, KeyM, KeyN, KeyO,
        KeyP, KeyQ, KeyR, KeyS, KeyT, KeyU, KeyV, KeyW, KeyX, KeyY, KeyZ,
    ];
    if let Some(index) = LETTERS.iter().position(|key| *key == code) {
        return Some(4 + index as u8);
    }
    const FUNCTION_KEYS: [winit::keyboard::KeyCode; 24] = [
        F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18, F19, F20,
        F21, F22, F23, F24,
    ];
    if let Some(index) = FUNCTION_KEYS.iter().position(|key| *key == code) {
        return Some(if index < 12 {
            58 + index as u8
        } else {
            92 + index as u8
        });
    }
    Some(match code {
        Digit1 => 30,
        Digit2 => 31,
        Digit3 => 32,
        Digit4 => 33,
        Digit5 => 34,
        Digit6 => 35,
        Digit7 => 36,
        Digit8 => 37,
        Digit9 => 38,
        Digit0 => 39,
        Enter => 40,
        Escape => 41,
        Backspace => 42,
        Tab => 43,
        Space => 44,
        Minus => 45,
        Equal => 46,
        BracketLeft => 47,
        BracketRight => 48,
        Backslash => 49,
        Semicolon => 51,
        Quote => 52,
        Backquote => 53,
        Comma => 54,
        Period => 55,
        Slash => 56,
        CapsLock => 57,
        PrintScreen => 70,
        ScrollLock => 71,
        Pause => 72,
        Insert => 73,
        Home => 74,
        PageUp => 75,
        Delete => 76,
        End => 77,
        PageDown => 78,
        ArrowRight => 79,
        ArrowLeft => 80,
        ArrowDown => 81,
        ArrowUp => 82,
        NumLock => 83,
        NumpadDivide => 84,
        NumpadMultiply => 85,
        NumpadSubtract => 86,
        NumpadAdd => 87,
        NumpadEnter => 88,
        Numpad1 => 89,
        Numpad2 => 90,
        Numpad3 => 91,
        Numpad4 => 92,
        Numpad5 => 93,
        Numpad6 => 94,
        Numpad7 => 95,
        Numpad8 => 96,
        Numpad9 => 97,
        Numpad0 => 98,
        NumpadDecimal => 99,
        IntlBackslash => 100,
        ContextMenu => 101,
        Power => 102,
        NumpadEqual => 103,
        ControlLeft => 224,
        ShiftLeft => 225,
        AltLeft => 226,
        SuperLeft => 227,
        ControlRight => 228,
        ShiftRight => 229,
        AltRight => 230,
        SuperRight => 231,
        _ => return None,
    })
}

/// Subscribes only to the native application's X11 window. A separate X11
/// connection keeps its event mask independent of winit's connection.
#[cfg(all(test, target_os = "linux"))]
pub struct X11KeyboardRecorder {
    connection: RustConnection,
    window: u32,
    usages: HashMap<u8, u8>,
    held: BTreeSet<u8>,
    last_time: Option<u32>,
    active: bool,
}

#[cfg(all(test, target_os = "linux"))]
impl X11KeyboardRecorder {
    pub fn start(window: u32) -> Result<Self> {
        Self::start_on_display(window, None)
    }

    fn start_on_display(window: u32, display: Option<&str>) -> Result<Self> {
        let (connection, _) = RustConnection::connect(display)
            .context("Не удалось подключиться к X11 для записи клавиш")?;
        let extension = xkb::use_extension(&connection, 1, 0)?.reply()?;
        ensure!(extension.supported, "X11 не поддерживает XKB");
        let names = xkb::get_names(&connection, 256, NameDetail::KEY_NAMES)?.reply()?;
        let key_names = names
            .value_list
            .key_names
            .context("XKB не вернул физические имена клавиш")?;
        let mut usages = HashMap::new();
        for (index, key) in key_names.iter().enumerate() {
            if let Some(usage) = physical_name_to_hid(&key.name) {
                if let Some(code) = names.first_key.checked_add(index as u8) {
                    usages.insert(code, usage);
                }
            }
        }
        ensure!(
            usages.values().any(|usage| *usage == 224)
                && usages.values().any(|usage| *usage == 231),
            "В XKB отсутствуют имена модификаторов"
        );
        let flags = xkb::per_client_flags(
            &connection,
            256,
            PerClientFlag::DETECTABLE_AUTO_REPEAT,
            PerClientFlag::DETECTABLE_AUTO_REPEAT,
            BoolCtrl::default(),
            BoolCtrl::default(),
            BoolCtrl::default(),
        )?
        .reply()?;
        ensure!(
            u32::from(flags.value) & u32::from(PerClientFlag::DETECTABLE_AUTO_REPEAT) != 0,
            "X11 не поддерживает фильтрацию повторных нажатий"
        );
        connection
            .change_window_attributes(
                window,
                &ChangeWindowAttributesAux::new().event_mask(
                    EventMask::KEY_PRESS | EventMask::KEY_RELEASE | EventMask::FOCUS_CHANGE,
                ),
            )?
            .check()?;
        connection.flush()?;
        Ok(Self {
            connection,
            window,
            usages,
            held: BTreeSet::new(),
            last_time: None,
            active: true,
        })
    }

    pub fn poll(&mut self) -> Result<KeyboardBatch> {
        let mut batch = KeyboardBatch::default();
        if !self.active {
            return Ok(batch);
        }
        while let Some(event) = self.connection.poll_for_event()? {
            let key = match event {
                Event::KeyPress(event) if event.event == self.window => {
                    Some((event.detail, true, event.time))
                }
                Event::KeyRelease(event) if event.event == self.window => {
                    Some((event.detail, false, event.time))
                }
                Event::FocusOut(event) if event.event == self.window => {
                    for usage in std::mem::take(&mut self.held) {
                        batch.events.push(key_event(usage, false));
                    }
                    batch.focus_lost = true;
                    self.unsubscribe()?;
                    break;
                }
                _ => None,
            };
            if let Some((code, pressed, time)) = key {
                if let Some(usage) = self.usages.get(&code).copied() {
                    let changed = if pressed {
                        self.held.insert(usage)
                    } else {
                        self.held.remove(&usage)
                    };
                    if changed {
                        let mut event = key_event(usage, pressed);
                        event.delay_ms = self.last_time.map_or(0, |previous| {
                            time.wrapping_sub(previous).min(u16::MAX as u32) as u16
                        });
                        self.last_time = Some(time);
                        batch.events.push(event);
                    }
                }
            }
        }
        Ok(batch)
    }

    fn unsubscribe(&mut self) -> Result<()> {
        if self.active {
            self.active = false;
            self.connection
                .change_window_attributes(
                    self.window,
                    &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
                )?
                .check()?;
            self.connection.flush()?;
        }
        self.held.clear();
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
impl Drop for X11KeyboardRecorder {
    fn drop(&mut self) {
        let _ = self.unsubscribe();
    }
}

#[cfg(all(test, target_os = "linux"))]
fn key_event(value: u8, pressed: bool) -> MacroEvent {
    MacroEvent {
        delay_ms: 0,
        kind: 17,
        pressed,
        value,
    }
}

#[cfg(all(test, target_os = "linux"))]
fn physical_name_to_hid(raw: &[u8; 4]) -> Option<u8> {
    let name = std::str::from_utf8(raw).ok()?.trim_matches('\0');
    let rows: [(&str, &[u8]); 4] = [
        ("AE", &[30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 45, 46]),
        ("AD", &[20, 26, 8, 21, 23, 28, 24, 12, 18, 19, 47, 48]),
        ("AC", &[4, 22, 7, 9, 10, 11, 13, 14, 15, 51, 52]),
        ("AB", &[29, 27, 6, 25, 5, 17, 16, 54, 55, 56]),
    ];
    for (prefix, values) in rows {
        if let Some(number) = name
            .strip_prefix(prefix)
            .and_then(|s| s.parse::<usize>().ok())
        {
            return number
                .checked_sub(1)
                .and_then(|index| values.get(index).copied());
        }
    }
    if let Some(number) = name.strip_prefix("FK").and_then(|s| s.parse::<u8>().ok()) {
        return match number {
            1..=12 => Some(57 + number),
            13..=24 => Some(91 + number),
            _ => None,
        };
    }
    Some(match name {
        "ESC" => 41,
        "TLDE" => 53,
        "BKSP" => 42,
        "TAB" => 43,
        "BKSL" => 49,
        "RTRN" => 40,
        "CAPS" => 57,
        "SPCE" => 44,
        "LCTL" => 224,
        "LFSH" => 225,
        "LALT" => 226,
        "LWIN" => 227,
        "RCTL" => 228,
        "RTSH" => 229,
        "RALT" => 230,
        "RWIN" => 231,
        "PRSC" => 70,
        "SCLK" => 71,
        "PAUS" => 72,
        "INS" => 73,
        "HOME" => 74,
        "PGUP" => 75,
        "DELE" => 76,
        "END" => 77,
        "PGDN" => 78,
        "RGHT" => 79,
        "LEFT" => 80,
        "DOWN" => 81,
        "UP" => 82,
        "NMLK" => 83,
        "KPDV" => 84,
        "KPMU" => 85,
        "KPSU" => 86,
        "KPAD" => 87,
        "KPEN" => 88,
        "KP1" => 89,
        "KP2" => 90,
        "KP3" => 91,
        "KP4" => 92,
        "KP5" => 93,
        "KP6" => 94,
        "KP7" => 95,
        "KP8" => 96,
        "KP9" => 97,
        "KP0" => 98,
        "KPDL" => 99,
        "LSGT" => 100,
        "MENU" | "COMP" => 101,
        "POWR" => 102,
        "KPEQ" => 103,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_capture_keeps_all_modifier_sides_and_real_delays() {
        use winit::keyboard::KeyCode::*;
        let modifiers = [
            ControlLeft,
            ShiftLeft,
            AltLeft,
            SuperLeft,
            ControlRight,
            ShiftRight,
            AltRight,
            SuperRight,
        ];
        let start = std::time::Instant::now();
        let mut capture = WindowCapture {
            active: true,
            ..Default::default()
        };
        for (index, code) in modifiers.into_iter().enumerate() {
            capture.key(
                code,
                true,
                false,
                false,
                start + std::time::Duration::from_millis(index as u64 * 7),
            );
        }
        capture.key(
            KeyA,
            true,
            false,
            false,
            start + std::time::Duration::from_millis(60),
        );
        capture.key(
            KeyA,
            true,
            true,
            false,
            start + std::time::Duration::from_millis(65),
        );
        capture.key(
            KeyB,
            true,
            false,
            true,
            start + std::time::Duration::from_millis(70),
        );
        capture.key(
            KeyA,
            false,
            false,
            false,
            start + std::time::Duration::from_millis(81),
        );
        assert_eq!(capture.events.len(), 10);
        for (index, event) in capture.events[..8].iter().enumerate() {
            assert_eq!(
                (event.kind, event.value, event.pressed),
                (17, 224 + index as u8, true)
            );
            assert_eq!(event.delay_ms, if index == 0 { 0 } else { 7 });
        }
        assert_eq!(capture.events[9].delay_ms, 21);
        capture.lose_focus(start + std::time::Duration::from_millis(2081));
        assert_eq!(capture.events[10].delay_ms, 2000);
        assert!(capture.events[11..].iter().all(|event| event.delay_ms == 0));
        assert!(capture.focus_lost);
        assert!(capture.held.is_empty());
        assert_eq!(capture.events.len(), 18);
        assert!(capture.events[10..].iter().all(|event| !event.pressed));
        capture.key(KeyC, true, false, false, start);
        assert_eq!(capture.events.len(), 18);
        assert_eq!(key_code_to_hid(F13), Some(104));
        assert_eq!(key_code_to_hid(F24), Some(115));
        assert_eq!(key_code_to_hid(NumpadEnter), Some(88));
    }

    #[test]
    fn window_hook_is_opt_in_scoped_and_removed_by_its_owner() {
        let context = eframe::egui::Context::default();
        let id = eframe::egui::Id::new("egui_winit_window_event_hook");
        assert!(
            context
                .data(|data| data.get_temp::<egui_winit::WindowEventHook>(id))
                .is_none()
        );
        let mut recorder = WindowKeyboardRecorder::start(&context).unwrap();
        (recorder.hook)(
            eframe::egui::ViewportId::from_hash_of("other-window"),
            &winit::event::WindowEvent::Focused(false),
        );
        assert!(!recorder.poll().unwrap().focus_lost);
        recorder.capture.lock().unwrap().key(
            winit::keyboard::KeyCode::SuperRight,
            true,
            false,
            false,
            std::time::Instant::now(),
        );
        (recorder.hook)(
            eframe::egui::ViewportId::ROOT,
            &winit::event::WindowEvent::Focused(false),
        );
        let batch = recorder.poll().unwrap();
        assert!(batch.focus_lost);
        assert_eq!(
            batch
                .events
                .iter()
                .map(|event| (event.kind, event.value, event.pressed))
                .collect::<Vec<_>>(),
            [(17, 231, true), (17, 231, false)]
        );
        drop(recorder);
        assert!(
            context
                .data(|data| data.get_temp::<egui_winit::WindowEventHook>(id))
                .is_none()
        );
        let old = WindowKeyboardRecorder::start(&context).unwrap();
        let current = WindowKeyboardRecorder::start(&context).unwrap();
        drop(old);
        assert!(
            context
                .data(|data| data.get_temp::<egui_winit::WindowEventHook>(id))
                .is_some_and(|hook| std::sync::Arc::ptr_eq(&hook, &current.hook))
        );
        drop(current);
        assert!(
            context
                .data(|data| data.get_temp::<egui_winit::WindowEventHook>(id))
                .is_none()
        );
    }

    #[test]
    fn mixed_window_input_keeps_order_and_releases_outside_area() {
        use winit::event::{MouseButton, MouseScrollDelta};
        use winit::keyboard::KeyCode;
        let start = std::time::Instant::now();
        let at = |ms| start + std::time::Duration::from_millis(ms);
        let mut capture = WindowCapture {
            active: true,
            pointer_area: Some((
                eframe::egui::Rect::from_min_max(
                    eframe::egui::pos2(10.0, 10.0),
                    eframe::egui::pos2(50.0, 50.0),
                ),
                2.0,
            )),
            pointer_position: Some(winit::dpi::PhysicalPosition::new(40.0, 40.0)),
            ..Default::default()
        };
        capture.mouse_button(MouseButton::Left, true, at(0));
        capture.key(KeyCode::ControlRight, true, false, false, at(15));
        capture.pointer_position = Some(winit::dpi::PhysicalPosition::new(120.0, 120.0));
        capture.mouse_button(MouseButton::Right, true, at(20));
        capture.wheel(MouseScrollDelta::LineDelta(0.0, 1.0), at(22));
        capture.mouse_button(MouseButton::Left, false, at(30));
        capture.pointer_position = Some(winit::dpi::PhysicalPosition::new(40.0, 40.0));
        capture.wheel(MouseScrollDelta::LineDelta(0.0, -1.0), at(42));
        capture.mouse_button(MouseButton::Middle, true, at(50));
        capture.lose_focus(at(2050));
        let actual: Vec<_> = capture
            .events
            .iter()
            .map(|e| (e.kind, e.value, e.pressed, e.delay_ms))
            .collect();
        assert_eq!(
            actual,
            [
                (0, 1, true, 0),
                (17, 228, true, 15),
                (0, 1, false, 15),
                (5, 1, false, 12),
                (0, 4, true, 8),
                (0, 4, false, 2000),
                (17, 228, false, 0),
            ]
        );
        assert!(capture.held.is_empty());
        assert!(capture.focus_lost);
    }

    #[test]
    fn window_capture_overflow_is_bounded_and_reports_failure() {
        let context = eframe::egui::Context::default();
        let mut recorder = WindowKeyboardRecorder::start(&context).unwrap();
        let now = std::time::Instant::now();
        {
            let mut capture = recorder.capture.lock().unwrap();
            for _ in 0..3000 {
                capture.key(winit::keyboard::KeyCode::KeyA, true, false, false, now);
                capture.key(winit::keyboard::KeyCode::KeyA, false, false, false, now);
            }
            assert!(capture.events.len() <= 2048);
            assert!(!capture.active);
        }
        assert!(recorder.poll().is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn physical_modifiers_and_rows_keep_sides_and_hid_codes() {
        for (name, usage) in [
            (b"LCTL", 224),
            (b"RCTL", 228),
            (b"LFSH", 225),
            (b"RTSH", 229),
            (b"LALT", 226),
            (b"RALT", 230),
            (b"LWIN", 227),
            (b"RWIN", 231),
            (b"AC01", 4),
            (b"AD01", 20),
            (b"AE10", 39),
            (b"FK12", 69),
            (b"FK24", 115),
        ] {
            assert_eq!(physical_name_to_hid(name), Some(usage));
        }
        assert_eq!(physical_name_to_hid(b"ZZZZ"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "Run only on an isolated Xvfb through MCHOSE_RECORDER_TEST_DISPLAY"]
    fn own_window_receives_exact_modifiers_and_stops_on_focus_loss() {
        use x11rb::protocol::xproto::{CreateWindowAux, InputFocus, WindowClass};
        let display = std::env::var("MCHOSE_RECORDER_TEST_DISPLAY")
            .expect("explicit isolated test display required");
        let (owner, screen) = RustConnection::connect(Some(&display)).unwrap();
        let window = owner.generate_id().unwrap();
        owner
            .create_window(
                0,
                window,
                owner.setup().roots[screen].root,
                0,
                0,
                200,
                120,
                0,
                WindowClass::INPUT_OUTPUT,
                0,
                &CreateWindowAux::new().event_mask(EventMask::KEY_PRESS | EventMask::KEY_RELEASE),
            )
            .unwrap()
            .check()
            .unwrap();
        owner.map_window(window).unwrap().check().unwrap();
        owner
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        owner.flush().unwrap();
        let mut recorder = X11KeyboardRecorder::start_on_display(window, Some(&display)).unwrap();
        let keycode = |usage| {
            *recorder
                .usages
                .iter()
                .find(|(_, mapped)| **mapped == usage)
                .unwrap()
                .0
        };
        let right_shift = keycode(229);
        for (usage, pressed) in [
            (228, true),
            (231, true),
            (4, true),
            (4, false),
            (231, false),
            (228, false),
        ] {
            x11rb::protocol::xtest::fake_input(
                &owner,
                if pressed { 2 } else { 3 },
                keycode(usage),
                x11rb::CURRENT_TIME,
                x11rb::NONE,
                0,
                0,
                0,
            )
            .unwrap()
            .check()
            .unwrap();
        }
        owner.get_input_focus().unwrap().reply().unwrap();
        let batch = recorder.poll().unwrap();
        let actual: Vec<(u8, bool)> = batch
            .events
            .iter()
            .map(|event| (event.value, event.pressed))
            .collect();
        assert_eq!(
            actual,
            [
                (228, true),
                (231, true),
                (4, true),
                (4, false),
                (231, false),
                (228, false)
            ]
        );
        x11rb::protocol::xtest::fake_input(
            &owner,
            2,
            right_shift,
            x11rb::CURRENT_TIME,
            x11rb::NONE,
            0,
            0,
            0,
        )
        .unwrap()
        .check()
        .unwrap();
        owner.get_input_focus().unwrap().reply().unwrap();
        let shift = recorder.poll().unwrap().events;
        assert_eq!(shift.len(), 1);
        assert_eq!((shift[0].value, shift[0].pressed), (229, true));
        owner
            .set_input_focus(InputFocus::NONE, x11rb::NONE, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        owner.flush().unwrap();
        let lost = recorder.poll().unwrap();
        assert!(lost.focus_lost);
        assert_eq!(lost.events, [key_event(229, false)]);
        owner.destroy_window(window).unwrap().check().unwrap();
    }
}
