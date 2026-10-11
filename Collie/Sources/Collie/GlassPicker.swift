import SwiftUI

struct GlassPicker<Selection: Hashable>: View {
    let title: LocalizedStringKey
    @Binding var selection: Selection
    let options: [(LocalizedStringKey, Selection)]
    var expands = false
    @Namespace private var pill
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: 0) {
            ForEach(options.indices, id: \.self) { index in
                let (label, value) = options[index]
                Button { selection = value } label: {
                    Text(label)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(selection == value ? AnyShapeStyle(.tint) : AnyShapeStyle(.primary))
                        .padding(.horizontal, 14)
                        .frame(maxWidth: expands ? .infinity : nil, minHeight: 38)
                        .background {
                            if selection == value {
                                Capsule()
                                    .fill(.black.opacity(colorScheme == .dark ? 0.69 : 0.07))
                                    .matchedGeometryEffect(id: "pill", in: pill)
                            }
                        }
                        .contentShape(.capsule)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(3)
        .tint(Color.accentColor)
        .glassEffect(.regular.interactive(), in: .capsule)
        .animation(.smooth(duration: 0.3), value: selection)
        .accessibilityRepresentation {
            Picker(title, selection: $selection) {
                ForEach(options.indices, id: \.self) { index in
                    Text(options[index].0).tag(options[index].1)
                }
            }
            .pickerStyle(.segmented)
        }
    }
}
