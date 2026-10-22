#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "{}",
                error
                    .chain()
                    .map(|context| mchose_hub_rust::i18n::message(&context.to_string()))
                    .collect::<Vec<_>>()
                    .join(": ")
            );
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    #[cfg(windows)]
    if std::env::args().len() > 1 {
        unsafe {
            let _ = windows::Win32::System::Console::AttachConsole(
                windows::Win32::System::Console::ATTACH_PARENT_PROCESS,
            );
        }
    }
    use mchose_hub_rust::{
        dock::Dock,
        mouse::Mouse,
        transport::{self, DeviceKind, HidTransport},
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| args.windows(2).find(|p| p[0] == name).map(|p| p[1].clone());
    let language = option("--lang");
    if args.iter().any(|v| v == "--lang") {
        anyhow::ensure!(language.is_some(), "--lang expects ru or en");
    }
    let selected_language = if let Some(language) = &language {
        mchose_hub_rust::i18n::Language::parse(language)
            .ok_or_else(|| anyhow::anyhow!("--lang expects ru or en"))?
    } else {
        mchose_hub_rust::i18n::load_preference()
    };
    mchose_hub_rust::i18n::set_language(selected_language);
    if args.iter().any(|v| v == "--help" || v == "-h") {
        if selected_language == mchose_hub_rust::i18n::Language::En {
            println!(
                "MCHOSE Hub Rust\n\nNo arguments: native interface\n--list                 USB/HID devices\n--status               Mouse and dock status as JSON\n--snapshot             Full current settings as JSON\n--read-only            GUI without device writes\n--offline              GUI without HID access\n--lang ru|en           Interface and CLI language\n--page PAGE            keys, dpi, performance, dock, macros, profiles, system\n--light                Light theme\n--size WIDTHxHEIGHT    Window size (minimum 855x700)\n--screenshot PATH      Save this window as PNG and exit"
            );
        } else {
            println!(
                "MCHOSE Hub Rust\n\nБез аргументов: нативный интерфейс\n--list                 USB/HID-устройства\n--status               Состояние мыши и дока в JSON\n--snapshot             Полные текущие настройки в JSON\n--read-only            GUI без записи в устройства\n--offline              GUI без доступа к HID\n--lang ru|en           Язык интерфейса и CLI\n--page PAGE            keys, dpi, performance, dock, macros, profiles, system\n--light                Светлая тема\n--size WIDTHxHEIGHT    Размер окна (минимум 855x700)\n--screenshot PATH      Сохранить собственное окно в PNG и завершить работу"
            );
        }
        return Ok(());
    }
    if !args
        .iter()
        .any(|v| matches!(v.as_str(), "--list" | "--status" | "--snapshot"))
    {
        let read_only = args.iter().any(|v| v == "--read-only");
        let offline = args.iter().any(|v| v == "--offline");
        let page = option("--page");
        let screenshot = option("--screenshot");
        let light = args.iter().any(|v| v == "--light");
        let size = option("--size")
            .and_then(|s| {
                let (width, height) = s.split_once('x')?;
                Some([
                    width.parse::<f32>().ok()?.max(855.0),
                    height.parse::<f32>().ok()?.max(700.0),
                ])
            })
            .unwrap_or([1440.0, 940.0]);
        if let Some(path) = &screenshot {
            anyhow::ensure!(
                !std::path::Path::new(path).exists(),
                "Файл снимка уже существует: {path}"
            );
        }
        let icon =
            image::load_from_memory(include_bytes!("../assets/mchose-icon.png"))?.into_rgba8();
        #[allow(unused_mut)]
        let mut options = eframe::NativeOptions {
            vsync: false,
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size(size)
                .with_min_inner_size([855.0, 700.0])
                .with_icon(eframe::egui::IconData {
                    width: icon.width(),
                    height: icon.height(),
                    rgba: icon.into_raw(),
                })
                .with_app_id("mchose-native"),
            ..Default::default()
        };
        #[cfg(target_os = "linux")]
        if std::env::var_os("DISPLAY").is_some() && x11rb::connect(None).is_ok() {
            use winit::platform::x11::EventLoopBuilderExtX11;
            options.event_loop_builder = Some(Box::new(|builder| {
                builder.with_x11();
            }));
        }
        eframe::run_native(
            "MCHOSE Hub Rust",
            options,
            Box::new(move |cc| {
                let mut app = mchose_hub_rust::ui::NativeApp::new(cc, read_only, offline);
                if let Some(language) = language {
                    app.set_language(&language);
                }
                if light {
                    app.set_dark(false);
                }
                if let Some(page) = page {
                    app.select_page(&page);
                }
                if let Some(path) = screenshot {
                    app.capture_to(path.into());
                }
                Ok(Box::new(app))
            }),
        )
        .map_err(|error| anyhow::anyhow!("Ошибка нативного интерфейса: {error}"))?;
        return Ok(());
    }
    let devices = transport::enumerate()?;
    if args.iter().any(|v| v == "--status" || v == "--snapshot") {
        let mut data = serde_json::Map::new();
        for device in &devices {
            let io = Box::new(HidTransport::open(device)?);
            match device.kind {
                DeviceKind::Mouse => {
                    let mut mouse = Mouse::new(io);
                    let result = if args.iter().any(|v| v == "--snapshot") {
                        mouse
                            .read_snapshot_with_receiver(device.product_id != 0x4033)
                            .and_then(|v| Ok(serde_json::to_value(v)?))
                    } else {
                        mouse
                            .read_status()
                            .and_then(|v| Ok(serde_json::to_value(v)?))
                    };
                    match result {
                        Ok(value) => {
                            data.insert("mouse".into(), value);
                        }
                        Err(error) => {
                            data.insert(
                                "mouse_error".into(),
                                error
                                    .chain()
                                    .map(|context| {
                                        mchose_hub_rust::i18n::message(&context.to_string())
                                    })
                                    .collect::<Vec<_>>()
                                    .join(": ")
                                    .into(),
                            );
                        }
                    }
                }
                DeviceKind::Dock => {
                    let result = Dock::new(io).read_settings();
                    match result {
                        Ok(value) => {
                            data.insert("dock".into(), serde_json::to_value(value)?);
                        }
                        Err(error) => {
                            data.insert(
                                "dock_error".into(),
                                error
                                    .chain()
                                    .map(|context| {
                                        mchose_hub_rust::i18n::message(&context.to_string())
                                    })
                                    .collect::<Vec<_>>()
                                    .join(": ")
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
        println!("{}", serde_json::to_string_pretty(&data)?);
        if data.keys().any(|key| key.ends_with("_error")) {
            std::process::exit(1);
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&devices)?);
    }
    Ok(())
}
