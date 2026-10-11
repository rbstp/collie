import SwiftUI
import UIKit

struct GlassPicker<Selection: Hashable>: View {
    let title: String
    @Binding var selection: Selection
    let options: [(String, Selection)]
    var expands = false

    var body: some View {
        NativeSegmentedPicker(title: title, selection: $selection, options: options, expands: expands)
            .foregroundStyle(Color.accentColor)
            .glassEffect(expands ? .regular : .identity)
    }
}

private struct NativeSegmentedPicker<Selection: Hashable>: UIViewRepresentable {
    let title: String
    @Binding var selection: Selection
    let options: [(String, Selection)]
    var expands = false
    @ScaledMetric(relativeTo: .subheadline) private var fontSize = 15
    @Environment(\.controlSize) private var controlSize

    func makeUIView(context: Context) -> UISegmentedControl {
        let control = UISegmentedControl()
        control.selectedSegmentTintColor = UIColor { traits in
            traits.userInterfaceStyle == .dark ? .black : UIColor(white: 0.9, alpha: 1)
        }
        control.addTarget(context.coordinator, action: #selector(Coordinator.changed(_:)), for: .valueChanged)
        return control
    }

    func updateUIView(_ control: UISegmentedControl, context: Context) {
        context.coordinator.parent = self
        if control.numberOfSegments != options.count {
            control.removeAllSegments()
            for index in options.indices { control.insertSegment(withTitle: "", at: index, animated: false) }
        }
        let font = UIFont.systemFont(ofSize: fontSize, weight: .semibold)
        let accent = UIColor(Color(Color.accentColor.resolve(in: context.environment)))
        control.setTitleTextAttributes([.font: font, .foregroundColor: UIColor.label], for: .normal)
        control.setTitleTextAttributes([.font: font, .foregroundColor: accent], for: .selected)
        for index in options.indices {
            let label = String(localized: String.LocalizationValue(options[index].0), locale: context.environment.locale)
            control.setTitle(label, forSegmentAt: index)
            control.setWidth(expands ? 0 : ceil((label as NSString).size(withAttributes: [.font: font]).width) + 28, forSegmentAt: index)
        }
        control.selectedSegmentIndex = options.firstIndex { $0.1 == selection } ?? UISegmentedControl.noSegment
        control.isEnabled = context.environment.isEnabled
        control.accessibilityLabel = String(localized: String.LocalizationValue(title), locale: context.environment.locale)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UISegmentedControl, context: Context) -> CGSize? {
        CGSize(width: expands ? proposal.width ?? uiView.intrinsicContentSize.width : uiView.intrinsicContentSize.width,
               height: controlSize == .small ? max(34, fontSize + 16) : max(44, fontSize + 24))
    }

    func makeCoordinator() -> Coordinator { Coordinator(parent: self) }

    final class Coordinator: NSObject {
        var parent: NativeSegmentedPicker

        init(parent: NativeSegmentedPicker) { self.parent = parent }

        @objc func changed(_ control: UISegmentedControl) {
            guard parent.options.indices.contains(control.selectedSegmentIndex) else { return }
            parent.selection = parent.options[control.selectedSegmentIndex].1
        }
    }
}
