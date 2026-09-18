//! 메뉴바(macOS)·트레이(Windows) 메뉴. **이 앱의 유일한 화면이다.**
//!
//! 창을 만들지 않는다. 계정 목록, 코드 복사, 잠금과 잠금 해제, 가져오기, 설정, 업데이트가
//! 모두 이 메뉴 안에서 끝난다. 암호 입력처럼 글자를 받아야 할 때만 시스템 다이얼로그를
//! 잠깐 띄우고 곧 닫는다.

use std::sync::{Mutex, OnceLock};

use otpbar_core::ipc::ListItem;
use tauri::menu::{
    CheckMenuItemBuilder, Menu, MenuBuilder, MenuEvent, MenuItemBuilder, PredefinedMenuItem,
};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

use crate::state::AppState;
use crate::{prompt, ui};

pub const TRAY_ID: &str = "otpbar-tray";
/// 메뉴바·트레이용 단색 아이콘. 컴파일 시점에 박아 두어 런타임 경로 문제를 없앤다.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/tray-mono@2x.png");

const ID_DASHBOARD: &str = "dashboard";
const ID_IMPORT: &str = "import";
const ID_LOCK: &str = "lock";
const ID_UNLOCK: &str = "unlock";
const ID_MASK: &str = "mask";
const ID_NOTIFY_CODE: &str = "notify-code";
const ID_AUTOSTART: &str = "autostart";
const ID_PASSWORD: &str = "password";
const ID_CHANGE_PW: &str = "change-pw";
const ID_UPDATE: &str = "update";
const ID_OPEN_DIR: &str = "open-dir";
const ID_QUIT: &str = "quit";
const ACCOUNT_PREFIX: &str = "acct:";

/// 현재 메뉴에 올라간 계정 항목. 글자만 바꿔 갱신하므로 열린 메뉴가 닫히지 않는다.
struct CodeRow {
    id: String,
    label: String,
    item: tauri::menu::MenuItem<Wry>,
}

fn code_rows() -> &'static Mutex<Vec<CodeRow>> {
    static ROWS: OnceLock<Mutex<Vec<CodeRow>>> = OnceLock::new();
    ROWS.get_or_init(|| Mutex::new(Vec::new()))
}

fn row_text(label: &str, code: &str, remaining: u64, mask: bool) -> String {
    let shown = if mask {
        ui::masked(code)
    } else {
        ui::pretty(code)
    };
    format!("{label}    {shown}    {remaining}초")
}

/// 1초마다 불린다. 메뉴 구조는 건드리지 않고 계정 항목의 글자만 바꾼다.
pub fn tick(app: &AppHandle) {
    let state = app.state::<AppState>();
    if !state.is_unlocked() {
        return;
    }
    let mask = state.settings().mask_codes;
    let Ok(rows) = code_rows().lock() else { return };
    for row in rows.iter() {
        if let Ok((_, code, remaining)) = state.code_for(&row.id) {
            row.item
                .set_text(row_text(&row.label, &code, remaining, mask))
                .ok();
        }
    }
}

/// 트레이 아이콘을 만든다(앱 시작 시 1회).
///
/// **이 함수가 `Ok`를 돌려주어도 아이콘이 화면에 있다는 뜻은 아니다.** Windows에서
/// 실제 등록이 되었는지는 [`watch_registration`]이 따로 확인한다. 이유는 그쪽 주석에 적었다.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("OTPBar")
        .show_menu_on_left_click(true)
        .on_menu_event(on_menu_event);
    match tauri::image::Image::from_bytes(TRAY_ICON_PNG) {
        Ok(icon) => builder = builder.icon(icon),
        Err(e) => crate::log!("트레이 아이콘을 읽지 못했습니다: {e}"),
    }
    // macOS 메뉴바에서는 템플릿 이미지로 다크·라이트 모드에 맞춘다.
    #[cfg(target_os = "macos")]
    {
        builder = builder.icon_as_template(true);
    }
    // 여기서 메뉴를 다시 만들면 방금 열린 메뉴가 닫힌다. 코드 갱신은 tick()이 담당한다.
    builder.build(app)?;
    Ok(())
}

/// 트레이가 운영체제에 실제로 등록되었는지 확인하고, 아니면 다시 만든다(Windows 전용).
///
/// # 왜 필요한가
///
/// `tray-icon` 크레이트의 Windows 구현은 `Shell_NotifyIconW(NIM_ADD)`가 실패해도
/// 오류를 돌려주지 않는다. 창 핸들만 살려 두고 탐색기가 보내는 `TaskbarCreated`
/// 브로드캐스트를 기다린다(`platform_impl/windows/mod.rs`). 앱이 explorer.exe의
/// 작업 표시줄보다 먼저 뜨는 경우를 위한 처리인데, 그 브로드캐스트를 놓치면
/// 아이콘은 영영 나타나지 않는다. `build()`는 성공했으므로 앱 쪽은 아무것도 모른다.
///
/// 2.1.0에서 창을 없앤 뒤로는 이 상태가 곧 "앱을 쓸 방법이 없음"을 뜻한다.
/// 그래서 등록 여부를 직접 확인한다. 확인 수단은 `set_icon`이다. 이 호출은
/// `NIM_MODIFY`로 내려가고, 등록되지 않은 아이콘에 대해서는 실패한다.
#[cfg(windows)]
pub fn watch_registration(app: &AppHandle) {
    /// 재시도 횟수. 1·2·4·8·16·30초 간격으로 약 1분간 지켜본다.
    const MAX_ATTEMPTS: u32 = 6;

    let app = app.clone();
    std::thread::spawn(move || {
        let mut delay = 1u64;
        for attempt in 1..=MAX_ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_secs(delay));
            if is_registered(&app) {
                crate::log!("트레이 등록을 확인했습니다 (시도 {attempt}/{MAX_ATTEMPTS})");
                return;
            }
            crate::log!(
                "트레이가 등록되지 않았습니다 — 다시 만듭니다 (시도 {attempt}/{MAX_ATTEMPTS})"
            );
            recreate(&app);
            delay = (delay * 2).min(30);
        }
        crate::log!("트레이 등록에 끝내 실패했습니다. 대시보드를 대신 엽니다.");
        crate::dashboard::open(&app, crate::dashboard::Reason::TrayFailed);
    });
}

/// Windows가 아닌 곳에서는 할 일이 없다. 메뉴바·앱인디케이터는 등록이 조용히 실패하지 않는다.
#[cfg(not(windows))]
pub fn watch_registration(_app: &AppHandle) {}

/// 트레이 조작은 주 스레드에서만 안전하므로 건너가서 확인하고 결과만 받아 온다.
#[cfg(windows)]
fn is_registered(app: &AppHandle) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = app.clone();
    if app
        .run_on_main_thread(move || {
            let ok = match handle.tray_by_id(TRAY_ID) {
                Some(tray) => match tauri::image::Image::from_bytes(TRAY_ICON_PNG) {
                    // NIM_MODIFY가 통하면 등록되어 있다는 뜻이다.
                    Ok(icon) => tray.set_icon(Some(icon)).is_ok(),
                    // 아이콘을 못 읽는 것은 별개 문제다. 등록 실패로 오판하지 않는다.
                    Err(_) => true,
                },
                None => false,
            };
            tx.send(ok).ok();
        })
        .is_err()
    {
        return false;
    }
    rx.recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or(false)
}

#[cfg(windows)]
fn recreate(app: &AppHandle) {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        handle.remove_tray_by_id(TRAY_ID);
        if let Err(e) = create(&handle) {
            crate::log!("트레이를 다시 만들지 못했습니다: {e}");
        }
    })
    .ok();
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
    let password_protected = !state.auto_unlock_enabled();
    let mut builder = MenuBuilder::new(app);

    if !state.is_unlocked() {
        let hint = MenuItemBuilder::with_id("locked-hint", "잠겨 있습니다")
            .enabled(false)
            .build(app)?;
        let unlock = MenuItemBuilder::with_id(ID_UNLOCK, "잠금 해제…").build(app)?;
        builder = builder.item(&hint).item(&unlock);
    } else {
        let items: Vec<ListItem> = state.list_items(true).unwrap_or_default();
        let mut rows: Vec<CodeRow> = Vec::with_capacity(items.len());
        if items.is_empty() {
            let empty =
                MenuItemBuilder::with_id("empty", "계정이 없습니다 — 아래에서 가져오십시오")
                    .enabled(false)
                    .build(app)?;
            builder = builder.item(&empty);
        }
        for item in &items {
            let code = item.code.clone().unwrap_or_else(|| "------".into());
            let remaining = item.remaining.unwrap_or(0);
            let label = format!("{} · {}", item.account.issuer, item.account.name);
            let entry = MenuItemBuilder::with_id(
                format!("{ACCOUNT_PREFIX}{}", item.account.id),
                row_text(&label, &code, remaining, settings.mask_codes),
            )
            .build(app)?;
            builder = builder.item(&entry);
            rows.push(CodeRow {
                id: item.account.id.clone(),
                label,
                item: entry,
            });
        }
        let lock = MenuItemBuilder::with_id(ID_LOCK, "잠그기").build(app)?;
        builder = builder
            .item(&PredefinedMenuItem::separator(app)?)
            .item(&lock);
        if let Ok(mut slot) = code_rows().lock() {
            *slot = rows;
        }
    }
    if !state.is_unlocked() {
        if let Ok(mut slot) = code_rows().lock() {
            slot.clear();
        }
    }

    let dashboard = MenuItemBuilder::with_id(ID_DASHBOARD, "대시보드 열기").build(app)?;
    let import = MenuItemBuilder::with_id(ID_IMPORT, "QR 이미지에서 가져오기…").build(app)?;
    let mask = CheckMenuItemBuilder::with_id(ID_MASK, "코드 가려서 표시")
        .checked(settings.mask_codes)
        .build(app)?;
    let notify_code = CheckMenuItemBuilder::with_id(ID_NOTIFY_CODE, "알림에 코드 표시")
        .checked(settings.show_code_in_notification)
        .build(app)?;
    let autostart = CheckMenuItemBuilder::with_id(ID_AUTOSTART, "로그인할 때 실행")
        .checked(crate::autostart_enabled(app))
        .build(app)?;
    let password = CheckMenuItemBuilder::with_id(ID_PASSWORD, "마스터 암호로 잠그기")
        .checked(password_protected)
        .build(app)?;
    let open_dir = MenuItemBuilder::with_id(ID_OPEN_DIR, "저장 폴더 열기").build(app)?;
    let update = MenuItemBuilder::with_id(ID_UPDATE, "업데이트 확인…").build(app)?;
    let quit = MenuItemBuilder::with_id(ID_QUIT, "OTPBar 종료").build(app)?;

    builder = builder
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&dashboard)
        .item(&import)
        .item(&mask)
        .item(&notify_code)
        .item(&autostart)
        .item(&password);
    if password_protected {
        let change = MenuItemBuilder::with_id(ID_CHANGE_PW, "마스터 암호 바꾸기…").build(app)?;
        builder = builder.item(&change);
    }
    builder = builder
        .item(&open_dir)
        .item(&update)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&quit);
    builder.build()
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().0.clone();
    let app = app.clone();
    // 메뉴 콜백 안에서 다이얼로그를 띄우면 메뉴가 닫히지 않아 잠길 수 있어 별도 스레드로 넘긴다.
    std::thread::spawn(move || handle(&app, &id));
}

fn handle(app: &AppHandle, id: &str) {
    match id {
        ID_UNLOCK => unlock_flow(app),
        ID_LOCK => {
            app.state::<AppState>().lock();
            ui::refresh(app);
            ui::notify(app, "OTPBar", "잠갔습니다. 메뉴에서 다시 열 수 있습니다.");
        }
        ID_DASHBOARD => crate::dashboard::open(app, crate::dashboard::Reason::User),
        ID_IMPORT => crate::import_via_dialog(app),
        ID_MASK => {
            let state = app.state::<AppState>();
            let mut s = state.settings();
            s.mask_codes = !s.mask_codes;
            state.set_settings(s).ok();
            ui::refresh(app);
        }
        ID_NOTIFY_CODE => {
            let state = app.state::<AppState>();
            let mut s = state.settings();
            s.show_code_in_notification = !s.show_code_in_notification;
            state.set_settings(s).ok();
            ui::refresh(app);
        }
        ID_AUTOSTART => {
            crate::toggle_autostart(app);
            ui::refresh(app);
        }
        ID_PASSWORD => toggle_password_protection(app),
        ID_CHANGE_PW => change_password_flow(app),
        ID_OPEN_DIR => {
            let dir = otpbar_core::util::config_dir();
            std::fs::create_dir_all(&dir).ok();
            crate::open_path(app, &dir);
        }
        ID_UPDATE => crate::check_update_interactive(app),
        ID_QUIT => {
            otpbar_core::ipc::RuntimeInfo::remove();
            app.exit(0);
        }
        other if other.starts_with(ACCOUNT_PREFIX) => {
            copy_from_tray(app, other.trim_start_matches(ACCOUNT_PREFIX));
        }
        _ => {}
    }
}

/// 잠금 해제. 자동 해제가 켜져 있으면 그대로 열고, 암호 보호 중이면 암호를 묻는다.
pub(crate) fn unlock_flow(app: &AppHandle) {
    let state = app.state::<AppState>();
    match state.ensure_ready() {
        Ok(true) => {
            ui::refresh(app);
            ui::notify(app, "OTPBar", "잠금을 해제했습니다.");
            return;
        }
        Ok(false) => {}
        Err(e) => {
            ui::notify(app, "OTPBar", &e.to_string());
            return;
        }
    }
    // 암호 보호 중: 최대 세 번까지 묻는다
    for attempt in 1..=3 {
        let title = if attempt == 1 {
            "마스터 암호를 입력하십시오".to_string()
        } else {
            format!("암호가 맞지 않습니다 ({attempt}/3)")
        };
        let Some(password) = prompt::password(&title) else {
            return;
        };
        match state.unlock(&password) {
            Ok(()) => {
                ui::refresh(app);
                ui::notify(app, "OTPBar", "잠금을 해제했습니다.");
                return;
            }
            Err(e) if attempt == 3 => {
                ui::notify(app, "OTPBar", &format!("잠금 해제 실패: {e}"));
                return;
            }
            Err(_) => continue,
        }
    }
}

/// 마스터 암호 보호를 켜고 끈다.
fn toggle_password_protection(app: &AppHandle) {
    let state = app.state::<AppState>();
    let protected = !state.auto_unlock_enabled();
    if protected {
        // 끄기: 현재 암호를 확인한 뒤 자동 해제로 되돌린다
        let Some(current) = prompt::password("현재 마스터 암호를 입력하십시오")
        else {
            ui::refresh(app);
            return;
        };
        match state.disable_password_protection(&current) {
            Ok(()) => ui::notify(app, "OTPBar", "암호 보호를 껐습니다. 이제 바로 열립니다."),
            Err(e) => ui::notify(app, "OTPBar", &format!("바꾸지 못했습니다: {e}")),
        }
    } else {
        // 켜기: 새 암호를 두 번 받는다
        let Some(new) = prompt::password("새 마스터 암호 (8자 이상)") else {
            ui::refresh(app);
            return;
        };
        let Some(again) = prompt::password("한 번 더 입력하십시오") else {
            ui::refresh(app);
            return;
        };
        if new != again {
            ui::notify(app, "OTPBar", "두 암호가 다릅니다. 다시 시도하십시오.");
        } else {
            match state.enable_password_protection(&new) {
                Ok(()) => ui::notify(
                    app,
                    "OTPBar",
                    "이제 잠금을 풀 때 마스터 암호를 묻습니다. 암호를 잊으면 복구할 수 없습니다.",
                ),
                Err(e) => ui::notify(app, "OTPBar", &format!("바꾸지 못했습니다: {e}")),
            }
        }
    }
    ui::refresh(app);
}

fn change_password_flow(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(current) = prompt::password("현재 마스터 암호") else {
        return;
    };
    let Some(new) = prompt::password("새 마스터 암호 (8자 이상)") else {
        return;
    };
    let Some(again) = prompt::password("한 번 더 입력하십시오") else {
        return;
    };
    if new != again {
        ui::notify(app, "OTPBar", "두 암호가 다릅니다.");
        return;
    }
    match state.change_password(&current, &new) {
        Ok(()) => ui::notify(app, "OTPBar", "마스터 암호를 바꿨습니다."),
        Err(e) => ui::notify(app, "OTPBar", &format!("바꾸지 못했습니다: {e}")),
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
