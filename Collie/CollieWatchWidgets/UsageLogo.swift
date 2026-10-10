import SwiftUI

struct UsageLogo: View {
    let name: String

    var body: some View {
        let image = Image(name).resizable().scaledToFit()
        if name == "Codex" {
            Color.primary.mask(image.luminanceToAlpha())
        } else {
            image
        }
    }
}
