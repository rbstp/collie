import SwiftUI
import VisionKit

struct QRScannerView: UIViewControllerRepresentable {
    let onScan: (String) -> Void
    let onError: (String) -> Void

    static var isSupported: Bool { DataScannerViewController.isSupported }
    static var isAvailable: Bool { DataScannerViewController.isAvailable }

    func makeUIViewController(context: Context) -> ScannerHost {
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            isHighlightingEnabled: true
        )
        scanner.delegate = context.coordinator
        return ScannerHost(scanner: scanner, onError: onError)
    }

    func updateUIViewController(_ host: ScannerHost, context: Context) {}

    static func dismantleUIViewController(_ host: ScannerHost, coordinator: Coordinator) {
        host.scanner.stopScanning()
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(onScan: onScan)
    }

    /// DataScannerViewController can only start once it is on screen.
    final class ScannerHost: UIViewController {
        let scanner: DataScannerViewController
        private let onError: (String) -> Void

        init(scanner: DataScannerViewController, onError: @escaping (String) -> Void) {
            self.scanner = scanner
            self.onError = onError
            super.init(nibName: nil, bundle: nil)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) { fatalError() }

        override func viewDidLoad() {
            super.viewDidLoad()
            addChild(scanner)
            scanner.view.frame = view.bounds
            scanner.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
            view.addSubview(scanner.view)
            scanner.didMove(toParent: self)
        }

        override func viewDidAppear(_ animated: Bool) {
            super.viewDidAppear(animated)
            guard !scanner.isScanning else { return }
            do {
                try scanner.startScanning()
            } catch {
                onError("The camera could not start (\(error.localizedDescription)). Paste the pairing link instead.")
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let onScan: (String) -> Void
        private var done = false

        init(onScan: @escaping (String) -> Void) {
            self.onScan = onScan
        }

        func dataScanner(_ scanner: DataScannerViewController, didAdd items: [RecognizedItem], allItems: [RecognizedItem]) {
            guard !done else { return }
            for case let .barcode(barcode) in items {
                if let payload = barcode.payloadStringValue, payload.hasPrefix("collie://pair#") {
                    done = true
                    scanner.stopScanning()
                    onScan(payload)
                    return
                }
            }
        }
    }
}
