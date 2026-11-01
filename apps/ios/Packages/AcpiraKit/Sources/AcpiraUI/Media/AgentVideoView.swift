import AcpiraDesign
import AcpiraModels
import AVKit
import Foundation
import SwiftUI

/// Recordings stay dormant until tapped. AVPlayer owns decoding; a view owns and removes its temporary file.
struct AgentVideoView: View {
  let video: ImageRef
  @Environment(\.media) private var media
  @Environment(\.scenePhase) private var phase
  @State private var requested = false
  @State private var failed = false
  @State private var player: AVPlayer?
  @State private var file: URL?
  @State private var visible = false

  var body: some View {
    Group {
      if let player {
        VideoPlayer(player: player)
      } else {
        Button { requested = true } label: {
          VStack(spacing: Metrics.gap) {
            if requested && !failed { ProgressView() }
            else { Icon(.play, size: Metrics.iconCtl) }
            Text(web: failed ? t("panel.error.media") : (video.uri.map { ($0 as NSString).lastPathComponent } ?? t("verb.showVideo")))
              .textStyle(WebType.text3)
              .lineLimit(2)
          }
          .foregroundStyle(Palette.fg3)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .buttonStyle(.plain)
        .disabled(requested)
      }
    }
    .frame(height: Metrics.agentImage)
    .frame(maxWidth: .infinity)
    .background(Palette.chip)
    .clipShape(RoundedRectangle(cornerRadius: Metrics.rMd))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.line))
    .onScrollVisibilityChange { showing in
      visible = showing
      if !showing { player?.pause() }
    }
    .onChange(of: phase) { _, next in if next != .active { player?.pause() } }
    .task(id: requested) {
      guard requested, player == nil else { return }
      guard let session = media.session,
            let blob = await media.loader.blobName(session: session, blob: video.blob, uri: video.uri) else { failed = true; return }
      do {
        let bytes = try await media.loader.video(session: session, blob: blob)
        try Task.checkCancellation()
        let ext = video.mimeType == "video/webm" ? "webm" : "mp4"
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).appendingPathExtension(ext)
        file = url
        try bytes.write(to: url, options: .atomic)
        let asset = AVURLAsset(url: url)
        guard try await asset.load(.isPlayable) else { failed = true; return }
        try Task.checkCancellation()
        let loaded = AVPlayer(playerItem: AVPlayerItem(asset: asset))
        player = loaded
        if visible && phase == .active { loaded.play() }
      } catch is CancellationError {
        requested = false
      } catch {
        failed = true
      }
    }
    .onDisappear {
      player?.pause()
      player = nil
      requested = false
      if let file { try? FileManager.default.removeItem(at: file) }
      file = nil
    }
  }
}
