use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use fan_core::{DeviceInfo, Engine, EngineConfig, EngineStats};
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub autostart: bool,
    pub start_on_launch: bool,
    pub close_to_tray: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            autostart: false,
            start_on_launch: false,
            close_to_tray: true,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Persisted {
    #[serde(default)]
    config: EngineConfig,
    #[serde(default)]
    settings: AppSettings,
}

#[derive(Debug, Serialize)]
pub struct AppState {
    config: EngineConfig,
    settings: AppSettings,
    running: bool,
}

type EngineSlot = Mutex<Option<Engine>>;
type Store = Mutex<Persisted>;

/// Kept so the tray entry can be relabelled Start/Stop.
struct TrayToggle(MenuItem<tauri::Wry>);

#[tauri::command]
fn list_devices() -> Result<Vec<DeviceInfo>, String> {
    fan_core::list_outputs().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_state(store: State<Store>, engine: State<EngineSlot>) -> Result<AppState, String> {
    let persisted = store.lock().unwrap();
    Ok(AppState {
        config: persisted.config.clone(),
        settings: persisted.settings.clone(),
        running: engine.lock().unwrap().is_some(),
    })
}

#[tauri::command]
fn save_config(
    app: AppHandle,
    store: State<Store>,
    engine: State<EngineSlot>,
    config: EngineConfig,
) -> Result<(), String> {
    {
        let mut persisted = store.lock().unwrap();
        persisted.config = config.clone();
        save(&app, &persisted)?;
    }
    if let Some(engine) = engine.lock().unwrap().as_ref() {
        engine.apply(config).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn start(
    app: AppHandle,
    store: State<Store>,
    engine: State<EngineSlot>,
    config: EngineConfig,
) -> Result<(), String> {
    {
        let mut slot = engine.lock().unwrap();
        if slot.is_some() {
            return Err(fan_core::Error::AlreadyRunning.to_string());
        }
        *slot = Some(Engine::start(config.clone()).map_err(|e| e.to_string())?);
    }
    set_toggle_label(&app, true);
    let mut persisted = store.lock().unwrap();
    persisted.config = config;
    save(&app, &persisted)
}

#[tauri::command]
fn stop(app: AppHandle, engine: State<EngineSlot>) -> Result<(), String> {
    if let Some(engine) = engine.lock().unwrap().take() {
        engine.stop();
    }
    set_toggle_label(&app, false);
    Ok(())
}

#[tauri::command]
fn get_stats(engine: State<EngineSlot>) -> Result<EngineStats, String> {
    Ok(match engine.lock().unwrap().as_ref() {
        Some(engine) => engine.stats(),
        None => EngineStats {
            running: false,
            source_name: String::new(),
            source_peak: 0.0,
            outputs: Vec::new(),
        },
    })
}

#[tauri::command]
fn get_settings(store: State<Store>) -> Result<AppSettings, String> {
    Ok(store.lock().unwrap().settings.clone())
}

#[tauri::command]
fn set_settings(app: AppHandle, store: State<Store>, settings: AppSettings) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let result = if settings.autostart {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    result.map_err(|e| e.to_string())?;
    let mut persisted = store.lock().unwrap();
    persisted.settings = settings;
    save(&app, &persisted)
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|dir| dir.join("config.json"))
        .map_err(|e| e.to_string())
}

fn save(app: &AppHandle, persisted: &Persisted) -> Result<(), String> {
    let path = config_path(app)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec_pretty(persisted).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

fn load(app: &AppHandle) -> Persisted {
    config_path(app)
        .ok()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn set_toggle_label(app: &AppHandle, running: bool) {
    if let Some(toggle) = app.try_state::<TrayToggle>() {
        let _ = toggle.0.set_text(if running { "Stop" } else { "Start" });
    }
}

fn toggle_engine(app: &AppHandle) {
    // Store is read before the engine slot is locked: every other path takes
    // them in that order, and swapping here would deadlock.
    let config = app.state::<Store>().lock().unwrap().config.clone();
    let slot = app.state::<EngineSlot>();
    let mut slot = slot.lock().unwrap();
    let running = match slot.take() {
        Some(engine) => {
            engine.stop();
            false
        }
        None => match Engine::start(config) {
            Ok(engine) => {
                *slot = Some(engine);
                true
            }
            Err(e) => {
                eprintln!("start failed: {e}");
                false
            }
        },
    };
    drop(slot);
    set_toggle_label(app, running);
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let persisted = load(app.handle());
            let launch_config =
                persisted.settings.start_on_launch && !persisted.config.outputs.is_empty();
            let config = persisted.config.clone();
            app.manage(Store::new(persisted));
            app.manage(EngineSlot::new(None));

            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let toggle = MenuItem::with_id(app, "toggle", "Start", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;
            app.manage(TrayToggle(toggle));

            TrayIconBuilder::with_id("fan")
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("FAN")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_window(app),
                    "toggle" => toggle_engine(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;

            if launch_config {
                match Engine::start(config) {
                    Ok(engine) => {
                        *app.state::<EngineSlot>().lock().unwrap() = Some(engine);
                        set_toggle_label(app.handle(), true);
                    }
                    Err(e) => eprintln!("start_on_launch failed: {e}"),
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let close_to_tray = window
                    .app_handle()
                    .state::<Store>()
                    .lock()
                    .unwrap()
                    .settings
                    .close_to_tray;
                if close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_devices,
            get_state,
            save_config,
            start,
            stop,
            get_stats,
            get_settings,
            set_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
