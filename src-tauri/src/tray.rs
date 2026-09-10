//! 메뉴바(macOS)·트레이(Windows) 아이콘과 메뉴.
//!
//! 잠긴 상태에서는 계정 이름조차 보여 주지 않는다. 코드는 설정에 따라 가려서 표시하고,
//! 항목을 누르면 복사한다.

use otpbar_core::ipc::ListItem;
use tauri::menu::{Menu, MenuBuilder, MenuEvent, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};

use crate::state::AppState;
use crate::ui;

pub const TRAY_ID: &str = "otpbar-tray";
/// 메뉴바·트레이용 단색 아이콘. 컴파일 시점에 박아 두어 런타임 경로 문제를 없앤다.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray-mono@2x.png");
const ID_SHOW: &str = "show";
const ID_LOCK: &str = "lock";
const ID_UNLOCK: &str = "unlock";
const ID_IMPORT: &str = "import";
const ID_UPDATE: &str = "update";
const ID_QUIT: &str = "quit";
const ACCOUNT_PREFIX: &str = "acct:";

/// 트레이 아이콘을 만든다(앱 시작 시 1회).
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("OTPBar")
        .on_menu_event(on_menu_event);
    match tauri::image::Image::from_bytes(TRAY_ICON_PNG) {
        Ok(icon) => builder = builder.icon(icon),
        Err(_) => {
            if let Some(icon) = app.default_window_icon().cloned() {
                builder = builder.icon(icon);
            }
        }
    }
    // macOS 메뉴바에서는 템플릿 이미지로 다크·라이트 모드에 맞춘다.
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
    }
    // Windows에서는 좌클릭으로 창을 연다(메뉴는 우클릭).
    builder = builder.on_tray_icon_event(|tray, event| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            #[cfg(not(target_os = "macos"))]
            ui::show_window(tray.app_handle());
            #[cfg(target_os = "macos")]
            let _ = tray;
        }
    });
    builder.build(app)?;
    Ok(())
}

/// 상태가 바뀔 때 메뉴를 다시 만든다.
pub fn rebuild(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = build_menu(app) {
            tray.set_menu(Some(menu)).ok();
        }
    }
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let state = app.state::<AppState>();
    let settings = state.settings();
    let mut builder = MenuBuilder::new(app);

    if !state.has_vault() {
        let item = MenuItemBuilder::with_id(ID_SHOW, "금고 만들기…").build(app)?;
        builder = builder.item(&item);
    } else if !state.is_unlocked() {
        let item = MenuItemBuilder::with_id(ID_UNLOCK, "잠금 해제…").build(app)?;
        let hint = MenuItemBuilder::with_id("locked-hint", "금고가 잠겨 있습니다")
            .enabled(false)
            .build(app)?;
        builder = builder.item(&hint).item(&item);
    } else {
        let items: Vec<ListItem> = state.list_items(true).unwrap_or_default();
        if items.is_empty() {
            let empty = MenuItemBuilder::with_id("empty", "등록된 계정이 없습니다")
                .enabled(false)
                .build(app)?;
            builder = builder.item(&empty);
        }
        for item in &items {
            let code = item.code.clone().unwrap_or_else(|| "------".into());
            let shown = if settings.mask_codes {
                ui::masked(&code)
            } else {
                ui::pretty(&code)
            };
            let remaining = item.remaining.unwrap_or(0);
            let label = format!(
                "{} · {}    {}    {}초",
                item.account.issuer, item.account.name, shown, remaining
            );
            let entry =
                MenuItemBuilder::with_id(format!("{ACCOUNT_PREFIX}{}", item.account.id), label)
                    .build(app)?;
            builder = builder.item(&entry);
        }
        let sep = PredefinedMenuItem::separator(app)?;
        let lock = MenuItemBuilder::with_id(ID_LOCK, "지금 잠그기").build(app)?;
        let import = MenuItemBuilder::with_id(ID_IMPORT, "QR 이미지에서 가져오기…").build(app)?;
        builder = builder.item(&sep).item(&import).item(&lock);
    }

    let sep2 = PredefinedMenuItem::separator(app)?;
    let show = MenuItemBuilder::with_id(ID_SHOW, "OTPBar 열기").build(app)?;
    let update = MenuItemBuilder::with_id(ID_UPDATE, "업데이트 확인…").build(app)?;
    let quit = MenuItemBuilder::with_id(ID_QUIT, "OTPBar 종료").build(app)?;
    builder = builder
        .item(&sep2)
        .item(&show)
        .item(&update)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&quit);
    builder.build()
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().0.clone();
    let app = app.clone();
    match id.as_str() {
        ID_SHOW | ID_UNLOCK => ui::show_window(&app),
        ID_LOCK => {
            app.state::<AppState>().lock();
            ui::refresh(&app);
            ui::notify(&app, "OTPBar", "금고를 잠갔습니다.");
        }
        ID_IMPORT => {
            ui::show_window(&app);
            app.emit_to("main", "otpbar://import-dialog", ()).ok();
        }
        ID_UPDATE => {
            ui::show_window(&app);
            app.emit_to("main", "otpbar://check-update", ()).ok();
        }
        ID_QUIT => {
            otpbar_core::ipc::RuntimeInfo::remove();
            app.exit(0);
        }
        other if other.starts_with(ACCOUNT_PREFIX) => {
            let query = other.trim_start_matches(ACCOUNT_PREFIX).to_string();
            copy_from_tray(&app, &query);
        }
        _ => {}
    }
}

fn copy_from_tray(app: &AppHandle, query: &str) {
    let state = app.state::<AppState>();
    state.touch();
    match state.code_for(query) {
        Ok((account, code, remaining)) => {
            let settings = state.settings();
            ui::copy_code(app, &code, settings.clipboard_clear_secs);
            let label = format!("{} · {}", account.issuer, account.name);
            ui::notify_code_used(app, &label, &code, remaining, true, &settings);
            ui::refresh(app);
        }
        Err(err) => ui::notify(app, "OTP 오류", &err.to_string()),
    }
}
