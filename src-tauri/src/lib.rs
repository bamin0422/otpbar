//! OTPBar 앱 본체.
//!
//! - 금고를 열고 잠그는 주체는 이 프로세스뿐이다.
//! - CLI는 로컬 소켓으로 코드만 받아 간다([`agent`]).
//! - 창을 닫아도 종료하지 않고 메뉴바·트레이에 남는다.

mod agent;
mod commands;
mod state;
mod tray;
mod ui;

use std::time::Duration;

use otpbar_core::{crypto, ipc, transport};
use tauri::{Manager, WindowEvent};

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let token = crypto::random_token();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))
            .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                // 두 번째 실행은 창만 띄우고 종료된다
                ui::show_window(app);
            }));
    }

    builder
        .invoke_handler(tauri::generate_handler![
            commands::vault_status,
            commands::create_vault,
            commands::unlock,
            commands::lock,
            commands::change_password,
            commands::list_accounts,
            commands::get_code,
            commands::import_source,
            commands::add_account,
            commands::remove_account,
            commands::rename_account,
            commands::get_settings,
            commands::set_settings,
            commands::hide_window,
            commands::config_dir,
            commands::check_update,
            commands::install_update,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            let state =
                AppState::new(token.clone()).map_err(|e| format!("금고를 읽지 못했습니다: {e}"))?;
            app.manage(state);

            // macOS에서는 Dock 아이콘 없이 메뉴바 앱으로 시작한다
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            tray::create(&handle)?;

            // 로컬 에이전트: CLI 요청을 받는다
            let endpoint = ipc::default_endpoint();
            match transport::Listener::bind(&endpoint) {
                Ok(listener) => {
                    // Windows(TCP)는 실제 포트가 bind 후에 정해진다
                    let actual = listener.local_endpoint().unwrap_or_default();
                    let advertised = if actual.is_empty() {
                        endpoint.clone()
                    } else {
                        actual
                    };
                    if let Err(e) = state::publish_runtime(&advertised, &token) {
                        eprintln!("[otpbar] 실행 정보 기록 실패: {e}");
                    }
                    agent::spawn(handle.clone(), advertised, listener);
                }
                Err(e) => eprintln!("[otpbar] 로컬 에이전트를 열지 못했습니다: {e}"),
            }

            // 자동 잠금 감시: 1초마다 확인하고, 잠기면 화면을 갱신한다
            let watch = handle.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let state = watch.state::<AppState>();
                if state.auto_lock_if_idle() {
                    ui::refresh(&watch);
                    ui::notify(
                        &watch,
                        "OTPBar",
                        "일정 시간 동안 사용하지 않아 금고를 잠갔습니다.",
                    );
                }
            });

            // 금고가 없으면 첫 실행이므로 창을 띄워 마스터 암호를 정하게 한다
            let state = handle.state::<AppState>();
            if !state.has_vault() {
                ui::show_window(&handle);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // 창을 닫아도 앱은 트레이에 남는다
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                ui::hide_window(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
        .expect("OTPBar를 시작하지 못했습니다")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let _ = app;
                ipc::RuntimeInfo::remove();
            }
        });
}
