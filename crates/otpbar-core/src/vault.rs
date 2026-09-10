//! 계정 모델과 금고 조작.
//!
//! 금고는 잠금(암호문만 보유)과 해제(메모리에 평문 계정) 두 상태를 가진다.
//! 계정 목록·발급자 이름까지 암호문 안에 들어가므로, 잠긴 상태에서는 어떤 서비스를
//! 쓰는지조차 파일에서 알 수 없다.

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::crypto::{random_salt, KdfParams, MasterKey, SealedVault};
use crate::error::{Error, Result};
use crate::totp::{self, Algorithm};
use crate::util;

/// 계정 하나. `secret`은 금고가 열려 있을 때만 메모리에 존재하며 Drop 시 지워진다.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Account {
    #[zeroize(skip)]
    pub id: String,
    #[zeroize(skip)]
    pub issuer: String,
    #[zeroize(skip)]
    pub name: String,
    pub secret: String,
    #[zeroize(skip)]
    #[serde(default)]
    pub algorithm: Algorithm,
    #[zeroize(skip)]
    pub digits: u32,
    #[zeroize(skip)]
    pub period: u64,
    #[zeroize(skip)]
    #[serde(default)]
    pub added: String,
}

impl Account {
    pub fn label(&self) -> String {
        if self.issuer.is_empty() {
            self.name.clone()
        } else {
            format!("{} · {}", self.issuer, self.name)
        }
    }

    pub fn code(&self) -> Result<String> {
        totp::code(&self.secret, self.digits, self.period, self.algorithm)
    }

    pub fn remaining(&self) -> u64 {
        totp::remaining(self.period)
    }

    /// 비밀키를 뺀 표시용 정보.
    pub fn view(&self) -> AccountView {
        AccountView {
            id: self.id.clone(),
            issuer: self.issuer.clone(),
            name: self.name.clone(),
            algorithm: self.algorithm,
            digits: self.digits,
            period: self.period,
            added: self.added.clone(),
        }
    }
}

/// UI·CLI에 건네는 계정 정보. 비밀키가 없다.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountView {
    pub id: String,
    pub issuer: String,
    pub name: String,
    pub algorithm: Algorithm,
    pub digits: u32,
    pub period: u64,
    #[serde(default)]
    pub added: String,
}

/// 새 계정 등록에 쓰는 입력값(가져오기 결과 포함).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewAccount {
    pub issuer: String,
    pub name: String,
    pub secret: String,
    #[serde(default)]
    pub algorithm: Algorithm,
    #[serde(default = "default_digits")]
    pub digits: u32,
    #[serde(default = "default_period")]
    pub period: u64,
}

fn default_digits() -> u32 {
    6
}
fn default_period() -> u64 {
    30
}

impl NewAccount {
    /// 자릿수·주기·비밀키를 검사한다. 손상되거나 악의적인 QR을 여기서 막는다.
    pub fn validate(&self) -> Result<()> {
        totp::validate_params(self.digits, self.period)?;
        totp::decode_secret(&self.secret)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultData {
    #[serde(default)]
    pub accounts: Vec<Account>,
}

/// 금고. 잠금 해제 시에만 `data`가 채워진다.
///
/// 파일 경로를 값으로 들고 있으므로 시험에서 임시 폴더를 쓰기 쉽고, 프로세스 전역
/// 환경변수에 의존하지 않는다.
pub struct Vault {
    path: std::path::PathBuf,
    sealed: Option<SealedVault>,
    key: Option<MasterKey>,
    data: Option<VaultData>,
    kdf_params: KdfParams,
}

impl Default for Vault {
    fn default() -> Self {
        Self::new()
    }
}

impl Vault {
    /// 기본 위치(설정 폴더)의 빈 금고.
    pub fn new() -> Self {
        Self::new_at(util::vault_path())
    }

    pub fn new_at(path: impl Into<std::path::PathBuf>) -> Self {
        Vault {
            path: path.into(),
            sealed: None,
            key: None,
            data: None,
            kdf_params: KdfParams::default(),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 기본 위치의 금고 파일을 읽는다(잠긴 상태 그대로).
    pub fn load() -> Result<Self> {
        Self::load_from(util::vault_path())
    }

    /// 지정한 경로의 금고 파일을 읽는다. 파일이 없으면 빈 금고를 돌려준다.
    pub fn load_from(path: impl Into<std::path::PathBuf>) -> Result<Self> {
        let path = path.into();
        if !path.exists() {
            return Ok(Vault::new_at(path));
        }
        let raw = std::fs::read(&path)?;
        let sealed: SealedVault = serde_json::from_slice(&raw)
            .map_err(|e| Error::Corrupt(format!("금고 파일을 해석하지 못했습니다: {e}")))?;
        let kdf_params = sealed.kdf_params;
        Ok(Vault {
            path,
            sealed: Some(sealed),
            key: None,
            data: None,
            kdf_params,
        })
    }

    /// 시험·저사양 환경용으로 KDF 비용을 낮춘다. 실제 사용에서는 호출하지 않는다.
    #[doc(hidden)]
    pub fn set_kdf_params(&mut self, params: KdfParams) {
        self.kdf_params = params;
    }

    pub fn exists(&self) -> bool {
        self.sealed.is_some()
    }

    pub fn is_unlocked(&self) -> bool {
        self.data.is_some()
    }

    /// 마스터 암호 없이 쓰는 금고를 만든다.
    ///
    /// 무작위 키를 만들어 운영체제 자격증명 저장소에 넣고 그것으로 금고를 봉인한다.
    /// 사용자는 암호를 입력하지 않고, 앱은 시작할 때 조용히 연다. 파일 자체는 여전히
    /// 암호문이므로 금고만 복사해 가서는 열 수 없다.
    pub fn create_auto(&mut self) -> Result<()> {
        let passphrase = crate::crypto::random_token();
        // 무작위 키에는 무거운 KDF가 필요 없다. 앱 시작이 빨라진다.
        self.kdf_params = KdfParams::light();
        self.create(&passphrase)?;
        crate::autounlock::store(&passphrase)?;
        Ok(())
    }

    /// 자격증명 저장소에 키가 있으면 조용히 연다. 없으면 `Ok(false)`.
    pub fn unlock_auto(&mut self) -> Result<bool> {
        let Some(passphrase) = crate::autounlock::load()? else {
            return Ok(false);
        };
        self.unlock(&passphrase)?;
        Ok(true)
    }

    /// 자동 해제를 끄고 사용자 마스터 암호로 보호한다.
    pub fn enable_password_protection(&mut self, new_password: &str) -> Result<()> {
        let Some(current) = crate::autounlock::load()? else {
            return Err(Error::other("이미 마스터 암호로 보호되고 있습니다"));
        };
        // 사람이 정한 암호로 바뀌므로 키 유도 비용을 기본값(무겁게)으로 올린다.
        self.kdf_params = KdfParams::default();
        self.change_password(&current, new_password)?;
        crate::autounlock::clear()?;
        Ok(())
    }

    /// 마스터 암호 보호를 풀고 자동 해제로 되돌린다.
    pub fn disable_password_protection(&mut self, current_password: &str) -> Result<()> {
        let passphrase = crate::crypto::random_token();
        self.unlock(current_password)?;
        self.kdf_params = KdfParams::light();
        self.change_password(current_password, &passphrase)?;
        crate::autounlock::store(&passphrase)?;
        Ok(())
    }

    /// 자동 해제가 켜져 있는지.
    pub fn auto_unlock_enabled(&self) -> bool {
        crate::autounlock::is_enabled()
    }

    /// 새 금고를 만든다. 이미 있으면 거부한다.
    pub fn create(&mut self, password: &str) -> Result<()> {
        if self.sealed.is_some() {
            return Err(Error::VaultExists);
        }
        if password.chars().count() < 8 {
            return Err(Error::InvalidParams(
                "마스터 암호는 8자 이상이어야 합니다".into(),
            ));
        }
        let salt = random_salt();
        let key = MasterKey::derive(password, &salt, self.kdf_params)?;
        let data = VaultData::default();
        let plaintext = serde_json::to_vec(&data)?;
        let sealed = SealedVault::seal(&plaintext, &key, &salt, self.kdf_params)?;
        self.persist(&sealed)?;
        self.sealed = Some(sealed);
        self.key = Some(key);
        self.data = Some(data);
        Ok(())
    }

    pub fn unlock(&mut self, password: &str) -> Result<()> {
        let sealed = self.sealed.as_ref().ok_or(Error::NoVault)?;
        let salt = sealed.salt_bytes()?;
        let key = MasterKey::derive(password, &salt, sealed.kdf_params)?;
        let plaintext = sealed.open(&key)?;
        let data: VaultData = serde_json::from_slice(&plaintext)
            .map_err(|e| Error::Corrupt(format!("금고 내용을 해석하지 못했습니다: {e}")))?;
        self.kdf_params = sealed.kdf_params;
        self.key = Some(key);
        self.data = Some(data);
        Ok(())
    }

    /// 메모리에서 평문과 키를 지운다.
    pub fn lock(&mut self) {
        if let Some(mut data) = self.data.take() {
            for acc in data.accounts.iter_mut() {
                acc.secret.zeroize();
            }
        }
        self.key = None;
    }

    pub fn change_password(&mut self, current: &str, new_password: &str) -> Result<()> {
        self.unlock(current)?;
        if new_password.chars().count() < 8 {
            return Err(Error::InvalidParams(
                "마스터 암호는 8자 이상이어야 합니다".into(),
            ));
        }
        let salt = random_salt();
        let key = MasterKey::derive(new_password, &salt, self.kdf_params)?;
        let data = self.data.as_ref().ok_or(Error::Locked)?;
        let plaintext = serde_json::to_vec(data)?;
        let sealed = SealedVault::seal(&plaintext, &key, &salt, self.kdf_params)?;
        self.persist(&sealed)?;
        self.sealed = Some(sealed);
        self.key = Some(key);
        Ok(())
    }

    fn data(&self) -> Result<&VaultData> {
        self.data.as_ref().ok_or(Error::Locked)
    }

    /// 변경 내용을 다시 봉인해 디스크에 쓴다.
    fn save(&mut self) -> Result<()> {
        let key = self.key.as_ref().ok_or(Error::Locked)?;
        let data = self.data.as_ref().ok_or(Error::Locked)?;
        let sealed_old = self.sealed.as_ref().ok_or(Error::NoVault)?;
        let salt = sealed_old.salt_bytes()?;
        let plaintext = serde_json::to_vec(data)?;
        let sealed = SealedVault::seal(&plaintext, key, &salt, self.kdf_params)?;
        self.persist(&sealed)?;
        self.sealed = Some(sealed);
        Ok(())
    }

    fn persist(&self, sealed: &SealedVault) -> Result<()> {
        let json = serde_json::to_vec_pretty(sealed)?;
        util::write_private(&self.path, &json)
    }

    pub fn accounts(&self) -> Result<Vec<AccountView>> {
        Ok(self.data()?.accounts.iter().map(|a| a.view()).collect())
    }

    pub fn find(&self, query: &str) -> Result<&Account> {
        let data = self.data()?;
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Err(Error::AccountNotFound(query.into()));
        }
        if let Some(a) = data.accounts.iter().find(|a| a.id == q) {
            return Ok(a);
        }
        let hits: Vec<&Account> = data
            .accounts
            .iter()
            .filter(|a| {
                a.id.contains(&q)
                    || a.issuer.to_lowercase().contains(&q)
                    || a.name.to_lowercase().contains(&q)
            })
            .collect();
        match hits.len() {
            0 => Err(Error::AccountNotFound(query.into())),
            1 => Ok(hits[0]),
            _ => {
                let exact: Vec<&&Account> = hits
                    .iter()
                    .filter(|a| a.issuer.to_lowercase() == q)
                    .collect();
                if exact.len() == 1 {
                    Ok(exact[0])
                } else {
                    Err(Error::Ambiguous(
                        hits.iter()
                            .map(|a| a.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ))
                }
            }
        }
    }

    pub fn code_for(&self, query: &str) -> Result<(AccountView, String, u64)> {
        let acc = self.find(query)?;
        let code = acc.code()?;
        Ok((acc.view(), code, acc.remaining()))
    }

    /// 계정을 추가한다. 같은 발급자·이름이 있으면 `replace`가 참일 때만 덮어쓴다.
    pub fn add(&mut self, new: NewAccount, replace: bool) -> Result<String> {
        new.validate()?;
        let existing_ids: Vec<String> =
            self.data()?.accounts.iter().map(|a| a.id.clone()).collect();
        let dup = self
            .data()?
            .accounts
            .iter()
            .find(|a| a.issuer == new.issuer && a.name == new.name)
            .map(|a| a.id.clone());
        if dup.is_some() && !replace {
            return Err(Error::other(format!(
                "이미 있는 계정입니다: {} · {}",
                new.issuer, new.name
            )));
        }
        let id = dup
            .clone()
            .unwrap_or_else(|| unique_id(&existing_ids, &util::slugify(&[&new.issuer, &new.name])));
        let account = Account {
            id: id.clone(),
            issuer: new.issuer,
            name: new.name,
            secret: new.secret,
            algorithm: new.algorithm,
            digits: new.digits,
            period: new.period,
            added: util::now_rfc3339(),
        };
        let data = self.data.as_mut().ok_or(Error::Locked)?;
        match dup {
            Some(existing) => {
                if let Some(slot) = data.accounts.iter_mut().find(|a| a.id == existing) {
                    *slot = account;
                }
            }
            None => data.accounts.push(account),
        }
        self.save()?;
        Ok(id)
    }

    pub fn remove(&mut self, query: &str) -> Result<AccountView> {
        let view = self.find(query)?.view();
        let data = self.data.as_mut().ok_or(Error::Locked)?;
        if let Some(pos) = data.accounts.iter().position(|a| a.id == view.id) {
            let mut removed = data.accounts.remove(pos);
            removed.secret.zeroize();
        }
        self.save()?;
        Ok(view)
    }

    pub fn rename(
        &mut self,
        query: &str,
        issuer: Option<String>,
        name: Option<String>,
    ) -> Result<AccountView> {
        let id = self.find(query)?.id.clone();
        let data = self.data.as_mut().ok_or(Error::Locked)?;
        let acc = data
            .accounts
            .iter_mut()
            .find(|a| a.id == id)
            .ok_or(Error::AccountNotFound(id.clone()))?;
        if let Some(i) = issuer {
            acc.issuer = i;
        }
        if let Some(n) = name {
            acc.name = n;
        }
        let view = acc.view();
        self.save()?;
        Ok(view)
    }

    /// 여러 계정을 한 번에 넣는다(QR 가져오기). 결과 메시지를 계정마다 돌려준다.
    pub fn add_many(&mut self, items: Vec<NewAccount>, replace: bool) -> Result<Vec<String>> {
        let mut messages = Vec::new();
        for item in items {
            let label = format!("{} · {}", item.issuer, item.name);
            match self.add(item, replace) {
                Ok(id) => messages.push(format!("등록: {label} (id={id})")),
                Err(e) => messages.push(format!("건너뜀: {label} — {e}")),
            }
        }
        Ok(messages)
    }
}

impl Drop for Vault {
    fn drop(&mut self) {
        self.lock();
    }
}

fn unique_id(existing: &[String], base: &str) -> String {
    if !existing.iter().any(|e| e == base) {
        return base.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    format!("{base}-{}", util::now_unix())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::KdfParams;

    /// 시험용 임시 금고 경로. 프로세스 전역 상태를 건드리지 않으므로 병렬 실행이 안전하다.
    struct TempVault(std::path::PathBuf);

    impl TempVault {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "otpbar-vault-{tag}-{}-{:?}",
                util::now_unix(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            TempVault(dir)
        }

        fn path(&self) -> std::path::PathBuf {
            self.0.join("vault.json")
        }

        /// KDF 비용을 낮춘 금고(시험 속도용).
        fn vault(&self) -> Vault {
            let mut v = Vault::new_at(self.path());
            v.set_kdf_params(KdfParams {
                m_cost: 8,
                t_cost: 1,
                p_cost: 1,
            });
            v
        }

        fn reopen(&self) -> Vault {
            Vault::load_from(self.path()).unwrap()
        }
    }

    impl Drop for TempVault {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn sample() -> NewAccount {
        NewAccount {
            issuer: "authentik".into(),
            name: "bamin0422".into(),
            secret: "JBSWY3DPEHPK3PXP".into(),
            algorithm: Algorithm::Sha1,
            digits: 6,
            period: 30,
        }
    }

    #[test]
    fn create_add_lock_unlock() {
        let tmp = TempVault::new("basic");
        let mut v = tmp.vault();
        v.create("correct horse battery").unwrap();
        v.add(sample(), false).unwrap();
        assert_eq!(v.accounts().unwrap().len(), 1);

        // 잠그면 목록조차 볼 수 없다
        v.lock();
        assert!(matches!(v.accounts(), Err(Error::Locked)));

        // 디스크에서 다시 읽어 열기
        let mut v2 = tmp.reopen();
        assert!(v2.exists() && !v2.is_unlocked());
        assert!(matches!(
            v2.unlock("wrong password"),
            Err(Error::BadPassword)
        ));
        v2.unlock("correct horse battery").unwrap();
        let (view, code, _) = v2.code_for("authentik").unwrap();
        assert_eq!(view.issuer, "authentik");
        assert_eq!(code.len(), 6);
    }

    #[test]
    fn vault_file_has_no_plaintext() {
        let tmp = TempVault::new("noplain");
        let mut v = tmp.vault();
        v.create("correct horse battery").unwrap();
        v.add(sample(), false).unwrap();
        let raw = std::fs::read_to_string(tmp.path()).unwrap();
        assert!(
            !raw.contains("JBSWY3DPEHPK3PXP"),
            "비밀키가 파일에 남으면 안 된다"
        );
        assert!(
            !raw.contains("authentik"),
            "발급자 이름도 암호문 안에 있어야 한다"
        );
        assert!(!raw.contains("bamin0422"), "계정 이름도 드러나지 않는다");
    }

    #[test]
    fn rejects_bad_import() {
        let tmp = TempVault::new("badimport");
        let mut v = tmp.vault();
        v.create("correct horse battery").unwrap();
        let mut bad = sample();
        bad.period = 0;
        assert!(v.add(bad, false).is_err(), "주기 0은 저장되지 않는다");
        let mut bad2 = sample();
        bad2.digits = 20;
        assert!(v.add(bad2, false).is_err(), "자릿수 20은 저장되지 않는다");
        let mut bad3 = sample();
        bad3.secret = "not base32!".into();
        assert!(v.add(bad3, false).is_err());
        assert!(v.accounts().unwrap().is_empty());
    }

    #[test]
    fn duplicate_needs_replace() {
        let tmp = TempVault::new("dup");
        let mut v = tmp.vault();
        v.create("correct horse battery").unwrap();
        v.add(sample(), false).unwrap();
        assert!(v.add(sample(), false).is_err());
        v.add(sample(), true).unwrap();
        assert_eq!(v.accounts().unwrap().len(), 1);
    }

    #[test]
    fn find_is_forgiving_but_flags_ambiguity() {
        let tmp = TempVault::new("find");
        let mut v = tmp.vault();
        v.create("correct horse battery").unwrap();
        v.add(sample(), false).unwrap();
        let mut other = sample();
        other.name = "second".into();
        v.add(other, false).unwrap();
        assert!(matches!(v.find("authentik"), Err(Error::Ambiguous(_))));
        assert!(v.find("authentik-second").is_ok());
        assert!(matches!(v.find("nope"), Err(Error::AccountNotFound(_))));
    }

    #[test]
    fn change_password_reencrypts() {
        let tmp = TempVault::new("chpw");
        let mut v = tmp.vault();
        v.create("old password").unwrap();
        v.add(sample(), false).unwrap();
        v.change_password("old password", "new password!").unwrap();
        let mut v2 = tmp.reopen();
        assert!(matches!(v2.unlock("old password"), Err(Error::BadPassword)));
        v2.unlock("new password!").unwrap();
        assert_eq!(v2.accounts().unwrap().len(), 1);
    }

    #[test]
    fn short_password_rejected() {
        let tmp = TempVault::new("shortpw");
        let mut v = tmp.vault();
        assert!(v.create("1234567").is_err(), "8자 미만은 거부");
        v.create("12345678").unwrap();
    }

    #[test]
    fn remove_and_rename() {
        let tmp = TempVault::new("edit");
        let mut v = tmp.vault();
        v.create("correct horse battery").unwrap();
        v.add(sample(), false).unwrap();
        let renamed = v.rename("authentik", None, Some("새이름".into())).unwrap();
        assert_eq!(renamed.name, "새이름");
        v.remove("authentik").unwrap();
        assert!(v.accounts().unwrap().is_empty());
        assert!(tmp.reopen().exists(), "계정을 다 지워도 금고 파일은 남는다");
    }
}
