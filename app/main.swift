// OTPBar: 메뉴바에서 TOTP 코드를 보여 주고 클릭으로 복사하는 macOS 앱.
// 데이터: ~/.config/otpbar/accounts.json (메타) + Keychain service "otpbar" (비밀키, base32)
// CLI(otp)와 같은 저장소를 읽으므로 두 경로가 항상 같은 코드를 낸다.
import Cocoa
import CryptoKit
import Security
import UserNotifications

struct Account: Codable {
    let id: String
    let issuer: String
    let name: String
    let algorithm: String
    let digits: Int
    let period: Int
    let secret: String
}

let configDir = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/otpbar")
var lastLoadError = ""

/// 계정 메타와 비밀키를 CLI(otp secrets --json)에서 받는다.
/// 앱이 Keychain을 직접 읽으면 macOS 파티션 검사(apple-tool:) 때문에 키체인 암호 창이 뜨므로,
/// `security` 도구로 조용히 읽을 수 있는 CLI를 경유한다. 비밀키는 메모리에만 머문다.
/// 실행할 otp 경로: 같은 설치 폴더(Cellar/otpbar/x/bin/otp, 또는 build 시 ../bin)의 것을 우선한다.
/// PATH 조작으로 다른 otp가 끼어드는 것을 막는다. 없으면 PATH에서 찾는다.
func otpExecutable() -> String? {
    let bundle = Bundle.main.bundleURL   // …/OTPBar.app
    let candidates = [
        bundle.deletingLastPathComponent().appendingPathComponent("bin/otp").path,          // Homebrew Cellar
        bundle.deletingLastPathComponent().appendingPathComponent("../cli/otp").standardized.path, // 개발 빌드
        "/opt/homebrew/bin/otp", "/usr/local/bin/otp",
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("bin/otp").path,
    ]
    return candidates.first { FileManager.default.isExecutableFile(atPath: $0) }
}

/// CLI(otp)를 인자 배열 그대로(셸 해석 없이) 실행하고 (종료코드, stdout, stderr)를 돌려준다.
func runOTP(_ args: [String]) -> (Int32, Data, String) {
    guard let exe = otpExecutable() else { return (-1, Data(), "otp CLI를 찾지 못했습니다. brew install bamin0422/tap/otpbar") }
    // 시스템 파이썬을 절대 경로로 실행하고 PATH를 시스템 디렉터리로 제한한다.
    // (/opt/homebrew/bin 은 admin 그룹이 쓸 수 있어 가짜 python3 를 심는 PATH 하이재킹이 가능하기 때문)
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
    p.arguments = [exe] + args
    var env = ProcessInfo.processInfo.environment
    env["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
    p.environment = env
    let out = Pipe(), err = Pipe()
    p.standardOutput = out
    p.standardError = err
    do { try p.run() } catch { return (-1, Data(), "otp 실행 실패: \(error)") }
    let data = out.fileHandleForReading.readDataToEndOfFile()
    let errText = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
    p.waitUntilExit()
    return (p.terminationStatus, data, errText)
}

func loadAccounts() -> [Account] {
    let (status, data, err) = runOTP(["secrets", "--json"])
    if status != 0 {
        lastLoadError = err.isEmpty ? "otp 오류" : err
        return []
    }
    lastLoadError = ""
    return (try? JSONDecoder().decode([Account].self, from: data)) ?? []
}

struct UpdateInfo: Codable {
    let current: String
    let latest: String
    let update_available: Bool
    let method: String
    let compare_url: String
}

func checkUpdate() -> UpdateInfo? {
    let (status, data, _) = runOTP(["update", "--check", "--json"])
    guard status == 0 else { return nil }
    return try? JSONDecoder().decode(UpdateInfo.self, from: data)
}

func postNotification(_ title: String, _ subtitle: String, _ body: String) {
    let content = UNMutableNotificationContent()
    content.title = title
    content.subtitle = subtitle
    content.body = body
    content.sound = .default
    UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
}

func base32Decode(_ input: String) -> Data? {
    let alphabet = Array("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567")
    var map = [Character: UInt8]()
    for (i, c) in alphabet.enumerated() { map[c] = UInt8(i) }
    var bits = 0, value = 0
    var out = Data()
    for ch in input.uppercased() where ch != "=" && ch != " " && ch != "-" {
        guard let v = map[ch] else { return nil }
        value = (value << 5) | Int(v)
        bits += 5
        if bits >= 8 {
            out.append(UInt8((value >> (bits - 8)) & 0xFF))
            bits -= 8
        }
    }
    return out
}

func totp(_ acc: Account, at date: Date = Date()) -> String? {
    // 잘못된 메타(주기 0, 자릿수 과대)로 0 나눗셈·오버플로 트랩이 나지 않도록 범위를 먼저 검사한다
    guard acc.period > 0, acc.period <= 600, (4...10).contains(acc.digits) else { return nil }
    guard let key = base32Decode(acc.secret), !key.isEmpty else { return nil }
    let counter = UInt64(date.timeIntervalSince1970) / UInt64(acc.period)
    var msg = counter.bigEndian
    let msgData = Data(bytes: &msg, count: 8)
    let symKey = SymmetricKey(data: key)
    let mac: [UInt8]
    switch acc.algorithm.uppercased() {
    case "SHA256": mac = Array(HMAC<SHA256>.authenticationCode(for: msgData, using: symKey))
    case "SHA512": mac = Array(HMAC<SHA512>.authenticationCode(for: msgData, using: symKey))
    default: mac = Array(HMAC<Insecure.SHA1>.authenticationCode(for: msgData, using: symKey))
    }
    let offset = Int(mac[mac.count - 1] & 0x0F)
    let bin = (UInt32(mac[offset]) & 0x7F) << 24 | UInt32(mac[offset + 1]) << 16
        | UInt32(mac[offset + 2]) << 8 | UInt32(mac[offset + 3])
    let mod = UInt32(pow(10.0, Double(acc.digits)))
    return String(format: "%0\(acc.digits)d", bin % mod)
}

func remaining(_ period: Int) -> Int {
    let t = Date().timeIntervalSince1970
    return period - Int(t.truncatingRemainder(dividingBy: Double(period)))
}

func pretty(_ code: String) -> String {
    code.count == 6 ? "\(code.prefix(3)) \(code.suffix(3))" : code
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate, UNUserNotificationCenterDelegate {
    var statusItem: NSStatusItem!
    let menu = NSMenu()
    var accounts: [Account] = []
    var codeItems: [(NSMenuItem, Account)] = []
    var timer: Timer?
    var updateInfo: UpdateInfo?
    var updating = false
    var updateTimer: Timer?
    let mono = NSFont.monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)

    func applicationDidFinishLaunching(_ notification: Notification) {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        if let img = NSImage(systemSymbolName: "key.horizontal.fill", accessibilityDescription: "OTP") {
            statusItem.button?.image = img
        } else {
            statusItem.button?.title = "OTP"
        }
        menu.delegate = self
        statusItem.menu = menu
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.requestAuthorization(options: [.alert, .sound]) { _, _ in }
        rebuildMenu()
        // 업데이트 자동 확인: 실행 5초 후 1회, 이후 24시간마다
        DispatchQueue.global().asyncAfter(deadline: .now() + 5) { [weak self] in self?.backgroundUpdateCheck(manual: false) }
        updateTimer = Timer.scheduledTimer(withTimeInterval: 86_400, repeats: true) { [weak self] _ in
            DispatchQueue.global().async { self?.backgroundUpdateCheck(manual: false) }
        }
    }

    func backgroundUpdateCheck(manual: Bool) {
        let info = checkUpdate()
        DispatchQueue.main.async {
            self.updateInfo = info
            guard let info = info else {
                if manual { self.alert("업데이트 확인 실패", "최신 버전을 조회하지 못했습니다. 네트워크 상태를 확인하십시오.") }
                return
            }
            if info.update_available {
                postNotification("OTPBar 업데이트", "\(info.current) → \(info.latest)", "메뉴에서 '업데이트 \(info.latest) 설치…'를 누르면 설치합니다.")
                if manual { self.offerInstall(info) }
            } else if manual {
                self.alert("최신 상태입니다", "현재 \(info.current), 최신 \(info.latest)")
            }
        }
    }

    func alert(_ title: String, _ text: String) {
        let a = NSAlert()
        a.messageText = title
        a.informativeText = text
        NSApp.activate(ignoringOtherApps: true)
        a.runModal()
    }

    func offerInstall(_ info: UpdateInfo) {
        let a = NSAlert()
        a.messageText = "새 버전 \(info.latest)이 있습니다"
        a.informativeText = "현재 \(info.current). 설치 방식: \(info.method)\n변경 내역: \(info.compare_url)"
        a.addButton(withTitle: "지금 설치")
        a.addButton(withTitle: "나중에")
        NSApp.activate(ignoringOtherApps: true)
        if a.runModal() == .alertFirstButtonReturn { installUpdate() }
    }

    @objc func manualCheckUpdate() {
        DispatchQueue.global().async { [weak self] in self?.backgroundUpdateCheck(manual: true) }
    }

    @objc func installUpdate() {
        guard !updating else { return }
        updating = true
        postNotification("OTPBar 업데이트", "설치 중…", "완료되면 앱이 자동으로 다시 시작됩니다.")
        DispatchQueue.global().async { [weak self] in
            let (status, data, err) = runOTP(["update", "--json"])
            DispatchQueue.main.async {
                self?.updating = false
                if status == 0 {
                    let text = String(data: data, encoding: .utf8) ?? ""
                    let result = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["result"] as? String ?? text
                    postNotification("OTPBar 업데이트 완료", "", result)
                } else {
                    self?.alert("업데이트 실패", err.isEmpty ? "otp update 가 실패했습니다." : err)
                }
            }
        }
    }

    // 알림은 앱이 앞에 있어도 배너로 보인다
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound])
    }

    func rebuildMenu() {
        menu.removeAllItems()
        codeItems.removeAll()
        if let info = updateInfo, info.update_available {
            let up = NSMenuItem(title: updating ? "업데이트 설치 중…" : "업데이트 \(info.latest) 설치…",
                                action: #selector(installUpdate), keyEquivalent: "")
            up.target = self
            up.isEnabled = !updating
            menu.addItem(up)
            menu.addItem(.separator())
        }
        accounts = loadAccounts()
        if accounts.isEmpty {
            let msg = lastLoadError.isEmpty ? "등록된 계정이 없습니다" : "otp CLI 오류: \(lastLoadError.trimmingCharacters(in: .whitespacesAndNewlines))"
            let empty = NSMenuItem(title: msg, action: nil, keyEquivalent: "")
            empty.isEnabled = false
            menu.addItem(empty)
        }
        for acc in accounts {
            let item = NSMenuItem(title: "", action: #selector(copyCode(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = acc.id
            menu.addItem(item)
            codeItems.append((item, acc))
        }
        menu.addItem(.separator())
        let imp = NSMenuItem(title: "QR 이미지에서 가져오기…", action: #selector(importQR), keyEquivalent: "i")
        imp.target = self
        menu.addItem(imp)
        let reload = NSMenuItem(title: "새로 고침", action: #selector(reloadIndex), keyEquivalent: "r")
        reload.target = self
        menu.addItem(reload)
        let folder = NSMenuItem(title: "설정 폴더 열기", action: #selector(openFolder), keyEquivalent: "")
        folder.target = self
        menu.addItem(folder)
        let upd = NSMenuItem(title: "업데이트 확인…", action: #selector(manualCheckUpdate), keyEquivalent: "u")
        upd.target = self
        menu.addItem(upd)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "OTPBar 종료", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        menu.addItem(quit)
        refreshTitles()
    }

    func refreshTitles() {
        for (item, acc) in codeItems {
            let code = totp(acc) ?? "------"
            let rem = remaining(acc.period)
            let label = "\(acc.issuer) · \(acc.name)"
            let text = "\(label)    \(pretty(code))    \(rem)s"
            let attr = NSMutableAttributedString(string: text, attributes: [.font: mono])
            if let r = text.range(of: pretty(code)) {
                attr.addAttributes([.font: NSFont.monospacedDigitSystemFont(ofSize: NSFont.systemFontSize, weight: .semibold)],
                                   range: NSRange(r, in: text))
            }
            if rem <= 5 {
                attr.addAttributes([.foregroundColor: NSColor.systemRed], range: NSRange(text.range(of: "\(rem)s")!, in: text))
            }
            item.attributedTitle = attr
        }
    }

    func menuWillOpen(_ menu: NSMenu) {
        rebuildMenu()
        timer?.invalidate()
        timer = Timer(timeInterval: 1.0, repeats: true) { [weak self] _ in self?.refreshTitles() }
        RunLoop.main.add(timer!, forMode: .common)
    }

    func menuDidClose(_ menu: NSMenu) {
        timer?.invalidate()
        timer = nil
        // 메뉴 항목 액션이 처리된 뒤 비밀키를 메모리에서 비운다 (메뉴가 다시 열리면 CLI에서 다시 읽는다)
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            guard let self = self, self.timer == nil else { return }
            self.accounts = []
            self.codeItems.removeAll()
        }
    }

    @objc func copyCode(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let acc = accounts.first(where: { $0.id == id }),
              let code = totp(acc) else { return }
        let pb = NSPasteboard.general
        pb.clearContents()
        pb.setString(code, forType: .string)
        let rem = remaining(acc.period)
        let count = pb.changeCount
        // 코드 만료 5초 뒤, 그 사이 다른 것을 복사하지 않았으면 클립보드를 비운다
        DispatchQueue.main.asyncAfter(deadline: .now() + .seconds(rem + 5)) {
            if pb.changeCount == count { pb.clearContents() }
        }
        postNotification("OTP 복사됨", "\(acc.issuer) · \(acc.name)", "\(pretty(code))   (\(rem)초 남음, 만료 후 자동 삭제)")
    }

    @objc func reloadIndex() { rebuildMenu() }

    @objc func openFolder() {
        try? FileManager.default.createDirectory(at: configDir, withIntermediateDirectories: true)
        NSWorkspace.shared.open(configDir)
    }

    @objc func importQR() {
        let panel = NSOpenPanel()
        panel.title = "Google OTP 내보내기 QR 또는 otpauth QR 이미지 선택"
        panel.allowedContentTypes = [.png, .jpeg, .heic, .tiff]
        panel.allowsMultipleSelection = true
        NSApp.activate(ignoringOtherApps: true)
        guard panel.runModal() == .OK else { return }
        var log = ""
        for url in panel.urls {
            // 경로를 셸 문자열에 끼워 넣지 않고 인자 배열로 전달한다(명령 주입 방지)
            let (_, data, err) = runOTP(["import", url.path])
            log += (String(data: data, encoding: .utf8) ?? "") + err
        }
        let alert = NSAlert()
        alert.messageText = "가져오기 결과"
        alert.informativeText = log.isEmpty ? "출력이 없습니다. 터미널에서 `otp import <이미지>`를 실행해 확인하십시오." : log
        alert.runModal()
        rebuildMenu()
    }
}

// 검증용: `OTPBar --print` 는 메뉴를 띄우지 않고 모든 계정의 현재 코드를 출력한 뒤 종료한다.
// CLI(otp list)와 같은 값이 나오면 Keychain 접근과 TOTP 구현이 일치함을 뜻한다.
if CommandLine.arguments.contains("--print") {
    let accs = loadAccounts()
    if accs.isEmpty && !lastLoadError.isEmpty { FileHandle.standardError.write(lastLoadError.data(using: .utf8)!) }
    for acc in accs {
        print("\(acc.id)\t\(totp(acc) ?? "SECRET_ERROR")\t\(remaining(acc.period))s")
    }
    exit(0)
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
