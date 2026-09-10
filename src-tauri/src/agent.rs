//! CLI를 받아 주는 로컬 에이전트.
//!
//! 요청마다 토큰을 검사하고, 잠긴 금고에는 아무 것도 내주지 않는다.
//! 응답에 비밀키가 실리는 경로는 존재하지 않는다(코드·계정 이름만 나간다).

use std::time::Duration;

use otpbar_core::error::Error;
use otpbar_core::ipc::{Envelope, Request, Response};
use otpbar_core::{crypto, transport};
use tauri::{AppHandle, Manager};

use crate::state::AppState;
use crate::ui;

/// 백그라운드 스레드에서 요청을 받는다.
pub fn spawn(app: AppHandle, endpoint: String, listener: transport::Listener) {
    std::thread::spawn(move || loop {
        match listener.accept() {
            Ok(mut conn) => {
                let app = app.clone();
                std::thread::spawn(move || {
                    // 느린 상대가 스레드를 붙잡지 못하게 시간 제한을 둔다
                    conn.set_timeout(Duration::from_secs(120)).ok();
                    if let Err(err) = handle(&app, &mut conn) {
                        let body = Response::Error { message: err.to_string(), locked: matches!(err, Error::Locked) };
                        if let Ok(line) = serde_json::to_string(&body) {
                            conn.write_line(&line).ok();
                        }
                    }
                });
            }
            Err(err) => {
                eprintln!("[otpbar] 에이전트 accept 실패: {err} (endpoint={endpoint})");
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    });
}

fn handle(app: &AppHandle, conn: &mut transport::Connection) -> Result<(), Error> {
    let Some(line) = conn.read_line()? else { return Ok(()) };
    let envelope: Envelope = serde_json::from_str(&line)
        .map_err(|e| Error::other(format!("요청을 해석하지 못했습니다: {e}")))?;

    let state = app.state::<AppState>();
    // 토큰 검사를 가장 먼저 한다. 실패하면 어떤 정보도 흘리지 않는다.
    if !crypto::constant_time_eq(&envelope.token, &state.token()) {
        conn.write_line(&serde_json::to_string(&Response::Error {
            message: "인증에 실패했습니다".into(),
            locked: false,
        })?)?;
        return Ok(());
    }

    let response = dispatch(app, &state, envelope.request)?;
    conn.write_line(&serde_json::to_string(&response)?)?;
    Ok(())
}

fn dispatch(app: &AppHandle, state: &AppState, request: Request) -> Result<Response, Error> {
    match request {
        Request::Status => Ok(Response::Status {
            unlocked: state.is_unlocked(),
            version: otpbar_core::VERSION.to_string(),
            account_count: state.account_count(),
            has_vault: state.has_vault(),
        }),

        Request::List { with_codes } => {
            state.require_unlocked()?;
            state.touch();
            Ok(Response::List { accounts: state.list_items(with_codes)? })
        }

        Request::Code { query, purpose, copy, wait } => {
            state.require_unlocked()?;
            state.touch();
            let (mut account, mut code, mut remaining) = state.code_for(&query)?;
            // 만료 직전이면 다음 코드를 준다. 입력하는 사이에 코드가 바뀌는 것을 막는다.
            if wait && remaining < 5 {
                std::thread::sleep(Duration::from_secs(remaining + 1));
                let fresh = state.code_for(&query)?;
                account = fresh.0;
                code = fresh.1;
                remaining = fresh.2;
            }
            let settings = state.settings();
            let mut copied = false;
            if copy {
                ui::copy_code(app, &code, settings.clipboard_clear_secs);
                copied = true;
            }
            let label = format!("{} · {}", account.issuer, account.name);
            let detail = if purpose.is_empty() { label.clone() } else { format!("{label} · {purpose}") };
            ui::notify_code_used(app, &detail, &code, remaining, copied, &settings);
            Ok(Response::Code { account, code, remaining, copied })
        }

        Request::Import { source, replace } => {
            state.require_unlocked()?;
            state.touch();
            let (messages, total) = state.import(&source, replace)?;
            ui::refresh(app);
            Ok(Response::Imported { messages, added: total })
        }

        Request::Init { password } => {
            state.create_vault(&password)?;
            ui::refresh(app);
            Ok(Response::Ok)
        }

        Request::Unlock { password } => {
            state.unlock(&password)?;
            ui::refresh(app);
            Ok(Response::Ok)
        }

        Request::Show => {
            ui::show_window(app);
            Ok(Response::Ok)
        }

        Request::Lock => {
            state.lock();
            ui::refresh(app);
            Ok(Response::Ok)
        }
    }
}
