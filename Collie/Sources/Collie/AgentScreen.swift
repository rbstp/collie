import CollieCore
import GhosttyTerminal
import SwiftUI
import UIKit

struct AgentScreen: View {
    @State private var model: AgentModel
    @State private var confirmingFirst = false
    @State private var confirmingSecond = false
    @Environment(\.dismiss) private var dismiss

    init(core: any AgentCore, route: AgentRoute) {
        _model = State(initialValue: AgentModel(core: core, route: route))
    }

    var body: some View {
        VStack(spacing: 0) {
            AgentHeader(model: model)
            AgentTerminal(ansi: model.ansi) { await model.refresh() }
                .overlay {
                    if model.ansi.isEmpty {
                        ProgressView("Waiting for output…").tint(.white).foregroundStyle(.white)
                    }
                }
            if let notice = model.notice {
                Label(notice, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal)
                    .padding(.top, 6)
            }
            KeyStrip(model: model)
            PromptBar(model: model)
        }
        .navigationTitle(model.agent?.displayTitle ?? "Agent")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Focus on Mac", systemImage: "macwindow") {
                    Task { await model.focus() }
                }
            }
            ToolbarItem(placement: .primaryAction) {
                Menu("More", systemImage: "ellipsis") {
                    Button("Close pane", systemImage: "xmark.square", role: .destructive) {
                        askToClose(.pane)
                    }
                    Button("Close workspace", systemImage: "xmark.rectangle.portrait", role: .destructive) {
                        if let workspaceId = model.agent?.workspaceId {
                            askToClose(.workspace(id: workspaceId))
                        }
                    }
                    .disabled(model.agent == nil)
                }
            }
        }
        .confirmationDialog(closeTitle, isPresented: $confirmingFirst, titleVisibility: .visible) {
            Button(closeAction, role: .destructive) {
                model.close.advance()
                confirmingSecond = true
            }
            Button("Cancel", role: .cancel) { model.close.cancel() }
        } message: {
            Text(closeMessage)
        }
        .alert("Are you sure?", isPresented: $confirmingSecond) {
            Button(closeAction, role: .destructive) {
                Task { await model.performClose() }
            }
            Button("Cancel", role: .cancel) { model.close.cancel() }
        } message: {
            Text("\(closeMessage) This cannot be undone.")
        }
        .onChange(of: model.closed) { _, closed in
            if closed { dismiss() }
        }
        .task { await model.run() }
    }

    private func askToClose(_ target: CloseTarget) {
        model.close.begin(target)
        confirmingFirst = true
    }

    private var closeTitle: String {
        model.close.target == .pane ? "Close this pane?" : "Close this workspace?"
    }

    private var closeAction: String {
        model.close.target == .pane ? "Close pane" : "Close workspace"
    }

    private var closeMessage: String {
        model.close.target == .pane
            ? "The agent and its shell on the Mac are ended."
            : "Every agent and shell in the workspace on the Mac is ended."
    }
}

private struct AgentHeader: View {
    let model: AgentModel

    var body: some View {
        HStack(spacing: 8) {
            if let agent = model.agent {
                StatusPill(state: agent.status)
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                if let kind = agent.kind {
                    Text(kind).font(.caption).foregroundStyle(.secondary)
                }
            } else {
                Text("Agent not in the flock").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
            if let link = model.link, link != .connected {
                Text(model.linkError ?? link.label)
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 6)
    }
}

/// GhosttyTerminalUIView with a refresh control; `.refreshable` does not reach a UIKit scroll view.
private struct AgentTerminal: UIViewRepresentable {
    let ansi: String
    let refresh: @MainActor () async -> Void

    func makeUIView(context: Context) -> GhosttyTerminalUIView {
        let view = GhosttyTerminalUIView(fontSize: 11)
        let control = UIRefreshControl()
        control.tintColor = .white
        let coordinator = context.coordinator
        control.addAction(
            UIAction { [weak control] _ in
                Task {
                    await coordinator.refresh?()
                    control?.endRefreshing()
                }
            },
            for: .valueChanged
        )
        view.refreshControl = control
        return view
    }

    func updateUIView(_ view: GhosttyTerminalUIView, context: Context) {
        context.coordinator.refresh = refresh
        if !ansi.isEmpty {
            view.show(ansiSnapshot: ansi)
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator {
        var refresh: (@MainActor () async -> Void)?
    }
}

private struct KeyStrip: View {
    let model: AgentModel

    var body: some View {
        HStack(spacing: 4) {
            ForEach(AgentKey.strip, id: \.self) { key in
                Button {
                    model.tap(key)
                } label: {
                    Text(key.symbol)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .frame(maxWidth: .infinity, minHeight: 34)
                        .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 8))
                        .contentShape(RoundedRectangle(cornerRadius: 8))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(key.accessibilityName)
            }
        }
        .padding(.horizontal)
        .padding(.top, 8)
        .sensoryFeedback(.impact(weight: .light), trigger: model.keyTaps)
    }
}

private struct PromptBar: View {
    @Bindable var model: AgentModel

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let error = model.promptError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            HStack(alignment: .bottom, spacing: 8) {
                TextField("Prompt the agent", text: $model.draft, axis: .vertical)
                    .lineLimit(1...6)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18))
                Button {
                    Task { await model.sendPrompt() }
                } label: {
                    if model.sendingPrompt {
                        ProgressView().frame(width: 32, height: 32)
                    } else {
                        Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                    }
                }
                .disabled(!model.canSendPrompt)
                .accessibilityLabel("Send prompt")
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
    }
}
