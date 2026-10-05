import AVFoundation
import Foundation
import Speech

/// SpeechAnalyzer runs on the device, so it needs the microphone only: speech recognition
/// authorization covers SFSpeechRecognizer's server path, which collie never uses.
@MainActor
final class SpeechDictationEngine: DictationEngine {
    func requestMicrophone() async -> Bool {
        await AVAudioApplication.requestRecordPermission()
    }

    func install(_ language: DictationLanguage, progress: @escaping @MainActor (Double) -> Void) async throws {
        let transcriber = try await Self.transcriber(for: language)
        guard let request = try await Self.unavailableIfUnsupported({ try await Self.installationRequest(for: transcriber) }) else { return }
        let polling = Task {
            while !Task.isCancelled {
                progress(request.progress.fractionCompleted)
                try? await Task.sleep(for: .milliseconds(200))
            }
        }
        defer { polling.cancel() }
        try await Self.unavailableIfUnsupported { try await request.downloadAndInstall() }
    }

    func start(_ language: DictationLanguage) async throws -> DictationSession {
        let transcriber = try await Self.transcriber(for: language)
        let live = try await LiveDictation(transcriber: transcriber)
        return DictationSession(events: live.events, finish: { await live.finish() }, cancel: { live.cancel() })
    }

    private static func transcriber(for language: DictationLanguage) async throws -> SpeechTranscriber {
        guard SpeechTranscriber.isAvailable, let locale = await SpeechTranscriber.supportedLocale(equivalentTo: language.locale) else {
            throw DictationError.unavailable
        }
        return SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
    }

    /// Each locale installed is reserved for the app, up to a per-device limit; collie only needs the one in use.
    private static func installationRequest(for transcriber: SpeechTranscriber) async throws -> AssetInstallationRequest? {
        do {
            return try await AssetInventory.assetInstallationRequest(supporting: [transcriber])
        } catch let error as SFSpeechError where error.code == .tooManyAssetLocalesAllocated {
            for locale in await AssetInventory.reservedLocales {
                await AssetInventory.release(reservedLocale: locale)
            }
            return try await AssetInventory.assetInstallationRequest(supporting: [transcriber])
        }
    }

    private static func unavailableIfUnsupported<T>(_ body: () async throws -> T) async throws -> T {
        do {
            return try await body()
        } catch let error as SFSpeechError where [.noModel, .cannotAllocateUnsupportedLocale, .tooManyAssetLocalesAllocated].contains(error.code) {
            throw DictationError.unavailable
        }
    }
}

@MainActor
private final class LiveDictation {
    let events: AsyncThrowingStream<DictationEvent, any Error>
    private let output: AsyncThrowingStream<DictationEvent, any Error>.Continuation
    private let input: AsyncStream<AnalyzerInput>.Continuation
    private let analyzer: SpeechAnalyzer
    private let audio = AVAudioEngine()
    private var results: Task<Void, Never>?
    private var listening = false
    private var observers: [any NSObjectProtocol] = []

    init(transcriber: SpeechTranscriber) async throws {
        analyzer = SpeechAnalyzer(modules: [transcriber])
        (events, output) = AsyncThrowingStream.makeStream(of: DictationEvent.self)
        let inputs: AsyncStream<AnalyzerInput>
        (inputs, input) = AsyncStream.makeStream(of: AnalyzerInput.self)
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .measurement)
            try session.setActive(true)
            listening = true
            let natural = audio.inputNode.outputFormat(forBus: 0)
            guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber], considering: natural) else {
                throw DictationError.unavailable
            }
            try await analyzer.prepareToAnalyze(in: format)
            try Task.checkCancellation()
            try Self.tap(audio.inputNode, from: natural, to: format, input: input, output: output)
            audio.prepare()
            try audio.start()
            try await analyzer.start(inputSequence: inputs)
            observeInterruptions()
        } catch {
            stopAudio()
            await analyzer.cancelAndFinishNow()
            throw error
        }
        let output = output
        results = Task {
            do {
                for try await result in transcriber.results {
                    let text = String(result.text.characters)
                    output.yield(result.isFinal ? .final(text) : .volatile(text))
                }
                output.finish()
            } catch {
                output.finish(throwing: error)
            }
        }
    }

    func finish() async {
        guard listening else { return }
        stopAudio()
        do {
            try await analyzer.finalizeAndFinishThroughEndOfInput()
        } catch {
            results?.cancel()
            output.finish()
        }
    }

    func cancel() {
        stopAudio()
        results?.cancel()
        output.finish()
        let analyzer = analyzer
        Task { await analyzer.cancelAndFinishNow() }
    }

    private func stopAudio() {
        guard listening else { return }
        listening = false
        audio.inputNode.removeTap(onBus: 0)
        audio.stop()
        input.finish()
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    /// A call, Siri or a route change stops the engine; finishing keeps what was heard instead of showing a dead microphone.
    private func observeInterruptions() {
        let interrupted: Notification.Name =
            if #available(iOS 27, *) { AVAudioSession.didBecomeInactiveNotification } else { AVAudioSession.interruptionNotification }
        let sources: [(Notification.Name, Any?)] = [(interrupted, nil), (.AVAudioEngineConfigurationChange, audio)]
        observers = sources.map { name, object in
            NotificationCenter.default.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                Task { await self?.finish() }
            }
        }
    }

    /// Nonisolated so the tap block, which runs on the audio thread, is not main-actor isolated.
    private nonisolated static func tap(
        _ node: AVAudioInputNode, from natural: AVAudioFormat, to format: AVAudioFormat,
        input: AsyncStream<AnalyzerInput>.Continuation, output: AsyncThrowingStream<DictationEvent, any Error>.Continuation
    ) throws {
        guard let converter = BufferConverter(from: natural, to: format) else { throw DictationError.unavailable }
        node.installTap(onBus: 0, bufferSize: 4096, format: natural) { buffer, _ in
            output.yield(.level(level(of: buffer)))
            if let converted = converter.convert(buffer) {
                input.yield(AnalyzerInput(buffer: converted))
            }
        }
    }

    private nonisolated static func level(of buffer: AVAudioPCMBuffer) -> Float {
        guard let samples = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return 0 }
        let count = Int(buffer.frameLength)
        var sum: Float = 0
        for index in 0..<count { sum += samples[index] * samples[index] }
        let decibels = 20 * log10(max((sum / Float(count)).squareRoot(), 1e-6))
        return min(max((decibels + 50) / 50, 0), 1)
    }
}

/// Always copies, since the tap's buffer may be reused once the block returns.
/// Unchecked: only the tap's audio thread uses it.
private final class BufferConverter: @unchecked Sendable {
    private let converter: AVAudioConverter
    private let format: AVAudioFormat

    init?(from natural: AVAudioFormat, to format: AVAudioFormat) {
        guard let converter = AVAudioConverter(from: natural, to: format) else { return nil }
        converter.primeMethod = .none
        self.converter = converter
        self.format = format
    }

    func convert(_ buffer: AVAudioPCMBuffer) -> AVAudioPCMBuffer? {
        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up))
        guard capacity > 0, let converted = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else { return nil }
        var supplied = false
        var error: NSError?
        let status = converter.convert(to: converted, error: &error) { _, status in
            if supplied {
                status.pointee = .noDataNow
                return nil
            }
            supplied = true
            status.pointee = .haveData
            return buffer
        }
        return status == .error || converted.frameLength == 0 ? nil : converted
    }
}
