#if canImport(UIKit)
import CoreText
import Foundation
import Testing

@testable import GhosttyTerminal

private struct Shaped {
    let font: CTFont
    let glyph: CGGlyph
    let index: Int

    var name: String { CTFontCopyPostScriptName(font) as String }

    var bounds: CGRect {
        var glyph = glyph
        return CTFontGetBoundingRectsForGlyphs(font, .horizontal, &glyph, nil, 1)
    }
}

private func shape(_ text: String, _ font: CTFont) -> [Shaped] {
    let line = CTLineCreateWithAttributedString(
        NSAttributedString(string: text, attributes: [NSAttributedString.Key(kCTFontAttributeName as String): font])
    )
    return (CTLineGetGlyphRuns(line) as! [CTRun]).flatMap { run in
        let runFont = (CTRunGetAttributes(run) as NSDictionary)[kCTFontAttributeName as String] as! CTFont
        let count = CTRunGetGlyphCount(run)
        var glyphs = [CGGlyph](repeating: 0, count: count)
        var indices = [CFIndex](repeating: 0, count: count)
        CTRunGetGlyphs(run, CFRange(), &glyphs)
        CTRunGetStringIndices(run, CFRange(), &indices)
        return (0..<count).map { Shaped(font: runFont, glyph: glyphs[$0], index: indices[$0]) }
    }
}

private func placed(_ text: String, _ m: CellMetrics, cells: Int = 1) throws -> CGRect {
    let shaped = try #require(shape(text, m.regular).first)
    let placement = try #require(m.placement(of: text.unicodeScalars.first!.value, bounds: shaped.bounds, cells: cells))
    return shaped.bounds.applying(placement)
}

private func near(_ a: CGFloat, _ b: CGFloat) -> Bool { abs(a - b) < 0.001 }

@Test func bundledFacesLoad() {
    let m = CellMetrics(size: 13)
    for font in [m.regular, m.bold, m.italic, m.boldItalic] {
        #expect(CTFontCopyFamilyName(font) as String == "JetBrains Mono")
    }
    #expect(CTFontGetSymbolicTraits(m.bold).contains(.traitBold))
    #expect(CTFontGetSymbolicTraits(m.italic).contains(.traitItalic))
    #expect(CTFontGetSymbolicTraits(m.boldItalic).contains([.traitBold, .traitItalic]))
}

@Test func nerdGlyphsFallBackToTheBundledSymbols() {
    let m = CellMetrics(size: 13)
    for font in [m.regular, m.bold, m.italic, m.boldItalic] {
        #expect(shape("\u{F113}", font).map(\.name) == ["SymbolsNF"])
    }
}

@Test func ligaturesMapEachGlyphToItsOwnCell() {
    let m = CellMetrics(size: 13)
    let ligature = shape("->", m.regular)
    let separate = shape("-", m.regular) + shape(">", m.regular)
    #expect(ligature.map(\.index) == [0, 1])
    #expect(ligature.map(\.glyph) != separate.map(\.glyph))
}

@Test func powerlineCapsFillTheCell() throws {
    let m = CellMetrics(size: 13)
    for cap in ["\u{E0B0}", "\u{E0B4}", "\u{E0B6}"] {
        let r = try placed(cap, m)
        #expect(near(r.minX, 0) && near(r.maxX, m.width))
        #expect(near(r.minY, m.ascent - m.height) && near(r.maxY, m.ascent))
    }
}

@Test func blocksMeetBetweenRows() throws {
    let m = CellMetrics(size: 13)
    for block in ["\u{2588}", "\u{2593}"] {
        let r = try placed(block, m)
        #expect(near(r.minY, m.ascent - m.height) && near(r.maxY, m.ascent))
    }
    #expect(near(try placed("\u{2580}", m).maxY, m.ascent))
}

@Test func iconsFitAndCenterInTheirCells() throws {
    let m = CellMetrics(size: 13)
    for icon in ["\u{F418}", "\u{F43A}", "\u{E711}", "\u{F0805}"] {
        let single = try placed(icon, m)
        #expect(single.minX >= -0.001 && single.maxX <= m.width + 0.001)
        #expect(near(single.midX, m.width / 2))
        #expect(single.minY >= m.ascent - m.height && single.maxY <= m.ascent)
        let double = try placed(icon, m, cells: 2)
        #expect(double.minX >= -0.001 && double.maxX <= 2 * m.width + 0.001)
        #expect(double.height >= single.height - 0.001)
    }
    #expect(m.placement(of: 0x41, bounds: .zero, cells: 1) == nil)
}
#endif
