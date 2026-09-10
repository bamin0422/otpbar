//! 금고 암호화: Argon2id로 마스터 암호에서 키를 만들고 XChaCha20-Poly1305로 봉인한다.
//!
//! 디스크에 남는 것은 암호문뿐이다. 같은 사용자 권한으로 도는 프로그램이 금고 파일을
//! 통째로 읽어도 마스터 암호 없이는 비밀키를 얻지 못한다. 이것이 예전 버전(Keychain에
//! 평문 비밀키 보관)과의 결정적 차이다.

use argon2::{Algorithm as ArgonAlg, Argon2, Params, Version};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::error::{Error, Result};

pub const KEY_LEN: usize = 32;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 24;

/// Argon2id 매개변수. OWASP 권장(메모리 64MiB, 3회 반복) 기준이며 금고 파일에 함께 저장해
/// 나중에 값을 올려도 기존 금고를 계속 열 수 있다.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KdfParams {
    /// 메모리 비용(KiB)
    pub m_cost: u32,
    /// 반복 횟수
    pub t_cost: u32,
    /// 병렬도
    pub p_cost: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        KdfParams {
            m_cost: 65_536,
            t_cost: 3,
            p_cost: 4,
        }
    }
}

impl KdfParams {
    fn build(&self) -> Result<Argon2<'static>> {
        let params = Params::new(self.m_cost, self.t_cost, self.p_cost, Some(KEY_LEN))
            .map_err(|e| Error::Corrupt(format!("KDF 매개변수 오류: {e}")))?;
        Ok(Argon2::new(ArgonAlg::Argon2id, Version::V0x13, params))
    }
}

/// 마스터 키. Drop 시 메모리에서 지운다.
#[derive(Clone)]
pub struct MasterKey(Zeroizing<[u8; KEY_LEN]>);

impl MasterKey {
    pub fn derive(password: &str, salt: &[u8], params: KdfParams) -> Result<Self> {
        if salt.len() < 8 {
            return Err(Error::Corrupt("salt가 너무 짧습니다".into()));
        }
        let mut key = [0u8; KEY_LEN];
        params
            .build()?
            .hash_password_into(password.as_bytes(), salt, &mut key)
            .map_err(|e| Error::other(format!("키 유도 실패: {e}")))?;
        let out = MasterKey(Zeroizing::new(key));
        key.zeroize();
        Ok(out)
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(self.0.as_slice().into())
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

/// 디스크에 저장되는 금고 파일 형식.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedVault {
    pub version: u32,
    pub kdf: String,
    pub kdf_params: KdfParams,
    pub salt: String,
    pub cipher: String,
    pub nonce: String,
    pub ciphertext: String,
    #[serde(default)]
    pub updated: String,
}

const AAD: &[u8] = b"otpbar-vault-v1";

impl SealedVault {
    /// 평문(JSON 바이트)을 새 nonce로 봉인한다.
    pub fn seal(
        plaintext: &[u8],
        key: &MasterKey,
        salt: &[u8],
        kdf_params: KdfParams,
    ) -> Result<Self> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);
        let ciphertext = key
            .cipher()
            .encrypt(
                nonce,
                Payload {
                    msg: plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| Error::other("암호화에 실패했습니다"))?;
        Ok(SealedVault {
            version: 1,
            kdf: "argon2id".into(),
            kdf_params,
            salt: B64.encode(salt),
            cipher: "xchacha20poly1305".into(),
            nonce: B64.encode(nonce_bytes),
            ciphertext: B64.encode(ciphertext),
            updated: crate::util::now_rfc3339(),
        })
    }

    pub fn salt_bytes(&self) -> Result<Vec<u8>> {
        B64.decode(&self.salt)
            .map_err(|e| Error::Corrupt(format!("salt 디코딩 실패: {e}")))
    }

    /// 마스터 키로 평문을 되돌린다. 인증 태그가 맞지 않으면 `BadPassword`.
    pub fn open(&self, key: &MasterKey) -> Result<Zeroizing<Vec<u8>>> {
        if self.version != 1 {
            return Err(Error::Corrupt(format!(
                "지원하지 않는 금고 버전: {}",
                self.version
            )));
        }
        if self.kdf != "argon2id" || self.cipher != "xchacha20poly1305" {
            return Err(Error::Corrupt("알 수 없는 암호 방식".into()));
        }
        let nonce_bytes = B64
            .decode(&self.nonce)
            .map_err(|e| Error::Corrupt(format!("nonce 디코딩 실패: {e}")))?;
        if nonce_bytes.len() != NONCE_LEN {
            return Err(Error::Corrupt("nonce 길이가 올바르지 않습니다".into()));
        }
        let ciphertext = B64
            .decode(&self.ciphertext)
            .map_err(|e| Error::Corrupt(format!("ciphertext 디코딩 실패: {e}")))?;
        key.cipher()
            .decrypt(
                XNonce::from_slice(&nonce_bytes),
                Payload {
                    msg: &ciphertext,
                    aad: AAD,
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| Error::BadPassword)
    }
}

pub fn random_salt() -> [u8; SALT_LEN] {
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

/// IPC 인증 토큰처럼 짧은 비밀 문자열을 만든다.
pub fn random_token() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let token = B64.encode(raw);
    raw.zeroize();
    token
}

/// 토큰 비교. 내용은 상수 시간으로 견준다.
///
/// 토큰 길이는 고정(base64 44자)이므로 길이가 다르면 곧바로 실패시켜도 비밀이 드러나지
/// 않는다. 길이가 같을 때만 바이트 비교가 이뤄지며, 이 비교는 조기 종료하지 않는다.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    use subtle::ConstantTimeEq;
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 시험에서는 KDF 비용을 낮춰 빠르게 돌린다(보안 기본값은 Default).
    fn fast() -> KdfParams {
        KdfParams {
            m_cost: 8,
            t_cost: 1,
            p_cost: 1,
        }
    }

    #[test]
    fn seal_open_roundtrip() {
        let salt = random_salt();
        let key = MasterKey::derive("hunter2", &salt, fast()).unwrap();
        let sealed = SealedVault::seal(b"{\"accounts\":[]}", &key, &salt, fast()).unwrap();
        let opened = sealed.open(&key).unwrap();
        assert_eq!(&opened[..], b"{\"accounts\":[]}");
    }

    #[test]
    fn wrong_password_fails() {
        let salt = random_salt();
        let key = MasterKey::derive("right", &salt, fast()).unwrap();
        let sealed = SealedVault::seal(b"secret", &key, &salt, fast()).unwrap();
        let bad = MasterKey::derive("wrong", &salt, fast()).unwrap();
        assert!(matches!(sealed.open(&bad), Err(Error::BadPassword)));
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let salt = random_salt();
        let key = MasterKey::derive("pw", &salt, fast()).unwrap();
        let mut sealed = SealedVault::seal(b"secret", &key, &salt, fast()).unwrap();
        let mut raw = B64.decode(&sealed.ciphertext).unwrap();
        raw[0] ^= 0xff;
        sealed.ciphertext = B64.encode(raw);
        assert!(
            matches!(sealed.open(&key), Err(Error::BadPassword)),
            "변조된 암호문은 열리지 않는다"
        );
    }

    #[test]
    fn nonce_differs_each_seal() {
        let salt = random_salt();
        let key = MasterKey::derive("pw", &salt, fast()).unwrap();
        let a = SealedVault::seal(b"same", &key, &salt, fast()).unwrap();
        let b = SealedVault::seal(b"same", &key, &salt, fast()).unwrap();
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn token_compare() {
        let t = random_token();
        assert!(constant_time_eq(&t, &t.clone()));
        assert!(!constant_time_eq(&t, "short"));
        assert!(!constant_time_eq(&t, &random_token()));
    }
}
