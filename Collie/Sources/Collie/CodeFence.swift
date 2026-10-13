enum CodeFence {
    static func block(_ text: String, info: String = "") -> String {
        var fence = "```"
        while text.contains(fence) { fence += "`" }
        return "\(fence)\(info)\n\(text)\(text.hasSuffix("\n") ? "" : "\n")\(fence)"
    }
}
