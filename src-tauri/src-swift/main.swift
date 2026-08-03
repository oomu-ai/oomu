import AVFoundation
import Foundation
import Speech

private struct BridgeOutput: Encodable {
    let text: String
    let isFinal: Bool
    let errorCode: String?

    enum CodingKeys: String, CodingKey {
        case text
        case isFinal = "is_final"
        case errorCode = "error_code"
    }
}

private final class JsonLineWriter: @unchecked Sendable {
    private let lock = NSLock()

    func write(text: String, isFinal: Bool, errorCode: String? = nil) {
        let output = BridgeOutput(text: text, isFinal: isFinal, errorCode: errorCode)
        guard let encoded = try? JSONEncoder().encode(output) else { return }

        lock.lock()
        defer { lock.unlock() }
        FileHandle.standardOutput.write(encoded)
        FileHandle.standardOutput.write(Data([0x0A]))
    }
}

private final class SpeechBridge: @unchecked Sendable {
    private let audioEngine = AVAudioEngine()
    private let writer = JsonLineWriter()
    private let stateLock = NSLock()
    private var recognitionRequest: SFSpeechAudioBufferRecognitionRequest?
    private var recognitionTask: SFSpeechRecognitionTask?
    private var signalSources: [DispatchSourceSignal] = []
    private var isStopping = false
    private var tapInstalled = false

    func run() {
        installSignalHandlers()
        requestSpeechPermission()
        dispatchMain()
    }

    private func requestSpeechPermission() {
        SFSpeechRecognizer.requestAuthorization { [weak self] status in
            guard let self else { return }
            guard status == .authorized else {
                self.finish(errorCode: "speech_permission_denied")
                return
            }
            self.requestMicrophonePermission()
        }
    }

    private func requestMicrophonePermission() {
        AVCaptureDevice.requestAccess(for: .audio) { [weak self] granted in
            guard let self else { return }
            guard granted else {
                self.finish(errorCode: "microphone_permission_denied")
                return
            }
            DispatchQueue.main.async { self.startRecognition() }
        }
    }

    private func startRecognition() {
        guard let recognizer = SFSpeechRecognizer(), recognizer.isAvailable else {
            finish(errorCode: "speech_unavailable")
            return
        }
        guard recognizer.supportsOnDeviceRecognition else {
            finish(errorCode: "on_device_unavailable")
            return
        }

        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.requiresOnDeviceRecognition = true
        request.addsPunctuation = true
        recognitionRequest = request

        let input = audioEngine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            finish(errorCode: "microphone_unavailable")
            return
        }

        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { buffer, _ in
            request.append(buffer)
        }
        tapInstalled = true
        audioEngine.prepare()

        do {
            try audioEngine.start()
        } catch {
            finish(errorCode: "audio_start_failed")
            return
        }

        recognitionTask = recognizer.recognitionTask(with: request) { [weak self] result, error in
            guard let self else { return }
            if let result {
                let transcript = result.bestTranscription.formattedString
                    .trimmingCharacters(in: .whitespacesAndNewlines)
                if !transcript.isEmpty {
                    self.writer.write(text: transcript, isFinal: result.isFinal)
                }
                if result.isFinal {
                    self.finish()
                    return
                }
            }
            if error != nil {
                self.finish(errorCode: "recognition_failed")
            }
        }
    }

    private func installSignalHandlers() {
        signal(SIGINT, SIG_IGN)
        signal(SIGTERM, SIG_IGN)
        signalSources = [SIGINT, SIGTERM].map { signalNumber in
            let source = DispatchSource.makeSignalSource(signal: signalNumber, queue: .main)
            source.setEventHandler { [weak self] in self?.finish() }
            source.resume()
            return source
        }
    }

    private func finish(errorCode: String? = nil) {
        stateLock.lock()
        guard !isStopping else {
            stateLock.unlock()
            return
        }
        isStopping = true
        stateLock.unlock()

        if audioEngine.isRunning {
            audioEngine.stop()
        }
        if tapInstalled {
            audioEngine.inputNode.removeTap(onBus: 0)
            tapInstalled = false
        }
        recognitionRequest?.endAudio()
        recognitionTask?.cancel()
        recognitionTask = nil
        recognitionRequest = nil

        if let errorCode {
            writer.write(text: "", isFinal: true, errorCode: errorCode)
        }
        fflush(stdout)
        exit(errorCode == nil ? EXIT_SUCCESS : EXIT_FAILURE)
    }
}

// Keep the bridge alive for the lifetime of the helper. In an optimized build,
// constructing it as a temporary lets Swift release it after `run()` enters the
// dispatch loop. The permission callbacks intentionally capture `self` weakly,
// so that release leaves the helper running with nobody left to start the audio
// engine after the user grants access.
private let speechBridge = SpeechBridge()
speechBridge.run()
