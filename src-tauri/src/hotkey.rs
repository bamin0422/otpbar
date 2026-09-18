//! 전역 단축키. 어느 창에 있든 코드를 바로 클립보드에 넣는다.
//!
//! 이 앱을 쓰는 이유는 "코드를 빨리 꺼내는 것"인데, 종전에는 메뉴를 열고 계정을 찾아
//! 눌러야 했다. 단축키 하나로 그 과정을 없앤다.
//!
//! 무엇을 복사할지는 다음 순서로 정한다.
//!
//! 1. 마지막에 쓴 계정 — 실제로 쓰는 계정은 대개 하나다
//! 2. 계정이 하나뿐이면 그것
//! 3. 그 밖에는 고르라고 대시보드를 연다

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::state::AppState;
use crate::ui;

/// 설정에 적힌 단축키를 등록한다. 이미 다른 앱이 쓰고 있으면 조용히 실패한다.
pub fn register(app: &AppHandle) {
    let settings = app.state::<AppState>().settings();
    if !settings.hotkey_enabled {
        return;
    }
    let combo = settings.hotkey.clone();
    let result = app
        .global_shortcut()
        .on_shortcut(combo.as_str(), |app, _shortcut, event| {
            // 누를 때 한 번만 반응한다. 떼는 순간까지 처리하면 두 번 복사된다.
            if event.state == ShortcutState::Pressed {
                let app = app.clone();
                std::thread::spawn(move || copy_last_used(&app));
            }
        });
    match result {
        Ok(()) => crate::log!("전역 단축키를 등록했습니다: {combo}"),
        // 다른 앱이 선점한 조합일 수 있다. 앱 전체를 막을 일은 아니다.
        Err(e) => crate::log!("전역 단축키를 등록하지 못했습니다({combo}): {e}"),
    }
}

/// 등록을 해제한다. 설정을 끄거나 조합을 바꿀 때 부른다.
pub fn unregister(app: &AppHandle) {
    let combo = app.state::<AppState>().settings().hotkey;
    if let Err(e) = app.global_shortcut().unregister(combo.as_str()) {
        crate::log!("전역 단축키를 해제하지 못했습니다: {e}");
    }
}

/// 설정을 뒤집고 등록 상태를 맞춘다.
pub fn toggle(app: &AppHandle) {
    let state = app.state::<AppState>();
    let mut s = state.settings();
    s.hotkey_enabled = !s.hotkey_enabled;
    let now = s.hotkey_enabled;
    let combo = s.hotkey.clone();
    state.set_settings(s).ok();

    if now {
        register(app);
        ui::notify(app, "OTPBar", &format!("{combo} 로 코드를 복사합니다."));
    } else {
        unregister(app);
        ui::notify(app, "OTPBar", "전역 단축키를 껐습니다.");
    }
    ui::refresh(app);
}

/// 단축키를 눌렀을 때의 동작.
fn copy_last_used(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        ui::notify(
            app,
            "OTPBar",
            "금고가 잠겨 있습니다. 먼저 잠금을 해제하십시오.",
        );
        crate::tray::unlock_flow(app);
        return;
    }

    let settings = state.settings();
    let target = match settings.last_used_id.clone() {
        Some(id) => Some(id),
        None => match state.list_items(false) {
            // 계정이 하나뿐이면 고를 것도 없다
            Ok(items) if items.len() == 1 => Some(items[0].account.id.clone()),
            _ => None,
        },
    };

    let Some(id) = target else {
        ui::notify(app, "OTPBar", "어느 계정을 쓸지 정하십시오.");
        crate::dashboard::open(app, crate::dashboard::Reason::User);
        return;
    };

    state.touch();
    match state.code_for(&id) {
        Ok((account, code, remaining)) => {
            ui::copy_code(app, &code, settings.clipboard_clear_secs);
            let label = format!("{} · {}", account.issuer, account.name);
            ui::notify_code_used(app, &label, &code, remaining, true, &settings);
        }
        Err(e) => {
            // 계정이 지워졌을 수 있다. 기억을 비우고 다음번에 다시 고르게 한다.
            crate::log!("단축키로 코드를 꺼내지 못했습니다: {e}");
            state.forget_last_used();
            ui::notify(app, "OTP 오류", &e.to_string());
        }
    }
}
