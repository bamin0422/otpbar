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
func loadAccounts() -> [Account] {
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/bin/zsh")
    p.arguments = ["-c", "export PATH=\"\(home)/bin:/opt/homebrew/bin:/usr/local/bin:$PATH\"; exec otp secrets --json"]
    let out = Pipe(), err = Pipe()
    p.standardOutput = out
    p.standardError = err
    do { try p.run() } catch { lastLoadError = "otp 실행 실패: \(error)"; return [] }
    p.waitUntilExit()
    let data = out.fileHandleForReading.readDataToEndOfFile()
    if p.terminationStatus != 0 {
        lastLoadError = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? "otp 오류"
        return []
    }
    lastLoadError = ""
    return (try? JSONDecoder().decode([Account].self, from: data)) ?? []
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
    }

    // 알림은 앱이 앞에 있어도 배너로 보인다
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification,
                                withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound])
    }

    func rebuildMenu() {
        menu.removeAllItems()
        codeItems.removeAll()
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
    }

    @objc func copyCode(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let acc = accounts.first(where: { $0.id == id }),
              let code = totp(acc) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(code, forType: .string)
        let content = UNMutableNotificationContent()
        content.title = "OTP 복사됨"
        content.subtitle = "\(acc.issuer) · \(acc.name)"
        content.body = "\(pretty(code))   (\(remaining(acc.period))초 남음)"
        content.sound = .default
        let req = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(req)
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
            let p = Process()
            p.executableURL = URL(fileURLWithPath: "/bin/zsh")
            let home = FileManager.default.homeDirectoryForCurrentUser.path
            p.arguments = ["-lc", "export PATH=\"\(home)/bin:/opt/homebrew/bin:$PATH\"; otp import \"\(url.path)\" 2>&1"]
            let pipe = Pipe()
            p.standardOutput = pipe
            p.standardError = pipe
            try? p.run()
            p.waitUntilExit()
            log += String(data: pipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
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
