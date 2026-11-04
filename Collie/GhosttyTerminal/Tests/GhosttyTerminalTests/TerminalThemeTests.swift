import Testing
#if canImport(UIKit)
import UIKit
#endif

@testable import GhosttyTerminal

private let light = TerminalTheme.light

@Test func lightAnsiColorsReadOnTheLightBackground() throws {
    let ansi = try #require(light.ansi)
    #expect(ansi.count == 16)
    for color in ansi + [light.foreground] {
        #expect(color.contrast(with: light.background) >= 4.5)
    }
}

@Test func themeSetsThePaletteAndSwitchesBack() throws {
    let screen = try #require(TerminalScreen(theme: .dark))
    let darkRed = try #require(screen.render(ansiSnapshot: "\u{1B}[31mx").runs.first?.style.foreground)

    screen.setColors(light)
    let frame = screen.render(ansiSnapshot: "\u{1B}[31mx\u{1B}[38;5;196my\u{1B}[0mz")
    #expect(frame.background == light.background)
    #expect(frame.runs.map(\.style.foreground) == [light.ansi?[1], TerminalRGB(255, 0, 0), light.foreground])

    screen.setColors(.dark)
    #expect(screen.render(ansiSnapshot: "\u{1B}[31mx").runs.first?.style.foreground == darkRed)
}

@Test func lightThemeRaisesTextToTheMinimumContrast() {
    // Claude Code's secondary gray and Codex's light text, as sent for a dark terminal.
    for sent in [TerminalRGB(153, 153, 153), TerminalRGB(230, 229, 229)] {
        let color = light.textColor(TerminalStyle(foreground: sent), on: light.background)
        #expect(color.contrast(with: light.background) >= 4.5)
        #expect(color.contrast(with: light.background) < 5)
        #expect(color.r == color.g || sent.r != sent.g)
    }
    // The default foreground on Claude Code's dark prompt band draws white, as in a dark terminal.
    let band = TerminalRGB(55, 55, 55)
    #expect(light.textColor(TerminalStyle(foreground: light.foreground), on: band) == TerminalRGB(255, 255, 255))
    #expect(light.textColor(TerminalStyle(foreground: light.foreground, faint: true), on: band).contrast(with: band) >= 4.5)
    let faint = light.textColor(TerminalStyle(foreground: light.foreground, faint: true), on: light.background)
    #expect(faint.contrast(with: light.background) >= 4.5)
    #expect(faint != light.foreground)
}

@Test func hiddenTextAndTheDarkThemeKeepTheirColors() {
    #expect(light.textColor(TerminalStyle(foreground: light.background), on: light.background) == light.background)
    let dark = TerminalTheme.dark
    let gray = TerminalRGB(80, 80, 80)
    #expect(dark.textColor(TerminalStyle(foreground: gray), on: dark.background) == gray)
    #expect(dark.textColor(TerminalStyle(foreground: dark.foreground, faint: true), on: dark.background) == dark.foreground.mixed(with: dark.background, 0.5))
}

#if canImport(UIKit)
@MainActor @Test func terminalFollowsTheAppearanceLive() throws {
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 320, height: 200))
    window.overrideUserInterfaceStyle = .dark
    let view = GhosttyTerminalUIView(fontSize: 13)
    view.frame = window.bounds
    window.addSubview(view)
    window.isHidden = false
    view.updateTraitsIfNeeded()
    view.show(ansiSnapshot: "\u{1B}[31mx")
    #expect(view.frameData?.background == TerminalTheme.dark.background)

    window.overrideUserInterfaceStyle = .light
    view.updateTraitsIfNeeded()
    #expect(view.frameData?.background == light.background)
    #expect(view.frameData?.runs.first?.style.foreground == light.ansi?[1])
    #expect(view.backgroundColor?.resolvedColor(with: view.traitCollection) == UIColor(light.background))
}
#endif
