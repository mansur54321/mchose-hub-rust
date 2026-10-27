use super::*;

impl Ace75Ui {
    pub(super) fn lighting_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        ui.horizontal_wrapped(|ui| {
            for (scope, title) in [(0, "Клавиши"), (1, "RGB-полоса"), (2, "Логотип")]
            {
                let active = self.light_scope == scope;
                if ui
                    .add(
                        egui::Button::new(RichText::new(t(title)).color(if active {
                            Color32::WHITE
                        } else {
                            palette.text
                        }))
                        .selected(active)
                        .fill(if active {
                            palette.accent
                        } else {
                            palette.field
                        })
                        .min_size(Vec2::new(116.0, 36.0)),
                    )
                    .clicked()
                {
                    self.light_scope = scope;
                    self.selected_lights
                        .retain(|index| light_index_in_scope(*index, scope));
                }
            }
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(t("Каждая область запоминает свой эффект. Выберите область сверху или нажмите на неё на схеме."))
                .small()
                .color(palette.muted),
        );
        ui.add_space(18.0);

        let width = ui.available_width();
        if width >= 980.0 {
            let options_width = 316.0;
            let canvas_width = width - options_width - 16.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(canvas_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        self.lighting_canvas(ui, palette);
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(options_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| self.lighting_effect_controls(ui, palette, state),
                );
            });
        } else {
            self.lighting_canvas(ui, palette);
            ui.add_space(16.0);
            self.lighting_effect_controls(ui, palette, state);
        }
        ui.add_space(16.0);

        if self.working().is_some() && ui.available_width() >= 900.0 {
            ui.columns(2, |columns| {
                self.lighting_colors(&mut columns[0], palette, state, busy, writable);
                self.lighting_music(&mut columns[1], palette, state, busy, writable);
            });
        } else {
            if self.working().is_some() {
                self.lighting_colors(ui, palette, state, busy, writable);
                ui.add_space(16.0);
            }
            self.lighting_music(ui, palette, state, busy, writable);
        }
        ui.add_space(16.0);
        setting_section(
            ui,
            palette,
            t("Статические и анимированные темы"),
            "",
            |ui| self.animation_editor(ui, palette, state, busy, writable),
        );
    }

    fn lighting_canvas(&mut self, ui: &mut egui::Ui, palette: Palette) {
        setting_section(
            ui,
            palette,
            t("Схема подсветки"),
            t(
                "Цвета на схеме взяты из профиля. Движение встроенных эффектов здесь не отображается.",
            ),
            |ui| {
                self.grid_section(ui, palette, GridPurpose::Lighting);
            },
        );
    }

    fn lighting_effect_controls(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
    ) {
        let title = match self.light_scope {
            1 => t("Эффект RGB-полосы"),
            2 => t("Эффект логотипа"),
            _ => t("Эффект клавиш"),
        };
        setting_section(
            ui,
            palette,
            title,
            t("Изменения этой области сохраняются кнопкой «Применить»."),
            |ui| {
                if self.no_data(ui, palette) {
                    return;
                }
                let scope = self.light_scope;
                let mut lighting = self.working().unwrap().functions.lighting.clone();
                let before = lighting.clone();
                let zone = match scope {
                    1 => &mut lighting.side,
                    2 => &mut lighting.side2,
                    _ => &mut lighting.main,
                };
                let effects = effect_options(scope, firmware_number(state));
                let current = effects
                    .iter()
                    .find(|effect| effect.value == zone.effect)
                    .copied();
                ui.label(RichText::new(t("Эффект")).strong());
                egui::ComboBox::from_id_salt("ace75-effect")
                    .width(ui.available_width() - 8.0)
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
                let current = effects
                    .iter()
                    .find(|effect| effect.value == zone.effect)
                    .copied();
                if let Some(effect) = current {
                    if effect.brightness {
                        ui.add_space(16.0);
                        ui.label(RichText::new(t("Яркость")).strong());
                        ui.spacing_mut().slider_width = (ui.available_width() - 72.0).max(96.0);
                        ui.add(
                            egui::Slider::new(&mut zone.brightness, 0..=100)
                                .suffix("%")
                                .clamping(egui::SliderClamping::Edits),
                        );
                    }
                    if effect.speed {
                        ui.add_space(16.0);
                        ui.label(RichText::new(t("Скорость")).strong());
                        ui.spacing_mut().slider_width = ui.available_width().max(96.0);
                        ui.add(
                            egui::Slider::new(&mut zone.speed, 0..=4)
                                .step_by(1.0)
                                .show_value(false)
                                .clamping(egui::SliderClamping::Edits),
                        );
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(t("Медленно")).small().color(palette.muted));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    ui.label(
                                        RichText::new(t("Быстро")).small().color(palette.muted),
                                    );
                                },
                            );
                        });
                    }
                    if effect.direction {
                        ui.add_space(16.0);
                        ui.label(RichText::new(t("Направление")).strong());
                        ui.horizontal_wrapped(|ui| {
                            for (id, label) in effect.direction_labels().into_iter().enumerate() {
                                ui.selectable_value(&mut zone.direction, id as u8, t(label));
                            }
                        });
                    }
                    if effect.color {
                        ui.add_space(16.0);
                        preference_toggle(
                            ui,
                            palette,
                            &mut zone.rainbow,
                            t("Радужные цвета"),
                            t("Выключите, чтобы выбрать один цвет."),
                        );
                        ui.horizontal(|ui| {
                            ui.label(t("Цвет эффекта"));
                            ui.add_enabled_ui(!zone.rainbow, |ui| {
                                ui.color_edit_button_srgb(&mut zone.color);
                            });
                        });
                    }
                    if (scope == 0 && zone.effect == 0) || (scope == 1 && zone.effect == 5) {
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(t("Задайте цвета в разделе «Индивидуальные цвета»."))
                                .small()
                                .color(palette.muted),
                        );
                    }
                }
                if lighting != before {
                    lighting.scope = match scope {
                        1 => LightScope::Side,
                        2 => LightScope::Side2,
                        _ => LightScope::Main,
                    };
                    self.working_mut().unwrap().functions.lighting = lighting;
                }
            },
        );
    }

    fn lighting_colors(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        setting_section(
            ui,
            palette,
            t("Индивидуальные цвета"),
            t("Выделите нужные клавиши или сегменты на схеме, затем выберите цвет."),
            |ui| {
                self.colors_editor(ui, state, busy, writable);
                ui.add_space(8.0);
                egui::CollapsingHeader::new(t("Дополнительная настройка цвета")).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(t("Цвет калибровки"));
                        if let Some(profile) = self.working_mut() {
                            ui.color_edit_button_srgb(
                                &mut profile.functions.lighting.calibration_color,
                            );
                        }
                    });
                });
            },
        );
    }

    fn lighting_music(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        setting_section(ui, palette, "", "", |ui| {
            self.music_controls(ui, palette, state, busy, writable);
            if self.hardware_dirty() {
                ui.add_space(8.0);
                ui.label(
                    RichText::new(t(
                        "Сначала примените изменения профиля, чтобы включить музыку или тему.",
                    ))
                    .small()
                    .color(palette.muted),
                );
            }
        });
    }
}
