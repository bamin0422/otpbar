//! QR 이미지와 otpauth URI 해석.
//!
//! 입력은 전부 신뢰할 수 없는 데이터로 다룬다. 길이 제한과 범위 검사를 거치며,
//! 깨진 protobuf를 만나도 패닉 대신 오류나 빈 결과를 돌려준다.

use percent_encoding::percent_decode_str;

use crate::error::{Error, Result};
use crate::totp::Algorithm;
use crate::vault::NewAccount;

/// 이미지 한 장에서 읽어 들일 QR 개수 상한(자원 소모 방지).
const MAX_QR_PER_IMAGE: usize = 16;
/// 내보내기 QR 하나에 담을 수 있는 계정 수 상한.
const MAX_ACCOUNTS_PER_PAYLOAD: usize = 256;
const MAX_LABEL_LEN: usize = 120;

/// 이미지 파일에서 QR 문자열을 모두 읽는다.
pub fn decode_image(path: &std::path::Path) -> Result<Vec<String>> {
    let img = image::open(path).map_err(|e| Error::Qr(format!("이미지를 열지 못했습니다: {e}")))?;
    let luma = img.to_luma8();
    let mut prepared = rqrr::PreparedImage::prepare(luma);
    let grids = prepared.detect_grids();
    let mut out = Vec::new();
    for grid in grids.into_iter().take(MAX_QR_PER_IMAGE) {
        match grid.decode() {
            Ok((_meta, content)) => out.push(content),
            Err(_) => continue, // 손상된 코드 하나 때문에 전체를 포기하지 않는다
        }
    }
    if out.is_empty() {
        return Err(Error::Qr("QR 코드를 찾지 못했습니다. 이미지가 선명한지 확인하십시오".into()));
    }
    Ok(out)
}

/// QR 문자열(또는 사용자가 직접 붙여넣은 URI)에서 계정을 뽑는다.
pub fn parse_payload(payload: &str) -> Result<Vec<NewAccount>> {
    let trimmed = payload.trim();
    if trimmed.starts_with("otpauth-migration://") {
        parse_migration(trimmed)
    } else if trimmed.starts_with("otpauth://") {
        Ok(vec![parse_otpauth(trimmed)?])
    } else {
        Err(Error::Qr("otpauth 형식이 아닙니다".into()))
    }
}

/// 여러 QR 문자열을 한꺼번에 처리한다. 해석되지 않는 것은 건너뛴다.
pub fn parse_payloads(payloads: &[String]) -> Result<Vec<NewAccount>> {
    let mut all = Vec::new();
    let mut last_err = None;
    for p in payloads {
        match parse_payload(p) {
            Ok(mut items) => all.append(&mut items),
            Err(e) => last_err = Some(e),
        }
    }
    if all.is_empty() {
        return Err(last_err.unwrap_or_else(|| Error::Qr("가져올 계정이 없습니다".into())));
    }
    Ok(all)
}

/// `otpauth://totp/발급자:이름?secret=...&issuer=...` 해석.
pub fn parse_otpauth(uri: &str) -> Result<NewAccount> {
    let parsed = url::Url::parse(uri).map_err(|e| Error::Qr(format!("URI 해석 실패: {e}")))?;
    if parsed.scheme() != "otpauth" {
        return Err(Error::Qr("otpauth:// URI가 아닙니다".into()));
    }
    let kind = parsed.host_str().unwrap_or("totp").to_ascii_lowercase();
    if kind != "totp" {
        return Err(Error::Unsupported(format!("{kind}는 지원하지 않습니다 (TOTP만 가능)")));
    }
    let label = percent_decode_str(parsed.path().trim_start_matches('/'))
        .decode_utf8_lossy()
        .to_string();
    let (mut issuer, mut name) = match label.split_once(':') {
        Some((i, n)) => (i.trim().to_string(), n.trim().to_string()),
        None => (String::new(), label.trim().to_string()),
    };

    let mut secret = None;
    let mut algorithm = Algorithm::Sha1;
    let mut digits = 6u32;
    let mut period = 30u64;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "secret" => secret = Some(v.to_string()),
            "issuer" => {
                if !v.trim().is_empty() {
                    issuer = v.trim().to_string();
                }
            }
            "algorithm" => algorithm = Algorithm::parse(&v)?,
            "digits" => {
                digits = v.parse().map_err(|_| Error::Qr(format!("digits 값이 숫자가 아닙니다: {v}")))?
            }
            "period" => {
                period = v.parse().map_err(|_| Error::Qr(format!("period 값이 숫자가 아닙니다: {v}")))?
            }
            _ => {}
        }
    }
    let secret = secret.ok_or_else(|| Error::Qr("URI에 secret 파라미터가 없습니다".into()))?;

    // 라벨이 "발급자:발급자:이름" 처럼 중복될 때 정리
    if !issuer.is_empty() {
        let prefix = format!("{}:", issuer.to_lowercase());
        if name.to_lowercase().starts_with(&prefix) {
            name = name[prefix.len()..].trim().to_string();
        }
    }
    truncate(&mut issuer, MAX_LABEL_LEN);
    truncate(&mut name, MAX_LABEL_LEN);

    let acc = NewAccount { issuer, name, secret, algorithm, digits, period };
    acc.validate()?;
    Ok(acc)
}

fn truncate(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max).collect();
    }
}

// ---------- Google Authenticator 내보내기(protobuf) ----------

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn done(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn varint(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift = 0;
        loop {
            if self.pos >= self.buf.len() {
                return Err(Error::Qr("내보내기 데이터가 중간에 끊겼습니다".into()));
            }
            if shift > 63 {
                return Err(Error::Qr("내보내기 데이터의 정수가 너무 깁니다".into()));
            }
            let byte = self.buf[self.pos];
            self.pos += 1;
            result |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).ok_or_else(|| Error::Qr("길이 값이 올바르지 않습니다".into()))?;
        if end > self.buf.len() {
            return Err(Error::Qr("내보내기 데이터가 중간에 끊겼습니다".into()));
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    /// (필드 번호, 값) 하나를 읽는다. 값은 varint 또는 바이트열.
    fn field(&mut self) -> Result<(u64, Field<'a>)> {
        let key = self.varint()?;
        let field_no = key >> 3;
        match key & 7 {
            0 => Ok((field_no, Field::Varint(self.varint()?))),
            1 => Ok((field_no, Field::Bytes(self.bytes(8)?))),
            2 => {
                let len = self.varint()? as usize;
                Ok((field_no, Field::Bytes(self.bytes(len)?)))
            }
            5 => Ok((field_no, Field::Bytes(self.bytes(4)?))),
            other => Err(Error::Qr(format!("지원하지 않는 protobuf wire type {other}"))),
        }
    }
}

enum Field<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

/// `otpauth-migration://offline?data=<base64 protobuf>` 해석.
pub fn parse_migration(uri: &str) -> Result<Vec<NewAccount>> {
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    let parsed = url::Url::parse(uri).map_err(|e| Error::Qr(format!("URI 해석 실패: {e}")))?;
    let data = parsed
        .query_pairs()
        .find(|(k, _)| k == "data")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| Error::Qr("내보내기 URI에 data 파라미터가 없습니다".into()))?;
    // URL 인코딩 과정에서 '+'가 공백이 되는 경우가 흔하다
    let normalized = data.replace(' ', "+");
    let payload = B64
        .decode(normalized.as_bytes())
        .map_err(|e| Error::Qr(format!("내보내기 데이터 디코딩 실패: {e}")))?;

    let mut out = Vec::new();
    let mut reader = Reader::new(&payload);
    while !reader.done() {
        let (field_no, value) = reader.field()?;
        if field_no != 1 {
            continue; // 버전·배치 정보 등은 건너뛴다
        }
        let Field::Bytes(param) = value else { continue };
        if out.len() >= MAX_ACCOUNTS_PER_PAYLOAD {
            break;
        }
        if let Some(acc) = parse_migration_param(param)? {
            out.push(acc);
        }
    }
    if out.is_empty() {
        return Err(Error::Qr("내보내기 QR에서 계정을 찾지 못했습니다".into()));
    }
    Ok(out)
}

fn parse_migration_param(buf: &[u8]) -> Result<Option<NewAccount>> {


    let mut secret_raw: Option<Vec<u8>> = None;
    let mut name = String::new();
    let mut issuer = String::new();
    let mut algorithm = Algorithm::Sha1;
    let mut digits = 6u32;
    let mut is_totp = true;

    let mut reader = Reader::new(buf);
    while !reader.done() {
        let (field_no, value) = reader.field()?;
        match (field_no, value) {
            (1, Field::Bytes(b)) => secret_raw = Some(b.to_vec()),
            (2, Field::Bytes(b)) => name = String::from_utf8_lossy(b).to_string(),
            (3, Field::Bytes(b)) => issuer = String::from_utf8_lossy(b).to_string(),
            (4, Field::Varint(v)) => {
                algorithm = match v {
                    0 | 1 => Algorithm::Sha1,
                    2 => Algorithm::Sha256,
                    3 => Algorithm::Sha512,
                    _ => return Ok(None), // MD5 등은 건너뛴다
                }
            }
            (5, Field::Varint(v)) => {
                digits = match v {
                    0 | 1 => 6,
                    2 => 8,
                    _ => return Ok(None),
                }
            }
            (6, Field::Varint(v)) => is_totp = v != 1, // 1 = HOTP
            _ => {}
        }
    }

    if !is_totp {
        return Ok(None);
    }
    let Some(raw) = secret_raw else { return Ok(None) };
    if raw.is_empty() {
        return Ok(None);
    }
    let secret = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &raw);

    // "발급자:이름" 형태 정리
    if issuer.is_empty() {
        if let Some((i, n)) = name.split_once(':') {
            issuer = i.trim().to_string();
            name = n.trim().to_string();
        }
    } else {
        let prefix = format!("{}:", issuer.to_lowercase());
        if name.to_lowercase().starts_with(&prefix) {
            name = name[prefix.len()..].trim().to_string();
        }
    }
    truncate(&mut issuer, MAX_LABEL_LEN);
    truncate(&mut name, MAX_LABEL_LEN);

    let acc = NewAccount { issuer, name, secret, algorithm, digits, period: 30 };
    match acc.validate() {
        Ok(()) => Ok(Some(acc)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    fn varint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n == 0 {
                out.push(b);
                return out;
            }
            out.push(b | 0x80);
        }
    }

    fn field_bytes(no: u64, v: &[u8]) -> Vec<u8> {
        let mut out = varint((no << 3) | 2);
        out.extend(varint(v.len() as u64));
        out.extend_from_slice(v);
        out
    }

    fn field_varint(no: u64, v: u64) -> Vec<u8> {
        let mut out = varint(no << 3);
        out.extend(varint(v));
        out
    }

    fn migration_uri(entries: &[(&str, &str, &str)]) -> String {
        let mut payload = Vec::new();
        for (secret_b32, name, issuer) in entries {
            let raw = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, secret_b32).unwrap();
            let mut param = field_bytes(1, &raw);
            param.extend(field_bytes(2, name.as_bytes()));
            param.extend(field_bytes(3, issuer.as_bytes()));
            param.extend(field_varint(4, 1));
            param.extend(field_varint(5, 1));
            param.extend(field_varint(6, 2));
            payload.extend(field_bytes(1, &param));
        }
        payload.extend(field_varint(2, 1));
        format!("otpauth-migration://offline?data={}", urlencode(&B64.encode(&payload)))
    }

    fn urlencode(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
                other => format!("%{:02X}", other as u32),
            })
            .collect()
    }

    #[test]
    fn otpauth_basic() {
        let acc = parse_otpauth(
            "otpauth://totp/authentik:bamin0422?secret=JBSWY3DPEHPK3PXP&issuer=authentik&digits=6&period=30",
        )
        .unwrap();
        assert_eq!(acc.issuer, "authentik");
        assert_eq!(acc.name, "bamin0422");
        assert_eq!(acc.secret, "JBSWY3DPEHPK3PXP");
    }

    #[test]
    fn otpauth_percent_encoded_label() {
        let acc = parse_otpauth("otpauth://totp/Jira:user%40example.com?secret=JBSWY3DPEHPK3PXP&issuer=Jira").unwrap();
        assert_eq!(acc.name, "user@example.com");
    }

    #[test]
    fn otpauth_rejects_bad_input() {
        assert!(parse_otpauth("otpauth://totp/X:y?digits=6").is_err(), "secret 없음");
        assert!(parse_otpauth("otpauth://totp/X:y?secret=JBSWY3DPEHPK3PXP&digits=abc").is_err(), "digits 비숫자");
        assert!(parse_otpauth("otpauth://totp/X:y?secret=JBSWY3DPEHPK3PXP&period=0").is_err(), "주기 0");
        assert!(parse_otpauth("otpauth://totp/X:y?secret=JBSWY3DPEHPK3PXP&digits=20").is_err(), "자릿수 20");
        assert!(parse_otpauth("otpauth://hotp/X:y?secret=JBSWY3DPEHPK3PXP").is_err(), "HOTP 미지원");
        assert!(parse_otpauth("https://example.com").is_err());
    }

    #[test]
    fn migration_multiple_accounts() {
        let uri = migration_uri(&[
            ("JBSWY3DPEHPK3PXP", "authentik:bamin0422", "authentik"),
            ("GEZDGNBVGY3TQOJQ", "second", "GitHub"),
        ]);
        let accs = parse_migration(&uri).unwrap();
        assert_eq!(accs.len(), 2);
        assert_eq!(accs[0].issuer, "authentik");
        assert_eq!(accs[0].name, "bamin0422", "발급자 접두어가 이름에서 제거된다");
        assert_eq!(accs[1].issuer, "GitHub");
        assert_eq!(accs[1].secret, "GEZDGNBVGY3TQOJQ");
    }

    #[test]
    fn migration_truncated_is_error_not_panic() {
        let uri = migration_uri(&[("JBSWY3DPEHPK3PXP", "a:b", "a")]);
        let cut = &uri[..uri.len() - 12];
        let result = parse_migration(cut);
        assert!(result.is_err(), "잘린 데이터는 오류로 처리되고 패닉하지 않는다");
    }

    #[test]
    fn migration_garbage_is_error() {
        assert!(parse_migration("otpauth-migration://offline?data=%%%").is_err());
        assert!(parse_migration("otpauth-migration://offline").is_err());
        assert!(parse_migration("otpauth-migration://offline?data=AAAAAAAAAAAA").is_err());
    }

    #[test]
    fn payload_dispatch() {
        let uri = migration_uri(&[("JBSWY3DPEHPK3PXP", "a:b", "a")]);
        assert_eq!(parse_payload(&uri).unwrap().len(), 1);
        assert_eq!(parse_payload("otpauth://totp/X:y?secret=JBSWY3DPEHPK3PXP").unwrap().len(), 1);
        assert!(parse_payload("hello world").is_err());
    }
}
