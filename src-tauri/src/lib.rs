pub mod assemblyai;
mod audio;
pub mod config;
mod dictation;
mod gemini;
mod paste;

use dictation::AppState;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const DEFAULT_HOTKEY_MODIFIERS: Modifiers = Modifiers::SUPER.union(Modifiers::SHIFT);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let cfg = config::Config::load().expect(
        "missing ASSEMBLYAI_API_KEY / GEMINI_API_KEY — check that .env exists in the project root",
    );

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app.state::<AppState>();
                            dictation::toggle(&state, &app).await;
                        });
                    }
                })
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            app.manage(AppState::new(cfg));

            let hotkey = Shortcut::new(Some(DEFAULT_HOTKEY_MODIFIERS), Code::Space);
            app.global_shortcut().register(hotkey)?;
            println!("[dictate] hotkey registered: Cmd/Ctrl+Shift+Space toggles listening");

            let quit_item = MenuItem::with_id(app, "quit", "Quit Dictate", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("Dictate — Cmd/Ctrl+Shift+Space to talk")
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;

            // Hide from the dock/taskbar + close the placeholder window; this app is
            // tray + global-hotkey driven, not window driven.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.hide();
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
