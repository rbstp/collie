import CollieCore
import SwiftUI

struct OnboardingView: View {
    let app: AppModel
    @State private var authKey = ""
    @State private var signInTask: Task<Void, Never>?
    @State private var web = WebAuthenticator()

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("collie").font(.largeTitle.bold())
                        Text("Watch and steer the coding agents on your machines. collie joins your tailnet on its own; the Tailscale app is not needed.")
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 4)
                }

                Section {
                    Button {
                        signIn { await app.signIn(openLogin: openLogin, closeLogin: web.close) }
                    } label: {
                        Label("Sign in to Tailscale", systemImage: "person.badge.key")
                    }
                    .disabled(app.signingIn)
                } footer: {
                    Text("Opens the Tailscale login page. Sign in with the account that owns your machines.")
                }

                Section {
                    SecureField("tskey-auth-…", text: $authKey)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                    Button("Use auth key") {
                        let key = authKey
                        authKey = ""
                        signIn { await app.signIn(authKey: key) }
                    }
                    .disabled(app.signingIn || authKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                } header: {
                    Text("Auth key")
                } footer: {
                    Text("Used once to join the tailnet, never stored. Use a key for your own user, not a tagged key.")
                }

                if let guidance = OnboardingStatus(node: app.node, signingIn: app.signingIn, error: app.signInError ?? app.nodeError) {
                    Section("Status") {
                        if app.signingIn {
                            HStack {
                                ProgressView()
                                Text(guidance.message)
                            }
                            Button("Cancel", role: .cancel) {
                                signInTask?.cancel()
                            }
                        } else {
                            Text(guidance.message).foregroundStyle(guidance.isError ? .red : .primary)
                        }
                    }
                }
            }
            .navigationTitle("Welcome")
        }
        .task {
            while !Task.isCancelled && !app.isRunning {
                await app.refreshNode()
                try? await Task.sleep(for: .seconds(2))
            }
        }
    }

    private func signIn(_ work: @escaping @MainActor () async -> Void) {
        signInTask?.cancel()
        signInTask = Task { await work() }
    }

    private func openLogin(_ url: URL) {
        web.open(url) { [signInTask] in
            if !app.isRunning { signInTask?.cancel() }
        }
    }
}

struct OnboardingStatus: Equatable {
    let message: String
    let isError: Bool

    init?(node: NodeState?, signingIn: Bool, error: String?) {
        if let error {
            self.init(message: error, isError: true)
            return
        }
        switch node?.backendState {
        case .needsMachineAuth:
            self.init(
                message: "Signed in. A tailnet admin must approve this device in the Tailscale admin console (Machines) before it can connect.",
                isError: false
            )
        case .needsLogin where signingIn:
            self.init(message: "Waiting for you to finish signing in…", isError: false)
        case .noState, .starting, .stopped, .needsLogin, .notStarted, nil:
            guard signingIn else { return nil }
            self.init(message: "Connecting to Tailscale…", isError: false)
        case .running:
            self.init(message: "Connected.", isError: false)
        case .unknown:
            self.init(message: "Tailscale is in an unexpected state.", isError: true)
        }
    }

    init(message: String, isError: Bool) {
        self.message = message
        self.isError = isError
    }
}
