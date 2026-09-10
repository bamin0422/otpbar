//! `otp` — OTPBar 명령줄 클라이언트.
//!
//! 기본 동작은 실행 중인 OTPBar 앱에 요청을 보내고 결과만 받아 오는 것이다.
//! **비밀키는 이 프로세스로 오지 않는다.** 코드 계산은 앱이 하고 CLI는 6자리만 받는다.
//!
//! 앱을 띄울 수 없는 상황(서버, 복구)에서는 `--offline`으로 마스터 암호를 직접 입력해
//! 금고를 열 수 있다. 이때만 비밀키가 이 프로세스 메모리에 잠시 존재한다.

use std::io::{IsTerminal, Write};
use std::time::Duration;

use clap::{Parser, Subcommand};
use otpbar_core::error::Error;
use otpbar_core::ipc::{Envelope, ListItem, Request, Response, RuntimeInfo};
use otpbar_core::{transport, Vault};

#[derive(Parser)]
#[command(
    name = "otp",
    version = otpbar_core::VERSION,
    about = "OTPBar 명령줄 클라이언트 — 일회용 코드(TOTP)를 앱에서 받아 온다",
    long_about = None,
)]
struct Cli {
    /// 앱을 거치지 않고 금고를 직접 연다(마스터 암호를 입력받는다)
    #[arg(long, global = true)]
    offline: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 앱 상태와 금고 잠금 여부
    Status {
        #[arg(long)]
        json: bool,
    },
    /// 등록된 계정 목록
    List {
        /// 코드도 함께 표시한다
        #[arg(long)]
        codes: bool,
        #[arg(long)]
        json: bool,
    },
    /// 계정 하나의 현재 코드
    Get {
        /// 계정 id 또는 발급자·이름 일부
        query: String,
        /// 클립보드에 복사한다(설정한 시간 뒤 자동으로 지운다)
        #[arg(long)]
        copy: bool,
        /// 알림에 표시할 사용 목적 (예: "authentik 로그인")
        #[arg(long, default_value = "")]
        purpose: String,
        /// 만료 직전이어도 기다리지 않고 지금 코드를 낸다
        #[arg(long)]
        no_wait: bool,
        #[arg(long)]
        json: bool,
    },
    /// QR 이미지나 otpauth URI에서 계정을 가져온다
    Import {
        /// 이미지 경로 또는 otpauth(-migration):// URI
        source: String,
        /// 같은 발급자·이름이 있으면 덮어쓴다
        #[arg(long)]
        replace: bool,
    },
    /// 금고를 처음 만든다(마스터 암호를 정한다)
    Init,
    /// 금고 잠금을 해제한다
    Unlock,
    /// 앱 창을 앞으로 가져온다(잠금 해제용)
    Show,
    /// 금고를 즉시 잠근다
    Lock,
}

fn main() {
    let cli = Cli::parse();
    if let Err(err) = run(&cli) {
        eprintln!("otp: {err}");
        std::process::exit(exit_code(&err));
    }
}

fn exit_code(err: &Error) -> i32 {
    match err {
        Error::Locked => 3,
        Error::AccountNotFound(_) | Error::Ambiguous(_) => 4,
        _ => 1,
    }
}

fn run(cli: &Cli) -> Result<(), Error> {
    if cli.offline {
        return run_offline(cli);
    }
    match &cli.command {
        Command::Status { json } => {
            let resp = send(Request::Status)?;
            match resp {
                Response::Status {
                    unlocked,
                    version,
                    account_count,
                    has_vault,
                } => {
                    if *json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "unlocked": unlocked, "version": version,
                                "accounts": account_count, "has_vault": has_vault
                            })
                        );
                    } else if !has_vault {
                        println!(
                            "금고가 아직 없습니다. 앱에서 마스터 암호를 정해 금고를 만드십시오."
                        );
                    } else if unlocked {
                        println!("잠금 해제됨 · 계정 {account_count}개 · 앱 {version}");
                    } else {
                        println!("잠김 · 앱 {version}  (otp show 로 앱을 열어 잠금 해제)");
                    }
                    Ok(())
                }
                other => unexpected(other),
            }
        }
        Command::List { codes, json } => {
            let resp = send(Request::List { with_codes: *codes })?;
            match resp {
                Response::List { accounts } => {
                    print_list(&accounts, *json);
                    Ok(())
                }
                other => unexpected(other),
            }
        }
        Command::Get {
            query,
            copy,
            purpose,
            no_wait,
            json,
        } => {
            let resp = send(Request::Code {
                query: query.clone(),
                purpose: purpose.clone(),
                copy: *copy,
                wait: !*no_wait,
            })?;
            match resp {
                Response::Code {
                    account,
                    code,
                    remaining,
                    copied,
                } => {
                    if *json {
                        println!(
                            "{}",
                            serde_json::json!({
                                "id": account.id, "issuer": account.issuer, "name": account.name,
                                "code": code, "remaining": remaining, "copied": copied
                            })
                        );
                    } else {
                        println!("{code}");
                        eprintln!(
                            "{} · {} · {remaining}초 남음{}",
                            account.issuer,
                            account.name,
                            if copied { " · 복사됨" } else { "" }
                        );
                    }
                    Ok(())
                }
                other => unexpected(other),
            }
        }
        Command::Import { source, replace } => {
            let resp = send(Request::Import {
                source: source.clone(),
                replace: *replace,
            })?;
            match resp {
                Response::Imported { messages, added } => {
                    for m in messages {
                        println!("{m}");
                    }
                    println!("가져오기 완료: {added}개 추가·갱신");
                    Ok(())
                }
                other => unexpected(other),
            }
        }
        Command::Init => {
            let pw = prompt_password("새 마스터 암호(8자 이상): ")?;
            let again = prompt_password("한 번 더: ")?;
            if pw != again {
                return Err(Error::other("두 암호가 다릅니다."));
            }
            send(Request::Init { password: pw })?;
            println!("금고를 만들었습니다.");
            Ok(())
        }
        Command::Unlock => {
            let pw = prompt_password("마스터 암호: ")?;
            send(Request::Unlock { password: pw })?;
            println!("잠금을 해제했습니다.");
            Ok(())
        }
        Command::Show => {
            send(Request::Show)?;
            println!("앱 창을 열었습니다.");
            Ok(())
        }
        Command::Lock => {
            send(Request::Lock)?;
            println!("금고를 잠갔습니다.");
            Ok(())
        }
    }
}

fn unexpected(resp: Response) -> Result<(), Error> {
    match resp {
        Response::Error { message, locked } => {
            if locked {
                Err(Error::Locked)
            } else {
                Err(Error::other(message))
            }
        }
        other => Err(Error::other(format!(
            "앱이 예상 밖의 응답을 보냈습니다: {other:?}"
        ))),
    }
}

/// 앱에 요청을 보내고 응답을 받는다.
fn send(request: Request) -> Result<Response, Error> {
    let info = RuntimeInfo::read()?;
    let mut conn = transport::connect(&info.endpoint)?;
    conn.set_timeout(Duration::from_secs(60))?;
    let envelope = Envelope {
        token: info.token,
        request,
    };
    conn.write_line(&serde_json::to_string(&envelope)?)?;
    let line = conn
        .read_line()?
        .ok_or_else(|| Error::other("앱이 응답 없이 연결을 끊었습니다"))?;
    let resp: Response = serde_json::from_str(&line)?;
    match resp {
        Response::Error { message, locked } => {
            if locked {
                Err(Error::Locked)
            } else {
                Err(Error::other(message))
            }
        }
        ok => Ok(ok),
    }
}

fn print_list(items: &[ListItem], json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(items).unwrap_or_default()
        );
        return;
    }
    if items.is_empty() {
        println!("등록된 계정이 없습니다. `otp import <QR 이미지>`로 추가하십시오.");
        return;
    }
    let width = items
        .iter()
        .map(|i| i.account.id.chars().count())
        .max()
        .unwrap_or(4)
        .max(2);
    for item in items {
        let label = format!("{} · {}", item.account.issuer, item.account.name);
        match (&item.code, item.remaining) {
            (Some(code), Some(rem)) => {
                println!(
                    "{:width$}  {:<32}  {}  {}초",
                    item.account.id,
                    label,
                    code,
                    rem,
                    width = width
                )
            }
            _ => println!("{:width$}  {}", item.account.id, label, width = width),
        }
    }
}

// ---------- 오프라인 모드 ----------

fn run_offline(cli: &Cli) -> Result<(), Error> {
    let mut vault = Vault::load()?;
    if !vault.exists() {
        return Err(Error::NoVault);
    }
    let password = prompt_password("마스터 암호: ")?;
    vault.unlock(&password)?;

    match &cli.command {
        Command::Status { json } => {
            let count = vault.accounts()?.len();
            if *json {
                println!(
                    "{}",
                    serde_json::json!({"unlocked": true, "accounts": count, "offline": true})
                );
            } else {
                println!("오프라인 · 계정 {count}개");
            }
        }
        Command::List { codes, json } => {
            let items: Vec<ListItem> = vault
                .accounts()?
                .into_iter()
                .map(|a| {
                    let (code, remaining) = if *codes {
                        match vault.code_for(&a.id) {
                            Ok((_, c, r)) => (Some(c), Some(r)),
                            Err(_) => (None, None),
                        }
                    } else {
                        (None, None)
                    };
                    ListItem {
                        account: a,
                        code,
                        remaining,
                    }
                })
                .collect();
            print_list(&items, *json);
        }
        Command::Get { query, json, .. } => {
            let (account, code, remaining) = vault.code_for(query)?;
            if *json {
                println!(
                    "{}",
                    serde_json::json!({"id": account.id, "code": code, "remaining": remaining, "offline": true})
                );
            } else {
                println!("{code}");
                eprintln!("{} · {} · {remaining}초 남음", account.issuer, account.name);
            }
        }
        Command::Import { source, replace } => {
            let accounts = otpbar_core::ipc::import_source_to_accounts(source)?;
            let added = accounts.len();
            for m in vault.add_many(accounts, *replace)? {
                println!("{m}");
            }
            println!("가져오기 완료: {added}건 처리");
        }
        Command::Show | Command::Lock | Command::Init | Command::Unlock => {
            return Err(Error::other("오프라인 모드에서는 쓸 수 없는 명령입니다"));
        }
    }
    Ok(())
}

fn prompt_password(prompt: &str) -> Result<String, Error> {
    if !std::io::stdin().is_terminal() {
        // 파이프로 넘어온 암호도 받아 준다(자동화용). 화면에는 남지 않는다.
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_string());
    }
    print!("{prompt}");
    std::io::stdout().flush()?;
    rpassword::read_password().map_err(|e| Error::other(format!("암호를 읽지 못했습니다: {e}")))
}
