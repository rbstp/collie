import Foundation

struct TerminalTheme: Equatable, Sendable {
    var background: TerminalRGB
    var foreground: TerminalRGB
    /// The 16 ANSI colors; without them, libghostty's.
    var ansi: [TerminalRGB]?
    var minimumContrast = 1.0

    static let dark = TerminalTheme(background: TerminalRGB(0x28, 0x2C, 0x34), foreground: TerminalRGB(0xFF, 0xFF, 0xFF))

    // Each ANSI color reads at 4.8:1 or more on the background. Agents color text for the Mac's
    // terminal, usually dark, and 256-color and truecolor text stays as sent, so text under the
    // minimum contrast with its cell (light gray, white) is darkened, or lightened on a dark cell.
    static let light = TerminalTheme(
        background: TerminalRGB(0xFA, 0xFA, 0xFA),
        foreground: TerminalRGB(0x38, 0x3A, 0x42),
        ansi: [
            0x383A42, 0xC91B1B, 0x2E7D32, 0x8A6100, 0x2C5CC5, 0xA626A4, 0x0A7A8F, 0x696C77,
            0x5C5F6A, 0xB4231F, 0x3B7A1E, 0x9A5B00, 0x3D4FC7, 0x8E33B8, 0x00728C, 0x202227,
        ].map { TerminalRGB(UInt8($0 >> 16), UInt8($0 >> 8 & 0xFF), UInt8($0 & 0xFF)) },
        minimumContrast: 4.5
    )

    /// The opaque color of text in `style` over `background`.
    func textColor(_ style: TerminalStyle, on background: TerminalRGB) -> TerminalRGB {
        // Text in its cell's own color is hidden on purpose.
        guard style.foreground != background else { return style.foreground }
        let black = TerminalRGB(0, 0, 0)
        let white = TerminalRGB(0xFF, 0xFF, 0xFF)
        let target = black.contrast(with: background) > white.contrast(with: background) ? black : white
        var foreground = style.foreground
        // The default text on a cell it does not read on, such as an agent's dark prompt band, is what the agent drew
        // for a dark terminal's white text: raised only to the minimum, it read as a dim placeholder.
        if foreground == self.foreground && foreground.contrast(with: background) < minimumContrast {
            foreground = target
        }
        let color = style.faint ? foreground.mixed(with: background, 0.5) : foreground
        guard color.contrast(with: background) < minimumContrast else { return color }
        var (low, high) = (0.0, 1.0)
        for _ in 0..<10 {
            let middle = (low + high) / 2
            if color.mixed(with: target, middle).contrast(with: background) < minimumContrast {
                low = middle
            } else {
                high = middle
            }
        }
        return color.mixed(with: target, high)
    }
}

extension TerminalRGB {
    func mixed(with other: TerminalRGB, _ amount: Double) -> TerminalRGB {
        func mix(_ a: UInt8, _ b: UInt8) -> UInt8 { UInt8((Double(a) + (Double(b) - Double(a)) * amount).rounded()) }
        return TerminalRGB(mix(r, other.r), mix(g, other.g), mix(b, other.b))
    }

    func contrast(with other: TerminalRGB) -> Double {
        let (a, b) = (luminance, other.luminance)
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    private var luminance: Double {
        func linear(_ v: UInt8) -> Double {
            let c = Double(v) / 255
            return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }
}
