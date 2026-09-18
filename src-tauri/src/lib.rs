//! OTPBar 앱 본체. **메뉴바·트레이 전용이며 창을 만들지 않는다.**
//!
//! - 금고를 열고 잠그는 주체는 이 프로세스뿐이다.
//! - CLI는 로컬 소켓으로 코드만 받아 간다([`agent`]).
//! - 사용자와의 대화는 메뉴, 시스템 알림, 그리고 입력이 필요한 순간의 짧은 다이얼로그로 한다.

mod agent;
mod dashboard;
mod hotkey;
mod log;
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
    log::session_start(otpbar_core::VERSION);
    let token = crypto::random_token();

    let mut builder = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            dashboard::dashboard_list,
            dashboard::dashboard_copy,
            dashboard::dashboard_unlock,
            dashboard::dashboard_toggle_mask,
            dashboard::dashboard_retry_tray,
            dashboard::dashboard_check_update,
        ])
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init());

    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_global_shortcut::Builder::new().build())
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

            let state = AppState::new(token.clone()).map_err(|e| {
                log!("금고를 읽지 못했습니다: {e}");
                format!("금고를 읽지 못했습니다: {e}")
            })?;
            log!("금고 파일을 읽었습니다.");
            app.manage(state);

            // 창이 없는 앱이므로 Dock·작업 표시줄에 나타나지 않는다
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // 금고를 준비한다. 기본 구성에서는 암호 없이 바로 열린다.
            let state = handle.state::<AppState>();
            match state.ensure_ready() {
                Ok(true) => log!("금고를 열었습니다."),
                Ok(false) => {
                    log!("금고가 잠겨 있습니다(암호 보호).");
                    ui::notify(
                        &handle,
                        "OTPBar",
                        "금고가 잠겨 있습니다. 메뉴바 아이콘 → 잠금 해제를 누르십시오.",
                    );
                }
                Err(e) => log!("금고를 준비하지 못했습니다: {e}"),
            }

            if let Err(e) = tray::create(&handle) {
                // 여기서 실패하면 트레이도 창도 없다. 대체 창이 마지막 수단이다.
                log!("트레이를 만들지 못했습니다: {e}");
                dashboard::open(&handle, dashboard::Reason::TrayFailed);
            } else {
                log!("트레이를 만들었습니다. 등록 여부를 확인합니다.");
                tray::watch_registration(&handle);
            }

            // 시작할 때 조용히 확인한다. 트레이를 쓸 수 없는 상황에서도 알림은 뜨므로,
            // 이번처럼 진입점이 막혔을 때 다음 버전으로 빠져나갈 길이 된다.
            hotkey::register(&handle);
            check_update_on_start(&handle);

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

/// 시작 직후 조용히 업데이트를 확인한다. 알림만 띄우고 설치는 하지 않는다.
///
/// 2.1.0에서 업데이트 경로가 트레이 메뉴 하나뿐이었던 탓에, Windows에서 트레이가
/// 뜨지 않은 사용자는 고친 버전을 받을 방법조차 없었다. 알림은 트레이와 무관하게
/// 뜨므로, 진입점이 막혀도 다음 버전이 있다는 사실은 전달된다.
pub(crate) fn check_update_on_start(app: &tauri::AppHandle) {
    #[cfg(desktop)]
    {
        /// 확인 간격. 오래 켜 두는 앱이므로 하루에 몇 번이면 충분하다.
        const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

        let app = app.clone();
        std::thread::spawn(move || {
            // 켜자마자 네트워크를 쓰면 시작이 느려 보인다. 잠깐 미룬다.
            std::thread::sleep(Duration::from_secs(10));
            loop {
                if !app.state::<AppState>().settings().auto_update_check {
                    log!("자동 업데이트 확인이 꺼져 있습니다.");
                    return;
                }
                if check_and_maybe_install(&app) {
                    return; // 설치를 시작했다. 곧 프로세스가 끝난다.
                }
                std::thread::sleep(EVERY);
            }
        });
    }
}

/// 자동 설치를 해도 되는 상태인지 판단한다.
///
/// Windows 설치 경로는 `std::process::exit(0)`으로 앱을 즉시 끝낸다
/// (`tauri-plugin-updater`의 `install_inner`). 코드를 복사하려는 순간에 걸리면 곤란하므로
/// 한동안 손대지 않은 때만 진행한다.
///
/// 마스터 암호 보호를 쓰는 사용자는 제외한다. 재시작 뒤 암호를 다시 물어야 해서
/// 편의를 주려는 기능이 오히려 번거로워진다.
#[cfg(desktop)]
fn may_auto_install(app: &tauri::AppHandle) -> bool {
    /// 이만큼 조작이 없으면 자리를 비웠다고 본다.
    const IDLE_SECS: u64 = 5 * 60;

    let state = app.state::<AppState>();
    state.auto_unlock_enabled() && state.idle_secs() >= IDLE_SECS
}

/// 업데이트를 확인하고, 조건이 맞으면 설치까지 한다. 설치를 시작했으면 `true`.
#[cfg(desktop)]
fn check_and_maybe_install(app: &tauri::AppHandle) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri_plugin_updater::UpdaterExt;
        let started = match handle.updater() {
            Ok(updater) => match updater.check().await {
                Ok(Some(update)) => {
                    let version = update.version.clone();
                    log!("새 버전 {version}을 찾았습니다.");
                    if may_auto_install(&handle) {
                        ui::notify(
                            &handle,
                            "OTPBar 업데이트",
                            &format!("{version} 설치를 시작합니다. 잠시 뒤 다시 켜집니다."),
                        );
                        match update.download_and_install(|_, _| {}, || {}).await {
                            Ok(()) => {
                                log!("{version} 설치를 마쳤습니다.");
                                true
                            }
                            Err(e) => {
                                log!("자동 설치 실패: {e}");
                                false
                            }
                        }
                    } else {
                        // 쓰는 중이거나 암호 보호 사용자다. 알리고 다음 주기로 미룬다.
                        log!("자동 설치 조건이 아닙니다. 알림만 띄웁니다.");
                        ui::notify(
                            &handle,
                            "OTPBar 업데이트",
                            &format!(
                                "{version} 버전이 나왔습니다. 메뉴의 '업데이트 확인'에서 설치하실 수 있습니다."
                            ),
                        );
                        false
                    }
                }
                Ok(None) => {
                    log!("최신 상태입니다.");
                    false
                }
                Err(e) => {
                    log!("업데이트 확인 실패: {e}");
                    false
                }
            },
            Err(e) => {
                log!("업데이터를 쓸 수 없습니다: {e}");
                false
            }
        };
        tx.send(started).ok();
    });
    // 내려받기까지 포함하므로 넉넉히 기다린다. 응답이 없으면 다음 주기에 다시 본다.
    rx.recv_timeout(Duration::from_secs(600)).unwrap_or(false)
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
