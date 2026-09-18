//! 시작 기록을 파일에 남긴다.
//!
//! Windows 릴리스 빌드는 `windows_subsystem = "windows"`로 묶여 있어 콘솔이 없다.
//! `eprintln!`으로 적은 내용은 어디에도 남지 않으므로, 앱이 어디서 막혔는지 알아낼
//! 방법이 없었다. 트레이 아이콘이 나타나지 않는다는 제보를 받고도 원인을 좁히지
//! 못한 것이 그래서다.
//!
//! 기록 대상은 시작 단계와 트레이 등록 결과로 한정한다. 계정 이름, 코드, 암호,
//! 금고 내용은 절대 적지 않는다.

use std::fmt::Arguments;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 이 크기를 넘으면 파일을 비우고 다시 쓴다. 진단용이므로 최근 기록만 있으면 된다.
const MAX_BYTES: u64 = 256 * 1024;

fn started_at() -> Instant {
    use std::sync::OnceLock;
    static T: OnceLock<Instant> = OnceLock::new();
    *T.get_or_init(Instant::now)
}

pub fn path() -> PathBuf {
    otpbar_core::util::config_dir().join("startup.log")
}

/// 한 줄 적는다. 실패해도 조용히 넘어간다 — 로그 때문에 앱이 막히면 본말이 전도된다.
pub fn write(args: Arguments<'_>) {
    let p = path();
    if let Some(dir) = p.parent() {
        otpbar_core::util::ensure_private_dir(dir).ok();
    }
    if std::fs::metadata(&p)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false)
    {
        std::fs::remove_file(&p).ok();
    }
    let Ok(mut f) = open_private(&p) else {
        return;
    };
    let elapsed = started_at().elapsed().as_secs_f64();
    writeln!(f, "[{} +{elapsed:6.3}s] {args}", stamp()).ok();
}

/// 앱을 켤 때마다 구분선을 넣어 이번 실행분을 찾기 쉽게 한다.
pub fn session_start(version: &str) {
    started_at();
    write(format_args!(
        "───── OTPBar {version} 시작 ({}) ─────",
        std::env::consts::OS
    ));
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}

/// 소유자만 읽을 수 있게 연다. 금고 파일과 같은 기준을 로그에도 적용한다.
///
/// 코드나 계정 이름은 적지 않지만, 어느 보호 모드를 쓰는지가 드러난다. 같은 기기의
/// 다른 사용자에게 공격 대상을 알려 줄 이유가 없다.
#[cfg(unix)]
fn open_private(p: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(p)
}

/// Windows에는 대응하는 모드 설정이 없다. 상위 폴더(`%APPDATA%`)의 ACL을 상속한다.
#[cfg(not(unix))]
fn open_private(p: &std::path::Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().create(true).append(true).open(p)
}

fn stamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let tod = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// 1970-01-01부터의 일수를 (년, 월, 일)로 바꾼다.
///
/// Howard Hinnant, "chrono-Compatible Low-Level Date Algorithms"의 `civil_from_days`.
/// 시간 표기 하나 때문에 날짜 크레이트를 더 들이지 않으려고 옮겨 적었다.
fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn 기준일과_윤년과_세기말을_바르게_바꾼다() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        // 2000-02-29: 400으로 나뉘는 해는 윤년이다
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        // 1900-02-28 다음은 3월 1일이다(100으로 나뉘지만 400으로는 안 나뉘므로 평년)
        assert_eq!(civil_from_days(-25_508), (1900, 3, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }
}
