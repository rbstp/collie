import Testing
import UIKit

@testable import Collie

@MainActor @Test func windowIsCoveredInTheBackgroundAndNotWhileInactive() async throws {
    let center = NotificationCenter()
    WindowCover.observe(center)
    try await hosting(UIViewController()) { window in
        let scene = try #require(window.windowScene)
        window.overrideUserInterfaceStyle = .light
        let content = UIView(frame: window.bounds)
        content.backgroundColor = .red
        window.addSubview(content)
        defer { content.removeFromSuperview() }
        func covers() -> [WindowCover] { window.subviews.compactMap { $0 as? WindowCover } }
        func centerIsRed() -> Bool {
            let shot = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
            }
            var pixel = [UInt8](repeating: 0, count: 4)
            pixel.withUnsafeMutableBytes { bytes in
                guard let middle = shot.cgImage.flatMap({ $0.cropping(to: CGRect(x: $0.width / 2, y: $0.height / 2, width: 1, height: 1)) }) else { return }
                CGContext(data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                          space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)?
                    .draw(middle, in: CGRect(x: 0, y: 0, width: 1, height: 1))
            }
            return pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80
        }

        center.post(name: UIScene.willDeactivateNotification, object: scene)
        #expect(covers().isEmpty)
        #expect(centerIsRed())

        // No await from here to the last check: iOS takes its snapshot as soon as the notification's observers return.
        center.post(name: UIScene.didEnterBackgroundNotification, object: scene)
        center.post(name: UIScene.didEnterBackgroundNotification, object: scene)
        #expect(covers().count == 1)
        #expect(covers().first?.frame == window.bounds)
        let presentedLater = UIView(frame: window.bounds)
        presentedLater.backgroundColor = .red
        window.addSubview(presentedLater)
        defer { presentedLater.removeFromSuperview() }
        #expect(!centerIsRed())

        center.post(name: UIScene.willEnterForegroundNotification, object: scene)
        #expect(covers().isEmpty)
    }
}
