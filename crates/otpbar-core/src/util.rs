use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Result;

/// 설정·금고 파일이 놓이는 폴더.
/// macOS/Linux: `~/.config/otpbar`, Windows: `%APPDATA%\otpbar`
pub fn config_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("OTPBAR_CONFIG_DIR") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    #[cfg(windows)]
    {
        if let Some(dirs) = directories::BaseDirs::new() {
            return dirs.config_dir().join("otpbar");
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = directories::BaseDirs::new() {
            return home.home_dir().join(".config").join("otpbar");
        }
    }
    PathBuf::from(".otpbar")
}

pub fn vault_path() -> PathBuf {
    config_dir().join("vault.json")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

/// 실행 중 앱이 IPC 접속 정보를 적어 두는 파일(0600).
pub fn runtime_path() -> PathBuf {
    config_dir().join("runtime.json")
}

/// 폴더를 소유자 전용(0700)으로 만든다.
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(dir)?.permissions();
        perm.set_mode(0o700);
        std::fs::set_permissions(dir, perm)?;
    }
    Ok(())
}

/// 소유자만 읽을 수 있는 파일로 원자적으로 쓴다.
/// 임시 파일을 0600으로 만든 뒤 rename 하므로 중간에 다른 사용자가 읽을 틈이 없다.
pub fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        ensure_private_dir(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        #[cfg(unix)]
        let mut file = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)?
        };
        #[cfg(not(unix))]
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;

        use std::io::Write;
        file.write_all(data)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 외부 crate 없이 만드는 RFC 3339(UTC) 시각 문자열.
pub fn now_rfc3339() -> String {
    let secs = now_unix() as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Howard Hinnant의 days→civil 알고리즘.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 계정 id로 쓰는 슬러그. 영숫자와 한글만 남기므로 경로·인자 주입에 쓰일 수 없다.
pub fn slugify(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("-");
    let mut out = String::with_capacity(joined.len());
    let mut prev_dash = false;
    for ch in joined.chars() {
        let keep = ch.is_ascii_alphanumeric() || ('가'..='힣').contains(&ch);
        if keep {
            out.extend(ch.to_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "account".into()
    } else {
        trimmed.chars().take(64).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_safe() {
        assert_eq!(slugify(&["GitHub", "bamin0422"]), "github-bamin0422");
        assert_eq!(slugify(&["Jira", "me@company.com"]), "jira-me-company-com");
        assert_eq!(slugify(&["", ""]), "account");
        assert_eq!(
            slugify(&["--replace; rm -rf /", "$(whoami)"]),
            "replace-rm-rf-whoami"
        );
        assert_eq!(slugify(&["기념일", "축하"]), "기념일-축하");
    }

    #[test]
    fn rfc3339_shape() {
        let s = now_rfc3339();
        assert_eq!(s.len(), 20, "{s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
    }

    #[test]
    fn civil_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    }

    #[test]
    fn private_write_sets_mode() {
        let dir = std::env::temp_dir().join(format!("otpbar-test-{}", now_unix()));
        let path = dir.join("f.json");
        write_private(&path, b"{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let dmode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(dmode, 0o700);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
