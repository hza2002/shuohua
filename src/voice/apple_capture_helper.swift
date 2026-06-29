import AVFoundation
import Foundation

@main
struct AppleCaptureHelper {
    static func main() {
        do {
            let options = try Options.parse(CommandLine.arguments)
            switch options.mode {
            case .selfTest:
                emitReady()
                exit(0)
            case .capture:
                try runCapture(durationMs: nil)
                exit(0)
            case let .captureSmoke(durationMs):
                try runCapture(durationMs: durationMs)
                exit(0)
            case let .lifecycleSmoke(durationMs, gapMs):
                try runLifecycleSmoke(durationMs: durationMs, gapMs: gapMs)
                exit(0)
            case .server:
                try runServer()
                exit(0)
            case .none:
                emitError("apple capture helper not implemented", code: "not_implemented")
                exit(1)
            }
        } catch {
            emitError("\(error)")
            exit(1)
        }
    }
}

private enum Mode {
    case none
    case selfTest
    case capture
    case captureSmoke(durationMs: Int)
    case lifecycleSmoke(durationMs: Int, gapMs: Int)
    case server
}

private struct Options {
    var mode: Mode = .none

    static func parse(_ args: [String]) throws -> Options {
        var options = Options()
        var index = 1
        while index < args.count {
            switch args[index] {
            case "--self-test":
                options.mode = .selfTest
            case "--capture":
                options.mode = .capture
            case "--capture-smoke-ms":
                let value = try value(after: "--capture-smoke-ms", args: args, index: &index)
                guard let durationMs = Int(value), durationMs > 0 else {
                    throw HelperError.invalidDuration(value)
                }
                options.mode = .captureSmoke(durationMs: durationMs)
            case "--lifecycle-smoke-ms":
                let value = try value(after: "--lifecycle-smoke-ms", args: args, index: &index)
                guard let durationMs = Int(value), durationMs > 0 else {
                    throw HelperError.invalidDuration(value)
                }
                options.mode = .lifecycleSmoke(durationMs: durationMs, gapMs: 2_000)
            case "--lifecycle-gap-ms":
                let value = try value(after: "--lifecycle-gap-ms", args: args, index: &index)
                guard let gapMs = Int(value), gapMs >= 0 else {
                    throw HelperError.invalidDuration(value)
                }
                switch options.mode {
                case let .lifecycleSmoke(durationMs, _):
                    options.mode = .lifecycleSmoke(durationMs: durationMs, gapMs: gapMs)
                default:
                    throw HelperError.unknownArgument(args[index])
                }
            case "--server":
                options.mode = .server
            default:
                throw HelperError.unknownArgument(args[index])
            }
            index += 1
        }
        return options
    }

    private static func value(after flag: String, args: [String], index: inout Int) throws -> String {
        let valueIndex = index + 1
        guard valueIndex < args.count else {
            throw HelperError.missingValue(flag)
        }
        index = valueIndex
        return args[valueIndex]
    }
}

private enum HelperError: Error, CustomStringConvertible {
    case invalidDuration(String)
    case microphoneDenied
    case missingValue(String)
    case protocolError(String)
    case unsupportedBufferFormat
    case converterUnavailable
    case unknownArgument(String)

    var description: String {
        switch self {
        case let .invalidDuration(value):
            return "invalid capture smoke duration \(value)"
        case .microphoneDenied:
            return "microphone permission denied"
        case let .missingValue(flag):
            return "\(flag) requires a value"
        case let .protocolError(message):
            return message
        case .unsupportedBufferFormat:
            return "unsupported input buffer format"
        case .converterUnavailable:
            return "unable to create 16k mono audio converter"
        case let .unknownArgument(argument):
            return "unknown argument \(argument)"
        }
    }
}

private func runCapture(durationMs: Int?) throws {
    let timing = TimingLog()
    let capture = try VoiceProcessedCapture(timing: timing)
    try capture.start()
    if let durationMs {
        Thread.sleep(forTimeInterval: Double(durationMs) / 1_000.0)
    } else {
        dispatchMain()
    }
    capture.stop()
}

private func runServer() throws {
    let timing = TimingLog()
    let capture = try VoiceProcessedCapture(timing: timing)
    emit([
        "event": "server_ready",
        "sample_rate": 16_000,
        "channels": 1,
    ])

    var running = true
    while running, let line = readLine(strippingNewline: true) {
        guard let command = try parseCommand(line) else {
            continue
        }
        switch command {
        case .start:
            try capture.start()
        case .stop:
            capture.stop()
            emit(["event": "stopped"])
        case .quit:
            capture.stop()
            running = false
        }
    }
    capture.stop()
}

private func runLifecycleSmoke(durationMs: Int, gapMs: Int) throws {
    let timing = TimingLog()
    guard requestRecordPermission() else {
        emitError("microphone permission denied", code: "tcc_denied")
        throw HelperError.microphoneDenied
    }
    timing.mark("permission")

    let engine = AVAudioEngine()
    timing.mark("engine_alloc")
    let input = engine.inputNode
    timing.mark("input_node")
    try input.setVoiceProcessingEnabled(true)
    timing.mark("voice_processing")

    let format = input.outputFormat(forBus: 0)

    for round in 1...2 {
        var frames = 0
        let frameLock = NSLock()
        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { _, _ in
            frameLock.lock()
            frames += 1
            frameLock.unlock()
        }
        timing.mark("install_tap_\(round)")

        let start = DispatchTime.now().uptimeNanoseconds
        try engine.start()
        let startMs = (DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
        emit([
            "event": "lifecycle",
            "phase": "started",
            "round": round,
            "engine_start_ms": Int(startMs),
        ])
        Thread.sleep(forTimeInterval: Double(durationMs) / 1_000.0)
        engine.stop()
        frameLock.lock()
        let roundFrames = frames
        frameLock.unlock()
        input.removeTap(onBus: 0)
        engine.reset()
        emit([
            "event": "lifecycle",
            "phase": "stopped",
            "round": round,
            "frames": roundFrames,
        ])
        if round == 1 && gapMs > 0 {
            emit([
                "event": "lifecycle",
                "phase": "gap",
                "duration_ms": gapMs,
            ])
            Thread.sleep(forTimeInterval: Double(gapMs) / 1_000.0)
        }
    }

    timing.mark("done")
}

private enum ServerCommand {
    case start
    case stop
    case quit
}

private func parseCommand(_ line: String) throws -> ServerCommand? {
    guard let data = line.data(using: .utf8),
          let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        throw HelperError.protocolError("invalid server command JSON")
    }
    guard let command = object["cmd"] as? String else {
        throw HelperError.protocolError("missing server command")
    }
    switch command {
    case "start":
        return .start
    case "stop":
        return .stop
    case "quit":
        return .quit
    default:
        throw HelperError.protocolError("unknown server command \(command)")
    }
}

private final class VoiceProcessedCapture {
    private let timing: TimingLog
    private let engine: AVAudioEngine
    private let input: AVAudioInputNode
    private let format: AVAudioFormat
    // 取第一声道转出的 mono float（采集源采样率）作为重采样输入格式。
    private let inputFormat: AVAudioFormat
    private let outputFormat: AVAudioFormat
    private let converter: AVAudioConverter
    private let writerQueue = DispatchQueue(label: "shuohua.apple-capture-writer")
    private var firstFrame = true
    private var readyEmitted = false
    private var accepting = false
    private var converterError: String?
    private var sessionId: UInt64 = 0
    private var running = false

    init(timing: TimingLog) throws {
        self.timing = timing
        guard requestRecordPermission() else {
            emitError("microphone permission denied", code: "tcc_denied")
            throw HelperError.microphoneDenied
        }
        timing.mark("permission")

        engine = AVAudioEngine()
        timing.mark("engine_alloc")
        input = engine.inputNode
        timing.mark("input_node")
        try input.setVoiceProcessingEnabled(true)
        timing.mark("voice_processing")
        format = input.outputFormat(forBus: 0)
        timing.mark("format")
        // AVAudioConverter 带抗混叠 SRC，替代手写线性插值降采样。
        guard let inputFormat = AVAudioFormat(
                  commonFormat: .pcmFormatFloat32,
                  sampleRate: format.sampleRate,
                  channels: 1,
                  interleaved: false
              ),
              let outputFormat = AVAudioFormat(
                  commonFormat: .pcmFormatInt16,
                  sampleRate: 16_000,
                  channels: 1,
                  interleaved: true
              ),
              let converter = AVAudioConverter(from: inputFormat, to: outputFormat)
        else {
            throw HelperError.converterUnavailable
        }
        self.inputFormat = inputFormat
        self.outputFormat = outputFormat
        self.converter = converter
    }

    func start() throws {
        guard !running else {
            return
        }
        let session = writerQueue.sync {
            converter.reset()
            firstFrame = true
            readyEmitted = false
            accepting = true
            converterError = nil
            sessionId &+= 1
            return sessionId
        }
        input.installTap(onBus: 0, bufferSize: 1_024, format: format) { [weak self] buffer, _ in
            self?.accept(buffer, session: session)
        }
        timing.mark("install_tap")
        try engine.start()
        running = true
        timing.mark("engine_start")
        writerQueue.sync {
            guard session == sessionId, accepting else {
                return
            }
            emitReady()
            readyEmitted = true
        }
        timing.mark("ready")
    }

    func stop() {
        guard running else {
            return
        }
        engine.stop()
        input.removeTap(onBus: 0)
        engine.reset()
        running = false
        // writerQueue 是串行队列：sync 块排在所有已入队的 accept 之后，先把 converter
        // 内部 SRC 残留抽干(对齐 cpal rubato finish 的 drain 不变量)，再发 is_last。
        writerQueue.sync {
            accepting = false
            let tail = drainConverter()
            if !tail.isEmpty {
                writePcmFrame(tail, isLast: false)
            }
            writePcmFrame([], isLast: true)
            readyEmitted = false
            if let converterError {
                emitError(converterError, code: "converter_error")
                self.converterError = nil
            }
        }
    }

    private func accept(_ buffer: AVAudioPCMBuffer, session: UInt64) {
        guard let samples = try? copyFirstChannelSamples(from: buffer) else {
            return
        }
        writerQueue.async { [self] in
            guard session == sessionId, accepting, readyEmitted else {
                return
            }
            let pcm = convertToPcm16k(samples)
            if !pcm.isEmpty {
                writePcmFrame(pcm, isLast: false)
                if firstFrame {
                    timing.mark("first_frame")
                    firstFrame = false
                }
            }
        }
    }

    private func convertToPcm16k(_ samples: [Float]) -> [Int16] {
        guard !samples.isEmpty, let inputBuffer = makeInputBuffer(samples) else {
            return []
        }
        let ratio = outputFormat.sampleRate / inputFormat.sampleRate
        let capacity = AVAudioFrameCount((Double(samples.count) * ratio).rounded(.up)) + 32
        guard let output = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: capacity) else {
            return []
        }
        var consumed = false
        var conversionError: NSError?
        let status = converter.convert(to: output, error: &conversionError) { _, inputStatus in
            if consumed {
                inputStatus.pointee = .noDataNow
                return nil
            }
            consumed = true
            inputStatus.pointee = .haveData
            return inputBuffer
        }
        guard status == .haveData || status == .inputRanDry,
              let channel = output.int16ChannelData else {
            if status == .error, let error = conversionError {
                converterError = "audio converter error: \(error.localizedDescription)"
                accepting = false
            }
            return []
        }
        let count = Int(output.frameLength)
        guard count > 0 else {
            return []
        }
        return Array(UnsafeBufferPointer(start: channel[0], count: count))
    }

    private func drainConverter() -> [Int16] {
        var drained: [Int16] = []
        while true {
            guard let output = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: 512) else {
                return drained
            }
            var conversionError: NSError?
            let status = converter.convert(to: output, error: &conversionError) { _, inputStatus in
                inputStatus.pointee = .endOfStream
                return nil
            }
            guard status == .haveData || status == .endOfStream,
                  let channel = output.int16ChannelData else {
                if status == .error, let error = conversionError {
                    converterError = "audio converter drain error: \(error.localizedDescription)"
                }
                return drained
            }
            let count = Int(output.frameLength)
            if count > 0 {
                drained.append(contentsOf: UnsafeBufferPointer(start: channel[0], count: count))
            }
            if status == .endOfStream {
                return drained
            }
        }
    }

    private func makeInputBuffer(_ samples: [Float]) -> AVAudioPCMBuffer? {
        let frames = AVAudioFrameCount(samples.count)
        guard frames > 0,
              let buffer = AVAudioPCMBuffer(pcmFormat: inputFormat, frameCapacity: frames),
              let channel = buffer.floatChannelData
        else {
            return nil
        }
        buffer.frameLength = frames
        samples.withUnsafeBufferPointer { src in
            // baseAddress 对 count > 0 的 buffer 永不为 nil。
            channel[0].update(from: src.baseAddress!, count: samples.count)
        }
        return buffer
    }
}

private final class TimingLog {
    private let started = DispatchTime.now().uptimeNanoseconds

    func mark(_ name: String) {
        let elapsedMs = (DispatchTime.now().uptimeNanoseconds - started) / 1_000_000
        FileHandle.standardError.write(Data("apple_capture_timing step=\(name) elapsed_ms=\(elapsedMs)\n".utf8))
    }
}

private func requestRecordPermission() -> Bool {
    switch AVAudioApplication.shared.recordPermission {
    case .granted:
        return true
    case .denied:
        return false
    case .undetermined:
        let semaphore = DispatchSemaphore(value: 0)
        var granted = false
        AVAudioApplication.requestRecordPermission { allowed in
            granted = allowed
            semaphore.signal()
        }
        semaphore.wait()
        return granted
    @unknown default:
        return false
    }
}

private func copyFirstChannelSamples(from buffer: AVAudioPCMBuffer) throws -> [Float] {
    let frameLength = Int(buffer.frameLength)
    if frameLength == 0 {
        return []
    }
    if let channels = buffer.floatChannelData {
        return Array(UnsafeBufferPointer(start: channels[0], count: frameLength))
    }
    if let channels = buffer.int16ChannelData {
        return UnsafeBufferPointer(start: channels[0], count: frameLength).map {
            Float($0) / 32_768.0
        }
    }
    throw HelperError.unsupportedBufferFormat
}

private func writePcmFrame(_ pcm: [Int16], isLast: Bool) {
    var data = Data(capacity: 5 + pcm.count * 2)
    data.append(isLast ? 1 : 0)
    var count = UInt32(pcm.count).littleEndian
    withUnsafeBytes(of: &count) { data.append(contentsOf: $0) }
    for sample in pcm {
        var value = sample.littleEndian
        withUnsafeBytes(of: &value) { data.append(contentsOf: $0) }
    }
    FileHandle.standardOutput.write(data)
}

private func emitReady() {
    emit([
        "event": "ready",
        "sample_rate": 16_000,
        "channels": 1,
    ])
}

private func emit(_ object: [String: Any]) {
    guard JSONSerialization.isValidJSONObject(object),
          let data = try? JSONSerialization.data(withJSONObject: object) else {
        return
    }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0a]))
}

private func emitError(_ message: String, code: String? = "helper_error") {
    var payload = ["event": "error", "message": message]
    if let code {
        payload["code"] = code
    }
    emit(payload)
}
