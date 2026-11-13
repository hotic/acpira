import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// SharePreview.tsx ShareCard and shareCapture.ts on the phone: a reply as a watermarked picture. The card is drawn off
// screen and opens in the chat's Lightbox, whose share button hands it to the system (save, copy, send).

/// The watermark line's facts (turnActionHelpers.ts ShareMeta)
struct ShareMeta: Equatable {
  var title: String
  var agent: AgentId
  var agentName: String
  var model: String?
  var brand: String?
}

/// turnShareMeta: the session's title and the model that answered with its vendor. The peer's own id when it reported
/// one, else the prompt's selection, else the session's current model
func shareMeta(title: String, agent: AgentId, agentName: String, usage: TurnUsage?, settings: TurnSettings?,
               controls: SessionControls) -> ShareMeta {
  let control = controls.options.first { $0.category == "model" }
  let id = usage?.model ?? control.flatMap { settings?.config[$0.id] ?? $0.value }
  var meta = ShareMeta(title: title, agent: agent, agentName: agentName)
  if let id, let option = control.flatMap({ offeredOption($0.options, id) }) {
    // The family alone, as the statistics card names it; a Fusion pair keeps its whole name
    let parsed = ModelCatalog.parse(option.name)
    meta.model = parsed.lead == nil ? parsed.family : option.name
    meta.brand = ModelCatalog.optionBrand(option)
  } else if let id {
    meta.model = id
    meta.brand = VendorBrand.of(id)
  }
  return meta
}

/// The option a reported id stands for: an exact match, else the only one differing by its `[…]` hint (Claude reports
/// `fable[1m]` for an offered `fable`), as the webview's offeredModel matches it
private func offeredOption(_ options: [SessionOption], _ id: String) -> SessionOption? {
  if let exact = options.first(where: { $0.id == id }) { return exact }
  let bare = { (s: String) -> Substring in s.hasSuffix("]") ? (s.lastIndex(of: "[").map { s[..<$0] } ?? Substring(s)) : Substring(s) }
  let same = options.filter { bare($0.id) == bare(id) }
  return same.count == 1 ? same[0] : nil
}

/// What a reply's picture holds: the image results that stay in view outside the process fold, then the reply's prose.
/// Videos and widgets are left out: neither draws off screen
struct ShareContent {
  var images: [ToolCallBlock]
  var reply: [TextBlock]

  init(_ turn: AgentTurn) {
    let split = splitCodexBlocks(turn.blocks)
    images = split.process.compactMap { block in
      guard let call = block.toolCall, isImageResult(call), !toolImages(call).isEmpty,
            call.verbKey != "verb.showVideo", call.verbKey != "verb.showWidget" else { return nil }
      return call
    }
    reply = split.reply
    // A tail the split finds nothing in: the turn's plain text, as the copy action takes it
    if reply.isEmpty { reply = turn.blocks.compactMap { if case .text(let b) = $0 { b } else { nil } } }
  }

  var isEmpty: Bool { images.isEmpty && !reply.contains { !$0.markdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } }
}

private enum ShareTokens {
  // .acp-share-card: the frame a step under the page, the sheet the lightest tone, code sunk into the sheet
  static let frame = Swatch(Swatches.bg0.dark.mix(.black(1), 0.96), RGBA(hex: 0xffffff).mix(.black(1), 0.94)).color
  static let sheet = Swatch(Swatches.bg0.dark.mix(.white(1), 0.99), RGBA(hex: 0xffffff)).color
  static let sheetLine = Swatch(.white(0.07), Swatches.conversationLine.light).color
  static let code = Swatch(.black(0.1), .black(0.03)).color
  /// The light sheet's lift off the frame; the dark one takes a tone step only
  static let lift = Swatch(.black(0), .black(0.07)).color
  static let liftRadius: CGFloat = 10
  static let liftY: CGFloat = 8
  /// The vendor's colour in the frame's corner: its strength, and how far it reaches (80% of the card, gone at 60%)
  static let glow = 0.09
  static let glowReach: CGFloat = 0.48
  static let watermark = 0.85
  /// shareCapture.ts MAX_CANVAS_PIXELS
  static let maxPixels: CGFloat = 16_000_000
  /// Between the layout pass and the drawn one: measured paddings settle in state
  static let settle = Duration.milliseconds(300)
}

/// The card in two layers: the reply on a raised sheet, the frame under it carrying one line that names the session on
/// the left and the model on the right
struct ShareCard: View {
  let meta: ShareMeta
  let content: ShareContent

  var body: some View {
    let tint = VendorBrand.color(meta.brand)
    VStack(spacing: 0) {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        ForEach(content.images, id: \.id) { ImageResults(block: $0) }
        ForEach(Array(content.reply.enumerated()), id: \.offset) { _, block in
          Prose(markdown: block.markdown).environment(\.markdownImages, block.images)
        }
      }
      .environment(\.codeFill, ShareTokens.code)
      .environment(\.imageInFold, false)
      // The CSS borders take room; the strokes below do not
      .padding(Metrics.pad + Metrics.hairline)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(ShareTokens.sheet, in: RoundedRectangle(cornerRadius: Metrics.rMd))
      .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(ShareTokens.sheetLine))
      .shadow(color: ShareTokens.lift, radius: ShareTokens.liftRadius, y: ShareTokens.liftY)
      watermark(tint)
    }
    // The frame (.acp-share-card px/pt-share-frame, pb-gap-half): gap × 5/4 on the sides and top
    .padding(EdgeInsets(top: Metrics.gap * 5 / 4, leading: Metrics.gap * 5 / 4, bottom: Metrics.s1_5, trailing: Metrics.gap * 5 / 4))
    .padding(Metrics.hairline)
    .background {
      ShareTokens.frame
      if let tint {
        EllipticalGradient(colors: [tint.opacity(ShareTokens.glow), tint.opacity(0)], center: .bottomTrailing,
                           startRadiusFraction: 0, endRadiusFraction: ShareTokens.glowReach)
      }
    }
    .clipShape(RoundedRectangle(cornerRadius: Metrics.rLg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.conversationLine))
  }

  private func watermark(_ tint: Color?) -> some View {
    HStack(spacing: Metrics.gap) {
      HStack(spacing: Metrics.s1) {
        VendorMark(brand: "acpira", name: "Acpira", size: WebMetrics.icon).foregroundStyle(Palette.fg3)
        Text(web: "Acpira", paragraph: false).lineLimit(1)
      }
      Spacer(minLength: 0)
      HStack(spacing: Metrics.s1) {
        if let model = meta.model {
          VendorMark(brand: meta.brand, name: model, size: WebMetrics.icon, colored: true).foregroundStyle(tint ?? Palette.fg2)
        } else {
          VendorMark(brand: meta.agent, name: meta.agentName, size: WebMetrics.icon).foregroundStyle(Palette.fg3)
        }
        Text(web: meta.model ?? meta.agentName, paragraph: false).lineLimit(1).fixedSize()
      }
    }
    .textStyle(WebType.text4, weight: .medium)
    .foregroundStyle(Palette.fg2)
    .opacity(ShareTokens.watermark)
    // Flush with the sheet's edges: the mark starts on its left line, the model ends on its right one
    .padding(.top, Metrics.pad)
    .padding(.bottom, Metrics.gap)
  }
}

#if canImport(UIKit)
enum ShareCapture {
  /// The card as a picture `width` points wide, at the screen's scale unless that passes the pixel budget. Its images
  /// must be at hand already: ImageRenderer runs no tasks
  static func render(_ card: ShareCard, width: CGFloat, scheme: ColorScheme, scale: CGFloat, media: MediaScope,
                     accent: Color) async -> DecodedImage? {
    let content = card
      .frame(width: width)
      .fixedSize(horizontal: false, vertical: true)
      .environment(\.colorScheme, scheme)
      .environment(\.snapshot, true)
      .environment(\.motionEnabled, false)
      .environment(\.quietEntrance, true)
      .environment(\.agentAccent, accent)
      // No Lightbox: the picture's images take no taps
      .environment(\.media, MediaScope(session: media.session, loader: media.loader))
    let renderer = ImageRenderer(content: content)
    renderer.scale = scale
    // The palette's dynamic colours resolve against the current trait collection, not the SwiftUI environment
    let traits = UITraitCollection(userInterfaceStyle: scheme == .dark ? .dark : .light)
    var image: UIImage?
    traits.performAsCurrent { image = renderer.uiImage }
    if let size = image?.size, size.width * size.height * scale * scale > ShareTokens.maxPixels {
      renderer.scale = max(1, (ShareTokens.maxPixels / (size.width * size.height)).squareRoot())
    }
    try? await Task.sleep(for: ShareTokens.settle)
    traits.performAsCurrent { image = renderer.uiImage }
    return image?.cgImage.map { DecodedImage(cgImage: $0) }
  }
}
#endif
