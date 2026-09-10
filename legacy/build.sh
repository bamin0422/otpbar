#!/bin/zsh
# OTPBar 빌드·설치 스크립트
#   ./build.sh          빌드 후 ~/bin(otp, otpqr)과 ~/Applications/OTPBar.app 에 설치
#   ./build.sh --login  위 작업 + 로그인 시 자동 실행(LaunchAgent) 등록
set -euo pipefail
cd "$(dirname "$0")"

mkdir -p bin build
echo "[1/4] otpqr (QR 디코더) 빌드"
swiftc -O tools/otpqr.swift -o bin/otpqr

echo "[2/4] OTPBar.app 빌드"
APP=build/OTPBar.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp app/Info.plist "$APP/Contents/Info.plist"
cp app/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
swiftc -O app/main.swift -o "$APP/Contents/MacOS/OTPBar"
codesign --force --sign - "$APP" >/dev/null 2>&1 || echo "  (codesign 생략)"

echo "[3/4] 설치: ~/bin/otp, ~/bin/otpqr, ~/Applications/OTPBar.app"
mkdir -p ~/bin ~/Applications
chmod +x cli/otp bin/otpqr
ln -sf "$PWD/cli/otp" ~/bin/otp
ln -sf "$PWD/bin/otpqr" ~/bin/otpqr
if pgrep -x OTPBar >/dev/null; then pkill -x OTPBar; sleep 0.5; fi
rm -rf ~/Applications/OTPBar.app
cp -R "$APP" ~/Applications/OTPBar.app
# 앱은 Keychain을 직접 읽지 않고 CLI(otp secrets --json)를 경유하므로 접근 허용 갱신이 필요 없다.

echo "[4/4] 자체 검증"
~/bin/otp selftest | tail -1

if [[ "${1:-}" == "--login" ]]; then
  PL=~/Library/LaunchAgents/com.bamin0422.otpbar.plist
  mkdir -p ~/Library/LaunchAgents
  cat > "$PL" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.bamin0422.otpbar</string>
  <key>ProgramArguments</key><array><string>$HOME/Applications/OTPBar.app/Contents/MacOS/OTPBar</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><false/>
</dict></plist>
EOF
  launchctl unload "$PL" 2>/dev/null || true
  launchctl load "$PL"
  echo "로그인 자동 실행 등록: $PL"
fi

echo "완료. 실행: open ~/Applications/OTPBar.app   |   CLI: otp list / otp get <계정>"
