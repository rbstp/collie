# Liquid Glass audit

Issue #207 uses the iOS 26 and watchOS 26 glass APIs directly, with minimum versions at 26. System navigation, toolbars, sheets, menus, pickers and search keep native styling. Prioritize dark mode, keeping native appearance and accessibility; glass uses the system material and respects the user's glass setting.

- **Agent inbox:** segmented filters, then the search field on its own glass, in a top inset the rows scroll under. Rows, state badges and unread indicators stay content.
- **Agent grid:** cards stay content, as terminal previews need a stable background; context menus use system styling. A preview's running subagents overlay matches the session header (clear glass over its dark dimming layer, always dark, white text).
- **Every Agents page** (grid, list, inbox, Usage): navigation and tab bars keep system glass with no scroll edge effect, glass or dimming behind them, so content runs under both at full clarity, leaving only the picker, buttons and tab bar, each on its own glass.
- **Approvals:** glass on decision buttons, green for Approve; commands, notes and question choices stay content.
- **Conversation:** glass on search, copy and navigation actions; messages and code stay content.
- **Git changes / diff review:** glass from #205, unchanged.
- **New task:** glass on the composer and shared attachment, upload and dictation controls; folder choices and task settings stay form content.
- **Saved prompts / editor:** plain row buttons (no blue tint dimming prompt text) and larger previews; rows and editor stay content.
- **Plan usage:** content; bars, figures and reset times need stable backgrounds.
- **Machines / machine settings / pairing:** system forms; machine details stay content.
- **QR scanner:** system scanner UI and a native Cancel toolbar item.
- **Settings / gestures:** system switches and a selectable accent color; diagnostic values stay list content.
- **Onboarding:** glass on sign-in and auth-key actions; instructions and input stay form content.
- **Watch app and widgets:** native `List`, `NavigationLink` and `Gauge` throughout, none needing replacement; agent status, commands and approval status stay content. Approve uses `.glassProminent`, Deny and question actions `.glass`.
- **Watch usage detail:** compact native list of logo gauges, window labels, percentages and reset countdowns; tapping a window shows its countdown, reading age and elapsed percentage; no glass on content.
- **Watch complications / Smart Stack:** the rectangular widget shows three native `.accessoryCircular` gauges, provider inside and bold reset countdown below; no window labels, for gauge space (the app shows percentages after a tap). A value-based tint dims the unused arc but keeps the native usage marker; the circular complication keeps elapsed-time ticks. Preserve stale and missing-data states. Gauges use `widgetAccentable` and no extra glass.
- **Live Activity / Dynamic Island:** system-hosted; no forced black background, white system actions or dark appearance on the Lock Screen. Status pills stay content; approval buttons keep the native widget style.

## Agent session

- The bold inline title wraps to two lines, cut without an ellipsis.
- The header (under the navigation bar, with the subagent list) is full-width, edge-to-edge clear glass over a dimming layer, 70% black in dark mode and 80% white in light, with content running under it and no scroll edge effect under the navigation bar, so it reads as one glass area.
- All header text, subagent time and tokens included, uses the primary color; secondary gray is hard to read on clear glass.
- Over the always-dark terminal, the header, composer controls and navigation bar are dark (header and navigation bar only while the terminal runs under them).
- The composer has no band; content shows around its glass controls. Its notice, error, retry and dictation problem lines each get their own regular glass, and each such control and line has the header's dimming color behind its glass, in its own shape (small glass alone left busy content hard to read in dark mode).
- Attachment content sits outside the glass composer; the lock notice stays content.

## In-session approvals

- The cards form a full-width, edge-to-edge band at the bottom of the session header, sharing its one clear glass and dimming layer (two edge-to-edge glass layers show a seam). Like the header, it is dark over the terminal, following the appearance over Chat.
- Text caps at xxLarge, one below the largest standard size, in compact footnote and caption styles with small decision buttons.
- The cards together take at most about a fifth of the whole screen's height, keyboard or not (21%; a quarter in landscape, so the prompt fits above Approve). With the keyboard up, they give way to what is left above the composer.
- The prompt scrolls inside its card above pinned Approve and Deny; a prompt cut at the card's edge fades over its last 16 points to read as scrollable. If the card would leave the prompt under two command lines (tight landscape, keyboard up), Approve and Deny scroll with it.
- The countdown pill is blocked red; the agent's reason sits under the command on a light orange fill with a shield symbol.

## Controls

- **Glass placement.** Glass covers the composer, terminal keys, jump and notice actions, upload and dictation controls, attachment removal and the unlock action. Standalone terminal keys, attachments, dictation, send, keyboard dismissal and attachment removal use native glass button styles; approval disclosure is a native button. Adjacent custom glass controls share `GlassEffectContainer`; nothing inside a glass composer or dictation capsule adds more glass.
- **Status pills and copy buttons.** App status pills (connection, session header, approval countdown) and the connection error and prompt copy buttons use regular glass without a dark fill. On the header, approval band and machine rows, they and the band's decision buttons sit glass on glass.
- **Attachment button.** Its native menu stays stable while open and presents Saved prompts, Files or Photo Library after it dismisses. Its 16-point paperclip uses the chosen accent; its glass circle matches the 38-point dictation, keyboard and send circles; its 44-point hit target extends past it. Pickers open locally, not waiting for the machine; the upload chip distinguishes preparing, waiting for the machine and uploading.
- **Wrap lines.** A toolbar button, not a button-style toggle, on the shared glass capsule; its icon is accent when on and primary when off, like other glass toolbar icons, and its title carries the state (Wrap lines on or off), as bar buttons pass only their title to accessibility.
- **Segmented controls** (Agents/Usage, inbox filters, Usage costs range, Chat/Terminal). Native, with semibold labels, accent-colored selected text and the system's sliding glass selection, never a forced black or white segment. UIKit leaves a tapped toolbar control's selected label in its pressed tint, so once the press lens settles the control is re-added to its window, restoring the accent and keeping its frame. Dropping UIKit's one-point bottom alignment inset, which the controls draw over, makes Agents/Usage match the capsule beside it in height.
- **Chat/Terminal.** Compact and icon-only (speech bubble, Ghostty icon) at the right of the session header, with 44-point segments and VoiceOver labels Chat and Terminal. No glass backing: extra glass beside other glass left keyboard animations unfinished after a picker closed.
- **Agents/Usage paging.** Native interactive page scrolling reveals Usage progressively during a drag rather than switching on release; selector-triggered transitions respect Reduce Motion.
- **Dictation waveform.** An accent-colored translucent gradient, solid with Reduce Transparency or Increase Contrast, fills the capsule width with recent microphone levels (silence and repeats included) on a more sensitive scale; its history clears when dictation stops or switches language.

Reference: [Apple's Liquid Glass guidance](https://developer.apple.com/documentation/swiftui/applying-liquid-glass-to-custom-views).

## Validation

- `just ios-test`: passed (274 app tests, 62 Ghostty tests).
- `just lint`: passed. The later Codex prompt reflow change also passed all 334 `collied` tests, targeted Clippy and formatting; its regression test fails before the fix and passes after.
- Signed iPhone build, embedded watch app and widgets: built and passed signature verification.
- Diff review covered approval guards, disabled states, composer behavior and widget missing and stale states; visual review fixed clipped terminal key labels. Physical watch feedback exposed a widget-only collapse in the geometry-scaled gauges, fixed by removing that transform and sharing the plain native gauge with usage detail.
- Simulator fixtures covered light, dark, Increase Contrast, accessibility text sizes, actual Reduce Transparency, and native paging at a partial offset (both pages visible) and settled on Usage. Approval actions stack at accessibility sizes. Existing agent header and key clipping at extreme text sizes is out of scope.
- Dictation fixtures cover dark and light, a narrow bar and accessibility text with Increase Contrast; session tests verify speech and silence history, its size bound, and clearing on stop, cancellation and language change. The user accepted the latest phone build after the microphone sensitivity adjustment.
- Native watch simulator previews and rectangular fixtures cover reset countdowns and missing data. Standalone previews missed the physical widget layout failure; physical feedback confirmed the fix and approved the compact usage list. The user accepted the final refinements (dimmed unused arc, bold countdowns) on the physical watch.

Screenshots render deterministic fixtures with production views; the watch simulator image hosts the production gauge view in a temporary app, not a configured watch face.

| Light | Dark | Reduce Transparency |
| --- | --- | --- |
| ![Light mode before and after](images/liquid-glass-light.jpg) | ![Dark mode before and after](images/liquid-glass-dark.jpg) | ![Reduce Transparency before and after](images/liquid-glass-reduce-transparency.jpg) |

![Rectangular watch widget before and after](images/watch-gauges.jpg)

![Native watch simulator gauge preview](images/watch-native.jpg)

![Watch usage detail with native gauges](images/watch-detail.jpg)

![Blue selectors and switches, progressive paging and readable prompts](images/controls-dark.jpg)

![Full-width dictation waveform in the glass capsule](images/dictation-waveform.jpg)

The latest signed build was installed and launched on the physical iPhone. The signed Mac companion, with the Codex-only prompt reflow fix, was updated and restarted, and status and doctor checks passed. Physical photos confirmed the watch update transferred and the Plan Usage layout was accepted. The user authorized auto-merge. Still manual, as Simulator fixtures cannot validate these system-hosted presentations: iOS 27 glass setting extremes, Live Activity clear and tinted Lock Screen styles, actual watch face accenting and physical watch readability.
