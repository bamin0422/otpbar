//! 앱 상태: 금고, 설정, 마지막 활동 시각(자동 잠금용).

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otpbar_core::error::{Error, Result};
use otpbar_core::ipc::{ListItem, RuntimeInfo};
use otpbar_core::{AccountView, Settings, Vault};

pub struct Inner {
    pub vault: Vault,
    pub settings: Settings,
    /// 마지막으로 사용자가 조작한 시각. 자동 잠금 판단에 쓴다.
    pub last_activity: Instant,
    /// IPC 인증 토큰(런타임 파일과 같은 값).
    pub token: String,
}

#[derive(Clone)]
pub struct AppState(Arc<Mutex<Inner>>);

impl AppState {
    pub fn new(token: String) -> Result<Self> {
        let vault = Vault::load()?;
        let settings = Settings::load();
        Ok(AppState(Arc::new(Mutex::new(Inner {
            vault,
            settings,
            last_activity: Instant::now(),
            token,
        }))))
    }

    fn lock_inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        // 잠금이 오염되어도 앱을 죽이지 않는다. 금고는 자물쇠가 아니라 데이터이므로
        // 복구해서 계속 쓰는 편이 사용자에게 낫다.
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn token(&self) -> String {
        self.lock_inner().token.clone()
    }

    pub fn touch(&self) {
        self.lock_inner().last_activity = Instant::now();
    }

    pub fn settings(&self) -> Settings {
        self.lock_inner().settings.clone()
    }

    pub fn set_settings(&self, settings: Settings) -> Result<()> {
        settings.save()?;
        self.lock_inner().settings = settings;
        Ok(())
    }

    pub fn has_vault(&self) -> bool {
        self.lock_inner().vault.exists()
    }

    pub fn is_unlocked(&self) -> bool {
        self.lock_inner().vault.is_unlocked()
    }

    pub fn create_vault(&self, password: &str) -> Result<()> {
        let mut inner = self.lock_inner();
        inner.vault.create(password)?;
        inner.last_activity = Instant::now();
        Ok(())
    }

    pub fn unlock(&self, password: &str) -> Result<()> {
        let mut inner = self.lock_inner();
        inner.vault.unlock(password)?;
        inner.last_activity = Instant::now();
        Ok(())
    }

    pub fn lock(&self) {
        self.lock_inner().vault.lock();
    }

    pub fn change_password(&self, current: &str, new_password: &str) -> Result<()> {
        let mut inner = self.lock_inner();
        inner.vault.change_password(current, new_password)?;
        inner.last_activity = Instant::now();
        Ok(())
    }

    pub fn accounts(&self) -> Result<Vec<AccountView>> {
        self.lock_inner().vault.accounts()
    }

    pub fn account_count(&self) -> usize {
        self.lock_inner().vault.accounts().map(|a| a.len()).unwrap_or(0)
    }

    /// 목록과 코드를 함께 만든다(트레이·창 표시에 쓴다).
    pub fn list_items(&self, with_codes: bool) -> Result<Vec<ListItem>> {
        let inner = self.lock_inner();
        let accounts = inner.vault.accounts()?;
        let mut out = Vec::with_capacity(accounts.len());
        for account in accounts {
            let (code, remaining) = if with_codes {
                match inner.vault.code_for(&account.id) {
                    Ok((_, c, r)) => (Some(c), Some(r)),
                    Err(_) => (None, None),
                }
            } else {
                (None, None)
            };
            out.push(ListItem { account, code, remaining });
        }
        Ok(out)
    }

    pub fn code_for(&self, query: &str) -> Result<(AccountView, String, u64)> {
        let inner = self.lock_inner();
        inner.vault.code_for(query)
    }

    pub fn import(&self, source: &str, replace: bool) -> Result<(Vec<String>, usize)> {
        let accounts = otpbar_core::ipc::import_source_to_accounts(source)?;
        let total = accounts.len();
        let mut inner = self.lock_inner();
        let messages = inner.vault.add_many(accounts, replace)?;
        inner.last_activity = Instant::now();
        Ok((messages, total))
    }

    pub fn add_manual(&self, new: otpbar_core::NewAccount, replace: bool) -> Result<String> {
        let mut inner = self.lock_inner();
        let id = inner.vault.add(new, replace)?;
        inner.last_activity = Instant::now();
        Ok(id)
    }

    pub fn remove(&self, query: &str) -> Result<AccountView> {
        let mut inner = self.lock_inner();
        let view = inner.vault.remove(query)?;
        inner.last_activity = Instant::now();
        Ok(view)
    }

    pub fn rename(&self, query: &str, issuer: Option<String>, name: Option<String>) -> Result<AccountView> {
        let mut inner = self.lock_inner();
        let view = inner.vault.rename(query, issuer, name)?;
        inner.last_activity = Instant::now();
        Ok(view)
    }

    /// 자동 잠금 시간이 지났으면 잠그고 참을 돌려준다.
    pub fn auto_lock_if_idle(&self) -> bool {
        let mut inner = self.lock_inner();
        let timeout = inner.settings.auto_lock_secs;
        if timeout == 0 || !inner.vault.is_unlocked() {
            return false;
        }
        if inner.last_activity.elapsed() >= Duration::from_secs(timeout) {
            inner.vault.lock();
            return true;
        }
        false
    }

    pub fn require_unlocked(&self) -> Result<()> {
        if self.is_unlocked() {
            Ok(())
        } else {
            Err(Error::Locked)
        }
    }
}

/// 실행 정보 파일을 쓰고, 앱 종료 시 지우도록 안내한다.
pub fn publish_runtime(endpoint: &str, token: &str) -> Result<()> {
    RuntimeInfo {
        endpoint: endpoint.to_string(),
        token: token.to_string(),
        pid: std::process::id(),
        version: otpbar_core::VERSION.to_string(),
    }
    .write()
}
