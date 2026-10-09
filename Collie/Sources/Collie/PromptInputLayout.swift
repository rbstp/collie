import SwiftUI

/// Keeps the same text field alive as it moves between the compact row and the full-width row.
struct PromptInputLayout: Layout {
    let expanded: Bool
    let showsStatus: Bool
    private let spacing: CGFloat = 8

    private struct Sizes {
        let leading: CGSize
        let field: CGSize
        let status: CGSize
        let trailing: CGSize
        let fieldWidth: CGFloat
        let statusWidth: CGFloat

        var controlsHeight: CGFloat { max(leading.height, status.height, trailing.height) }
    }

    private func gap(after size: CGSize) -> CGFloat { size.width > 0 ? spacing : 0 }

    private func sizes(width: CGFloat, subviews: Subviews) -> Sizes {
        let leading = subviews[0].sizeThatFits(.unspecified)
        let trailing = subviews[3].sizeThatFits(.unspecified)
        let sides = leading.width + trailing.width + gap(after: leading) + gap(after: trailing)
        let fieldWidth = expanded ? width : max(0, width - sides)
        let field = subviews[1].sizeThatFits(ProposedViewSize(width: fieldWidth, height: nil))
        let statusWidth = max(0, width - sides)
        let status = showsStatus
            ? subviews[2].sizeThatFits(ProposedViewSize(width: statusWidth, height: nil)) : .zero
        return Sizes(leading: leading, field: field, status: status, trailing: trailing,
                     fieldWidth: fieldWidth, statusWidth: statusWidth)
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320
        let measured = sizes(width: width, subviews: subviews)
        let height = expanded
            ? measured.field.height + spacing + measured.controlsHeight
            : max(measured.field.height, measured.controlsHeight)
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let measured = sizes(width: bounds.width, subviews: subviews)
        let rowHeight = max(measured.field.height, measured.controlsHeight)
        let controlsY = expanded ? bounds.minY + measured.field.height + spacing : bounds.minY
        let controlsHeight = measured.controlsHeight
        let controlsBottom = expanded ? controlsY + controlsHeight : bounds.minY + rowHeight
        let fieldX = expanded ? bounds.minX : bounds.minX + measured.leading.width + gap(after: measured.leading)
        let fieldY = expanded ? bounds.minY : bounds.minY + rowHeight - measured.field.height

        subviews[1].place(at: CGPoint(x: fieldX, y: fieldY), anchor: .topLeading,
                          proposal: ProposedViewSize(width: measured.fieldWidth, height: measured.field.height))
        subviews[0].place(at: CGPoint(x: bounds.minX, y: controlsBottom - measured.leading.height),
                          anchor: .topLeading, proposal: ProposedViewSize(measured.leading))
        subviews[3].place(at: CGPoint(x: bounds.maxX - measured.trailing.width,
                                      y: controlsBottom - measured.trailing.height),
                          anchor: .topLeading, proposal: ProposedViewSize(measured.trailing))
        if showsStatus {
            subviews[2].place(at: CGPoint(x: bounds.minX + measured.leading.width + gap(after: measured.leading),
                                          y: controlsBottom - measured.status.height),
                              anchor: .topLeading,
                              proposal: ProposedViewSize(width: measured.statusWidth, height: measured.status.height))
        }
    }
}
