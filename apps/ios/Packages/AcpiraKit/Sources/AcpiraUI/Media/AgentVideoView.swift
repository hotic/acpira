import AcpiraDesign
import AcpiraModels
import QuickLook
import SwiftUI

/// A `show_video` recording: a card that stays dormant until tapped (nothing is downloaded before), then fetches the
/// file once into the attached videos' cache (VideoCache) and plays it in Quick Look, as an attached video does. Once
/// the file is on the phone the card shows its first frame. Not an inline player: the thread's cells are
/// `UIHostingConfiguration`, which cannot host the view controller behind SwiftUI's `VideoPlayer` and drew its
/// unsupported-view placeholder instead
struct AgentVideoView: View {
  let video: ImageRef
  @Environment(\.media) private var media
  @Environment(\.displayScale) private var scale
  @State private var opening = false
  @State private var failed = false
  @State private var poster: CGImage?
  @State private var preview: URL?

  /// Where the bytes are: the saved copy, else the file on the engine's side, staged on demand (not over a paired link)
  private var source: VideoSource? {
    guard let session = media.session else { return nil }
    if let blob = video.blob { return .blob(session: session, name: blob) }
    guard let uri = video.uri, MediaSource.isFilePath(uri), media.loader.canStage else { return nil }
    return .staged(session: session, path: uri)
  }

  private var name: String { video.uri.map { ($0 as NSString).lastPathComponent } ?? t("verb.showVideo") }

  /// The poster's long side: a wide recording fills the card's width at about twice its height
  private var posterPixels: CGFloat { Metrics.agentImage * 2 * scale }

  var body: some View {
    Button(action: open) {
      ZStack {
        if let poster {
          Image(decorative: poster, scale: 1).resizable().scaledToFit()
          // The play mark on its own disc reads on any frame (VideoAttachmentTile)
          Group {
            if opening { ProgressView().controlSize(.small).tint(Color.white) }
            else { Icon(.play, size: WebMetrics.icon).foregroundStyle(Color.white) }
          }
          .frame(width: WebMetrics.ctlSm, height: WebMetrics.ctlSm)
          .background(Circle().fill(Palette.scrim))
        } else {
          VStack(spacing: Metrics.gap) {
            if opening { ProgressView() } else { Icon(.play, size: Metrics.iconCtl) }
            Text(web: failed ? t("panel.error.media") : name)
              .textStyle(WebType.text3)
              .lineLimit(2)
          }
          .foregroundStyle(Palette.fg3)
        }
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .disabled(opening)
    .frame(height: Metrics.agentImage)
    .frame(maxWidth: .infinity)
    .background(Palette.chip)
    .clipShape(RoundedRectangle(cornerRadius: Metrics.rMd))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.line))
    .accessibilityLabel(Text(web: name))
    .accessibilityHint(Text(web: t("common.open")))
    .quickLookPreview($preview)
    // A recording opened before (here or as the same bytes elsewhere) shows its first frame without a download
    .task(id: source) {
      guard poster == nil, let source, VideoCache.shared.kept(source) else { return }
      poster = try? await VideoCache.shared.poster(source, loader: media.loader, pixels: posterPixels)
    }
  }

  private func open() {
    guard let source else { failed = true; return }
    opening = true
    failed = false
    Task {
      defer { opening = false }
      do {
        preview = try await VideoCache.shared.file(source, loader: media.loader)
        if poster == nil { poster = try? await VideoCache.shared.poster(source, loader: media.loader, pixels: posterPixels) }
      } catch {
        failed = true
      }
    }
  }
}
