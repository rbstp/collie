import SwiftUI
import UIKit

extension EnvironmentValues {
    @Entry var appAccent: Color = .accentColor
    @Entry var neutralAccent = false
}

struct GlassPicker<Selection: Hashable>: View {
    let title: String
    @Binding var selection: Selection
    let options: [(String, Selection)]
    var expands = false
    var icons: [UIImage?] = []

    var body: some View {
        NativeSegmentedPicker(title: title, selection: $selection, options: options, expands: expands, icons: icons)
            .glassEffect(expands ? .regular : .identity)
    }
}

private struct NativeSegmentedPicker<Selection: Hashable>: UIViewRepresentable {
    let title: String
    @Binding var selection: Selection
    let options: [(String, Selection)]
    var expands = false
    var icons: [UIImage?] = []
    @ScaledMetric(relativeTo: .subheadline) private var fontSize = 15
    @Environment(\.controlSize) private var controlSize

    func makeUIView(context: Context) -> SegmentedControl {
        let control = SegmentedControl()
        control.addTarget(context.coordinator, action: #selector(Coordinator.changed(_:)), for: .valueChanged)
        return control
    }

    func updateUIView(_ control: SegmentedControl, context: Context) {
        context.coordinator.parent = self
        if control.numberOfSegments != options.count {
            control.removeAllSegments()
            for index in options.indices { control.insertSegment(withTitle: "", at: index, animated: false) }
        }
        let font = UIFont.systemFont(ofSize: fontSize, weight: .semibold)
        // Keep the selected label's app accent explicit: native defaults lose it, and UIKit tint can change during actions.
        let accent = UIColor(Color(context.environment.appAccent.resolve(in: context.environment)))
        control.setTitleTextAttributes([.font: font], for: .normal)
        control.setTitleTextAttributes([.font: font, .foregroundColor: accent], for: .selected)
        for index in options.indices {
            let label = String(localized: String.LocalizationValue(options[index].0), locale: context.environment.locale)
            if let icon = icons.indices.contains(index) ? icons[index] : nil {
                let image = icon.isSymbolImage ? icon.withConfiguration(UIImage.SymbolConfiguration(font: font, scale: .small)) : icon
                image.accessibilityLabel = label
                control.setImage(image, forSegmentAt: index)
                control.setWidth(expands ? 0 : 44, forSegmentAt: index)
            } else {
                control.setTitle(label, forSegmentAt: index)
                control.setWidth(expands ? 0 : ceil((label as NSString).size(withAttributes: [.font: font]).width) + 28, forSegmentAt: index)
            }
        }
        control.selectedSegmentIndex = options.firstIndex { $0.1 == selection } ?? UISegmentedControl.noSegment
        control.isEnabled = context.environment.isEnabled
        control.accessibilityLabel = String(localized: String.LocalizationValue(title), locale: context.environment.locale)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: SegmentedControl, context: Context) -> CGSize? {
        CGSize(width: expands ? proposal.width ?? uiView.intrinsicContentSize.width : uiView.intrinsicContentSize.width,
               height: controlSize == .small ? max(34, fontSize + 16) : max(44, fontSize + 24))
    }

    func makeCoordinator() -> Coordinator { Coordinator(parent: self) }

    @MainActor
    final class Coordinator: NSObject {
        var parent: NativeSegmentedPicker

        init(parent: NativeSegmentedPicker) { self.parent = parent }

        @objc func changed(_ control: UISegmentedControl) {
            guard parent.options.indices.contains(control.selectedSegmentIndex) else { return }
            parent.selection = parent.options[control.selectedSegmentIndex].1
        }
    }
}

/// In a toolbar, UIKit leaves the press lens over the selection after a tap, which keeps the selected label
/// in its pressed tint until the control leaves the window. Re-adding it once the lens has settled resets it;
/// the frame is restored because UIKit grows the re-added control to its own height.
private final class SegmentedControl: UISegmentedControl {
    private var activeTouches = 0
    private var reset: DispatchWorkItem?

    // UIKit's bottom alignment inset is drawn over, which left the control a point taller and lower than the glass beside it.
    override var alignmentRectInsets: UIEdgeInsets { .zero }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent?) {
        activeTouches += touches.count
        reset?.cancel()
        super.touchesBegan(touches, with: event)
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesEnded(touches, with: event)
        released(touches)
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent?) {
        super.touchesCancelled(touches, with: event)
        released(touches)
    }

    private func released(_ touches: Set<UITouch>) {
        activeTouches = max(0, activeTouches - touches.count)
        guard activeTouches == 0 else { return }
        reset?.cancel()
        let reset = DispatchWorkItem { [weak self] in
            guard let self, let superview else { return }
            let frame = frame
            removeFromSuperview()
            superview.addSubview(self)
            self.frame = frame
        }
        self.reset = reset
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.6, execute: reset)
    }
}
