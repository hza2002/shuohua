import AVFoundation
import CoreMedia
import Foundation
import Speech

@main
struct AppleHelper {
    static func main() async {
        do {
            let options = try Options.parse(CommandLine.arguments)
            if #available(macOS 26.0, *) {
                try await run(options)
            } else {
                emitError("SpeechAnalyzer requires macOS 26 or newer", code: "unsupported_os")
                exit(1)
            }
        } catch {
            emitError("\(error)")
            exit(1)
        }
    }

    @available(macOS 26.0, *)
    private static func run(_ options: Options) async throws {
        let locale = Locale(identifier: options.language)
        let transcriber = SpeechTranscriber(
            locale: locale,
            transcriptionOptions: [],
            reportingOptions: [.volatileResults, .fastResults],
            attributeOptions: [.audioTimeRange]
        )

        if options.installAssets {
            try await installAssetsIfNeeded(for: [transcriber])
        }

        let targetFormat = await SpeechAnalyzer.bestAvailableAudioFormat(
            compatibleWith: [transcriber],
            considering: canonicalFormat()
        ) ?? canonicalFormat()
        guard isSupportedFormat(targetFormat) else {
            throw HelperError.unsupportedFormat(describe(targetFormat))
        }

        let analyzer = SpeechAnalyzer(
            modules: [transcriber],
            options: SpeechAnalyzer.Options(priority: .userInitiated, modelRetention: .whileInUse)
        )
        try await analyzer.setContext(analysisContext(hotwords: options.hotwords))

        let resultTask = Task {
            try await emitResults(from: transcriber)
        }

        try await analyzer.prepareToAnalyze(in: targetFormat)
        let stream = AsyncThrowingStream<AnalyzerInput, Error> { continuation in
            Task.detached {
                do {
                    try readPcmFrames(into: continuation, format: targetFormat)
                } catch {
                    continuation.finish(throwing: error)
                }
            }
        }
        try await analyzer.start(inputSequence: stream)
        try await analyzer.finalizeAndFinishThroughEndOfInput()
        try await resultTask.value
        emit(["event": "done"])
    }

    @available(macOS 26.0, *)
    private static func emitResults(from transcriber: SpeechTranscriber) async throws {
        var seq: UInt64 = 0
        for try await result in transcriber.results {
            let text = String(result.text.characters)
            if result.isFinal {
                emit([
                    "event": "segment",
                    "text": text,
                    "start_ms": milliseconds(result.range.start),
                    "end_ms": milliseconds(CMTimeRangeGetEnd(result.range)),
                ])
            } else {
                seq += 1
                emit([
                    "event": "partial",
                    "text": text,
                    "seq": seq,
                ])
            }
        }
    }

    @available(macOS 26.0, *)
    private static func readPcmFrames(
        into continuation: AsyncThrowingStream<AnalyzerInput, Error>.Continuation,
        format: AVAudioFormat
    ) throws {
        let input = FileHandle.standardInput
        var sampleOffset: Int64 = 0

        while true {
            guard let header = readExactly(5, from: input) else {
                continuation.finish()
                return
            }
            let isLast = header[0] & 1 == 1
            let sampleCount = Int(UInt32(header[1]) |
                (UInt32(header[2]) << 8) |
                (UInt32(header[3]) << 16) |
                (UInt32(header[4]) << 24))

            guard let payload = readExactly(sampleCount * 2, from: input) else {
                throw HelperError.truncatedFrame
            }
            if sampleCount > 0 {
                let buffer = try makeBuffer(samples: payload, sampleCount: sampleCount, format: format)
                let start = CMTime(value: sampleOffset, timescale: 16_000)
                continuation.yield(AnalyzerInput(buffer: buffer, bufferStartTime: start))
                sampleOffset += Int64(sampleCount)
            }
            if isLast {
                continuation.finish()
                return
            }
        }
    }

    private static func readExactly(_ count: Int, from input: FileHandle) -> Data? {
        var data = Data()
        while data.count < count {
            let chunk = input.readData(ofLength: count - data.count)
            if chunk.isEmpty {
                return data.isEmpty ? nil : data
            }
            data.append(chunk)
        }
        return data
    }

    private static func makeBuffer(
        samples: Data,
        sampleCount: Int,
        format: AVAudioFormat
    ) throws -> AVAudioPCMBuffer {
        guard let buffer = AVAudioPCMBuffer(
            pcmFormat: format,
            frameCapacity: AVAudioFrameCount(sampleCount)
        ) else {
            throw HelperError.bufferAllocationFailed
        }
        try samples.withUnsafeBytes { raw in
            let bytes = raw.bindMemory(to: UInt8.self)
            switch format.commonFormat {
            case .pcmFormatInt16:
                guard let channel = buffer.int16ChannelData?[0] else {
                    throw HelperError.bufferChannelUnavailable
                }
                for i in 0..<sampleCount {
                    channel[i] = readInt16(bytes, at: i)
                }
            case .pcmFormatFloat32:
                guard let channel = buffer.floatChannelData?[0] else {
                    throw HelperError.bufferChannelUnavailable
                }
                for i in 0..<sampleCount {
                    channel[i] = Float(readInt16(bytes, at: i)) / 32768.0
                }
            default:
                throw HelperError.unsupportedFormat(describe(format))
            }
        }
        buffer.frameLength = AVAudioFrameCount(sampleCount)
        return buffer
    }

    @available(macOS 26.0, *)
    private static func installAssetsIfNeeded(for modules: [any SpeechModule]) async throws {
        let status = await AssetInventory.status(forModules: modules)
        guard status != .installed else {
            return
        }
        guard let request = try await AssetInventory.assetInstallationRequest(supporting: modules) else {
            throw HelperError.assetsUnavailable
        }
        try await request.downloadAndInstall()
    }

    @available(macOS 26.0, *)
    private static func analysisContext(hotwords: [String]) -> AnalysisContext {
        let context = AnalysisContext()
        if !hotwords.isEmpty {
            context.contextualStrings[.general] = hotwords
        }
        return context
    }
}

struct Options {
    var language = "zh-CN"
    var hotwords: [String] = []
    var installAssets = false

    static func parse(_ args: [String]) throws -> Options {
        var options = Options()
        var index = 1
        while index < args.count {
            switch args[index] {
            case "--language":
                options.language = try value(after: "--language", args: args, index: &index)
            case "--hotwords":
                options.hotwords = try value(after: "--hotwords", args: args, index: &index)
                    .split(separator: ",")
                    .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                    .filter { !$0.isEmpty }
            case "--install-assets":
                options.installAssets = true
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

enum HelperError: Error, CustomStringConvertible {
    case assetsUnavailable
    case bufferAllocationFailed
    case bufferChannelUnavailable
    case missingValue(String)
    case truncatedFrame
    case unknownArgument(String)
    case unsupportedFormat(String)

    var description: String {
        switch self {
        case .assetsUnavailable:
            return "SpeechAnalyzer assets unavailable"
        case .bufferAllocationFailed:
            return "failed to allocate AVAudioPCMBuffer"
        case .bufferChannelUnavailable:
            return "failed to access AVAudioPCMBuffer channel data"
        case let .missingValue(flag):
            return "\(flag) requires a value"
        case .truncatedFrame:
            return "truncated PCM frame"
        case let .unknownArgument(arg):
            return "unknown argument \(arg)"
        case let .unsupportedFormat(format):
            return "unsupported SpeechAnalyzer audio format \(format)"
        }
    }
}

private func canonicalFormat() -> AVAudioFormat {
    AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: 16_000,
        channels: 1,
        interleaved: false
    )!
}

private func isSupportedFormat(_ format: AVAudioFormat) -> Bool {
    (format.commonFormat == .pcmFormatFloat32 || format.commonFormat == .pcmFormatInt16)
        && abs(format.sampleRate - 16_000) < 0.1
        && format.channelCount == 1
}

private func readInt16(_ bytes: UnsafeBufferPointer<UInt8>, at sampleIndex: Int) -> Int16 {
    let lo = UInt16(bytes[sampleIndex * 2])
    let hi = UInt16(bytes[sampleIndex * 2 + 1]) << 8
    return Int16(bitPattern: hi | lo)
}

private func milliseconds(_ time: CMTime) -> UInt64 {
    guard time.isNumeric else {
        return 0
    }
    return UInt64((time.seconds * 1000.0).rounded())
}

private func describe(_ format: AVAudioFormat) -> String {
    "\(Int(format.sampleRate.rounded()))Hz \(format.channelCount)ch \(format.commonFormat)"
}

private func emit(_ object: [String: Any]) {
    guard JSONSerialization.isValidJSONObject(object),
          let data = try? JSONSerialization.data(withJSONObject: object) else {
        emitError("failed to encode helper event")
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
