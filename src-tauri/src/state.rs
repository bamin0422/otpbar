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
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    /// 앱 시작 시 호출한다. 금고가 없으면 자동 해제용으로 만들고, 있으면 조용히 연다.
    /// 마스터 암호 보호를 켠 사용자만 잠긴 상태로 남는다.
    pub fn ensure_ready(&self) -> Result<bool> {
        let mut inner = self.lock_inner();
        if inner.vault.is_unlocked() {
            return Ok(true);
        }
        if !inner.vault.exists() {
            inner.vault.create_auto()?;
            inner.last_activity = Instant::now();
            return Ok(true);
        }
        let opened = inner.vault.unlock_auto()?;
        if opened {
            inner.last_activity = Instant::now();
        }
        Ok(opened)
    }

    pub fn auto_unlock_enabled(&self) -> bool {
        self.lock_inner().vault.auto_unlock_enabled()
    }

    pub fn enable_password_protection(&self, new_password: &str) -> Result<()> {
        let mut inner = self.lock_inner();
        inner.vault.enable_password_protection(new_password)?;
        inner.last_activity = Instant::now();
        Ok(())
    }

    pub fn disable_password_protection(&self, current: &str) -> Result<()> {
        let mut inner = self.lock_inner();
        inner.vault.disable_password_protection(current)?;
        inner.last_activity = Instant::now();
        Ok(())
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

        // 2.1.1까지의 결함으로 경량 KDF에 봉인된 마스터 암호 금고를 여기서 올린다.
        // 암호를 아는 순간이 이때뿐이다. 자동 해제 금고는 경량이 정상이므로 건너뛴다.
        if !otpbar_core::autounlock::is_enabled() {
            match inner
                .vault
                .upgrade_password_kdf(password, otpbar_core::crypto::KdfParams::default())
            {
                Ok(true) => crate::log!("마스터 암호 금고의 KDF 비용을 기본값으로 올렸습니다."),
                Ok(false) => {}
                Err(e) => crate::log!("KDF 비용을 올리지 못했습니다: {e}"),
            }
        }
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
        self.lock_inner()
            .vault
            .accounts()
            .map(|a| a.len())
            .unwrap_or(0)
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
            out.push(ListItem {
                account,
                code,
                remaining,
            });
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

    pub fn rename(
        &self,
        query: &str,
        issuer: Option<String>,
        name: Option<String>,
    ) -> Result<AccountView> {
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

    /// 전역 단축키가 꺼낼 계정을 기억한다. 코드를 복사할 때마다 부른다.
    pub fn remember_last_used(&self, id: &str) {
        let mut s = self.settings();
        if s.last_used_id.as_deref() == Some(id) {
            return; // 같은 계정을 연달아 쓰면 파일을 다시 쓸 이유가 없다
        }
        s.last_used_id = Some(id.to_string());
        self.set_settings(s).ok();
    }

    /// 기억을 비운다. 계정이 사라진 경우에 부른다.
    pub fn forget_last_used(&self) {
        let mut s = self.settings();
        if s.last_used_id.is_none() {
            return;
        }
        s.last_used_id = None;
        self.set_settings(s).ok();
    }

    /// 마지막 조작 이후 지난 시간(초).
    pub fn idle_secs(&self) -> u64 {
        self.lock_inner().last_activity.elapsed().as_secs()
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
