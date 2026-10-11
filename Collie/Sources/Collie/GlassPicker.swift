import SwiftUI

struct GlassPicker<Selection: Hashable>: View {
    let title: LocalizedStringKey
    @Binding var selection: Selection
    let options: [(LocalizedStringKey, Selection)]
    var expands = false

    var body: some View {
        Picker(title, selection: $selection) {
            ForEach(options.indices, id: \.self) { index in
                Text(options[index].0).tag(options[index].1)
            }
        }
        .pickerStyle(.segmented)
        .controlSize(.large)
        .tint(Color.accentColor)
        .fixedSize(horizontal: !expands, vertical: true)
    }
}
