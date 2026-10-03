import AuthenticationServices
import UIKit

/// Tailscale's login page never redirects to collie: the session stays open until the
/// node reports Running, then it is cancelled.
@MainActor
final class WebAuthenticator: NSObject, ASWebAuthenticationPresentationContextProviding {
    private var session: ASWebAuthenticationSession?

    func open(_ url: URL, onDismiss: @escaping @MainActor () -> Void) {
        close()
        let session = ASWebAuthenticationSession(url: url, callback: .customScheme("collie-auth")) { _, _ in
            Task { @MainActor in onDismiss() }
        }
        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = true
        self.session = session
        session.start()
    }

    func close() {
        session?.cancel()
        session = nil
    }

    nonisolated func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            if let window = scenes.flatMap(\.windows).first(where: \.isKeyWindow) {
                return window
            }
            return UIWindow(windowScene: scenes[0])
        }
    }
}
