import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class SessionTitleTests: XCTestCase {
    func testLongTitlesFillTwoLinesWithoutAnEllipsis() async throws {
        for title in [
            "Fix rbstp/collie Issues #23 and make the approval tests stop flaking on the slow CI runners again",
            "add_approval_tests_for_the_codex_menu_and_every_other_menu_in_one_single_identifier",
        ] {
            let host = UIHostingController(rootView: SessionTitle(title: title).frame(width: 200))
            try await hosting(host) { _ in
                let label = try await waitFor("title") {
                    host.view.layoutIfNeeded()
                    return descendants(host.view).compactMap { $0 as? UILabel }.first { $0.bounds.width > 0 }
                }
                XCTAssertEqual(label.text, title)
                XCTAssertEqual(label.accessibilityLabel, title)
                XCTAssertTrue(label.accessibilityTraits.contains(.header))
                XCTAssertLessThanOrEqual(label.bounds.width, 200)
                XCTAssertEqual(label.bounds.height, label.sizeThatFits(CGSize(width: label.bounds.width, height: 1000)).height)
                XCTAssertLessThan(label.bounds.height, label.font.lineHeight * 2.5)

                let shown = render(label)
                let fitting = UILabel(frame: label.bounds)
                fitting.numberOfLines = 0
                fitting.lineBreakMode = .byWordWrapping
                fitting.textAlignment = label.textAlignment
                fitting.font = label.font
                let cut = (1..<title.count).reversed().first { length in
                    fitting.text = String(title.prefix(length))
                    return render(fitting) == shown
                }
                XCTAssertGreaterThan(try XCTUnwrap(cut, title), title.count / 3)
            }
        }
    }

    private func render(_ label: UILabel) -> Data? {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 2
        return UIGraphicsImageRenderer(bounds: label.bounds, format: format).image { context in
            label.layer.render(in: context.cgContext)
        }.pngData()
    }
}
