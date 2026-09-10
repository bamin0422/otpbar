//! 앱(금고 보유자)과 CLI 사이의 로컬 프로토콜.
//!
//! 설계 의도: **CLI에는 비밀키를 절대 주지 않는다.** CLI는 "이 계정의 코드를 달라"고
//! 요청하고 앱이 계산한 6자리만 받는다. 따라서 CLI 프로세스를 들여다봐도 비밀키가 없다.
//!
//! 전송 계층은 유닉스 도메인 소켓(파일 권한 0600)과 Windows 명명 파이프다. 여기에 더해
//! 요청마다 런타임 파일(0600)에 적힌 토큰을 검사하므로, 소켓에 접근하더라도 토큰 없이는
//! 아무 것도 얻지 못한다.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::util;
use crate::vault::{AccountView, NewAccount};

/// 실행 중인 앱이 남기는 접속 정보.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeInfo {
    /// 유닉스 소켓 경로 또는 Windows 파이프 이름
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
    pub version: String,
}

impl RuntimeInfo {
    pub fn write(&self) -> Result<()> {
        util::write_private(&util::runtime_path(), &serde_json::to_vec_pretty(self)?)
    }

    pub fn read() -> Result<Self> {
        let path = util::runtime_path();
        if !path.exists() {
            return Err(Error::other(
                "실행 중인 OTPBar 앱을 찾지 못했습니다. 앱을 먼저 실행하십시오.",
            ));
        }
        let raw = std::fs::read(&path)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub fn remove() {
        std::fs::remove_file(util::runtime_path()).ok();
    }
}

/// 기본 엔드포인트 이름. 유닉스는 소켓 파일 경로, Windows는 자리표시자(실제 포트는 bind 후 결정).
///
/// 유닉스 소켓 경로에는 약 100바이트 제한(`SUN_LEN`)이 있다. 설정 폴더가 깊으면 제한을
/// 넘기므로, 짧은 사용자 전용 런타임 폴더를 차례로 시도한다.
pub fn default_endpoint() -> String {
    #[cfg(windows)]
    {
        String::new() // bind 시 루프백 임의 포트를 잡고 runtime.json에 실제 주소를 적는다
    }
    #[cfg(not(windows))]
    {
        const MAX_SUN: usize = 100;
        let uid = unsafe { libc::getuid() };
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        candidates.push(util::config_dir().join("agent.sock"));
        // 리눅스: 사용자 전용 런타임 디렉터리
        if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
            if !dir.is_empty() {
                candidates.push(std::path::PathBuf::from(dir).join("otpbar.sock"));
            }
        }
        // macOS: TMPDIR 은 사용자 전용(/var/folders/../T/)
        if let Ok(dir) = std::env::var("TMPDIR") {
            if !dir.is_empty() {
                candidates.push(std::path::PathBuf::from(dir).join(format!("otpbar-{uid}.sock")));
            }
        }
        candidates.push(std::path::PathBuf::from(format!("/tmp/otpbar-{uid}.sock")));
        for c in &candidates {
            if c.to_string_lossy().len() < MAX_SUN {
                return c.to_string_lossy().to_string();
            }
        }
        format!("/tmp/otpbar-{uid}.sock")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// 앱 상태(잠김 여부, 버전)
    Status,
    /// 계정 목록(비밀키 제외). `with_codes`면 코드도 함께 계산해 돌려준다.
    List {
        #[serde(default)]
        with_codes: bool,
    },
    /// 계정 하나의 코드. `copy`면 앱이 클립보드에 넣고 만료 후 지운다.
    Code {
        query: String,
        #[serde(default)]
        purpose: String,
        #[serde(default)]
        copy: bool,
        /// 만료 직전이면 다음 코드를 기다린다
        #[serde(default = "default_true")]
        wait: bool,
    },
    /// QR 이미지나 URI에서 계정 추가(앱이 처리하므로 CLI는 비밀키를 보지 않는다)
    Import {
        source: String,
        #[serde(default)]
        replace: bool,
    },
    /// 금고를 만든다(첫 실행). 앱이 처리하므로 상태가 어긋나지 않는다.
    Init { password: String },
    /// 금고를 연다. 암호는 사용자가 터미널에 직접 입력한 것만 전달된다.
    Unlock { password: String },
    /// 앱 창을 앞으로 가져온다(잠금 해제 유도)
    Show,
    /// 즉시 잠근다
    Lock,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Status {
        unlocked: bool,
        version: String,
        account_count: usize,
        has_vault: bool,
    },
    List {
        accounts: Vec<ListItem>,
    },
    Code {
        account: AccountView,
        code: String,
        remaining: u64,
        copied: bool,
    },
    Imported {
        messages: Vec<String>,
        added: usize,
    },
    Ok,
    Error {
        message: String,
        locked: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListItem {
    #[serde(flatten)]
    pub account: AccountView,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<u64>,
}

/// 실제로 오가는 한 줄 JSON. 토큰이 매 요청에 포함된다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub token: String,
    pub request: Request,
}

/// 가져오기 결과를 앱 쪽에서 만들 때 쓰는 도우미.
pub fn import_source_to_accounts(source: &str) -> Result<Vec<NewAccount>> {
    let trimmed = source.trim();
    if trimmed.starts_with("otpauth://") || trimmed.starts_with("otpauth-migration://") {
        crate::qr::parse_payload(trimmed)
    } else {
        let path = std::path::PathBuf::from(shellexpand_tilde(trimmed));
        let payloads = crate::qr::decode_image(&path)?;
        crate::qr::parse_payloads(&payloads)
    }
}

fn shellexpand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(dirs) = directories::BaseDirs::new() {
            return dirs.home_dir().join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_roundtrip() {
        let env = Envelope {
            token: "t".into(),
            request: Request::Code {
                query: "authentik".into(),
                purpose: "로그인".into(),
                copy: true,
                wait: true,
            },
        };
        let line = serde_json::to_string(&env).unwrap();
        assert!(line.contains("\"op\":\"code\""));
        let back: Envelope = serde_json::from_str(&line).unwrap();
        matches!(back.request, Request::Code { .. });
    }

    #[test]
    fn response_error_shape() {
        let r = Response::Error {
            message: "잠김".into(),
            locked: true,
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"status\":\"error\""));
        assert!(s.contains("\"locked\":true"));
    }

    #[test]
    fn endpoint_is_stable() {
        let a = default_endpoint();
        assert_eq!(a, default_endpoint(), "같은 환경에서는 같은 주소를 낸다");
        #[cfg(unix)]
        {
            assert!(!a.is_empty());
            assert!(
                a.len() < 100,
                "유닉스 소켓 경로 길이 제한(SUN_LEN) 안에 들어야 한다: {a}"
            );
            assert!(a.ends_with(".sock"));
        }
    }
}
