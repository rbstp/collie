# Liquid Glass audit

Issue #207 uses the iOS 26 and watchOS 26 glass APIs directly. The minimum versions stay at 26. System navigation, sheets, menus, pickers and search retain their native styling.

| Screen or surface | Decision |
| --- | --- |
| Agents / Usage switcher | Already uses interactive glass. Navigation and tab bars retain system glass. |
| Agent inbox | Glass applied to filter buttons. Agent rows, state badges and unread indicators stay content. Search uses the system control. |
| Agent grid | Left as content: terminal previews need a stable background. Context menus use system styling. |
| Agent session | Glass applied to the composer, terminal keys, jump and notice actions, upload and dictation controls, attachment removal and unlock action. Attachment content sits outside the glass composer. The lock notice remains content. |
| Conversation | Already uses glass for search, copy and navigation actions. Messages and code remain content. |
| Git changes / diff review | Already uses glass from #205. No changes in this audit. |
| New task | Glass applied to the composer and shared attachment, upload and dictation controls. Folder choices and task settings remain form content. |
| Approvals | Glass applied to decision buttons, with green for Approve. Actions stack at accessibility text sizes. Commands, notes and question choices remain content. |
| Saved prompts / editor | Already uses system navigation, sheets and toolbar actions. Prompt rows and editor remain content. |
| Plan usage | Left as content: usage bars, figures and reset times need stable backgrounds. |
| Machines / machine settings / pairing | Already uses system navigation, forms, sheets and toolbar actions. Machine details remain content. |
| QR scanner | Already uses system scanner UI and a native Cancel toolbar item. |
| Settings / gestures | Already uses system navigation and controls. Diagnostic values remain list content. |
| Onboarding | Glass applied to sign-in and auth-key actions. Instructions and input remain form content. |
| Watch home / agents | Already uses system lists and navigation. Agent status remains content. |
| Watch approval detail | Glass applied to Approve, Deny and question actions. Commands and status remain content. |
| Watch usage detail | Left as content: replace glass cards with a normal fill for usage figures and charts. |
| Watch complications / Smart Stack | Rectangular widget redesigned with three native `Gauge` controls using `.accessoryCircular`. Provider and percentage sit inside, the window label sits in the opening, and exact reset date and time sit below. Native markers indicate usage; the circular complication retains elapsed-time ticks. Preserve stale and missing-data states. Gauges use `widgetAccentable`; no extra glass layer. |
| Live Activity / Dynamic Island | System-hosted surface. Remove forced black background, white system actions and dark appearance from the Lock Screen activity. Status pills remain content and approval buttons keep the native widget button style. |

Adjacent custom glass controls share `GlassEffectContainer`. Controls inside a glass composer or dictation capsule do not add another glass surface. No new animation or navigation model is introduced.

Reference: [Apple's Liquid Glass guidance](https://developer.apple.com/documentation/swiftui/applying-liquid-glass-to-custom-views).

## Validation

Prioritize dark mode while retaining native appearance and accessibility behavior. Glass uses the system material and respects the user's glass setting.

- `just ios-test`: passed, including 273 app tests and 62 Ghostty tests.
- `just lint`: passed.
- Signed iPhone build, embedded watch app and widgets: built successfully and passed signature verification.
- Reviewed the final diff for approval guards, disabled states, composer behavior and widget missing/stale states. Fixed clipped terminal key labels and gauge clipping at smaller widget widths during visual review.
- Simulator fixtures covered light, dark, Increase Contrast, accessibility text sizes and actual Reduce Transparency. Approval actions stack at accessibility sizes. Existing agent header/key clipping at extreme text sizes remains outside this change.
- Native watch simulator preview and 156-point and 184-point rectangular fixtures verified gauge sizing, reset labels and missing data.

These screenshots use deterministic fixtures rendered with production views. The watch simulator image uses a temporary host app with the production gauge view, rather than a configured watch face.

| Light | Dark | Reduce Transparency |
| --- | --- | --- |
| ![Light mode before and after](images/liquid-glass-light.jpg) | ![Dark mode before and after](images/liquid-glass-dark.jpg) | ![Reduce Transparency before and after](images/liquid-glass-reduce-transparency.jpg) |

![Rectangular watch widget before and after](images/watch-gauges.jpg)

![Native watch simulator gauge preview](images/watch-native.jpg)

Physical iPhone installation succeeded. User acceptance is pending. Remaining manual checks: the iOS 27 glass setting extremes, Live Activity clear/tinted Lock Screen styles, actual watch face accenting, and physical watch readability. Simulator fixtures do not validate those system-hosted presentations.
