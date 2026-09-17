//! 트레이를 끝내 만들지 못했을 때 여는 대체 창.
//!
//! 2.1.0에서 창을 없애고 메뉴바·트레이 하나만 남겼다. 평소에는 이 편이 낫지만,
//! 트레이 등록이 실패하면 사용자에게 남는 진입점이 하나도 없다는 약점이 있었다.
//! Windows에서 실제로 그 일이 벌어졌다([`crate::tray::watch_registration`] 참고).
//!
//! 이 창은 평상시에 뜨지 않는다. 트레이 재시도가 모두 실패했을 때만 열리며,
//! 코드를 꺼내 쓰는 최소 기능과 트레이를 다시 시도할 방법, 그리고 업데이트 경로를 준다.

use otpbar_core::ipc::ListItem;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::state::AppState;

pub const WINDOW_LABEL: &str = "fallback";

/// 대체 창을 연다. 이미 열려 있으면 앞으로 가져오기만 한다.
pub fn open(app: &AppHandle) {
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

        match WebviewWindowBuilder::new(
            &handle,
            WINDOW_LABEL,
            WebviewUrl::App("fallback.html".into()),
        )
        .title("OTPBar")
        .inner_size(420.0, 560.0)
        .min_inner_size(360.0, 420.0)
        .center()
        .build()
        {
            Ok(_) => crate::log!("대체 창을 열었습니다."),
            Err(e) => crate::log!("대체 창을 열지 못했습니다: {e}"),
        }
    });
    if let Err(e) = result {
        crate::log!("대체 창 요청을 주 스레드로 넘기지 못했습니다: {e}");
    }
}

/// 계정과 현재 코드를 돌려준다. 잠겨 있으면 빈 목록이다.
#[tauri::command]
pub fn fallback_list(app: AppHandle) -> Result<FallbackView, String> {
    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        return Ok(FallbackView {
            unlocked: false,
            items: Vec::new(),
        });
    }
    Ok(FallbackView {
        unlocked: true,
        items: state.list_items(true).map_err(|e| e.to_string())?,
    })
}

/// 코드를 클립보드에 넣는다. 트레이 항목을 누른 것과 같은 동작이다.
#[tauri::command]
pub fn fallback_copy(app: AppHandle, id: String) -> Result<String, String> {
    let state = app.state::<AppState>();
    state.touch();
    let (account, code, remaining) = state.code_for(&id).map_err(|e| e.to_string())?;
    let settings = state.settings();
    crate::ui::copy_code(&app, &code, settings.clipboard_clear_secs);
    Ok(format!(
        "{} · {} 복사됨 ({remaining}초 남음)",
        account.issuer, account.name
    ))
}

/// 잠금을 푼다. 암호 보호 중이면 시스템 입력 상자를 띄운다.
#[tauri::command]
pub fn fallback_unlock(app: AppHandle) {
    std::thread::spawn(move || crate::tray::unlock_flow(&app));
}

/// 트레이를 다시 만들어 본다. 성공하면 이 창은 닫는다.
#[tauri::command]
pub fn fallback_retry_tray(app: AppHandle) -> Result<bool, String> {
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

/// 업데이트를 확인한다. 이 창이 열렸다는 것은 트레이 메뉴를 쓸 수 없다는 뜻이므로,
/// 새 버전으로 빠져나갈 길을 여기에도 둔다.
#[tauri::command]
pub fn fallback_check_update(app: AppHandle) {
    crate::check_update_interactive(&app);
}

#[derive(serde::Serialize)]
pub struct FallbackView {
    pub unlocked: bool,
    pub items: Vec<ListItem>,
}
