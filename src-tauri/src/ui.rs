//! 알림과 클립보드. 창이 없으므로 사용자에게 말하는 수단은 시스템 알림뿐이다.
//!
//! 기본값은 보수적이다. 알림에는 코드를 넣지 않고(잠금 화면 미리보기 노출 방지),
//! 복사한 코드는 설정한 시간 뒤 클립보드에서 지운다.

use std::time::Duration;

use otpbar_core::Settings;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;

/// 메뉴를 다시 그린다. 상태가 바뀔 때마다 부른다.
pub fn refresh(app: &AppHandle) {
    crate::tray::rebuild(app);
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
pub fn notify_code_used(
    app: &AppHandle,
    detail: &str,
    code: &str,
    remaining: u64,
    copied: bool,
    settings: &Settings,
) {
    let shown = if settings.show_code_in_notification {
        format!("{}   ", pretty(code))
    } else {
        String::new()
    };
    let suffix = if copied { ", 복사됨" } else { "" };
    notify(
        app,
        "OTP 코드 사용",
        &format!("{detail}\n{shown}({remaining}초 남음{suffix})"),
    );
}

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .ok();
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
