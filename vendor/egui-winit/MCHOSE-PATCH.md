# Local egui-winit 0.33.3 patch

Upstream source is the crates.io release already used by eframe 0.33.3.
Only `src/lib.rs` changes: an opt-in per-context `WindowEventHook`, its setter
and identity-checked removal, and dispatch at the start of `State::on_window_event`.
The hook observes window events already delivered to this application; it
does not select, capture, or monitor system-wide input.

MCHOSE Native uses this to record physical key codes (including left/right
Ctrl, Shift, Alt and Super) before egui reduces them to aggregate modifiers.
The application registers it only while macro recording is active and filters
to the root viewport. Keys, mouse buttons and wheel events share one timed
queue; pointer presses/wheel are restricted to the recording area. It works
with both native Wayland and X11.

The MIT and Apache-2.0 license texts are included from the upstream release:
`https://raw.githubusercontent.com/emilk/egui/0.33.3/LICENSE-MIT` and
`https://raw.githubusercontent.com/emilk/egui/0.33.3/LICENSE-APACHE`.
