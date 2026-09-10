# OTPBar Windows installer (Windows PowerShell 5.1+ / PowerShell 7)
#   powershell -ExecutionPolicy Bypass -File windows\install.ps1            # install CLI + tray
#   powershell -ExecutionPolicy Bypass -File windows\install.ps1 -Login     # also start tray at login
#   powershell -ExecutionPolicy Bypass -File windows\install.ps1 -Uninstall
param(
    [switch]$Login,
    [switch]$Uninstall
)
$ErrorActionPreference = "Stop"
$Repo = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$BinDir = Join-Path $env:LOCALAPPDATA "otpbar\bin"
$Startup = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Startup\OTPBar.lnk"

if ($Uninstall) {
    Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $BinDir "otp.cmd"), (Join-Path $BinDir "otpbar.cmd"), $Startup
    Write-Host "Removed launcher scripts and startup shortcut. Account data (%APPDATA%\otpbar) was kept."
    exit 0
}

$py = Get-Command python -ErrorAction SilentlyContinue
if (-not $py) {
    Write-Host "Python 3.8+ is required. Install with:  winget install Python.Python.3.12"
    exit 1
}
$pyExe = $py.Source
$pywExe = Join-Path (Split-Path $pyExe) "pythonw.exe"
if (-not (Test-Path $pywExe)) { $pywExe = $pyExe }

Write-Host "[1/4] Installing Python packages (pystray, pillow, opencv-python-headless)"
& $pyExe -m pip install --user --quiet --disable-pip-version-check pystray pillow opencv-python-headless

Write-Host "[2/4] Creating launchers in $BinDir"
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
# batch files expand %VAR%, so escape literal percent signs in paths
$RepoEsc = $Repo.Replace('%', '%%'); $pyEsc = $pyExe.Replace('%', '%%'); $pywEsc = $pywExe.Replace('%', '%%')
$otpCmd = "@echo off`r`n`"$pyEsc`" `"$RepoEsc\cli\otp`" %*`r`n"
[IO.File]::WriteAllText((Join-Path $BinDir "otp.cmd"), $otpCmd, [Text.Encoding]::ASCII)
$trayCmd = "@echo off`r`nstart `"`" `"$pywEsc`" `"$RepoEsc\windows\otpbar_tray.py`"`r`n"
[IO.File]::WriteAllText((Join-Path $BinDir "otpbar.cmd"), $trayCmd, [Text.Encoding]::ASCII)

Write-Host "[3/4] Adding $BinDir to the user PATH"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not $userPath) { $userPath = "" }
if (($userPath -split ";") -notcontains $BinDir) {
    [Environment]::SetEnvironmentVariable("Path", ($userPath.TrimEnd(";") + ";" + $BinDir), "User")
}
$env:Path = "$env:Path;$BinDir"

if ($Login) {
    Write-Host "[3.5/4] Registering startup shortcut"
    $ws = New-Object -ComObject WScript.Shell
    $lnk = $ws.CreateShortcut($Startup)
    $lnk.TargetPath = $pywExe
    $lnk.Arguments = "`"$Repo\windows\otpbar_tray.py`""
    $lnk.WorkingDirectory = $Repo
    $lnk.Description = "OTPBar tray"
    $lnk.Save()
}

Write-Host "[4/4] Self-test"
& $pyExe "$Repo\cli\otp" selftest
if ($LASTEXITCODE -ne 0) { Write-Host "Self-test FAILED"; exit 1 }

Write-Host ""
Write-Host "Done. Open a new terminal and run:"
Write-Host "  otp import <export-qr.png>   # import Google Authenticator accounts"
Write-Host "  otp list                     # show codes"
Write-Host "  otpbar                       # start the tray app"
