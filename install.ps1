# OTPBar Windows installer
#
#   irm https://raw.githubusercontent.com/bamin0422/otpbar/main/install.ps1 | iex
#
# What it does
#   1. downloads the latest signed release from GitHub
#   2. installs the OTPBar app (NSIS, current user - no admin rights needed)
#   3. drops the `otp` CLI into %LOCALAPPDATA%\otpbar\bin and adds it to your PATH
#   4. starts the app
#
# Uninstall: "OTPBar" in Settings > Apps, then remove %LOCALAPPDATA%\otpbar
# Your vault stays in %APPDATA%\otpbar until you delete it yourself.

#Requires -Version 5.1
[CmdletBinding()]
param(
    # Install a specific version instead of the latest, e.g. -Version 2.0.0
    [string]$Version,
    # Only install the CLI, skip the desktop app
    [switch]$CliOnly,
    # Do not launch the app after installing
    [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'bamin0422/otpbar'
$BinDir = Join-Path $env:LOCALAPPDATA 'otpbar\bin'

function Write-Step($n, $text) { Write-Host "[$n/5] $text" -ForegroundColor Cyan }
function Fail($text) { Write-Host "오류: $text" -ForegroundColor Red; exit 1 }

if ($env:PROCESSOR_ARCHITECTURE -notin @('AMD64', 'ARM64')) {
    Fail "지원하지 않는 아키텍처입니다: $env:PROCESSOR_ARCHITECTURE"
}

Write-Step 1 '최신 릴리스 확인'
$api = if ($Version) {
    "https://api.github.com/repos/$Repo/releases/tags/v$Version"
} else {
    "https://api.github.com/repos/$Repo/releases/latest"
}
try {
    $release = Invoke-RestMethod -Uri $api -Headers @{ 'User-Agent' = 'otpbar-installer' }
} catch {
    Fail "릴리스 정보를 가져오지 못했습니다: $($_.Exception.Message)"
}
$tag = $release.tag_name
Write-Host "    버전 $tag"

$tmp = Join-Path $env:TEMP "otpbar-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

function Get-Asset([string]$pattern) {
    $asset = $release.assets | Where-Object { $_.name -like $pattern } | Select-Object -First 1
    if (-not $asset) { return $null }
    $dest = Join-Path $tmp $asset.name
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $dest -Headers @{ 'User-Agent' = 'otpbar-installer' }
    return $dest
}

if (-not $CliOnly) {
    Write-Step 2 '앱 설치 (관리자 권한 없이 현재 사용자에게 설치)'
    $setup = Get-Asset '*-setup.exe'
    if (-not $setup) { Fail '릴리스에서 Windows 설치 파일을 찾지 못했습니다.' }
    # /S = NSIS 무인 설치
    $proc = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
    if ($proc.ExitCode -ne 0) { Fail "설치 프로그램이 실패했습니다 (종료 코드 $($proc.ExitCode))" }
    Write-Host '    완료'
} else {
    Write-Step 2 '앱 설치 건너뜀 (-CliOnly)'
}

Write-Step 3 "CLI 설치: $BinDir"
$cliZip = Get-Asset 'otp-windows-*.zip'
if ($cliZip) {
    New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
    Expand-Archive -Path $cliZip -DestinationPath $BinDir -Force
    Write-Host '    otp.exe 배치 완료'
} else {
    Write-Host '    CLI 파일이 릴리스에 없어 건너뜁니다.' -ForegroundColor Yellow
}

Write-Step 4 'PATH 등록'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not $userPath) { $userPath = '' }
if (($userPath -split ';') -notcontains $BinDir) {
    [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $BinDir), 'User')
    Write-Host '    사용자 PATH에 추가했습니다 (새 터미널부터 적용)'
} else {
    Write-Host '    이미 등록되어 있습니다'
}
$env:Path = "$env:Path;$BinDir"

Write-Step 5 '정리'
Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue

Write-Host ''
Write-Host 'OTPBar 설치 완료' -ForegroundColor Green
Write-Host ''
Write-Host '  1. 트레이의 OTPBar 아이콘을 눌러 마스터 암호를 정하십시오.'
Write-Host '  2. 구글 OTP에서 "계정 이전 → 계정 내보내기" QR을 캡처해 가져오십시오.'
Write-Host '  3. 자동화에서는 다음처럼 씁니다:'
Write-Host '       otp list'
Write-Host '       otp get authentik --copy'
Write-Host ''

if (-not $NoLaunch -and -not $CliOnly) {
    $exe = Get-ChildItem -Path @(
        (Join-Path $env:LOCALAPPDATA 'Programs\OTPBar\OTPBar.exe'),
        (Join-Path ${env:ProgramFiles} 'OTPBar\OTPBar.exe')
    ) -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($exe) { Start-Process $exe.FullName }
}
