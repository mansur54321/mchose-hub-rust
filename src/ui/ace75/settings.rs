use super::*;
use crate::ace75::Performance;

impl Ace75Ui {
    pub(super) fn trigger_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        _busy: bool,
        _writable: bool,
    ) {
        if ui.available_width() >= 980.0 {
            let controls_width = 400.0;
            let keyboard_width = ui.available_width() - controls_width - 16.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.allocate_ui_with_layout(
                    Vec2::new(keyboard_width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(keyboard_width);
                        self.grid_section(ui, palette, GridPurpose::Trigger);
                    },
                );
                ui.allocate_ui_with_layout(
                    Vec2::new(controls_width, 590.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(controls_width);
                        egui::ScrollArea::vertical()
                            .id_salt("ace75-trigger-controls")
                            .max_height(570.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| self.trigger_controls(ui, palette, state));
                    },
                );
            });
        } else {
            self.prepare_selection(GridPurpose::Trigger);
            let names = layout()
                .keys
                .iter()
                .filter(|cell| self.selected_keys.contains(&cell.matrix_index))
                .take(8)
                .map(Cell::description)
                .collect::<Vec<_>>()
                .join(", ");
            ui.label(RichText::new(tf!("Выбранные клавиши: {}", names)).strong());
            egui::CollapsingHeader::new(t("Клавиатура: изменить выбор клавиш"))
                .default_open(false)
                .show(ui, |ui| {
                    self.grid_section(ui, palette, GridPurpose::Trigger)
                });
            ui.add_space(12.0);
            self.trigger_controls(ui, palette, state);
        }
    }

    fn trigger_controls(&mut self, ui: &mut egui::Ui, palette: Palette, state: &KeyboardState) {
        ui.horizontal_wrapped(|ui| {
            for (panel, label) in [(0, "Нажатие"), (1, "Rapid Trigger"), (2, "Мёртвые зоны")]
            {
                ui.selectable_value(&mut self.trigger_panel, panel, t(label));
            }
        });
        ui.add_space(12.0);
        if self.no_data(ui, palette) {
            return;
        }
        if self.trigger_panel == 1 {
            self.tachyon_settings(ui, palette);
            ui.add_space(10.0);
        }
        let Some(index) = self.selected_keys.first().copied() else {
            setting_section(
                ui,
                palette,
                t("Выберите клавиши"),
                t(
                    "Нажмите клавишу на схеме. Для одинаковой настройки нескольких клавиш используйте Ctrl или множественный выбор.",
                ),
                |_| {},
            );
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
        match self.trigger_panel {
            1 => {
                let max_actuation = if value.switch_type == 3 { 350 } else { 340 };
                let tachyon = self.working().unwrap().functions.performance.tachyon_mode;
                rapid_trigger_settings(ui, palette, &mut value, max_actuation, tachyon);
            }
            2 => {
                setting_section(
                    ui,
                    palette,
                    t("Мёртвые зоны"),
                    t("Участки в начале и конце хода, в которых движения клавиши игнорируются."),
                    |ui| {
                        deadzone_control(ui, &mut value.press_deadzone, t("Верхняя мёртвая зона"));
                        ui.add_space(12.0);
                        deadzone_control(ui, &mut value.release_deadzone, t("Нижняя мёртвая зона"));
                    },
                );
            }
            _ => actuation_settings(ui, palette, &mut value, firmware_number(state)),
        }
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

        ui.add_space(10.0);
        ui.label(
            RichText::new(t("Изменяются только выбранные клавиши. Остальные значения каждой клавиши сохраняются."))
                .small().color(palette.muted),
        );
        if self.selected_keys.len() > 1 {
            let key = cells(GridPurpose::Trigger)
                .find(|cell| cell.matrix_index == index)
                .map(Cell::description)
                .unwrap_or_else(|| index.to_string());
            ui.label(
                RichText::new(tf!(
                    "Показаны значения клавиши {}. Меняются только отредактированные поля.",
                    key
                ))
                .small()
                .color(palette.muted),
            );
        }
    }

    fn tachyon_settings(&mut self, ui: &mut egui::Ui, palette: Palette) {
        let mut tachyon = self.working().unwrap().functions.performance.tachyon_mode;
        if preference_toggle(
            ui,
            palette,
            &mut tachyon,
            t("Высокая точность RT · Tachyon 2.0"),
            t("Открывает шаг 0,005 и 0,001 мм. При отключении выбранные клавиши возвращаются к шагу 0,01 мм."),
        )
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
    }

    pub(super) fn performance_page(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
    ) {
        if self.no_data(ui, palette) {
            return;
        }
        let mut value = self.working().unwrap().functions.performance.clone();
        let before = value.clone();
        let profile_index = self.working().unwrap().index;
        if ui.available_width() >= 900.0 {
            ui.columns(2, |columns| {
                self.polling_settings(&mut columns[0], palette, state, busy, writable, &mut value);
                columns[0].add_space(12.0);
                response_settings(&mut columns[0], palette, &mut value);
                system_settings(&mut columns[1], palette, &mut value, profile_index);
                columns[1].add_space(12.0);
                sleep_settings(&mut columns[1], palette, &mut value);
            });
        } else {
            self.polling_settings(ui, palette, state, busy, writable, &mut value);
            ui.add_space(12.0);
            response_settings(ui, palette, &mut value);
            ui.add_space(12.0);
            system_settings(ui, palette, &mut value, profile_index);
            ui.add_space(12.0);
            sleep_settings(ui, palette, &mut value);
        }
        if value != before {
            self.working_mut().unwrap().functions.performance = value;
        }
    }

    fn polling_settings(
        &mut self,
        ui: &mut egui::Ui,
        palette: Palette,
        state: &KeyboardState,
        busy: bool,
        writable: bool,
        value: &mut Performance,
    ) {
        setting_section(
            ui,
            palette,
            t("Частота опроса"),
            t("Сколько раз в секунду клавиатура передаёт своё состояние по USB."),
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (code, label) in [
                        (4, "1000 Hz"),
                        (3, "2000 Hz"),
                        (2, "4000 Hz"),
                        (1, "8000 Hz"),
                    ] {
                        ui.selectable_value(&mut value.report_rate, code, label);
                    }
                    ui.add_space(8.0);
                    if ui
                        .add_enabled(
                            !busy
                                && writable
                                && self.local_edit.is_none()
                                && self.target(state).is_some(),
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
                ui.label(
                    RichText::new(t("Частота применяется отдельно от остальных параметров."))
                        .small()
                        .color(palette.muted),
                );
            },
        );
    }
}

fn response_settings(ui: &mut egui::Ui, palette: Palette, value: &mut Performance) {
    setting_section(
        ui,
        palette,
        t("Отклик и стабильность"),
        t("Защита от случайных нажатий и настройка поведения магнитных переключателей."),
        |ui| {
            preference_toggle(
                ui,
                palette,
                &mut value.bottom_rapid_trigger,
                t("Защита от случайных срабатываний"),
                t("Снижает ложные нажатия из-за вибраций стола и небольшого давления на клавиши."),
            );
            let mut debounce = value.debounce_level > 0;
            if preference_toggle(
                ui,
                palette,
                &mut debounce,
                t("Оптимизация дребезга клавиш"),
                t("Уменьшает повторные срабатывания из-за дребезга переключателя."),
            )
            .changed()
            {
                value.debounce_level = if debounce { 7 } else { 0 };
            }
            preference_toggle(
                ui,
                palette,
                &mut value.travel_check,
                t("Адаптивная динамическая калибровка (бета)"),
                t("Адаптирует распознавание хода клавиш во время работы."),
            );
            let mut smart = value.rt_smart > 0;
            if preference_toggle(
                ui,
                palette,
                &mut smart,
                "RT Smart",
                t("Стабилизация распознавания Rapid Trigger."),
            )
            .changed()
            {
                value.rt_smart = if smart { 8 } else { 0 };
            }
            if smart {
                ui.add(
                    egui::Slider::new(&mut value.rt_smart, 1..=15)
                        .text(t("Стабильность"))
                        .clamping(egui::SliderClamping::Edits),
                );
            }
        },
    );
}

fn system_settings(
    ui: &mut egui::Ui,
    palette: Palette,
    value: &mut Performance,
    profile_index: u8,
) {
    setting_section(
        ui,
        palette,
        t("Система и блокировки"),
        t("Выберите раскладку ОС и сочетания, которые нужно отключить."),
        |ui| {
            let mut mac = value.mac_mode % 4 >= 2;
            if preference_toggle(
                ui,
                palette,
                &mut mac,
                t("Режим Mac"),
                t("Использует основной и Fn-слой для macOS."),
            )
            .changed()
            {
                value.mac_mode = profile_index * 4 + if mac { 2 } else { 0 };
                if mac {
                    value.lock_win = false;
                }
            }
            ui.add_enabled_ui(!mac, |ui| {
                preference_toggle(
                    ui,
                    palette,
                    &mut value.lock_win,
                    t("Блокировать Win"),
                    t("Предотвращает случайное открытие меню Windows."),
                );
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut value.lock_alt_tab, t("Блокировать Alt + Tab"));
                ui.checkbox(&mut value.lock_alt_f4, t("Блокировать Alt + F4"));
            });
        },
    );
}

fn sleep_settings(ui: &mut egui::Ui, palette: Palette, value: &mut Performance) {
    setting_section(
        ui,
        palette,
        t("Сон подсветки"),
        t("Определяет, когда подсветка выключается при бездействии."),
        |ui| {
            let mut never = value.sleep_mode == 1;
            if preference_toggle(
                ui,
                palette,
                &mut never,
                t("Не выключать автоматически"),
                t("Оставлять подсветку включённой, пока клавиатура подключена."),
            )
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
                        .text(t("Выключить через"))
                        .suffix(t(" мин"))
                        .clamping(egui::SliderClamping::Edits),
                )
                .changed()
            {
                value.sleep_time = (minutes * 2.0) as u8;
            }
        },
    );
}

fn actuation_settings(ui: &mut egui::Ui, palette: Palette, value: &mut KeyTrigger, firmware: u16) {
    setting_section(
        ui,
        palette,
        t("Точка срабатывания"),
        t("Глубина нажатия, на которой клавиша считается нажатой."),
        |ui| {
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
            actuation_diagram(ui, palette, value.actuation, max_actuation);
            mm_slider(
                ui,
                &mut value.actuation,
                100.0,
                10..=max_actuation,
                t("Глубина нажатия"),
            );
        },
    );
}

fn rapid_trigger_settings(
    ui: &mut egui::Ui,
    palette: Palette,
    value: &mut KeyTrigger,
    max_actuation: u16,
    tachyon: bool,
) {
    setting_section(
        ui,
        palette,
        "Rapid Trigger",
        t("Отдельные расстояния движения для отпускания и повторного нажатия клавиши."),
        |ui| {
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [(0, "Отключён"), (1, "Включён"), (2, "Непрерывный")]
                {
                    ui.selectable_value(&mut value.key_mode, mode, t(label));
                }
            });
            ui.add_space(8.0);
            ui.add_enabled_ui(value.key_mode != 0, |ui| {
                rt_field(
                    ui,
                    "ace75-rt-press",
                    t("Нажатие"),
                    &mut value.press_precision,
                    &mut value.rt_press,
                    max_actuation,
                    tachyon,
                );
                ui.add_space(10.0);
                rt_field(
                    ui,
                    "ace75-rt-release",
                    t("Отпускание"),
                    &mut value.release_precision,
                    &mut value.rt_release,
                    max_actuation,
                    tachyon,
                );
            });
        },
    );
}

fn rt_field(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    precision: &mut u8,
    value: &mut u16,
    max_travel: u16,
    tachyon: bool,
) {
    ui.push_id(id, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(label).strong());
            ui.add_enabled_ui(tachyon, |ui| {
                let previous = *precision;
                egui::ComboBox::from_id_salt("precision")
                    .selected_text(match precision {
                        0 => "0.01 mm",
                        1 => "0.005 mm",
                        2 => "0.001 mm",
                        _ => t("Неизвестный"),
                    })
                    .show_ui(ui, |ui| {
                        for (code, name) in [(0, "0.01 mm"), (1, "0.005 mm"), (2, "0.001 mm")] {
                            ui.selectable_value(precision, code, name);
                        }
                    });
                if previous != *precision {
                    *value = ((f64::from(*value) / precision_base(previous)
                        * precision_base(*precision))
                    .round() as u16)
                        .clamp(
                            if tachyon { 1 } else { 2 },
                            if *precision == 0 { max_travel } else { 500 },
                        );
                }
            });
        });
        mm_slider(
            ui,
            value,
            precision_base(*precision),
            if tachyon { 1 } else { 2 }..=if *precision == 0 { max_travel } else { 500 },
            "",
        );
    });
}

fn actuation_diagram(ui: &mut egui::Ui, palette: Palette, value: u16, max: u16) {
    if ui.ctx().content_rect().width() < 1000.0 {
        return;
    }
    let dark = ui.visuals().dark_mode;
    let id = egui::Id::new(("ace75-switch-illustration", dark));
    let texture = ui
        .ctx()
        .data(|data| data.get_temp::<egui::TextureHandle>(id))
        .unwrap_or_else(|| {
            let bytes: &[u8] = if dark {
                include_bytes!("../../../assets/ace75-switch-dark.png")
            } else {
                include_bytes!("../../../assets/ace75-switch-light.png")
            };
            let image = image::load_from_memory(bytes)
                .expect("Embedded Ace75 switch")
                .into_rgba8();
            let texture = ui.ctx().load_texture(
                format!("ace75-switch-{dark}"),
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ),
                egui::TextureOptions::LINEAR,
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    ui.horizontal(|ui| {
        ui.image((texture.id(), Vec2::new(112.0, 112.0)));
        ui.vertical(|ui| {
            ui.add_space(22.0);
            ui.label(
                RichText::new(tf!("{:.2} мм", f64::from(value) / 100.0))
                    .size(23.0)
                    .strong()
                    .color(palette.text),
            );
            ui.label(
                RichText::new(tf!("До упора · {:.1} мм", f64::from(max) / 100.0))
                    .size(12.5)
                    .color(palette.muted),
            );
        });
    });
}
