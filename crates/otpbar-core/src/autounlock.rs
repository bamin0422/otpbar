//! 자동 잠금 해제.
//!
//! 기본 사용 방식은 "앱을 켜면 메뉴바에 코드가 바로 보이는 것"이다. 그래서 금고를 여는
//! 키를 운영체제 자격증명 저장소(macOS 키체인, Windows 자격증명 관리자, Linux
//! secret-service)에 넣어 두고 앱이 시작할 때 조용히 연다.
//!
//! 보안상 의미
//! - 금고 파일 자체는 여전히 암호문이다. 파일만 복사해 가서는 열 수 없다.
//! - 다만 이 기기에서 사용자 계정 권한을 가진 프로그램은 자격증명 저장소에 접근할 수
//!   있으므로, 자동 해제를 켜 두면 보호 수준은 "이 계정으로 로그인한 사람"까지다.
//! - 더 강하게 지키려면 설정에서 마스터 암호 보호를 켠다. 그러면 이 항목을 지우고
//!   앱을 열 때마다 암호를 묻는다.

use zeroize::Zeroizing;

use crate::error::{Error, Result};

const SERVICE: &str = "com.bamin0422.otpbar";
const ACCOUNT: &str = "vault-key";

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| Error::other(format!("자격증명 저장소를 열지 못했습니다: {e}")))
}

/// 자동 해제용 비밀 문자열을 저장한다.
pub fn store(secret: &str) -> Result<()> {
    entry()?
        .set_password(secret)
        .map_err(|e| Error::other(format!("자격증명 저장 실패: {e}")))
}

/// 저장된 비밀 문자열을 읽는다. 없으면 `Ok(None)`.
pub fn load() -> Result<Option<Zeroizing<String>>> {
    match entry()?.get_password() {
        Ok(secret) => Ok(Some(Zeroizing::new(secret))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(Error::other(format!("자격증명 읽기 실패: {e}"))),
    }
}

/// 저장된 항목을 지운다(마스터 암호 보호로 전환할 때).
pub fn clear() -> Result<()> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(Error::other(format!("자격증명 삭제 실패: {e}"))),
    }
}

pub fn is_enabled() -> bool {
    matches!(load(), Ok(Some(_)))
}
