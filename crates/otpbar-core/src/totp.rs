//! RFC 6238 TOTP 계산.
//!
//! 비밀키는 `Zeroizing`으로 감싸 사용 후 메모리에서 지운다. 자릿수·주기는 계산 전에
//! 범위를 검사하므로 손상된 계정 정보로 패닉이 나지 않는다.

use hmac::{Hmac, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha512};
use std::time::{SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// TOTP 해시 알고리즘. 문자열은 otpauth URI의 `algorithm` 값과 같다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Algorithm {
    Sha1,
    Sha256,
    Sha512,
}

impl Default for Algorithm {
    fn default() -> Self {
        Algorithm::Sha1
    }
}

impl Algorithm {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_uppercase().as_str() {
            "SHA1" | "" => Ok(Algorithm::Sha1),
            "SHA256" => Ok(Algorithm::Sha256),
            "SHA512" => Ok(Algorithm::Sha512),
            other => Err(Error::Unsupported(format!("지원하지 않는 알고리즘: {other}"))),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Algorithm::Sha1 => "SHA1",
            Algorithm::Sha256 => "SHA256",
            Algorithm::Sha512 => "SHA512",
        }
    }
}

pub const MIN_DIGITS: u32 = 4;
pub const MAX_DIGITS: u32 = 10;
pub const MAX_PERIOD: u64 = 600;

/// 자릿수(4~10)와 주기(1~600초)를 검사한다. 손상된 QR로 앱이 죽지 않게 하는 방어선이다.
pub fn validate_params(digits: u32, period: u64) -> Result<()> {
    if !(MIN_DIGITS..=MAX_DIGITS).contains(&digits) {
        return Err(Error::InvalidParams(format!(
            "자릿수는 {MIN_DIGITS}~{MAX_DIGITS} 범위여야 합니다 (받은 값: {digits})"
        )));
    }
    if period == 0 || period > MAX_PERIOD {
        return Err(Error::InvalidParams(format!(
            "주기는 1~{MAX_PERIOD}초 범위여야 합니다 (받은 값: {period})"
        )));
    }
    Ok(())
}

/// base32 비밀키를 바이트로 푼다. 공백·하이픈·패딩을 허용하고 대소문자를 가리지 않는다.
pub fn decode_secret(secret: &str) -> Result<Zeroizing<Vec<u8>>> {
    let cleaned: String = secret
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '=')
        .collect::<String>()
        .to_ascii_uppercase();
    if cleaned.is_empty() {
        return Err(Error::InvalidSecret("비밀키가 비어 있습니다".into()));
    }
    if !cleaned.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)) {
        return Err(Error::InvalidSecret("base32(A~Z, 2~7) 문자열이어야 합니다".into()));
    }
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &cleaned)
        .filter(|b| !b.is_empty())
        .map(Zeroizing::new)
        .ok_or_else(|| Error::InvalidSecret("base32 디코딩에 실패했습니다".into()))
}

fn hmac_digest(algorithm: Algorithm, key: &[u8], counter: u64) -> Vec<u8> {
    let msg = counter.to_be_bytes();
    match algorithm {
        Algorithm::Sha1 => {
            let mut m = <Hmac<Sha1> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(&msg);
            m.finalize().into_bytes().to_vec()
        }
        Algorithm::Sha256 => {
            let mut m = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(&msg);
            m.finalize().into_bytes().to_vec()
        }
        Algorithm::Sha512 => {
            let mut m = <Hmac<Sha512> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
            m.update(&msg);
            m.finalize().into_bytes().to_vec()
        }
    }
}

/// 주어진 유닉스 시각의 TOTP 코드를 만든다.
pub fn code_at(secret: &str, digits: u32, period: u64, algorithm: Algorithm, unix_time: u64) -> Result<String> {
    validate_params(digits, period)?;
    let key = decode_secret(secret)?;
    let counter = unix_time / period;
    let digest = hmac_digest(algorithm, &key, counter);
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    let modulus = 10u32.checked_pow(digits).ok_or_else(|| Error::InvalidParams("자릿수가 너무 큽니다".into()))?;
    Ok(format!("{:0width$}", bin % modulus, width = digits as usize))
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 현재 코드.
pub fn code(secret: &str, digits: u32, period: u64, algorithm: Algorithm) -> Result<String> {
    code_at(secret, digits, period, algorithm, now_unix())
}

/// 현재 코드가 몇 초 뒤에 바뀌는지.
pub fn remaining(period: u64) -> u64 {
    if period == 0 {
        return 0;
    }
    period - (now_unix() % period)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b32(raw: &str) -> String {
        base32::encode(base32::Alphabet::Rfc4648 { padding: false }, raw.as_bytes())
    }

    /// RFC 6238 부록 B 테스트 벡터.
    #[test]
    fn rfc6238_vectors() {
        let s1 = b32("12345678901234567890");
        let s256 = b32("12345678901234567890123456789012");
        let s512 = b32("1234567890123456789012345678901234567890123456789012345678901234");
        let cases = [
            (&s1, Algorithm::Sha1, 59u64, "94287082"),
            (&s1, Algorithm::Sha1, 1111111109, "07081804"),
            (&s1, Algorithm::Sha1, 1111111111, "14050471"),
            (&s1, Algorithm::Sha1, 1234567890, "89005924"),
            (&s1, Algorithm::Sha1, 2000000000, "69279037"),
            (&s1, Algorithm::Sha1, 20000000000, "65353130"),
            (&s256, Algorithm::Sha256, 59, "46119246"),
            (&s256, Algorithm::Sha256, 1111111109, "68084774"),
            (&s512, Algorithm::Sha512, 59, "90693936"),
            (&s512, Algorithm::Sha512, 1234567890, "93441116"),
        ];
        for (secret, algo, t, expected) in cases {
            assert_eq!(code_at(secret, 8, 30, algo, t).unwrap(), expected, "algo={algo:?} t={t}");
        }
    }

    #[test]
    fn rejects_bad_params() {
        let s = b32("12345678901234567890");
        assert!(code_at(&s, 6, 0, Algorithm::Sha1, 0).is_err(), "주기 0은 거부");
        assert!(code_at(&s, 20, 30, Algorithm::Sha1, 0).is_err(), "자릿수 20은 거부");
        assert!(code_at(&s, 3, 30, Algorithm::Sha1, 0).is_err(), "자릿수 3은 거부");
        assert!(code_at(&s, 6, 601, Algorithm::Sha1, 0).is_err(), "주기 601은 거부");
    }

    #[test]
    fn rejects_bad_secret() {
        assert!(decode_secret("").is_err());
        assert!(decode_secret("not-base32!!").is_err());
        assert!(decode_secret("0189").is_err(), "base32 알파벳에 없는 숫자");
        assert!(decode_secret("JBSWY3DPEHPK3PXP").is_ok());
        assert!(decode_secret("jbswy3dp ehpk-3pxp==").is_ok(), "공백·하이픈·패딩·소문자 허용");
    }
}
