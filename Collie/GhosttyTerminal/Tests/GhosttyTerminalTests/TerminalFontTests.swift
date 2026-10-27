#if canImport(UIKit)
import CoreText
import Foundation
import Testing

@testable import GhosttyTerminal

private func glyphs(_ text: String, _ font: CTFont) -> [(font: String, glyph: CGGlyph)] {
    let line = CTLineCreateWithAttributedString(
        NSAttributedString(string: text, attributes: [NSAttributedString.Key(kCTFontAttributeName as String): font])
    )
    return (CTLineGetGlyphRuns(line) as! [CTRun]).flatMap { run in
        let runFont = (CTRunGetAttributes(run) as NSDictionary)[kCTFontAttributeName as String] as! CTFont
        var glyphs = [CGGlyph](repeating: 0, count: CTRunGetGlyphCount(run))
        CTRunGetGlyphs(run, CFRange(), &glyphs)
        return glyphs.map { (CTFontCopyPostScriptName(runFont) as String, $0) }
    }
}

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
        #expect(glyphs("\u{F113}", font).map { $0.font } == ["SymbolsNF"])
    }
}

@Test func ligaturesKeepOneGlyphPerCell() {
    let m = CellMetrics(size: 13)
    let ligature = glyphs("->", m.regular).map { $0.glyph }
    let separate = (glyphs("-", m.regular) + glyphs(">", m.regular)).map { $0.glyph }
    #expect(ligature.count == 2)
    #expect(ligature != separate)
}
#endif
