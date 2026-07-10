import AVFoundation
import Darwin
import Foundation
import Speech

private let noSpeechExitCode: Int32 = 2

private func writeStandardOutput(_ value: String) {
    if let data = value.data(using: .utf8) {
        FileHandle.standardOutput.write(data)
    }
}

private func writeStandardError(_ value: String) {
    if let data = value.data(using: .utf8) {
        FileHandle.standardError.write(data)
    }
}

private final class DictationSession {
    private let audioEngine = AVAudioEngine()
    private let request = SFSpeechAudioBufferRecognitionRequest()
    private var task: SFSpeechRecognitionTask?
    private var latestTranscript = ""
    private var finished = false

    func start() {
        guard let recognizer = SFSpeechRecognizer(), recognizer.isAvailable else {
            finish(error: "speech recognition is unavailable for the current macOS locale")
            return
        }

        request.shouldReportPartialResults = true
        request.taskHint = .dictation
        request.addsPunctuation = true
        request.requiresOnDeviceRecognition = recognizer.supportsOnDeviceRecognition

        let input = audioEngine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            finish(error: "no microphone input format is available")
            return
        }

        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { [weak self] buffer, _ in
            self?.request.append(buffer)
        }

        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            DispatchQueue.main.async {
                guard let self, !self.finished else { return }
                if let result {
                    self.latestTranscript = result.bestTranscription.formattedString
                    if result.isFinal {
                        self.finish(transcript: self.latestTranscript)
                        return
                    }
                }
                if let error {
                    if self.latestTranscript.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                        self.finish(error: "macOS speech recognition failed: \(error.localizedDescription)")
                    } else {
                        self.finish(transcript: self.latestTranscript)
                    }
                }
            }
        }

        do {
            audioEngine.prepare()
            try audioEngine.start()
        } catch {
            finish(error: "could not start the microphone: \(error.localizedDescription)")
            return
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 15) { [weak self] in
            guard let self, !self.finished else { return }
            self.audioEngine.stop()
            self.audioEngine.inputNode.removeTap(onBus: 0)
            self.request.endAudio()

            DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
                guard let self, !self.finished else { return }
                self.finish(transcript: self.latestTranscript)
            }
        }
    }

    private func finish(transcript: String? = nil, error: String? = nil) {
        guard !finished else { return }
        finished = true

        if audioEngine.isRunning {
            audioEngine.stop()
            audioEngine.inputNode.removeTap(onBus: 0)
        }
        request.endAudio()
        task?.cancel()

        if let error {
            writeStandardError(error)
            Darwin.exit(1)
        }

        let value = (transcript ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else {
            Darwin.exit(noSpeechExitCode)
        }
        writeStandardOutput(value)
        Darwin.exit(0)
    }
}

private var session: DictationSession?

SFSpeechRecognizer.requestAuthorization { status in
    DispatchQueue.main.async {
        guard status == .authorized else {
            writeStandardError("speech recognition permission is denied; enable GridBash Speech in System Settings > Privacy & Security > Speech Recognition")
            Darwin.exit(1)
        }

        AVCaptureDevice.requestAccess(for: .audio) { granted in
            DispatchQueue.main.async {
                guard granted else {
                    writeStandardError("microphone permission is denied; enable GridBash Speech in System Settings > Privacy & Security > Microphone")
                    Darwin.exit(1)
                }

                let activeSession = DictationSession()
                session = activeSession
                activeSession.start()
            }
        }
    }
}

dispatchMain()
