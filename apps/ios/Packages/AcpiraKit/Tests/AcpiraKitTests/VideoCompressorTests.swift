import AVFoundation
import Foundation
import Testing
@testable import AcpiraUI

// The picked-video compressor against a clip made on the spot: a portrait 1080p 60 fps recording with sound comes out
// as a 720p, 30 fps MP4 with its rotation and audio kept, and a budget nothing could fit is refused.

@Suite struct VideoCompressorTests {
  @Test func shrinksAndKeepsRotationAndSound() async throws {
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent("video-test-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: folder) }
    let source = folder.appendingPathComponent("source.mov")
    let portrait = CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: 0, ty: 0)
    try await Clip.write(to: source, size: CGSize(width: 1920, height: 1080), fps: 60, seconds: 2, transform: portrait)

    let out = folder.appendingPathComponent("out/video.mp4")
    let seen = Seen()
    try await VideoCompressor.compress(source, to: out, budget: VideoCompressor.maxBytes) { seen.last = $0 }

    let asset = AVURLAsset(url: out)
    let video = try #require(try await asset.loadTracks(withMediaType: .video).first)
    let (size, transform, fps) = try await video.load(.naturalSize, .preferredTransform, .nominalFrameRate)
    #expect(size == CGSize(width: 1280, height: 720))
    #expect(transform == portrait)
    #expect(fps <= 31)
    #expect(try await asset.loadTracks(withMediaType: .audio).count == 1)
    #expect(abs(try await asset.load(.duration).seconds - 2) < 0.2)
    #expect(seen.last == 1)
  }

  @Test func refusesWhatNoBitrateFits() async throws {
    let folder = FileManager.default.temporaryDirectory.appendingPathComponent("video-test-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: folder) }
    let source = folder.appendingPathComponent("source.mov")
    try await Clip.write(to: source, size: CGSize(width: 640, height: 360), fps: 30, seconds: 2, transform: .identity)
    await #expect(throws: VideoCompressor.Failure.tooBig) {
      try await VideoCompressor.compress(source, to: folder.appendingPathComponent("out.mp4"), budget: 50_000) { _ in }
    }
  }
}

private final class Seen: @unchecked Sendable {
  var last: Double?
}

/// A synthetic recording: moving grey frames and a sine tone
private enum Clip {
  static func write(to url: URL, size: CGSize, fps: Int, seconds: Int, transform: CGAffineTransform) async throws {
    try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
    let writer = try AVAssetWriter(outputURL: url, fileType: .mov)
    let video = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: size.width, AVVideoHeightKey: size.height,
    ])
    video.transform = transform
    let pixels = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: video, sourcePixelBufferAttributes: [
      kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
      kCVPixelBufferWidthKey as String: size.width, kCVPixelBufferHeightKey as String: size.height,
    ])
    let audio = AVAssetWriterInput(mediaType: .audio, outputSettings: [
      AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: 44_100, AVNumberOfChannelsKey: 1,
    ])
    writer.add(video)
    writer.add(audio)
    #expect(writer.startWriting())
    writer.startSession(atSourceTime: .zero)

    for frame in 0..<(fps * seconds) {
      while !video.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(1)) }
      var buffer: CVPixelBuffer?
      CVPixelBufferPoolCreatePixelBuffer(nil, try #require(pixels.pixelBufferPool), &buffer)
      let pixel = try #require(buffer)
      CVPixelBufferLockBaseAddress(pixel, [])
      memset(CVPixelBufferGetBaseAddress(pixel), Int32(frame * 4 % 256), CVPixelBufferGetDataSize(pixel))
      CVPixelBufferUnlockBaseAddress(pixel, [])
      pixels.append(pixel, withPresentationTime: CMTime(value: CMTimeValue(frame), timescale: CMTimeScale(fps)))
    }
    video.markAsFinished()

    // One second of 16-bit mono PCM per buffer
    let rate = 44_100
    var asbd = AudioStreamBasicDescription(
      mSampleRate: Double(rate), mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kLinearPCMFormatFlagIsSignedInteger | kLinearPCMFormatFlagIsPacked,
      mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2, mChannelsPerFrame: 1, mBitsPerChannel: 16, mReserved: 0)
    var format: CMAudioFormatDescription?
    CMAudioFormatDescriptionCreate(allocator: nil, asbd: &asbd, layoutSize: 0, layout: nil, magicCookieSize: 0,
                                   magicCookie: nil, extensions: nil, formatDescriptionOut: &format)
    for second in 0..<seconds {
      while !audio.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(1)) }
      var samples = (0..<rate).map { Int16(sin(Double($0) * 2 * .pi * 440 / Double(rate)) * 8000) }
      var block: CMBlockBuffer?
      CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: rate * 2, blockAllocator: nil,
                                         customBlockSource: nil, offsetToData: 0, dataLength: rate * 2, flags: 0,
                                         blockBufferOut: &block)
      CMBlockBufferReplaceDataBytes(with: &samples, blockBuffer: try #require(block), offsetIntoDestination: 0, dataLength: rate * 2)
      var sample: CMSampleBuffer?
      CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: nil, dataBuffer: try #require(block), formatDescription: try #require(format), sampleCount: rate,
        presentationTimeStamp: CMTime(value: CMTimeValue(second), timescale: 1), packetDescriptions: nil,
        sampleBufferOut: &sample)
      audio.append(try #require(sample))
    }
    audio.markAsFinished()
    await writer.finishWriting()
    #expect(writer.status == .completed)
  }
}
