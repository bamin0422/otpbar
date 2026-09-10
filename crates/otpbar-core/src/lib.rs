//! OTPBar 핵심 라이브러리.
//!
//! - [`crypto`] 마스터 암호 기반 금고 봉인(Argon2id + XChaCha20-Poly1305)
//! - [`vault`] 계정 모델과 금고 조작
//! - [`totp`] RFC 6238 코드 계산
//! - [`qr`] QR 이미지·otpauth URI·구글 내보내기 해석
//! - [`ipc`] 앱과 CLI 사이 프로토콜(비밀키를 전달하지 않는다)
//! - [`settings`] 사용자 설정

pub mod autounlock;
pub mod crypto;
pub mod error;
pub mod ipc;
pub mod qr;
pub mod settings;
pub mod totp;
pub mod transport;
pub mod util;
pub mod vault;

pub use error::{Error, Result};
pub use settings::Settings;
pub use totp::Algorithm;
pub use vault::{Account, AccountView, NewAccount, Vault};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
