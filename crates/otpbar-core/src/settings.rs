//! 사용자 설정. 비밀 정보는 담지 않는다.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util;

/// `#[serde(default)]`가 구조체 전체에 붙어 있어야 한다. 이것이 없으면 필드를 하나
/// 더할 때마다 옛 설정 파일의 파싱이 실패하고, [`Settings::load`]가
/// `unwrap_or_default()`로 **사용자 설정 전체를 초기화**한다.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 마지막 조작 후 이 시간(초)이 지나면 자동으로 잠근다. 0이면 잠그지 않는다.
    /// 기본은 0이다. 자동 해제를 쓰는 기본 구성에서는 잠가도 곧바로 다시 열리므로
    /// 의미가 없고, 마스터 암호 보호를 켠 사용자만 이 값을 올린다.
    pub auto_lock_secs: u64,
    /// 코드를 가린 채 표시하고, 누를 때만 보여 준다.
    pub mask_codes: bool,
    /// 알림 본문에 코드를 넣는다(기본 꺼짐 — 잠금 화면 미리보기 노출 방지).
    pub show_code_in_notification: bool,
    /// 복사 후 이 시간(초)이 지나면 클립보드를 비운다. 0이면 비우지 않는다.
    pub clipboard_clear_secs: u64,
    /// 업데이트를 자동으로 확인한다.
    pub auto_update_check: bool,
    /// 전역 단축키로 마지막에 쓴 계정의 코드를 복사한다.
    pub hotkey_enabled: bool,
    /// 전역 단축키 조합. Tauri 표기법을 쓴다(예: `CmdOrCtrl+Shift+O`).
    pub hotkey: String,
    /// 단축키가 꺼낼 계정. 코드를 복사할 때마다 갱신한다.
    pub last_used_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_lock_secs: 0,
            mask_codes: true,
            show_code_in_notification: false,
            clipboard_clear_secs: 20,
            auto_update_check: true,
            hotkey_enabled: true,
            hotkey: "CmdOrCtrl+Shift+O".into(),
            last_used_id: None,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        let path = util::settings_path();
        std::fs::read(&path)
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        util::write_private(&util::settings_path(), &serde_json::to_vec_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 필드를 더해도 옛 설정 파일의 값이 살아남아야 한다.
    /// `#[serde(default)]`가 빠지면 `load()`가 통째로 기본값으로 되돌린다.
    #[test]
    fn old_settings_survive_new_fields() {
        let raw = r#"{
            "auto_lock_secs": 300,
            "mask_codes": false,
            "show_code_in_notification": true,
            "clipboard_clear_secs": 45,
            "auto_update_check": false
        }"#;
        let s: Settings = serde_json::from_str(raw).expect("옛 형식도 읽혀야 한다");
        assert_eq!(s.auto_lock_secs, 300);
        assert!(!s.mask_codes, "사용자가 끈 값이 유지되어야 한다");
        assert!(s.show_code_in_notification);
        assert_eq!(s.clipboard_clear_secs, 45);
        assert!(!s.auto_update_check);
        // 새 필드는 기본값으로 채워진다
        assert!(s.hotkey_enabled);
        assert_eq!(s.hotkey, "CmdOrCtrl+Shift+O");
        assert_eq!(s.last_used_id, None);
    }

    #[test]
    fn defaults_are_conservative() {
        let s = Settings::default();
        assert!(s.mask_codes, "코드는 기본으로 가린다");
        assert_eq!(
            s.auto_lock_secs, 0,
            "기본 구성에서는 자동 잠금을 걸지 않는다"
        );
        assert!(!s.show_code_in_notification, "알림에 코드를 넣지 않는다");
        assert!(s.clipboard_clear_secs > 0, "클립보드를 비운다");
    }
}
