import Foundation

enum TerminalLink {
    /// The http or https URL in `text` that overlaps the offsets `touched`, or nil when none or
    /// several different ones do. Only printable ASCII that URLs allow is taken, so spaces, control
    /// characters and other schemes never become a link. A URL cut short by another non-space
    /// character, or one with user info, is not offered: it would open somewhere else than it reads.
    static func url(in text: [Unicode.Scalar], touching touched: Range<Int>) -> URL? {
        var found: URL?
        var i = 0
        while i < text.count {
            guard let hostStart = schemeEnd(text, at: i) else {
                i += 1
                continue
            }
            var end = hostStart
            while end < text.count && allowed(text[end]) { end += 1 }
            if end < text.count && !text[end].isASCII && !text[end].properties.isWhitespace {
                i = end
                continue
            }
            end = trimmed(text, i..<end, keeping: hostStart)
            if end > hostStart && i < touched.upperBound && end > touched.lowerBound,
                let url = URL(string: String(String.UnicodeScalarView(text[i..<end]))),
                ["http", "https"].contains(url.scheme?.lowercased()), url.host?.isEmpty == false,
                url.user == nil && url.password == nil
            {
                if found != nil && found != url { return nil }
                found = url
            }
            i = end
        }
        return found
    }

    private static func schemeEnd(_ text: [Unicode.Scalar], at i: Int) -> Int? {
        if i > 0 && text[i - 1].isASCII && (text[i - 1].properties.isAlphabetic || text[i - 1].properties.numericType != nil) {
            return nil
        }
        for scheme in ["https://", "http://"] {
            let scheme = Array(scheme.unicodeScalars)
            guard i + scheme.count <= text.count else { continue }
            if zip(text[i...], scheme).allSatisfy({ $0.isASCII && $0.properties.lowercaseMapping == String($1) }) {
                return i + scheme.count
            }
        }
        return nil
    }

    private static func allowed(_ scalar: Unicode.Scalar) -> Bool {
        (0x21...0x7E).contains(scalar.value) && !"<>\"{}|\\^`".unicodeScalars.contains(scalar)
    }

    /// Drops sentence punctuation after the URL, and a closing bracket it does not open.
    private static func trimmed(_ text: [Unicode.Scalar], _ range: Range<Int>, keeping floor: Int) -> Int {
        var brackets: [Unicode.Scalar: Int] = [:]
        for scalar in text[range] where "()[]".unicodeScalars.contains(scalar) { brackets[scalar, default: 0] += 1 }
        var end = range.upperBound
        while end > floor {
            let last = text[end - 1]
            func unbalanced(_ open: Unicode.Scalar) -> Bool {
                brackets[open, default: 0] < brackets[last, default: 0]
            }
            guard ".,:;!?'".unicodeScalars.contains(last) || (last == ")" && unbalanced("("))
                || (last == "]" && unbalanced("["))
            else { break }
            brackets[last, default: 0] -= 1
            end -= 1
        }
        return end
    }
}
