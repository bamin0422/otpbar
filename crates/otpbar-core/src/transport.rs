//! IPC 전송 계층. 한 줄 JSON을 주고받는다.
//!
//! 플랫폼별 선택
//! - 유닉스: 도메인 소켓 파일(권한 0600). 파일 권한이 곧 접근 제어다.
//! - Windows: 루프백 TCP(127.0.0.1, 임의 포트). 외부에서 접근할 수 없고,
//!   접속 후 토큰 검사를 통과해야 한다. 토큰은 사용자 프로필 안의 0600 파일에 있다.
//!
//! 어느 쪽이든 토큰 검사는 공통이므로, 소켓·포트에 도달하더라도 토큰 없이는 아무 것도
//! 얻지 못한다.

use std::io::{BufRead, BufReader, Read, Write};

use crate::error::{Error, Result};

pub struct Connection {
    reader: BufReader<Inner>,
    writer: Inner,
}

/// 한 줄 요청의 최대 길이(가져오기 경로를 넉넉히 담되 자원 고갈은 막는다).
const MAX_LINE: u64 = 64 * 1024;

impl Connection {
    /// 한 줄(JSON)을 읽는다. 상대가 끊으면 `None`.
    pub fn read_line(&mut self) -> Result<Option<String>> {
        let mut line = String::new();
        let n = (&mut self.reader).take(MAX_LINE).read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim_end().to_string()))
    }

    pub fn write_line(&mut self, line: &str) -> Result<()> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    pub type Inner = UnixStream;

    pub struct Listener(UnixListener);

    impl Listener {
        /// 소켓 파일을 만들고 권한을 0600으로 좁힌다. 이미 있던 파일은 지운다.
        pub fn bind(endpoint: &str) -> Result<Self> {
            let path = std::path::Path::new(endpoint);
            if let Some(parent) = path.parent() {
                crate::util::ensure_private_dir(parent)?;
            }
            // 죽은 앱이 남긴 소켓 파일 정리
            if path.exists() {
                std::fs::remove_file(path).ok();
            }
            let listener = UnixListener::bind(path)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Listener(listener))
        }

        pub fn accept(&self) -> Result<super::Connection> {
            let (stream, _addr) = self.0.accept()?;
            super::Connection::from_stream(stream)
        }

        /// 실제 접속에 쓸 문자열(유닉스는 소켓 경로 그대로).
        pub fn endpoint_hint(endpoint: &str) -> String {
            endpoint.to_string()
        }
    }

    pub fn connect(endpoint: &str) -> Result<super::Connection> {
        let stream = UnixStream::connect(endpoint)
            .map_err(|e| Error::other(format!("앱에 연결하지 못했습니다({endpoint}): {e}")))?;
        super::Connection::from_stream(stream)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};

    pub type Inner = TcpStream;

    pub struct Listener(TcpListener);

    impl Listener {
        /// 루프백에 임의 포트로 연다. `endpoint_hint`가 실제 포트를 알려 준다.
        pub fn bind(_endpoint: &str) -> Result<Self> {
            let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0);
            let listener = TcpListener::bind(addr)?;
            Ok(Listener(listener))
        }

        pub fn accept(&self) -> Result<super::Connection> {
            let (stream, peer) = self.0.accept()?;
            // 루프백 이외의 주소는 즉시 끊는다(이중 방어)
            if !peer.ip().is_loopback() {
                return Err(Error::other("루프백이 아닌 접속을 거부했습니다"));
            }
            stream.set_nodelay(true).ok();
            super::Connection::from_stream(stream)
        }

        pub fn endpoint_hint(_endpoint: &str) -> String {
            String::new() // bind 후 local_addr로 채운다
        }
    }

    impl Listener {
        pub fn local_endpoint(&self) -> Result<String> {
            Ok(format!("127.0.0.1:{}", self.0.local_addr()?.port()))
        }
    }

    pub fn connect(endpoint: &str) -> Result<super::Connection> {
        let stream = TcpStream::connect(endpoint)
            .map_err(|e| Error::other(format!("앱에 연결하지 못했습니다({endpoint}): {e}")))?;
        stream.set_nodelay(true).ok();
        super::Connection::from_stream(stream)
    }
}

use imp::Inner;
pub use imp::{connect, Listener};

impl Connection {
    fn from_stream(stream: Inner) -> Result<Self> {
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Connection {
            reader,
            writer: stream,
        })
    }

    /// 응답을 기다리는 시간 제한을 건다(앱이 멈춰도 CLI가 매달리지 않게).
    pub fn set_timeout(&mut self, dur: std::time::Duration) -> Result<()> {
        self.writer.set_read_timeout(Some(dur))?;
        self.writer.set_write_timeout(Some(dur))?;
        self.reader.get_ref().set_read_timeout(Some(dur))?;
        Ok(())
    }
}

#[cfg(unix)]
impl Listener {
    /// 유닉스에서는 bind에 쓴 경로가 그대로 접속 주소다.
    pub fn local_endpoint(&self) -> Result<String> {
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_over_socket() {
        let dir = std::env::temp_dir().join(format!("otpbar-ipc-{}", crate::util::now_unix()));
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        let endpoint = dir.join("t.sock").to_string_lossy().to_string();
        #[cfg(windows)]
        let endpoint = String::new();

        let listener = Listener::bind(&endpoint).unwrap();
        #[cfg(windows)]
        let endpoint = listener.local_endpoint().unwrap();

        let server = std::thread::spawn(move || {
            let mut conn = listener.accept().unwrap();
            let line = conn.read_line().unwrap().unwrap();
            conn.write_line(&format!("echo:{line}")).unwrap();
        });

        let mut client = connect(&endpoint).unwrap();
        client
            .set_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        client.write_line("hello").unwrap();
        assert_eq!(client.read_line().unwrap().unwrap(), "echo:hello");
        server.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn socket_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("otpbar-ipc-perm-{}", crate::util::now_unix()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.sock");
        let endpoint = path.to_string_lossy().to_string();
        let _listener = Listener::bind(&endpoint).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "소켓 파일은 소유자만 접근할 수 있어야 한다");
        std::fs::remove_dir_all(&dir).ok();
    }
}
