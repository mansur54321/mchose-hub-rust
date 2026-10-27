use super::*;

impl Ace75Ui {
    pub(super) fn show_grid(&mut self, ui: &mut egui::Ui, palette: Palette, purpose: GridPurpose) {
        let layout = layout();
        let width = ui.available_width().min(980.0);
        let scale = width / layout.canvas.width;
        let height = (layout.canvas.height + layout.canvas.top_inset) * scale;
        let (area, _) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::hover());
        let origin = area.min + Vec2::new(0.0, layout.canvas.top_inset * scale);
        let dark = ui.visuals().dark_mode;
        let case_color = if dark {
            Color32::from_rgb(48, 49, 52)
        } else {
            Color32::from_rgb(248, 249, 251)
        };
        let cap_color = if dark {
            Color32::from_rgb(33, 34, 37)
        } else {
            Color32::from_rgb(229, 233, 240)
        };
        let edge = if dark {
            Color32::from_rgb(117, 124, 137)
        } else {
            Color32::from_rgb(116, 123, 138)
        };
        let case = egui::Rect::from_min_max(area.min + Vec2::new(11.5 * scale, 0.0), area.max);
        ui.painter().rect_filled(
            case.translate(Vec2::new(0.0, 3.0 * scale)),
            16.0 * scale,
            if dark {
                Color32::from_gray(8)
            } else {
                Color32::from_rgb(196, 201, 210)
            },
        );
        ui.painter().rect_filled(case, 16.0 * scale, case_color);
        ui.painter().rect_stroke(
            case,
            16.0 * scale,
            Stroke::new(1.0_f32, edge),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            case.min + Vec2::new(23.0, 15.0) * scale,
            egui::Align2::LEFT_CENTER,
            "ACE 75",
            egui::FontId::proportional(11.0 * scale),
            palette.muted,
        );
        let colors = if purpose == GridPurpose::Lighting {
            self.frame_preview
                .clone()
                .or_else(|| self.working().map(|profile| profile.colors.clone()))
        } else {
            None
        };
        let remapped: BTreeSet<_> = if purpose == GridPurpose::Assignment {
            self.working()
                .map(|profile| {
                    cells(purpose)
                        .filter_map(|cell| {
                            let actual =
                                profile.layers[self.layer as usize][cell.matrix_index as usize];
                            (default_assignment(cell.matrix_index, profile.index, self.layer)
                                .is_some_and(|value| value != actual))
                            .then_some(cell.matrix_index)
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            BTreeSet::new()
        };
        let preview = self.frame_preview.is_some();
        if purpose == GridPurpose::Lighting && self.logo_texture.is_none() {
            let image = image::load_from_memory(include_bytes!(
                "../../../assets/mchose-hub-logo-dark-original.png"
            ))
            .expect("Embedded MCHOSE mark")
            .into_rgba8();
            self.logo_texture = Some(ui.ctx().load_texture(
                "ace75-mchose-logo",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            ));
        }
        let logo = self.logo_texture.as_ref().map(egui::TextureHandle::id);
        let selected = if purpose == GridPurpose::Lighting {
            &mut self.selected_lights
        } else {
            &mut self.selected_keys
        };
        let additive = self.multi_select
            || ui.input(|input| {
                input.modifiers.ctrl || input.modifiers.command || input.modifiers.shift
            });
        let grid = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("ace75-grid", preview))
                .max_rect(area),
        );
        for cell in layout.keys.iter().chain(&layout.controls).chain(
            layout
                .lighting_regions
                .iter()
                .filter(|_| purpose == GridPurpose::Lighting),
        ) {
            let supported = cell.supports(purpose);
            let mut rect = egui::Rect::from_min_size(
                origin + Vec2::new(cell.x, cell.y) * scale,
                Vec2::new(cell.width, cell.height) * scale,
            );
            if cell.label == "sideLight" {
                rect.set_height((10.0 * scale).max(6.0));
                rect.set_center(egui::pos2(rect.center().x, origin.y + 73.0 * scale));
            }
            if cell.label == "gunlun" {
                rect.max.x = rect.max.x.max(rect.min.x + 22.0 * scale);
            }
            let response = grid.interact(
                rect,
                grid.id().with(("ace75-key", cell.source_index)),
                if supported {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            let active = supported && selected.contains(&cell.matrix_index);
            let completed = self.calibrating
                && self.calibrated.contains(&cell.matrix_index)
                && purpose == GridPurpose::Trigger;
            let rgb = colors
                .as_ref()
                .and_then(|values| values.get(cell.matrix_index as usize))
                .copied()
                .filter(|rgb| *rgb != [0, 0, 0]);
            let rgb = rgb.map(|rgb| Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
            let fill = if completed {
                Color32::from_rgb(35, 170, 92)
            } else if let Some(rgb) = rgb {
                rgb
            } else if active {
                blend(cap_color, palette.accent, if dark { 0.32 } else { 0.16 })
            } else if response.hovered() {
                blend(cap_color, palette.accent, 0.09)
            } else {
                cap_color
            };
            let text = if completed {
                Color32::BLACK
            } else {
                rgb.map(contrast_text).unwrap_or(palette.text)
            };
            let radius = 4.0 * scale;
            if cell.label == "sideLight" {
                grid.painter()
                    .rect_filled(rect, 2.0, rgb.unwrap_or(cap_color));
            } else {
                grid.painter().rect_filled(
                    rect,
                    radius,
                    if dark {
                        Color32::from_rgb(9, 10, 12)
                    } else {
                        Color32::from_rgb(175, 182, 194)
                    },
                );
                let face = rect
                    .shrink2(Vec2::new(1.5, 1.0) * scale)
                    .translate(Vec2::new(0.0, -1.0 * scale));
                grid.painter().rect_filled(face, radius, fill);
                grid.painter().line_segment(
                    [
                        face.left_top() + Vec2::new(4.0, 1.0),
                        face.right_top() + Vec2::new(-4.0, 1.0),
                    ],
                    Stroke::new(
                        1.0_f32,
                        blend(fill, Color32::WHITE, if dark { 0.14 } else { 0.55 }),
                    ),
                );
                if cell.label == "side2Light" {
                    if let Some(logo) = logo {
                        let size = Vec2::splat(24.0 * scale);
                        grid.painter().image(
                            logo,
                            egui::Rect::from_center_size(face.center(), size),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                } else if cell.label == "gunlun" {
                    for step in -2..=2 {
                        let y = face.center().y + step as f32 * 5.0 * scale;
                        grid.painter().line_segment(
                            [
                                egui::pos2(face.left() + 5.0 * scale, y),
                                egui::pos2(face.right() - 5.0 * scale, y),
                            ],
                            Stroke::new(1.4_f32, text),
                        );
                    }
                } else if matches!(cell.code, 79..=82) {
                    draw_arrow(grid.painter(), face.center(), cell.code, text, scale);
                } else {
                    grid.painter().text(
                        face.center(),
                        egui::Align2::CENTER_CENTER,
                        cell.legend(),
                        egui::FontId::proportional((12.5 * scale).max(10.0)),
                        text,
                    );
                }
                if remapped.contains(&cell.matrix_index) {
                    grid.painter().circle_filled(
                        face.right_top() + Vec2::new(-5.0, 5.0) * scale,
                        2.3 * scale,
                        palette.accent_text,
                    );
                }
            }
            if active || response.hovered() || response.has_focus() {
                grid.painter().rect_stroke(
                    rect,
                    radius,
                    Stroke::new(
                        if active || response.has_focus() {
                            2.2_f32
                        } else {
                            1.3_f32
                        },
                        palette.accent_text,
                    ),
                    egui::StrokeKind::Inside,
                );
            } else {
                grid.painter().rect_stroke(
                    rect,
                    radius,
                    Stroke::new(0.8_f32, edge),
                    egui::StrokeKind::Inside,
                );
            }
            if active && rect.width() > 27.0 && rect.height() > 27.0 {
                let point = rect.right_bottom() - Vec2::new(6.0, 6.0) * scale;
                grid.painter().line_segment(
                    [
                        point + Vec2::new(-4.0, -1.0) * scale,
                        point + Vec2::new(-1.5, 1.5) * scale,
                    ],
                    Stroke::new(
                        1.4_f32,
                        if rgb.is_some() {
                            text
                        } else {
                            palette.accent_text
                        },
                    ),
                );
                grid.painter().line_segment(
                    [
                        point + Vec2::new(-1.5, 1.5) * scale,
                        point + Vec2::new(3.0, -3.0) * scale,
                    ],
                    Stroke::new(
                        1.4_f32,
                        if rgb.is_some() {
                            text
                        } else {
                            palette.accent_text
                        },
                    ),
                );
            }
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    grid.is_enabled() && supported,
                    active,
                    cell.description(),
                )
            });
            if response.clicked() && supported && !self.calibrating {
                if purpose == GridPurpose::Lighting && !preview {
                    let scope = if (95..=122).contains(&cell.matrix_index) {
                        1
                    } else if cell.matrix_index == 123 {
                        2
                    } else {
                        0
                    };
                    if self.light_scope != scope {
                        self.light_scope = scope;
                        selected.clear();
                    }
                }
                toggle_selection(selected, cell.matrix_index, additive);
            }
            response.on_hover_text(cell.description());
        }
    }
}

fn blend(a: Color32, b: Color32, amount: f32) -> Color32 {
    let channel =
        |x: u8, y: u8| (f32::from(x) * (1.0 - amount) + f32::from(y) * amount).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}

fn draw_arrow(painter: &egui::Painter, center: egui::Pos2, code: u16, color: Color32, scale: f32) {
    let direction = match code {
        79 => Vec2::X,
        80 => -Vec2::X,
        81 => Vec2::Y,
        _ => -Vec2::Y,
    };
    let normal = egui::vec2(-direction.y, direction.x);
    let start = center - direction * 6.0 * scale;
    let tip = center + direction * 6.0 * scale;
    let stroke = Stroke::new(1.6 * scale, color);
    painter.line_segment([start, tip], stroke);
    painter.line_segment(
        [tip, tip - direction * 4.0 * scale + normal * 4.0 * scale],
        stroke,
    );
    painter.line_segment(
        [tip, tip - direction * 4.0 * scale - normal * 4.0 * scale],
        stroke,
    );
}
