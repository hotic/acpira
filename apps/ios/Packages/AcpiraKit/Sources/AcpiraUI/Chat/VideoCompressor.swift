import AVFoundation
import Foundation

/// Re-encodes a picked video into an H.264 / AAC MP4 small enough for the paired link, the way chat apps do before an
/// upload. A reader / writer pair rather than an export preset: the presets fix the bitrate, and the bitrate is what
/// has to fit the size budget. The long side, the bitrate and the frame rate step down together as the video gets
/// longer; a video that would need less than the floor bitrate is refused rather than sent as mush.
nonisolated enum VideoCompressor {
  enum Failure: Error { case tooBig, unreadable }

  /// The largest copy that goes. The engine takes up to 50 MiB (`MAX_FILE_BYTES`), but the paired relay uploads slowly,
  /// so the budget stays where chat apps put theirs (WeChat about 25 MB, WhatsApp 16 MB); longer videos step down the ladder
  static let maxBytes = 35 << 20

  /// Long side and its full bitrate, best first; a step is taken while the budget affords its bitrate
  private static let ladder: [(side: CGFloat, bitrate: Double)] = [(1280, 2_500_000), (960, 1_500_000), (640, 800_000)]
  /// Below this the picture falls apart: the video is too long for the budget
  private static let floorBitrate: Double = 300_000
  private static let audioBitrate: Double = 96_000
  private static let maxFrameRate: Float = 30
  /// How long the copy loop waits with neither input taking data before it gives up
  private static let stallLimit: Duration = .seconds(20)

  /// Writes `source` compressed to `out` (its folder is made), reporting 0...1 as it goes
  @concurrent static func compress(_ source: URL, to out: URL, budget: Int,
                                    progress: @escaping @Sendable (Double) -> Void) async throws {
    let asset = AVURLAsset(url: source)
    guard let video = try await asset.loadTracks(withMediaType: .video).first else { throw Failure.unreadable }
    let audio = try await asset.loadTracks(withMediaType: .audio).first
    let duration = try await asset.load(.duration).seconds
    let (natural, transform, fps) = try await video.load(.naturalSize, .preferredTransform, .nominalFrameRate)
    guard duration > 0, natural.width > 0, natural.height > 0 else { throw Failure.unreadable }

    // What the budget allows per second, less the audio and a tenth for the encoder's swings around its average
    let affordable = Double(budget) * 8 * 0.9 / duration - (audio == nil ? 0 : audioBitrate)
    guard affordable >= floorBitrate else { throw Failure.tooBig }
    let step = ladder.first { $0.bitrate <= affordable } ?? ladder[ladder.count - 1]
    let bitrate = min(step.bitrate, affordable)
    // Never upscaled; even sides, as H.264 wants
    let scale = min(1, step.side / max(natural.width, natural.height))
    let width = Int((natural.width * scale / 2).rounded()) * 2
    let height = Int((natural.height * scale / 2).rounded()) * 2

    let reader = try AVAssetReader(asset: asset)
    // Decoded straight to the target size in Rec. 709: asking for SDR colour tone-maps an HDR (HLG / Dolby Vision) clip
    // instead of leaving its washed-out 10-bit values in an 8-bit stream
    let sdr: [String: Any] = [
      AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
      AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
      AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2,
    ]
    let videoOut = AVAssetReaderTrackOutput(track: video, outputSettings: [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
      kCVPixelBufferWidthKey as String: width,
      kCVPixelBufferHeightKey as String: height,
      AVVideoColorPropertiesKey: sdr,
    ])
    videoOut.alwaysCopiesSampleData = false
    reader.add(videoOut)
    let audioOut = audio.map {
      AVAssetReaderTrackOutput(track: $0, outputSettings: [
        AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 2,
        AVLinearPCMBitDepthKey: 16, AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false,
        AVLinearPCMIsNonInterleaved: false,
      ])
    }
    if let audioOut { reader.add(audioOut) }

    try FileManager.default.createDirectory(at: out.deletingLastPathComponent(), withIntermediateDirectories: true)
    try? FileManager.default.removeItem(at: out)
    let writer = try AVAssetWriter(outputURL: out, fileType: .mp4)
    // The moov atom up front, so a player on the computer starts before reading the whole file
    writer.shouldOptimizeForNetworkUse = true
    let frameRate = min(fps > 0 ? fps : maxFrameRate, maxFrameRate)
    let videoIn = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: AVVideoCodecType.h264,
      AVVideoWidthKey: width,
      AVVideoHeightKey: height,
      AVVideoScalingModeKey: AVVideoScalingModeResizeAspect,
      AVVideoColorPropertiesKey: sdr,
      AVVideoCompressionPropertiesKey: [
        AVVideoAverageBitRateKey: bitrate,
        AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
        AVVideoExpectedSourceFrameRateKey: frameRate,
        AVVideoMaxKeyFrameIntervalDurationKey: 2,
        AVVideoAllowFrameReorderingKey: true,
      ],
    ])
    // Rotation stays the track's own: a portrait clip keeps its stored landscape frames and its transform
    videoIn.transform = transform
    videoIn.expectsMediaDataInRealTime = false
    writer.add(videoIn)
    let audioIn = audioOut.map { _ in
      AVAssetWriterInput(mediaType: .audio, outputSettings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 2,
        AVEncoderBitRateKey: audioBitrate,
      ])
    }
    if let audioIn {
      audioIn.expectsMediaDataInRealTime = false
      writer.add(audioIn)
    }

    guard reader.startReading(), writer.startWriting() else { throw Failure.unreadable }
    writer.startSession(atSourceTime: .zero)

    // One loop feeds both inputs in turn: the writer interleaves them and stalls one that runs ahead of the other.
    // A clip shot above 30 fps drops frames down to it; frame times keep their spacing, so playback speed holds
    var videoDone = false, audioDone = audioIn == nil
    var lastKept = -Double.infinity
    let minGap = 1 / Double(maxFrameRate) - 0.002
    var lastMoved = ContinuousClock.now
    do {
      while !videoDone || !audioDone {
        try Task.checkCancellation()
        var moved = false
        if !videoDone, videoIn.isReadyForMoreMediaData {
          moved = true
          if let buffer = videoOut.copyNextSampleBuffer() {
            let at = buffer.presentationTimeStamp.seconds
            if at - lastKept >= minGap {
              lastKept = at
              guard videoIn.append(buffer) else { throw Failure.unreadable }
              progress(min(1, at / duration))
            }
          } else {
            videoIn.markAsFinished()
            videoDone = true
          }
        }
        if !audioDone, let audioIn, let audioOut, audioIn.isReadyForMoreMediaData {
          moved = true
          if let buffer = audioOut.copyNextSampleBuffer() {
            guard audioIn.append(buffer) else { throw Failure.unreadable }
          } else {
            audioIn.markAsFinished()
            audioDone = true
          }
        }
        if moved {
          lastMoved = .now
        } else {
          // A failed writer never reports ready again, and an encoder can stall without failing (seen on a CI VM
          // without a hardware encoder): either would spin here for good
          if writer.status == .failed || reader.status == .failed { throw Failure.unreadable }
          if lastMoved.duration(to: .now) > stallLimit { throw Failure.unreadable }
          try await Task.sleep(for: .milliseconds(2))
        }
      }
    } catch {
      reader.cancelReading()
      writer.cancelWriting()
      throw error
    }
    guard reader.status == .completed else {
      writer.cancelWriting()
      throw Failure.unreadable
    }
    await writer.finishWriting()
    guard writer.status == .completed else { throw Failure.unreadable }
    let size = (try? out.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
    guard size <= budget else { throw Failure.tooBig }
    progress(1)
  }
}
