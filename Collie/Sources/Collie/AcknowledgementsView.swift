import SwiftUI

struct Acknowledgement: Decodable, Identifiable {
    let name: String
    let version: String?
    let license: String
    let text: String

    var id: String { "\(name) \(version ?? "")" }

    /// Written by scripts/notices.py when `just ios-project` generates the project.
    static let bundled: [Acknowledgement] = {
        guard let url = Bundle.main.url(forResource: "Acknowledgements", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return [] }
        return (try? JSONDecoder().decode([Acknowledgement].self, from: data)) ?? []
    }()
}

struct AcknowledgementsView: View {
    var body: some View {
        List(Acknowledgement.bundled) { item in
            NavigationLink {
                ScrollView {
                    Text(verbatim: item.text)
                        .font(.system(.caption, design: .monospaced))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding()
                }
                .navigationTitle(item.name)
                .navigationBarTitleDisplayMode(.inline)
            } label: {
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: [item.name, item.version].compactMap { $0 }.joined(separator: " "))
                    Text(verbatim: item.license).font(.subheadline).foregroundStyle(.secondary)
                }
            }
        }
        .navigationTitle("Acknowledgements")
    }
}
