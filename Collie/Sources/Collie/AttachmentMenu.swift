import SwiftUI
import UIKit

enum AttachmentChoice: CaseIterable {
    case savedPrompts, files, photos

    var title: String {
        switch self {
        case .savedPrompts: "Saved prompts"
        case .files: "Files"
        case .photos: "Photo Library"
        }
    }

    var symbol: String {
        switch self {
        case .savedPrompts: "text.badge.plus"
        case .files: "folder"
        case .photos: "photo.on.rectangle"
        }
    }
}

struct AttachmentMenu: View {
    var savedPromptsEnabled = true
    var filesEnabled = true
    let select: (AttachmentChoice) -> Void

    var body: some View {
        AttachmentMenuControl(savedPromptsEnabled: savedPromptsEnabled, filesEnabled: filesEnabled, select: select)
            .padding(-AttachmentMenuButton.outset)
    }
}

private struct AttachmentMenuControl: UIViewRepresentable {
    let savedPromptsEnabled: Bool
    let filesEnabled: Bool
    let select: (AttachmentChoice) -> Void

    func makeUIView(context: Context) -> AttachmentMenuButton {
        AttachmentMenuButton()
    }

    func updateUIView(_ button: AttachmentMenuButton, context: Context) {
        button.select = select
        button.isEnabled = context.environment.isEnabled
        button.configure(
            savedPromptsEnabled: savedPromptsEnabled, filesEnabled: filesEnabled,
            accent: UIColor(Color(context.environment.appAccent.resolve(in: context.environment))),
            locale: context.environment.locale
        )
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: AttachmentMenuButton, context: Context) -> CGSize? {
        CGSize(width: AttachmentMenuButton.hitSize, height: AttachmentMenuButton.hitSize)
    }
}

final class AttachmentMenuButton: UIButton {
    /// Matches the small circular glass buttons beside it. The hit target extends past the circle.
    static let diameter: CGFloat = 38
    static let hitSize: CGFloat = 44
    static var outset: CGFloat { (hitSize - diameter) / 2 }

    var select: ((AttachmentChoice) -> Void)?
    private(set) var pendingChoice: AttachmentChoice?
    private var savedPromptsEnabled = true
    private var filesEnabled = true

    init() {
        super.init(frame: .zero)
        var configuration = UIButton.Configuration.glass()
        configuration.image = UIImage(systemName: "paperclip")
        configuration.preferredSymbolConfigurationForImage = .init(pointSize: 16)
        configuration.cornerStyle = .capsule
        configuration.buttonSize = .small
        configuration.background.backgroundInsets = NSDirectionalEdgeInsets(
            top: Self.outset, leading: Self.outset, bottom: Self.outset, trailing: Self.outset
        )
        self.configuration = configuration
        showsMenuAsPrimaryAction = true
        preferredMenuElementOrder = .fixed
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
    }

    func configure(savedPromptsEnabled: Bool, filesEnabled: Bool, accent: UIColor, locale: Locale) {
        self.savedPromptsEnabled = savedPromptsEnabled
        self.filesEnabled = filesEnabled
        guard !isHeld else { return }
        configuration?.baseForegroundColor = accent
        configuration?.image = UIImage(systemName: "paperclip")?.withTintColor(accent, renderingMode: .alwaysOriginal)
        accessibilityLabel = String(localized: "Attachments and saved prompts", locale: locale)
        let actions = AttachmentChoice.allCases.map { choice in
            UIAction(
                title: String(localized: String.LocalizationValue(choice.title), locale: locale),
                image: UIImage(systemName: choice.symbol),
                attributes: enabled(choice) ? [] : .disabled
            ) { [weak self] _ in
                self?.queueSelection(choice)
            }
        }
        menu = UIMenu(children: [
            actions[0],
            UIMenu(options: .displayInline, children: Array(actions.dropFirst()))
        ])
    }

    private func enabled(_ choice: AttachmentChoice) -> Bool {
        choice == .savedPrompts ? savedPromptsEnabled : filesEnabled
    }

    func queueSelection(_ choice: AttachmentChoice) {
        guard isEnabled, enabled(choice) else { return }
        pendingChoice = choice
    }

    override func contextMenuInteraction(
        _ interaction: UIContextMenuInteraction, willEndFor configuration: UIContextMenuConfiguration,
        animator: (any UIContextMenuInteractionAnimating)?
    ) {
        super.contextMenuInteraction(interaction, willEndFor: configuration, animator: animator)
        finishSelection(animator: animator)
    }

    func finishSelection(animator: (any UIContextMenuInteractionAnimating)?) {
        guard let choice = pendingChoice else { return }
        pendingChoice = nil
        let present = { [weak self] in
            guard let self, isEnabled, enabled(choice) else { return }
            select?(choice)
        }
        if let animator {
            animator.addCompletion(present)
        } else {
            present()
        }
    }
}
