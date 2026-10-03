import CollieCore
import Foundation
import Observation

@MainActor
@Observable
final class PairingModel {
    var deviceLabel = "iPhone"
    var invite = ""
    private(set) var pairing = false
    private(set) var error: String?
    private(set) var paired: Machine?

    var trimmedLabel: String { deviceLabel.trimmingCharacters(in: .whitespacesAndNewlines) }

    var canPair: Bool {
        !pairing && !trimmedLabel.isEmpty && trimmedLabel.count <= 64
            && invite.trimmingCharacters(in: .whitespacesAndNewlines).hasPrefix("collie://pair#")
    }

    /// The invite carries a one-time code: it is cleared whatever the outcome.
    func pair(core: CollieCore?) async {
        guard let core, canPair else { return }
        let uri = invite
        invite = ""
        pairing = true
        defer { pairing = false }
        do {
            paired = try await core.pair(inviteUri: uri, deviceLabel: trimmedLabel)
            error = nil
        } catch {
            self.error = describe(error)
        }
    }
}
