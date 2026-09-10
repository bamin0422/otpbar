//! 글자를 받아야 할 때만 잠깐 띄우는 시스템 다이얼로그.
//!
//! 이 앱에는 상주 창이 없다. 마스터 암호처럼 입력이 필요한 순간에만 운영체제의 표준
//! 입력 상자를 띄우고, 값을 받으면 곧바로 닫는다.
//!
//! 프롬프트 문구는 앱이 정한 고정 문자열이며, 사용자 입력이 스크립트로 되돌아가는
//! 경로는 없다(값은 표준 출력으로만 받는다).

/// 가림 입력으로 암호를 받는다. 취소하면 `None`.
pub fn password(prompt: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        macos_password(prompt)
    }
    #[cfg(target_os = "windows")]
    {
        windows_password(prompt)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux_password(prompt)
    }
}

fn sanitize(prompt: &str) -> String {
    prompt.replace(['"', '\\', '\n', '\r'], " ")
}

#[cfg(target_os = "macos")]
fn macos_password(prompt: &str) -> Option<String> {
    let script = format!(
        r#"display dialog "{}" default answer "" with hidden answer with title "OTPBar" buttons {{"취소","확인"}} default button "확인""#,
        sanitize(prompt)
    );
    let out = std::process::Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .ok()?;
    if !out.status.success() {
        return None; // 취소
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // "button returned:확인, text returned:비밀번호"
    let marker = "text returned:";
    let idx = text.find(marker)?;
    Some(
        text[idx + marker.len()..]
            .trim_end_matches(['\n', '\r'])
            .to_string(),
    )
}

#[cfg(target_os = "windows")]
fn windows_password(prompt: &str) -> Option<String> {
    // WPF 창 하나로 가림 입력을 받는다. 값은 표준 출력으로만 나온다.
    let script = format!(
        r#"
Add-Type -AssemblyName PresentationFramework
$w = New-Object Windows.Window
$w.Title = 'OTPBar'; $w.Width = 380; $w.Height = 170
$w.WindowStartupLocation = 'CenterScreen'; $w.Topmost = $true; $w.ResizeMode = 'NoResize'
$sp = New-Object Windows.Controls.StackPanel; $sp.Margin = '16'
$tb = New-Object Windows.Controls.TextBlock; $tb.Text = "{}"; $tb.Margin = '0,0,0,10'; $tb.TextWrapping = 'Wrap'
$pb = New-Object Windows.Controls.PasswordBox; $pb.Margin = '0,0,0,12'; $pb.MinWidth = 320
$row = New-Object Windows.Controls.StackPanel; $row.Orientation = 'Horizontal'; $row.HorizontalAlignment = 'Right'
$ok = New-Object Windows.Controls.Button; $ok.Content = '확인'; $ok.Width = 80; $ok.Margin = '0,0,8,0'; $ok.IsDefault = $true
$cancel = New-Object Windows.Controls.Button; $cancel.Content = '취소'; $cancel.Width = 80; $cancel.IsCancel = $true
$ok.Add_Click({{ $w.DialogResult = $true }})
$row.AddChild($ok); $row.AddChild($cancel)
$sp.AddChild($tb); $sp.AddChild($pb); $sp.AddChild($row); $w.Content = $sp
$w.Add_ContentRendered({{ $pb.Focus() }})
if ($w.ShowDialog()) {{ [Console]::Out.Write($pb.Password) }}
"#,
        sanitize(prompt)
    );
    let sys32 = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let ps = format!(r"{sys32}\System32\WindowsPowerShell\v1.0\powershell.exe");
    let out = std::process::Command::new(ps)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&out.stdout).to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_password(prompt: &str) -> Option<String> {
    for (bin, args) in [
        (
            "zenity",
            vec![
                "--password".to_string(),
                format!("--title={}", sanitize(prompt)),
            ],
        ),
        ("kdialog", vec!["--password".to_string(), sanitize(prompt)]),
    ] {
        if let Ok(out) = std::process::Command::new(bin).args(&args).output() {
            if out.status.success() {
                return Some(String::from_utf8_lossy(&out.stdout).trim_end().to_string());
            }
        }
    }
    None
}
