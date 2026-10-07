import CollieCore
import Foundation
import SwiftUI
import Testing

@testable import Collie

@MainActor
final class FakeDictationEngine: DictationEngine {
    var microphone = true
    var unavailable: Set<DictationLanguage> = []
    var downloads: [Double] = []
    var holdsInstall = false
    private var installHeld: CheckedContinuation<Void, Never>?
    var started: [DictationLanguage] = []
    var finished = 0
    var cancelled = 0
    private var continuation: AsyncThrowingStream<DictationEvent, any Error>.Continuation?

    func requestMicrophone() async -> Bool { microphone }

    func install(_ language: DictationLanguage, progress: @escaping @MainActor (Double) -> Void) async throws {
        if unavailable.contains(language) { throw DictationError.unavailable }
        downloads.forEach(progress)
        if holdsInstall { await withCheckedContinuation { installHeld = $0 } }
    }

    var installing: Bool { installHeld != nil }

    func finishInstall() {
        installHeld?.resume()
        installHeld = nil
    }

    func start(_ language: DictationLanguage) async throws -> DictationSession {
        started.append(language)
        let (events, continuation) = AsyncThrowingStream.makeStream(of: DictationEvent.self)
        self.continuation = continuation
        return DictationSession(
            events: events,
            finish: { [weak self] in
                self?.finished += 1
                continuation.finish()
            },
            cancel: { [weak self] in
                self?.cancelled += 1
                continuation.finish()
            }
        )
    }

    func send(_ event: DictationEvent) async {
        continuation?.yield(event)
        for _ in 0..<20 { await Task.yield() }
    }

    func fail(_ error: any Error) {
        continuation?.finish(throwing: error)
    }
}

@MainActor
private func until(_ condition: () -> Bool) async {
    for _ in 0..<500 where !condition() {
        try? await Task.sleep(for: .milliseconds(2))
    }
}

@MainActor
private func dictating(_ engine: FakeDictationEngine, prefsFile: URL? = nil) async -> AgentModel {
    let model = AgentModel(
        core: FakeCore(), route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: prefsFile, dictationEngine: engine
    )
    model.draft = "fix"
    model.startDictation()
    await until { model.dictation.phase == .listening }
    return model
}

@MainActor
@Test func dictationAppendsVolatileThenFinalTextAndStopsWithoutSending() async {
    let engine = FakeDictationEngine()
    let model = await dictating(engine)
    #expect(model.dictation.phase == .listening)
    #expect(engine.started == [.english])

    await engine.send(.level(0.5))
    await until { model.dictation.level == 0.5 }
    #expect(model.dictation.level == 0.5)
    await engine.send(.volatile("the bil"))
    await until { model.draft == "fix the bil" }
    #expect(model.draft == "fix the bil")
    await engine.send(.volatile("the build"))
    await until { model.draft == "fix the build" }
    #expect(model.draft == "fix the build")
    await engine.send(.final("the build."))
    await until { model.draft == "fix the build." }
    #expect(model.draft == "fix the build.")
    await engine.send(.volatile("and"))
    await until { model.draft == "fix the build. and" }
    #expect(model.draft == "fix the build. and")
    await engine.send(.final(" and the tests"))
    await until { model.draft == "fix the build. and the tests" }
    #expect(model.draft == "fix the build. and the tests")

    model.dictation.stop()
    #expect(model.dictation.phase == .finishing)
    await until { !model.dictation.isActive }
    #expect(engine.finished == 1)
    #expect(model.draft == "fix the build. and the tests")
    #expect(model.dictation.level == 0)
    #expect(model.canSendPrompt)
}

@MainActor
@Test func theFieldIsLockedWhileDictating() async {
    let engine = FakeDictationEngine()
    let model = await dictating(engine)
    await engine.send(.volatile("hello"))
    await until { model.draft == "fix hello" }

    model.paste("pasted")
    #expect(model.draft == "fix hello")
    #expect(!model.canSendPrompt)
    await model.sendPrompt()
    #expect(model.draft == "fix hello")
    #expect(model.startDictation() == nil)

    model.dictation.cancel()
    #expect(engine.cancelled == 1)
    model.paste(" pasted")
    #expect(model.draft == "fix hello pasted")
}

@MainActor
@Test func dictationWaitsForASendInFlight() async {
    let core = FakeCore()
    let engine = FakeDictationEngine()
    let model = AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil, dictationEngine: engine)
    model.draft = "go"
    core.set(hold: true)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    #expect(model.startDictation() == nil)
    core.release()
    await send.value
    #expect(model.startDictation() != nil)
    model.dictation.cancel()
}

@MainActor
@Test func dictationLanguageIsEnglishByDefaultAndRememberedOnThisDevice() async throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "prefs.json")

    try Data(#"{"wrapLines":false}"#.utf8).write(to: file)
    let engine = FakeDictationEngine()
    let model = await dictating(engine, prefsFile: file)
    #expect(model.dictation.language == .english)
    await engine.send(.volatile("bonjour"))
    await until { model.draft == "fix bonjour" }

    model.dictation.select(.french)
    await until { engine.started.count == 2 && model.dictation.phase == .listening }
    #expect(engine.started == [.english, .french])
    #expect(engine.cancelled == 1)
    await engine.send(.final("à tous"))
    await until { model.draft == "fix bonjour à tous" }
    #expect(model.draft == "fix bonjour à tous")
    #expect(DevicePrefs.load(from: file) == DevicePrefs(wrapLines: false, dictationLanguage: .french))

    let route = AgentRoute(machineId: "m1", terminalId: "term_1")
    #expect(AgentModel(core: FakeCore(), route: route, prefsFile: file, dictationEngine: engine).dictation.language == .french)
    try Data(#"{"dictationLanguage":"de-DE"}"#.utf8).write(to: file)
    #expect(AgentModel(core: FakeCore(), route: route, prefsFile: file, dictationEngine: engine).dictation.language == .english)
    model.dictation.cancel()
}

@MainActor
@Test func deniedMicrophoneExplainsHowToEnableIt() async {
    let engine = FakeDictationEngine()
    engine.microphone = false
    let model = AgentModel(core: FakeCore(), route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil, dictationEngine: engine)
    model.draft = "keep"
    await model.startDictation()?.value
    #expect(!model.dictation.isActive)
    #expect(model.dictation.problem == .microphoneDenied)
    #expect(model.dictation.problem?.message.contains("Settings") == true)
    #expect(engine.started.isEmpty)
    #expect(model.draft == "keep")
    await model.sendPrompt()
    #expect(model.dictation.problem == nil)

    engine.microphone = true
    model.startDictation()
    await until { model.dictation.phase == .listening }
    #expect(model.dictation.problem == nil)
    model.dictation.cancel()
}

@MainActor
@Test func missingModelIsReportedAndDownloadProgressIsShown() async {
    let engine = FakeDictationEngine()
    engine.unavailable = [.french]
    let dictation = DictationModel(engine: engine, language: .english, prefsFile: nil)
    dictation.select(.french)
    await dictation.start(appendingTo: "") { _ in }?.value
    #expect(dictation.problem == .unavailable(.french))
    #expect(dictation.problem?.message == "On-device dictation in French (Canada) is not available on this device.")
    #expect(engine.started.isEmpty)

    engine.downloads = [0.25, 0.5]
    engine.holdsInstall = true
    dictation.select(.english)
    #expect(dictation.problem == nil)
    dictation.start(appendingTo: "") { _ in }
    await until { engine.installing }
    #expect(dictation.phase == .preparing(download: 0.5))
    engine.finishInstall()
    await until { dictation.phase == .listening }
    #expect(engine.started == [.english])
    dictation.cancel()
}

@MainActor
@Test func stoppingWhilePreparingNeverStartsTheMicrophone() async {
    let engine = FakeDictationEngine()
    engine.holdsInstall = true
    let dictation = DictationModel(engine: engine, language: .english, prefsFile: nil)
    let task = dictation.start(appendingTo: "") { _ in }
    await until { engine.installing }
    dictation.stop()
    #expect(!dictation.isActive)
    engine.finishInstall()
    await task?.value
    #expect(engine.started.isEmpty)
    #expect(!dictation.isActive && dictation.problem == nil)
}

@MainActor
@Test func backgroundingStopsDictationAndKeepsTheText() async {
    let engine = FakeDictationEngine()
    let model = await dictating(engine)
    await engine.send(.volatile("half a sentence"))

    model.dictation.scenePhaseChanged(to: .inactive)
    #expect(model.dictation.phase == .listening)
    model.dictation.scenePhaseChanged(to: .background)
    await until { !model.dictation.isActive }
    #expect(engine.finished == 1)
    #expect(model.draft == "fix half a sentence")
}

@MainActor
@Test func aFailedEngineEndsDictationWithTheTextSoFar() async {
    let engine = FakeDictationEngine()
    let model = await dictating(engine)
    await engine.send(.final("done"))
    engine.fail(CocoaError(.featureUnsupported))
    await until { !model.dictation.isActive }
    #expect(model.draft == "fix done")
    #expect(model.dictation.problem?.message.hasPrefix("Dictation stopped: ") == true)
    #expect(engine.cancelled == 1)
}

@MainActor
@Test func dictatedTextIsSetApartBySpaces() {
    #expect(DictationModel.join("", "hello") == "hello")
    #expect(DictationModel.join("fix", "") == "fix")
    #expect(DictationModel.join("fix", "it") == "fix it")
    #expect(DictationModel.join("fix ", "it") == "fix it")
    #expect(DictationModel.join("fix\n", "it") == "fix\nit")
    #expect(DictationModel.join("fix", " it") == "fix it")
}
