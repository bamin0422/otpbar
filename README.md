# OTPBar

![OTPBar](assets/banner.png)

Google OTP(Google Authenticator)에 등록한 계정의 일회용 코드를 Mac·Windows에서 만들고, 메뉴바(트레이)와 CLI로 꺼내 쓰는 개인용 도구입니다. Claude Code 같은 자동화 도구는 CLI(`otp get`)로 코드를 받아 로그인 폼에 입력합니다.

```bash
brew install bamin0422/tap/otpbar   # macOS
```

## 구성

| 구성 요소 | 역할 |
|---|---|
| `cli/otp` | Python CLI. 코드 생성, 계정 가져오기·추가·삭제, 알림·복사, 자체 검증 |
| `tools/otpqr.swift` | QR 이미지 디코더(Vision). Google OTP 내보내기 QR과 otpauth QR을 읽음 |
| `app/main.swift` | 메뉴바 앱. 계정별 코드와 남은 시간 표시, 클릭으로 복사 + 알림, QR 가져오기 메뉴 |
| `build.sh` | 빌드와 설치(`~/bin/otp`, `~/bin/otpqr`, `~/Applications/OTPBar.app`) |

비밀키는 macOS Keychain(service `otpbar`, account = 계정 id)에만 저장합니다. 계정 메타데이터(발급자·이름·자릿수·주기·알고리즘)는 `~/.config/otpbar/accounts.json`에 있습니다. CLI와 앱이 같은 저장소를 읽으므로 두 경로가 항상 같은 코드를 냅니다.

## 설치

Homebrew:

```bash
brew install bamin0422/tap/otpbar
otpbar                      # 메뉴바 앱 실행
brew services start otpbar  # 로그인 시 자동 실행 (선택)
```

소스에서 직접:

```bash
git clone https://github.com/bamin0422/otpbar ~/Projects/otpbar
cd ~/Projects/otpbar
./build.sh            # 또는 ./build.sh --login (로그인 시 자동 실행)
open ~/Applications/OTPBar.app
```

## Windows

Windows에서는 같은 CLI를 Python으로 실행하고, 트레이 앱은 `windows/otpbar_tray.py`(pystray)를 씁니다. 비밀키는 Windows DPAPI로 암호화해 `%APPDATA%\otpbar\secrets.json`에 저장하므로 같은 Windows 계정에서만 풀립니다. 알림은 Windows 토스트로 표시합니다.

요구 사항: Python 3.8 이상 (`winget install Python.Python.3.12`)

```powershell
git clone https://github.com/bamin0422/otpbar $env:USERPROFILE\otpbar
cd $env:USERPROFILE\otpbar
powershell -ExecutionPolicy Bypass -File windows\install.ps1 -Login   # -Login: 로그인 시 트레이 자동 실행
# 새 터미널에서
otp import C:\Users\me\Downloads\qr.png
otp list
otpbar                                                                  # 트레이 앱 실행
```

`install.ps1`은 pystray·pillow·opencv-python-headless를 사용자 영역에 설치하고, `%LOCALAPPDATA%\otpbar\bin`에 `otp.cmd`·`otpbar.cmd` 런처를 만들어 사용자 PATH에 추가합니다. 제거는 `install.ps1 -Uninstall`입니다(계정 데이터는 남습니다).

Linux에서도 CLI와 트레이 앱이 동작합니다. 비밀키는 `secret-tool`(libsecret)이 있으면 거기에, 없으면 0600 파일에 저장합니다.

## Google OTP에서 계정 가져오기

1. 폰의 Google Authenticator에서 메뉴(⋮) → 계정 이전 → 계정 내보내기 → 옮길 계정 선택. QR이 표시됩니다(계정이 많으면 여러 장).
2. QR을 Mac으로 옮깁니다. 폰에서 스크린샷 후 AirDrop, 또는 Mac의 Photo Booth로 폰 화면을 촬영합니다.
3. 가져오기:
   ```bash
   otp import ~/Downloads/qr.png --dry-run   # 무엇이 들어 있는지 먼저 확인
   otp import ~/Downloads/qr.png             # Keychain + accounts.json 에 저장
   otp list
   ```
   메뉴바 앱의 "QR 이미지에서 가져오기…"로도 같은 작업을 할 수 있습니다.
4. 가져온 뒤 QR 이미지는 삭제합니다. 이미지 한 장에 모든 비밀키가 들어 있습니다.

다른 서비스에서 2차인증을 새로 등록할 때는 QR을 저장해 `otp import`하거나, 설정 키를 `otp add --issuer <서비스> --name <계정> --secret <키>`로 넣습니다.

## Claude가 쓰는 방식

```bash
otp get webmail --purpose "웹메일 로그인"      # stdout: 6자리 코드, 알림: 어느 계정을 어디에 쓰는지
otp get jira --copy                          # 클립보드 복사까지
otp list --json
```

`otp get`은 코드 만료 5초 전이면 다음 코드가 나올 때까지 기다린 뒤 출력합니다(입력 중 만료 방지). 알림은 기본으로 켜져 있고 `--no-notify`로 끕니다.

## 업데이트

```bash
otp update --check   # 새 버전 확인 (GitHub 태그 기준)
otp update           # Homebrew 설치면 brew upgrade, git 설치면 git pull + 재빌드, 그 뒤 앱 재시작
```

메뉴바 앱과 트레이 앱은 실행 5초 후와 24시간마다 자동으로 확인해 새 버전이 있으면 알림을 띄우고, 메뉴 맨 위에 "업데이트 x.y.z 설치…" 항목을 보여 줍니다. "업데이트 확인…" 항목으로 바로 확인할 수도 있습니다.

## 보안

- 비밀키를 비밀번호와 같은 기기에 두면 2차인증의 의미가 "이 Mac을 가진 사람"으로 줄어듭니다. 개인 계정에 한정하고, 회사 시스템의 2차인증을 자동화할 때는 정보보호 담당과 허용 여부를 먼저 확인하십시오.
- Keychain 항목은 `security` CLI가 만들며 파티션 ID가 `apple-tool:`로 잠깁니다. ad-hoc 서명 앱이 이 항목을 직접 읽으면 macOS가 로그인 키체인 암호를 요구하므로, 앱은 Keychain을 직접 읽지 않고 `otp secrets --json`(CLI)에서 비밀키를 받아 메모리에서만 씁니다. 그래서 앱을 다시 빌드해도 허용 창이 뜨지 않습니다.
- `accounts.json`에는 비밀키가 없습니다. 이 파일만으로는 코드를 만들 수 없습니다.
- 복사한 코드는 만료 5초 뒤 클립보드에서 자동으로 지웁니다(그 사이 다른 것을 복사했으면 건드리지 않습니다).
- `otp add`에서 `--secret`을 생략하면 화면에 표시되지 않는 프롬프트로 비밀키를 받습니다. 명령줄에 비밀키를 적으면 셸 히스토리에 남습니다.
- `otp secrets --json`은 메뉴바·트레이 앱이 쓰는 내부 명령으로 모든 비밀키를 출력합니다. 같은 사용자 권한으로 실행되는 다른 프로그램도 이 명령(또는 `security`)으로 비밀키를 읽을 수 있으므로, 신뢰할 수 없는 소프트웨어를 함께 쓰는 기기에는 설치하지 마십시오.
- 알림에는 코드가 그대로 표시됩니다. 잠금 화면 알림 미리보기를 켜 두었다면 코드가 노출될 수 있으니 시스템 설정에서 OTPBar 알림의 미리보기를 "잠금 해제 시"로 두는 것을 권합니다.
- Linux에서 `secret-tool`이 없으면 저장을 거부합니다. `OTPBAR_ALLOW_PLAINTEXT=1`을 설정한 경우에만 0600 파일에 평문으로 저장합니다.
- 메뉴바 앱은 같은 설치 폴더의 `otp`를 `/usr/bin/python3`로 직접 실행하고 PATH를 시스템 디렉터리로 제한합니다. CLI도 `security`·`pbcopy`·`osascript` 같은 시스템 도구를 절대 경로로 부릅니다(PATH 하이재킹 방지).
- Keychain에 저장할 때 비밀키를 명령 인자에 싣지 않고 `security -i`의 표준입력으로 넘깁니다(프로세스 목록 노출 방지).
- 가져온 계정의 자릿수(4~10)와 주기(1~600초)를 검증해 손상된 QR로 앱이 멈추지 않게 합니다.
- 업데이트: Homebrew 설치는 tap의 sha256 검증을 거칩니다. git 설치는 origin이 공식 저장소일 때만 릴리스 태그를 정확히 체크아웃해 빌드하며, 브랜치 최신을 당기지 않습니다. 서명된 태그 검증은 아직 없으므로 GitHub 저장소를 신뢰하는 모델입니다. Windows 의존성(pystray·pillow·opencv)은 pip에서 받으며 해시 고정은 하지 않습니다.

## 검증

```bash
otp selftest   # RFC 6238 부록 B 벡터(SHA1·SHA256·SHA512) + Google 내보내기 protobuf 왕복 + otpauth 파서
```
