#!/bin/bash
# OTPBar 1.x(키체인 평문 보관) → 2.0(암호화 금고) 이전 도구.
#
# 하는 일
#   1. 1.x 계정 목록과 비밀키를 키체인에서 읽는다
#   2. otpauth:// URI로 만들어 2.0 앱에 하나씩 넘긴다
#   3. 옮긴 개수를 보고하고, 키체인 정리 방법을 안내한다
#
# 비밀키는 이 스크립트의 메모리와 파이프에만 잠시 존재하며 파일로 쓰지 않는다.
set -euo pipefail

V1_INDEX="$HOME/.config/otpbar/accounts.json"
OTP="${OTP_BIN:-otp}"

red() { printf '\033[31m%s\033[0m\n' "$1"; }
green() { printf '\033[32m%s\033[0m\n' "$1"; }

if [[ "$(uname -s)" != "Darwin" ]]; then
  red "이 스크립트는 macOS 전용입니다(1.x가 macOS 키체인을 썼습니다)."
  exit 1
fi

if [[ ! -f "$V1_INDEX" ]]; then
  red "1.x 계정 목록을 찾지 못했습니다: $V1_INDEX"
  echo "옮길 것이 없다면 그대로 2.0을 쓰시면 됩니다."
  exit 1
fi

if ! command -v "$OTP" >/dev/null 2>&1; then
  red "otp 명령을 찾지 못했습니다. OTPBar 2.0을 먼저 설치하십시오."
  exit 1
fi

echo "2.0 앱 상태를 확인합니다…"
if ! "$OTP" status >/dev/null 2>&1; then
  red "실행 중인 OTPBar 2.0 앱이 없습니다. 앱을 먼저 실행하고 잠금을 해제하십시오."
  exit 1
fi

count=$(python3 -c "import json;print(len(json.load(open('$V1_INDEX'))['accounts']))")
echo "1.x 계정 ${count}개를 찾았습니다."
read -r -p "2.0 금고로 옮길까요? [y/N] " answer
[[ "$answer" =~ ^[Yy]$ ]] || { echo "취소했습니다."; exit 0; }

moved=0
failed=0
while IFS=$'\t' read -r id issuer name digits period algorithm; do
  [[ -z "$id" ]] && continue
  if ! secret=$(security find-generic-password -s otpbar -a "$id" -w 2>/dev/null); then
    red "  건너뜀: $issuer · $name (키체인에서 비밀키를 읽지 못했습니다)"
    failed=$((failed + 1))
    continue
  fi
  # otpauth URI 조립 (라벨은 퍼센트 인코딩)
  uri=$(ISSUER="$issuer" NAME="$name" SECRET="$secret" DIGITS="$digits" PERIOD="$period" ALGO="$algorithm" \
    python3 -c "
import os, urllib.parse
label = urllib.parse.quote(f\"{os.environ['ISSUER']}:{os.environ['NAME']}\")
q = urllib.parse.urlencode({
    'secret': os.environ['SECRET'], 'issuer': os.environ['ISSUER'],
    'digits': os.environ['DIGITS'], 'period': os.environ['PERIOD'],
    'algorithm': os.environ['ALGO'] or 'SHA1',
})
print(f'otpauth://totp/{label}?{q}')")
  if "$OTP" import "$uri" --replace >/dev/null 2>&1; then
    echo "  옮김: $issuer · $name"
    moved=$((moved + 1))
  else
    red "  실패: $issuer · $name"
    failed=$((failed + 1))
  fi
  unset secret uri
done < <(python3 -c "
import json
for a in json.load(open('$V1_INDEX'))['accounts']:
    print('\t'.join([a['id'], a.get('issuer',''), a.get('name',''),
                     str(a.get('digits',6)), str(a.get('period',30)), a.get('algorithm','SHA1')]))")

echo
green "완료: ${moved}개 이전, ${failed}개 실패"
echo
echo "확인 후 1.x 흔적을 지우십시오."
echo "  1) 계정 확인:   otp list"
echo "  2) 키체인 정리: python3 -c \"import json;[print(a['id']) for a in json.load(open('$V1_INDEX'))['accounts']]\" \\"
echo "                    | xargs -I{} security delete-generic-password -s otpbar -a {}"
echo "  3) 목록 파일:   rm $V1_INDEX"
