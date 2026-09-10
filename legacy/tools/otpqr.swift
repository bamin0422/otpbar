// otpqr: 이미지 파일에서 QR 코드를 읽어 내용(payload)을 한 줄씩 출력한다.
//        --encode <text> <out.png> 로 시험용 QR 이미지를 만들 수도 있다.
// 빌드: swiftc -O tools/otpqr.swift -o bin/otpqr
import AppKit
import CoreImage
import Foundation
import Vision

func fail(_ msg: String, code: Int32 = 1) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(code)
}

func loadCGImage(_ path: String) -> CGImage {
    let url = URL(fileURLWithPath: (path as NSString).expandingTildeInPath)
    guard let src = CGImageSourceCreateWithURL(url as CFURL, nil),
          let img = CGImageSourceCreateImageAtIndex(src, 0, nil) else {
        fail("이미지를 열 수 없습니다: \(path)")
    }
    return img
}

func decodeQR(_ path: String) -> [String] {
    let image = loadCGImage(path)
    let request = VNDetectBarcodesRequest()
    request.symbologies = [.qr]
    let handler = VNImageRequestHandler(cgImage: image, options: [:])
    do { try handler.perform([request]) } catch { fail("QR 인식 중 오류: \(error)") }
    let payloads = (request.results ?? []).compactMap { $0.payloadStringValue }
    return Array(NSOrderedSet(array: payloads)) as! [String]
}

func encodeQR(_ text: String, to path: String) {
    guard let filter = CIFilter(name: "CIQRCodeGenerator") else { fail("CIQRCodeGenerator 없음") }
    filter.setValue(text.data(using: .utf8), forKey: "inputMessage")
    filter.setValue("M", forKey: "inputCorrectionLevel")
    guard let out = filter.outputImage else { fail("QR 생성 실패") }
    let scaled = out.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
    let rep = NSCIImageRep(ciImage: scaled)
    let nsimg = NSImage(size: rep.size)
    nsimg.addRepresentation(rep)
    guard let tiff = nsimg.tiffRepresentation, let bmp = NSBitmapImageRep(data: tiff),
          let png = bmp.representation(using: .png, properties: [:]) else { fail("PNG 변환 실패") }
    do { try png.write(to: URL(fileURLWithPath: (path as NSString).expandingTildeInPath)) } catch { fail("저장 실패: \(error)") }
    print("wrote \(path)")
}

let args = CommandLine.arguments
if args.count >= 4 && args[1] == "--encode" {
    encodeQR(args[2], to: args[3])
    exit(0)
}
guard args.count == 2, !args[1].hasPrefix("-") else {
    fail("사용법: otpqr <이미지>  |  otpqr --encode <텍스트> <출력.png>", code: 2)
}
let found = decodeQR(args[1])
if found.isEmpty { fail("QR 코드를 찾지 못했습니다: \(args[1])") }
for p in found { print(p) }
