#!/usr/bin/env python3
"""OTPBar 트레이 앱 (Windows·Linux). pystray 기반.

CLI(cli/otp)를 모듈로 불러 같은 저장소(계정 메타 + DPAPI/secret-tool 비밀키)로 코드를 만든다.
트레이 아이콘 우클릭 → 계정 클릭 → 코드 복사 + 알림.

설치: python -m pip install pystray pillow      (QR 가져오기까지 쓰려면 opencv-python-headless 추가)
실행: pythonw windows/otpbar_tray.py            (Windows)  |  python3 windows/otpbar_tray.py (Linux)
"""
import importlib.machinery
import importlib.util
import os
import subprocess
import sys
import threading

try:
    import pystray
    from PIL import Image, ImageDraw, ImageFont
except ImportError:
    print("필요한 패키지가 없습니다: python -m pip install pystray pillow", file=sys.stderr)
    sys.exit(1)

HERE = os.path.dirname(os.path.abspath(__file__))
CLI_PATH = os.path.join(os.path.dirname(HERE), "cli", "otp")


def load_cli():
    spec = importlib.util.spec_from_loader("otpcli", importlib.machinery.SourceFileLoader("otpcli", CLI_PATH))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


cli = load_cli()


def make_icon() -> Image.Image:
    img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle((4, 4, 60, 60), radius=16, fill=(30, 36, 50, 255))
    try:
        font = ImageFont.truetype("arial.ttf", 22)
    except Exception:
        font = ImageFont.load_default()
    d.text((32, 32), "OTP", fill=(255, 255, 255, 255), font=font, anchor="mm")
    return img


class Tray:
    def __init__(self):
        self.icon = pystray.Icon("OTPBar", make_icon(), "OTPBar", menu=pystray.Menu(self.build_items))

    # ---- 메뉴 ----
    def build_items(self):
        accounts = cli.load_index()["accounts"]
        items = []
        if not accounts:
            items.append(pystray.MenuItem("등록된 계정이 없습니다", None, enabled=False))
        for a in accounts:
            items.append(pystray.MenuItem(self._text_for(a), self._copy_for(a)))
        items += [
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("QR 이미지에서 가져오기…", self.import_qr),
            pystray.MenuItem("새로 고침", lambda icon, item: icon.update_menu()),
            pystray.MenuItem("설정 폴더 열기", self.open_folder),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("OTPBar 종료", lambda icon, item: icon.stop()),
        ]
        return items

    def _text_for(self, a):
        def text(item):
            try:
                code = cli.fmt_code(cli.totp(cli.store_get(a["id"]), a["digits"], a["period"], a["algorithm"]))
            except SystemExit as e:
                code = f"오류: {e}"
            return f"{a['issuer']} · {a['name']}    {code}    {cli.remaining(a['period'])}s"
        return text

    def _copy_for(self, a):
        def action(icon, item):
            try:
                code = cli.totp(cli.store_get(a["id"]), a["digits"], a["period"], a["algorithm"])
            except SystemExit as e:
                cli.notify("OTP 오류", a["id"], str(e))
                return
            cli.copy_clipboard(code)
            cli.notify("OTP 복사됨", f"{a['issuer']} · {a['name']}", f"{cli.fmt_code(code)}   ({cli.remaining(a['period'])}초 남음)")
        return action

    # ---- 동작 ----
    def import_qr(self, icon, item):
        def worker():
            try:
                import tkinter as tk
                from tkinter import filedialog
                root = tk.Tk()
                root.withdraw()
                root.attributes("-topmost", True)
                paths = filedialog.askopenfilenames(title="Google OTP 내보내기 QR 또는 otpauth QR 이미지",
                                                    filetypes=[("이미지", "*.png *.jpg *.jpeg *.bmp *.webp"), ("모든 파일", "*.*")])
                root.destroy()
            except Exception as e:
                cli.notify("가져오기 실패", "파일 선택 창을 열 수 없습니다", str(e))
                return
            for p in paths:
                try:
                    cli.main(["import", p])
                    cli.notify("가져오기 완료", os.path.basename(p), "메뉴를 다시 열면 계정이 보입니다.")
                except SystemExit as e:
                    cli.notify("가져오기 실패", os.path.basename(p), str(e))
            icon.update_menu()
        threading.Thread(target=worker, daemon=True).start()

    def open_folder(self, icon, item):
        os.makedirs(cli.CONFIG_DIR, exist_ok=True)
        if cli.IS_WIN:
            os.startfile(cli.CONFIG_DIR)  # type: ignore[attr-defined]
        else:
            subprocess.run(["xdg-open", cli.CONFIG_DIR])

    def run(self):
        self.icon.run()


if __name__ == "__main__":
    Tray().run()
