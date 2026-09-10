//! 사용자 설정. 비밀 정보는 담지 않는다.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::util;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            auto_lock_secs: 0,
            mask_codes: true,
            show_code_in_notification: false,
            clipboard_clear_secs: 20,
            auto_update_check: true,
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
