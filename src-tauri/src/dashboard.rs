//! 계정을 한 화면에 모아 보는 대시보드 창.
//!
//! 이 앱의 주 화면은 여전히 메뉴바·트레이 메뉴다. 대시보드는 계정이 많아 메뉴가
//! 길어질 때 한눈에 보기 위한 보조 화면이며, 두 가지 경로로 열린다.
//!
//! 1. 메뉴의 "대시보드 열기"
//! 2. 트레이 등록이 끝내 실패했을 때 자동으로([`crate::tray::watch_registration`])
//!
//! 2번 경로로 열릴 때만 트레이를 다시 시도하는 안내와 단추를 보여 준다. 2.1.0에서 창을
//! 없앤 뒤 트레이가 유일한 진입점이 되었고, 그것이 실패하면 앱을 쓸 방법이 사라졌다.
//! 대시보드는 그 단일 실패 지점을 없애는 역할도 겸한다.

use otpbar_core::ipc::ListItem;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::AppState;

pub const WINDOW_LABEL: &str = "dashboard";

/// 창을 연 이유. 트레이 실패로 열렸을 때만 복구 안내를 띄운다.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// 사용자가 메뉴에서 열었다.
    User,
    /// 트레이 등록이 실패해 대신 열었다.
    TrayFailed,
}

/// 대시보드를 연다. 이미 열려 있으면 앞으로 가져오기만 한다.
pub fn open(app: &AppHandle, reason: Reason) {
    let handle = app.clone();
    let result = app.run_on_main_thread(move || {
        if let Some(win) = handle.get_webview_window(WINDOW_LABEL) {
            win.show().ok();
            win.set_focus().ok();
            return;
        }
        // 창 없는 앱으로 설정해 두었으므로, 창을 띄우는 동안에는 일반 앱으로 돌아온다.
        #[cfg(target_os = "macos")]
        handle
            .set_activation_policy(tauri::ActivationPolicy::Regular)
            .ok();

        let url = match reason {
            Reason::User => "dashboard.html",
            Reason::TrayFailed => "dashboard.html?tray=failed",
        };
        match WebviewWindowBuilder::new(&handle, WINDOW_LABEL, WebviewUrl::App(url.into()))
            .title("OTPBar")
            .inner_size(460.0, 600.0)
            .min_inner_size(380.0, 420.0)
            .center()
            .build()
        {
            Ok(win) => {
                crate::log!("대시보드를 열었습니다.");
                // 창을 닫으면 다시 메뉴바 전용 앱으로 돌아간다. 이 처리가 없으면
                // macOS Dock에 아이콘이 남는다(2.1.1의 누락).
                #[cfg(target_os = "macos")]
                {
                    let h = win.app_handle().clone();
                    win.on_window_event(move |event| {
                        if matches!(event, tauri::WindowEvent::Destroyed) {
                            h.set_activation_policy(tauri::ActivationPolicy::Accessory)
                                .ok();
                        }
                    });
                }
                #[cfg(not(target_os = "macos"))]
                let _ = win;
            }
            Err(e) => crate::log!("대시보드를 열지 못했습니다: {e}"),
        }
    });
    if let Err(e) = result {
        crate::log!("대시보드 요청을 주 스레드로 넘기지 못했습니다: {e}");
    }
}

/// 계정과 현재 코드를 돌려준다. 잠겨 있으면 빈 목록이다.
#[tauri::command]
pub fn dashboard_list(app: AppHandle) -> Result<View, String> {
    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        return Ok(View {
            unlocked: false,
            masked: state.settings().mask_codes,
            items: Vec::new(),
        });
    }
    Ok(View {
        unlocked: true,
        masked: state.settings().mask_codes,
        items: state.list_items(true).map_err(|e| e.to_string())?,
    })
}

/// 코드를 클립보드에 넣는다. 트레이 항목을 누른 것과 같은 동작이다.
#[tauri::command]
pub fn dashboard_copy(app: AppHandle, id: String) -> Result<String, String> {
    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        return Err("금고가 잠겨 있습니다".into());
    }
    state.touch();
    let (account, code, remaining) = state.code_for(&id).map_err(|e| e.to_string())?;
    state.remember_last_used(&account.id);
    let settings = state.settings();
    crate::ui::copy_code(&app, &code, settings.clipboard_clear_secs);
    Ok(format!(
        "{} · {} 복사 ({remaining}초 남음)",
        account.issuer, account.name
    ))
}

/// 잠금을 푼다. 암호 보호 중이면 시스템 입력 상자를 띄운다.
#[tauri::command]
pub fn dashboard_unlock(app: AppHandle) {
    std::thread::spawn(move || crate::tray::unlock_flow(&app));
}

/// 코드 가리기를 켜고 끈다.
#[tauri::command]
pub fn dashboard_toggle_mask(app: AppHandle) -> bool {
    let state = app.state::<AppState>();
    let mut s = state.settings();
    s.mask_codes = !s.mask_codes;
    let now = s.mask_codes;
    state.set_settings(s).ok();
    crate::ui::refresh(&app);
    now
}

/// 트레이를 다시 만들어 본다. 트레이 실패로 열린 경우에만 화면에 노출된다.
#[tauri::command]
pub fn dashboard_retry_tray(app: AppHandle) -> Result<bool, String> {
    let handle = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        handle.remove_tray_by_id(crate::tray::TRAY_ID);
        let ok = crate::tray::create(&handle).is_ok();
        tx.send(ok).ok();
    })
    .map_err(|e| e.to_string())?;
    let created = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or(false);
    crate::log!("사용자가 트레이 재시도를 눌렀습니다 — 결과 {created}");
    Ok(created)
}

/// 업데이트를 확인한다.
#[tauri::command]
pub fn dashboard_check_update(app: AppHandle) {
    crate::check_update_interactive(&app);
}

#[derive(serde::Serialize)]
pub struct View {
    pub unlocked: bool,
    pub masked: bool,
    pub items: Vec<ListItem>,
}
