//! OTPBar 앱 본체. **메뉴바·트레이 전용이며 창을 만들지 않는다.**
//!
//! - 금고를 열고 잠그는 주체는 이 프로세스뿐이다.
//! - CLI는 로컬 소켓으로 코드만 받아 간다([`agent`]).
//! - 사용자와의 대화는 메뉴, 시스템 알림, 그리고 입력이 필요한 순간의 짧은 다이얼로그로 한다.

mod agent;
mod prompt;
mod state;
mod tray;
mod ui;

use std::path::Path;
use std::time::Duration;

use otpbar_core::{crypto, ipc, transport};
use tauri::Manager;

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
                // 두 번째 실행은 메뉴만 새로 그리고 끝난다
                tray::rebuild(app);
                ui::notify(
                    app,
                    "OTPBar",
                    "이미 실행 중입니다. 메뉴바 아이콘을 누르십시오.",
                );
            }));
    }

    builder
        .setup(move |app| {
            let handle = app.handle().clone();

            let state =
                AppState::new(token.clone()).map_err(|e| format!("금고를 읽지 못했습니다: {e}"))?;
            app.manage(state);

            // 창이 없는 앱이므로 Dock·작업 표시줄에 나타나지 않는다
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // 금고를 준비한다. 기본 구성에서는 암호 없이 바로 열린다.
            let state = handle.state::<AppState>();
            match state.ensure_ready() {
                Ok(true) => {}
                Ok(false) => ui::notify(
                    &handle,
                    "OTPBar",
                    "금고가 잠겨 있습니다. 메뉴바 아이콘 → 잠금 해제를 누르십시오.",
                ),
                Err(e) => eprintln!("[otpbar] 금고를 준비하지 못했습니다: {e}"),
            }

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

            // 1초마다 메뉴의 코드·남은 시간 글자를 갱신한다. 메뉴 구조는 건드리지 않으므로
            // 열려 있는 메뉴가 닫히지 않는다. 자동 잠금도 같은 주기로 확인한다.
            let watch = handle.clone();
            std::thread::spawn(move || {
                let mut ticks: u64 = 0;
                loop {
                    std::thread::sleep(Duration::from_secs(1));
                    let handle = watch.clone();
                    // 메뉴 조작은 주 스레드에서 해야 안전하다
                    handle
                        .clone()
                        .run_on_main_thread(move || {
                            tray::tick(&handle);
                        })
                        .ok();
                    ticks += 1;
                    if ticks % 5 == 0 {
                        let state = watch.state::<AppState>();
                        if state.auto_lock_if_idle() {
                            ui::refresh(&watch);
                            ui::notify(&watch, "OTPBar", "한동안 쓰지 않아 잠갔습니다.");
                        }
                    }
                }
            });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("OTPBar를 시작하지 못했습니다")
        .run(|_app, event| {
            // 창이 하나도 없는 앱이므로, 그냥 두면 Tauri가 곧바로 종료한다.
            // 사용자가 메뉴에서 종료를 고른 경우(code 있음)만 실제로 끝낸다.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                } else {
                    ipc::RuntimeInfo::remove();
                }
            }
        });
}

// ---------- 메뉴에서 부르는 동작들 ----------

pub(crate) fn autostart_enabled(app: &tauri::AppHandle) -> bool {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        return app.autolaunch().is_enabled().unwrap_or(false);
    }
    #[allow(unreachable_code)]
    {
        let _ = app;
        false
    }
}

pub(crate) fn toggle_autostart(app: &tauri::AppHandle) {
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        let now = manager.is_enabled().unwrap_or(false);
        let result = if now {
            manager.disable()
        } else {
            manager.enable()
        };
        match result {
            Ok(()) => ui::notify(
                app,
                "OTPBar",
                if now {
                    "로그인 시 실행을 껐습니다."
                } else {
                    "로그인할 때 자동으로 실행됩니다."
                },
            ),
            Err(e) => ui::notify(app, "OTPBar", &format!("설정을 바꾸지 못했습니다: {e}")),
        }
    }
}

/// QR 이미지를 골라 가져온다. 파일 선택은 운영체제 표준 패널을 쓴다.
pub(crate) fn import_via_dialog(app: &tauri::AppHandle) {
    use tauri_plugin_dialog::DialogExt;

    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        ui::notify(app, "OTPBar", "먼저 잠금을 해제하십시오.");
        return;
    }
    let picked = app
        .dialog()
        .file()
        .set_title("구글 OTP 내보내기 QR 또는 otpauth QR 이미지")
        .add_filter("이미지", &["png", "jpg", "jpeg", "bmp", "webp", "tiff"])
        .blocking_pick_files();
    let Some(paths) = picked else { return };

    let mut added = 0usize;
    let mut lines: Vec<String> = Vec::new();
    for p in paths {
        let path = p.to_string();
        match state.import(&path, false) {
            Ok((messages, total)) => {
                added += total;
                lines.extend(messages);
            }
            Err(e) => lines.push(format!("실패: {e}")),
        }
    }
    ui::refresh(app);
    let summary = if lines.is_empty() {
        "가져올 계정이 없습니다.".to_string()
    } else {
        let head: Vec<&str> = lines.iter().take(4).map(|s| s.as_str()).collect();
        let more = if lines.len() > 4 {
            format!("\n… 외 {}건", lines.len() - 4)
        } else {
            String::new()
        };
        format!("{}{}", head.join("\n"), more)
    };
    ui::notify(app, &format!("가져오기 {added}건"), &summary);
}

pub(crate) fn open_path(app: &tauri::AppHandle, path: &Path) {
    use tauri_plugin_opener::OpenerExt;
    if let Err(e) = app
        .opener()
        .open_path(path.to_string_lossy().to_string(), None::<&str>)
    {
        ui::notify(app, "OTPBar", &format!("폴더를 열지 못했습니다: {e}"));
    }
}

/// 메뉴에서 업데이트를 확인하고, 있으면 바로 설치한다.
pub(crate) fn check_update_interactive(app: &tauri::AppHandle) {
    #[cfg(desktop)]
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri_plugin_updater::UpdaterExt;
            let updater = match app.updater() {
                Ok(u) => u,
                Err(e) => {
                    ui::notify(
                        &app,
                        "OTPBar",
                        &format!("업데이트를 확인하지 못했습니다: {e}"),
                    );
                    return;
                }
            };
            match updater.check().await {
                Ok(Some(update)) => {
                    let version = update.version.clone();
                    ui::notify(
                        &app,
                        "OTPBar 업데이트",
                        &format!("{version} 설치를 시작합니다. 끝나면 다시 알려 드립니다."),
                    );
                    match update.download_and_install(|_, _| {}, || {}).await {
                        Ok(()) => ui::notify(
                            &app,
                            "OTPBar 업데이트 완료",
                            &format!("{version} 설치를 마쳤습니다. 앱을 다시 시작하면 적용됩니다."),
                        ),
                        Err(e) => ui::notify(&app, "OTPBar", &format!("설치 실패: {e}")),
                    }
                }
                Ok(None) => ui::notify(
                    &app,
                    "OTPBar",
                    &format!("최신 상태입니다 ({}).", otpbar_core::VERSION),
                ),
                Err(e) => {
                    let msg = e.to_string();
                    let friendly = if msg.contains("valid release JSON") || msg.contains("404") {
                        "아직 배포된 업데이트가 없습니다.".to_string()
                    } else {
                        format!("업데이트 확인 실패: {msg}")
                    };
                    ui::notify(&app, "OTPBar", &friendly);
                }
            }
        });
    }
}
