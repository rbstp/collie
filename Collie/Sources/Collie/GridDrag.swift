import SwiftUI
import UniformTypeIdentifiers

struct GridDrag: Codable, Transferable {
    static let type = UTType(exportedAs: "dev.rbstp.collie.grid-card", conformingTo: .data)
    private(set) var id = UUID()
    let route: AgentRoute
    let starred: Bool

    static var transferRepresentation: some TransferRepresentation {
        CodableRepresentation(contentType: type).visibility(.ownProcess)
    }
}

struct GridDragSource: ViewModifier {
    let route: AgentRoute
    let starred: Bool
    @Binding var dragging: GridDrag?

    func body(content: Content) -> some View {
        let source = content.onDrag {
            let drag = GridDrag(route: route, starred: starred)
            dragging = drag
            let provider = GridDragProvider {
                if dragging?.id == drag.id { dragging = nil }
            }
            provider.registerDataRepresentation(forTypeIdentifier: GridDrag.type.identifier, visibility: .ownProcess) { completion in
                completion(Data(), nil)
                return nil
            }
            return provider
        }
        if #available(iOS 27, *) {
            content.draggable(GridDrag(route: route, starred: starred))
                .dragConfiguration(.init(allowMove: true))
                .onDragSessionUpdated { session in
                    switch session.phase {
                    case .active:
                        if dragging?.route != route { dragging = GridDrag(route: route, starred: starred) }
                    case .ended, .dataTransferCompleted:
                        if dragging?.route == route { dragging = nil }
                    default:
                        break
                    }
                }
        } else {
            source
        }
    }
}

private final class GridDragProvider: NSItemProvider, @unchecked Sendable {
    let ended: @MainActor @Sendable () -> Void

    init(ended: @escaping @MainActor @Sendable () -> Void) {
        self.ended = ended
        super.init()
    }

    required init?(coder: NSCoder) { nil }

    deinit {
        let ended = ended
        Task { @MainActor in ended() }
    }
}

enum GridDropEdge {
    case top, bottom, leading, trailing

    var after: Bool { self == .bottom || self == .trailing }
    var vertical: Bool { self == .leading || self == .trailing }
    var alignment: Alignment {
        switch self {
        case .top: .top
        case .bottom: .bottom
        case .leading: .leading
        case .trailing: .trailing
        }
    }

    static func nearest(to point: CGPoint, size: CGSize, axes: Axis.Set, rightToLeft: Bool = false) -> Self {
        let vertical: Self = point.y < size.height / 2 ? .top : .bottom
        guard axes.contains(.horizontal), !axes.contains(.vertical) || min(point.x, size.width - point.x) < min(point.y, size.height - point.y) else { return vertical }
        return (point.x < size.width / 2) != rightToLeft ? .leading : .trailing
    }
}

struct GridDropTarget: ViewModifier {
    @Binding var dragging: GridDrag?
    let machineId: String
    var destination: AgentRoute?
    var axes: Axis.Set = .vertical
    let move: (AgentRoute, Bool) -> Bool
    @State private var edge: GridDropEdge?
    @State private var size: CGSize = .zero
    @Environment(\.layoutDirection) private var direction

    func body(content: Content) -> some View {
        content
            .onGeometryChange(for: CGSize.self) { $0.size } action: { size = $0 }
            .onDrop(of: [GridDrag.type], delegate: Target(
                dragging: $dragging, edge: $edge, machineId: machineId, destination: destination,
                size: size, axes: axes, rightToLeft: direction == .rightToLeft, move: move
            ))
            .onChange(of: dragging?.id) { _, _ in edge = nil }
            .overlay {
                if dragging != nil, let edge {
                    if destination == nil {
                        RoundedRectangle(cornerRadius: 12).fill(.primary.opacity(0.08)).allowsHitTesting(false)
                    } else {
                        Capsule().fill(.tint)
                            .frame(width: edge.vertical ? 3 : nil, height: edge.vertical ? nil : 3)
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: edge.alignment)
                            .allowsHitTesting(false)
                    }
                }
            }
    }

    private struct Target: DropDelegate {
        @Binding var dragging: GridDrag?
        @Binding var edge: GridDropEdge?
        let machineId: String
        let destination: AgentRoute?
        let size: CGSize
        let axes: Axis.Set
        let rightToLeft: Bool
        let move: (AgentRoute, Bool) -> Bool

        func validateDrop(info: DropInfo) -> Bool {
            guard let source = dragging?.route else { return false }
            return source.machineId == machineId && source != destination && info.hasItemsConforming(to: [GridDrag.type])
        }

        func dropUpdated(info: DropInfo) -> DropProposal? {
            guard validateDrop(info: info) else { return DropProposal(operation: .forbidden) }
            edge = GridDropEdge.nearest(to: info.location, size: size, axes: axes, rightToLeft: rightToLeft)
            return DropProposal(operation: .move)
        }

        func dropExited(info: DropInfo) { edge = nil }

        func performDrop(info: DropInfo) -> Bool {
            guard validateDrop(info: info), let source = dragging?.route else { return false }
            let placement = GridDropEdge.nearest(to: info.location, size: size, axes: axes, rightToLeft: rightToLeft)
            let moved = move(source, placement.after)
            edge = nil
            dragging = nil
            return moved
        }
    }
}

struct GridPageLock: UIViewRepresentable {
    let locked: Bool

    func makeUIView(context: Context) -> Probe { Probe() }
    func updateUIView(_ view: Probe, context: Context) {
        view.locked = locked
        view.update()
    }
    static func dismantleUIView(_ view: Probe, coordinator: ()) { view.restore() }

    final class Probe: UIView {
        var locked = false
        private weak var pager: UIScrollView?
        private var wasEnabled = true

        override func didMoveToWindow() {
            super.didMoveToWindow()
            update()
        }

        func restore() { pager?.isScrollEnabled = wasEnabled }

        func update() {
            var ancestor = superview
            while let view = ancestor {
                if let scroll = view as? UIScrollView {
                    if pager !== scroll {
                        pager?.isScrollEnabled = wasEnabled
                        pager = scroll
                        wasEnabled = scroll.isScrollEnabled
                    }
                    scroll.isScrollEnabled = locked ? false : wasEnabled
                    return
                }
                ancestor = view.superview
            }
        }
    }
}
