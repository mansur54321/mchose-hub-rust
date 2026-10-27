use super::*;

pub(super) fn setting_section<R>(
    ui: &mut egui::Ui,
    palette: Palette,
    title: &str,
    help: &str,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let enabled = ui.is_enabled();
    egui::Frame::new()
        .fill(palette.field)
        .corner_radius(12.0)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            if !title.is_empty() {
                ui.label(RichText::new(title).size(16.0).strong().color(if enabled {
                    palette.text
                } else {
                    palette.muted
                }));
            }
            if !help.is_empty() {
                ui.label(RichText::new(help).size(12.5).color(palette.muted));
            }
            if !title.is_empty() || !help.is_empty() {
                ui.add_space(12.0);
            }
            content(ui)
        })
        .inner
}

pub(super) fn preference_toggle(
    ui: &mut egui::Ui,
    palette: Palette,
    value: &mut bool,
    title: &str,
    help: &str,
) -> egui::Response {
    let enabled = ui.is_enabled();
    let text_width = (ui.available_width() - 64.0).max(80.0);
    let row = ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(text_width);
            ui.label(RichText::new(title).size(14.0).color(if enabled {
                palette.text
            } else {
                palette.muted
            }));
            if !help.is_empty() {
                ui.label(RichText::new(help).size(12.5).color(palette.muted));
            }
        });
        ui.allocate_exact_size(Vec2::new(44.0, 26.0), egui::Sense::hover())
            .0
    });
    let mut response = ui.interact(
        row.response.rect,
        ui.id().with(("ace75-toggle", title)),
        egui::Sense::click(),
    );
    if response.clicked() {
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, title)
    });
    let track = row.inner;
    let fill = if !enabled {
        palette.border
    } else if *value {
        palette.accent
    } else if ui.visuals().dark_mode {
        Color32::from_rgb(118, 123, 135)
    } else {
        Color32::from_rgb(116, 123, 138)
    };
    ui.painter().rect_filled(track, 13.0, fill);
    ui.painter().circle_filled(
        track.center() + Vec2::new(if *value { 9.0 } else { -9.0 }, 0.0),
        9.0,
        Color32::WHITE,
    );
    if response.has_focus() {
        ui.painter().rect_stroke(
            track.expand(3.0),
            15.0,
            Stroke::new(2.0_f32, palette.accent_text),
            egui::StrokeKind::Outside,
        );
    }
    ui.add_space(10.0);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preference_switch_accepts_tab_and_space() {
        let context = egui::Context::default();
        let mut value = false;
        let mut changed = false;
        for event in [
            None,
            Some((egui::Key::Tab, true)),
            Some((egui::Key::Tab, false)),
            Some((egui::Key::Space, true)),
            Some((egui::Key::Space, false)),
        ] {
            let mut input = egui::RawInput::default();
            if let Some((key, pressed)) = event {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = context.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    changed |=
                        preference_toggle(ui, Palette::new(true), &mut value, "Test", "Help")
                            .changed();
                });
            });
        }
        assert!(value && changed);
    }
}
