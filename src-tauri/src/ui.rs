//! 창·알림·클립보드 같은 사용자 표면.
//!
//! 기본값은 보수적이다. 알림에는 코드를 넣지 않고(잠금 화면 미리보기 노출 방지),
//! 복사한 코드는 설정한 시간 뒤 클립보드에서 지운다.

use std::time::Duration;

use otpbar_core::Settings;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;

pub const EVENT_REFRESH: &str = "otpbar://refresh";

/// 프론트엔드와 트레이에 "상태가 바뀌었다"고 알린다.
pub fn refresh(app: &AppHandle) {
    app.emit(EVENT_REFRESH, ()).ok();
    crate::tray::rebuild(app);
}

pub fn show_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        win.show().ok();
        win.unminimize().ok();
        win.set_focus().ok();
    }
    #[cfg(target_os = "macos")]
    {
        // 창을 보여 줄 때만 Dock에 나타나게 한다(평소에는 메뉴바 전용).
        app.set_activation_policy(tauri::ActivationPolicy::Regular).ok();
    }
}

pub fn hide_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        win.hide().ok();
    }
    #[cfg(target_os = "macos")]
    {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory).ok();
    }
}

/// 클립보드에 코드를 넣고, 지정 시간 뒤 그대로 남아 있으면 지운다.
pub fn copy_code(app: &AppHandle, code: &str, clear_after_secs: u64) {
    if app.clipboard().write_text(code.to_string()).is_err() {
        return;
    }
    if clear_after_secs == 0 {
        return;
    }
    let app = app.clone();
    let code = code.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(clear_after_secs));
        // 그 사이 사용자가 다른 것을 복사했으면 건드리지 않는다
        if let Ok(current) = app.clipboard().read_text() {
            if current == code {
                app.clipboard().write_text(String::new()).ok();
            }
        }
    });
}

/// 코드가 쓰였음을 알린다. 설정에 따라 코드 자체는 감춘다.
pub fn notify_code_used(app: &AppHandle, detail: &str, code: &str, remaining: u64, copied: bool, settings: &Settings) {
    let shown = if settings.show_code_in_notification {
        format!("{}   ", pretty(code))
    } else {
        String::new()
    };
    let suffix = if copied { ", 복사됨" } else { "" };
    let body = format!("{shown}({remaining}초 남음{suffix})");
    app.notification()
        .builder()
        .title("OTP 코드 사용")
        .body(format!("{detail}\n{body}"))
        .show()
        .ok();
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    app.notification().builder().title(title).body(body).show().ok();
}

pub fn pretty(code: &str) -> String {
    if code.len() == 6 {
        format!("{} {}", &code[..3], &code[3..])
    } else {
        code.to_string()
    }
}

/// 가림 표시용 문자열(자릿수만큼 점).
pub fn masked(code: &str) -> String {
    let n = code.len();
    if n == 6 {
        "••• •••".to_string()
    } else {
        "•".repeat(n)
    }
}
