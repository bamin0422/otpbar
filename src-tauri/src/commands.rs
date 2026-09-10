//! 프론트엔드(창 UI)가 부르는 명령들.
//!
//! 어떤 명령도 비밀키를 돌려주지 않는다. 창은 계정 이름과 코드만 본다.

use otpbar_core::ipc::ListItem;
use otpbar_core::{AccountView, NewAccount, Settings};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::state::AppState;
use crate::ui;

/// 커맨드 오류를 프론트엔드에 문자열로 넘긴다.
type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct StatusView {
    pub has_vault: bool,
    pub unlocked: bool,
    pub version: String,
    pub account_count: usize,
    pub settings: Settings,
}

#[tauri::command]
pub fn vault_status(state: State<'_, AppState>) -> StatusView {
    StatusView {
        has_vault: state.has_vault(),
        unlocked: state.is_unlocked(),
        version: otpbar_core::VERSION.to_string(),
        account_count: state.account_count(),
        settings: state.settings(),
    }
}

#[tauri::command]
pub fn create_vault(app: AppHandle, state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.create_vault(&password).map_err(err)?;
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn unlock(app: AppHandle, state: State<'_, AppState>, password: String) -> CmdResult<()> {
    state.unlock(&password).map_err(err)?;
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn lock(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    state.lock();
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn change_password(state: State<'_, AppState>, current: String, new_password: String) -> CmdResult<()> {
    state.change_password(&current, &new_password).map_err(err)
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>, with_codes: bool) -> CmdResult<Vec<ListItem>> {
    state.touch();
    state.list_items(with_codes).map_err(err)
}

#[derive(Serialize)]
pub struct CodeView {
    pub account: AccountView,
    pub code: String,
    pub remaining: u64,
    pub copied: bool,
}

#[tauri::command]
pub fn get_code(app: AppHandle, state: State<'_, AppState>, query: String, copy: bool) -> CmdResult<CodeView> {
    state.touch();
    let (account, code, remaining) = state.code_for(&query).map_err(err)?;
    let settings = state.settings();
    if copy {
        ui::copy_code(&app, &code, settings.clipboard_clear_secs);
        let label = format!("{} · {}", account.issuer, account.name);
        ui::notify_code_used(&app, &label, &code, remaining, true, &settings);
    }
    Ok(CodeView { account, code, remaining, copied: copy })
}

#[derive(Serialize)]
pub struct ImportResult {
    pub messages: Vec<String>,
    pub total: usize,
}

#[tauri::command]
pub fn import_source(app: AppHandle, state: State<'_, AppState>, source: String, replace: bool) -> CmdResult<ImportResult> {
    let (messages, total) = state.import(&source, replace).map_err(err)?;
    ui::refresh(&app);
    Ok(ImportResult { messages, total })
}

#[tauri::command]
pub fn add_account(
    app: AppHandle,
    state: State<'_, AppState>,
    issuer: String,
    name: String,
    secret: String,
    digits: Option<u32>,
    period: Option<u64>,
    algorithm: Option<String>,
    replace: Option<bool>,
) -> CmdResult<String> {
    let algorithm = match algorithm {
        Some(a) => otpbar_core::Algorithm::parse(&a).map_err(err)?,
        None => otpbar_core::Algorithm::Sha1,
    };
    let new = NewAccount {
        issuer,
        name,
        secret,
        algorithm,
        digits: digits.unwrap_or(6),
        period: period.unwrap_or(30),
    };
    let id = state.add_manual(new, replace.unwrap_or(false)).map_err(err)?;
    ui::refresh(&app);
    Ok(id)
}

#[tauri::command]
pub fn remove_account(app: AppHandle, state: State<'_, AppState>, query: String) -> CmdResult<AccountView> {
    let view = state.remove(&query).map_err(err)?;
    ui::refresh(&app);
    Ok(view)
}

#[tauri::command]
pub fn rename_account(
    app: AppHandle,
    state: State<'_, AppState>,
    query: String,
    issuer: Option<String>,
    name: Option<String>,
) -> CmdResult<AccountView> {
    let view = state.rename(&query, issuer, name).map_err(err)?;
    ui::refresh(&app);
    Ok(view)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn set_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    state.set_settings(settings).map_err(err)?;
    ui::refresh(&app);
    Ok(())
}

#[tauri::command]
pub fn hide_window(app: AppHandle) {
    ui::hide_window(&app);
}

#[tauri::command]
pub fn config_dir() -> String {
    otpbar_core::util::config_dir().to_string_lossy().to_string()
}

/// 업데이트 확인·설치. 서명 검증은 updater 플러그인이 공개키로 수행한다.
#[cfg(desktop)]
#[tauri::command]
pub async fn check_update(app: AppHandle) -> CmdResult<Option<String>> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(err)?;
    match updater.check().await.map_err(err)? {
        Some(update) => Ok(Some(update.version)),
        None => Ok(None),
    }
}

#[cfg(desktop)]
#[tauri::command]
pub async fn install_update(app: AppHandle) -> CmdResult<bool> {
    use tauri_plugin_updater::UpdaterExt;
    let updater = app.updater().map_err(err)?;
    let Some(update) = updater.check().await.map_err(err)? else {
        return Ok(false);
    };
    // 서명이 유효하지 않으면 download_and_install 이 오류를 낸다.
    update.download_and_install(|_chunk, _total| {}, || {}).await.map_err(err)?;
    Ok(true)
}

#[cfg(not(desktop))]
#[tauri::command]
pub async fn check_update(_app: AppHandle) -> CmdResult<Option<String>> {
    Ok(None)
}

#[cfg(not(desktop))]
#[tauri::command]
pub async fn install_update(_app: AppHandle) -> CmdResult<bool> {
    Ok(false)
}
