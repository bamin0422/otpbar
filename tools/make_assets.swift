// make_assets: 앱 아이콘(.icns 원본 PNG들)과 README 배너를 그린다. 외부 의존성 없음.
// 실행: swift tools/make_assets.swift   (또는 swiftc -O 후 실행)
// 산출: assets/icon-1024.png, assets/icon-256.png, assets/AppIcon.iconset/*, assets/banner.png
import AppKit
import Foundation

let root = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")
let assets = root.appendingPathComponent("assets")
try? FileManager.default.createDirectory(at: assets, withIntermediateDirectories: true)

// 색: 토스 계열 cool navy / grey (sRGB 근사)
let navy = NSColor(srgbRed: 0.118, green: 0.141, blue: 0.200, alpha: 1)      // #1E2433
let navyDeep = NSColor(srgbRed: 0.086, green: 0.106, blue: 0.157, alpha: 1)  // #161B28
let grey50 = NSColor(srgbRed: 0.969, green: 0.973, blue: 0.980, alpha: 1)    // #F7F8FA
let grey200 = NSColor(srgbRed: 0.894, green: 0.906, blue: 0.925, alpha: 1)   // #E4E7EC
let grey700 = NSColor(srgbRed: 0.361, green: 0.400, blue: 0.459, alpha: 1)   // #5C6675
let blue500 = NSColor(srgbRed: 0.192, green: 0.510, blue: 0.965, alpha: 1)   // #3182F6

func render(width: Int, height: Int, _ draw: (CGRect) -> Void) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height, bitsPerSample: 8,
                               samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
                               bytesPerRow: 0, bitsPerPixel: 0)!
    rep.size = NSSize(width: width, height: height)
    NSGraphicsContext.saveGraphicsState()
    let ctx = NSGraphicsContext(bitmapImageRep: rep)!
    NSGraphicsContext.current = ctx
    ctx.cgContext.setShouldAntialias(true)
    ctx.cgContext.interpolationQuality = .high
    draw(CGRect(x: 0, y: 0, width: width, height: height))
    ctx.flushGraphics()
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

func savePNG(_ rep: NSBitmapImageRep, _ name: String) {
    let url = assets.appendingPathComponent(name)
    try! rep.representation(using: .png, properties: [:])!.write(to: url)
    print("wrote \(url.path)")
}

func font(_ size: CGFloat, weight: NSFont.Weight = .regular) -> NSFont {
    let names = weight == .bold || weight == .semibold ? ["Pretendard Variable", "Pretendard", "Apple SD Gothic Neo"]
                                                        : ["Pretendard Variable", "Pretendard", "Apple SD Gothic Neo"]
    for n in names {
        if let f = NSFont(name: n, size: size) {
            let traits: NSFontDescriptor.SymbolicTraits = (weight == .bold || weight == .semibold) ? [.bold] : []
            let d = f.fontDescriptor.withSymbolicTraits(traits)
            return NSFont(descriptor: d, size: size) ?? f
        }
    }
    return NSFont.systemFont(ofSize: size, weight: weight)
}

func symbol(_ name: String, points: CGFloat, weight: NSFont.Weight, color: NSColor) -> NSImage {
    let cfg = NSImage.SymbolConfiguration(pointSize: points, weight: weight).applying(.init(paletteColors: [color]))
    return NSImage(systemSymbolName: name, accessibilityDescription: nil)!.withSymbolConfiguration(cfg)!
}

// ---------- 앱 아이콘 ----------
// 1024 캔버스: 라운드 사각 배경(navy) + 흰 열쇠 + 하단 6자리 코드 점
func drawIcon(_ r: CGRect) {
    let s = r.width
    let inset = s * 0.03
    let bg = NSBezierPath(roundedRect: r.insetBy(dx: inset, dy: inset), xRadius: s * 0.225, yRadius: s * 0.225)
    navy.setFill()
    bg.fill()
    // 상단 미세 하이라이트(평면 느낌 유지, 아주 옅게)
    let hl = NSBezierPath(roundedRect: r.insetBy(dx: inset, dy: inset), xRadius: s * 0.225, yRadius: s * 0.225)
    NSColor.white.withAlphaComponent(0.035).setFill()
    hl.fill()
    // 열쇠
    let key = symbol("key.horizontal.fill", points: s * 0.42, weight: .bold, color: .white)
    let ks = key.size
    let scale = (s * 0.62) / ks.width
    let kw = ks.width * scale, kh = ks.height * scale
    key.draw(in: CGRect(x: (s - kw) / 2, y: s * 0.44, width: kw, height: kh), from: .zero, operation: .sourceOver, fraction: 1)
    // 6자리 코드 점 (3+3)
    let dot = s * 0.052, gap = s * 0.028, group = s * 0.06
    let total = dot * 6 + gap * 4 + group
    var x = (s - total) / 2
    let y = s * 0.27
    for i in 0..<6 {
        let p = NSBezierPath(ovalIn: CGRect(x: x, y: y, width: dot, height: dot))
        (i < 3 ? NSColor.white : blue500).setFill()
        p.fill()
        x += dot + (i == 2 ? group : gap)
    }
}

let icon1024 = render(width: 1024, height: 1024) { drawIcon($0) }
savePNG(icon1024, "icon-1024.png")

// iconset (macOS): 크기별 재렌더링(리샘플링 대신 벡터 재그리기)
let iconset = assets.appendingPathComponent("AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try! FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for (base, names) in [(16, ["icon_16x16.png"]), (32, ["icon_16x16@2x.png", "icon_32x32.png"]),
                      (64, ["icon_32x32@2x.png"]), (128, ["icon_128x128.png"]),
                      (256, ["icon_128x128@2x.png", "icon_256x256.png"]), (512, ["icon_256x256@2x.png", "icon_512x512.png"]),
                      (1024, ["icon_512x512@2x.png"])] {
    let rep = render(width: base, height: base) { drawIcon($0) }
    let data = rep.representation(using: .png, properties: [:])!
    for n in names { try! data.write(to: iconset.appendingPathComponent(n)) }
}
print("wrote \(iconset.path)")
savePNG(render(width: 256, height: 256) { drawIcon($0) }, "icon-256.png")
savePNG(render(width: 64, height: 64) { drawIcon($0) }, "icon-64.png")

// ---------- README 배너 (1280×640) ----------
func drawBanner(_ r: CGRect) {
    grey50.setFill()
    r.fill()
    // 아이콘
    let iconRect = CGRect(x: 96, y: 200, width: 300, height: 300)
    NSGraphicsContext.saveGraphicsState()
    let t = NSAffineTransform()
    t.translateX(by: iconRect.minX, yBy: iconRect.minY)
    t.scale(by: iconRect.width / 1024)
    t.concat()
    drawIcon(CGRect(x: 0, y: 0, width: 1024, height: 1024))
    NSGraphicsContext.restoreGraphicsState()
    // 텍스트
    func text(_ s: String, _ f: NSFont, _ c: NSColor, x: CGFloat, y: CGFloat) {
        let a: [NSAttributedString.Key: Any] = [.font: f, .foregroundColor: c]
        NSAttributedString(string: s, attributes: a).draw(at: NSPoint(x: x, y: y))
    }
    text("OTPBar", font(92, weight: .bold), navy, x: 468, y: 452)
    text("구글 OTP를 메뉴바 · 트레이 · CLI로", font(36, weight: .semibold), grey700, x: 470, y: 396)
    text("macOS  ·  Windows  ·  Homebrew  ·  Claude Code 자동화", font(24), grey700, x: 472, y: 352)
    // 메뉴 미리보기 카드
    let card = CGRect(x: 468, y: 128, width: 716, height: 190)
    let cp = NSBezierPath(roundedRect: card, xRadius: 20, yRadius: 20)
    NSColor.white.setFill(); cp.fill()
    grey200.setStroke(); cp.lineWidth = 2; cp.stroke()
    let rows: [(String, String, String, Bool)] = [("GitHub · bamin0422", "483 921", "22s", true),
                                                   ("Jira · me@company", "107 336", "22s", false),
                                                   ("Google · me@gmail.com", "590 214", "22s", false)]
    var ry = card.maxY - 56
    let mono = NSFont.monospacedDigitSystemFont(ofSize: 26, weight: .semibold)
    for (label, code, rem, active) in rows {
        if active {
            let hp = NSBezierPath(roundedRect: CGRect(x: card.minX + 14, y: ry - 12, width: card.width - 28, height: 50), xRadius: 12, yRadius: 12)
            navy.setFill(); hp.fill()
        }
        let fg = active ? NSColor.white : navy
        text(label, font(26), fg, x: card.minX + 30, y: ry)
        let ca: [NSAttributedString.Key: Any] = [.font: mono, .foregroundColor: fg]
        NSAttributedString(string: code, attributes: ca).draw(at: NSPoint(x: card.minX + 470, y: ry))
        text(rem, font(22), active ? NSColor.white.withAlphaComponent(0.8) : grey700, x: card.minX + 620, y: ry + 2)
        ry -= 58
    }
}
savePNG(render(width: 1280, height: 640) { drawBanner($0) }, "banner.png")
