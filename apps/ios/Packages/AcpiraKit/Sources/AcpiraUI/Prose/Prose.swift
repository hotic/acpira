import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import SwiftUI

// chat/Prose.tsx + styles/prose.css: agent markdown. Blocks stack with a --gap; every typographic rule (heading
// scale, list markers, quote rail, table frame, inline code chip) follows prose.css. `tone: .thought` is the quieter
// reasoning scale: text-2 in fg-2, headings at body size, strong one shade up instead of fg-strong.

public enum ProseTone: Sendable { case reply, thought }

public struct Prose: View {
  let blocks: [MDBlock]
  var tone: ProseTone
  /// Streaming fade signal handed down while `StreamProse` is live
  var fade: StreamFadeSignal?
  /// Top-level block index offset for a split live tail so surviving blocks keep their SwiftUI identity and heading margin
  var blockIdOffset: Int = 0
  @Environment(\.agentAccent) private var accent

  /// `streaming`: the text is still arriving, so its unfinished tail is repaired first (streamdown's streaming mode)
  public init(markdown: String, tone: ProseTone = .reply, streaming: Bool = false) {
    let source = streaming ? Remend.repair(markdown) : markdown
    blocks = Markdown.parse(source.trimmingCharacters(in: .newlines))
    self.tone = tone
  }

  public init(block: TextBlock, tone: ProseTone = .reply) {
    self.init(markdown: block.markdown, tone: tone, streaming: block.streaming == true)
  }

  /// Blocks parsed already (a streamed text's `StreamParse`)
  init(blocks: [MDBlock], tone: ProseTone, fade: StreamFadeSignal? = nil, blockIdOffset: Int = 0) {
    self.blocks = blocks
    self.tone = tone
    self.fade = fade
    self.blockIdOffset = blockIdOffset
  }

  public var body: some View {
    if !blocks.isEmpty {
      ProseBlocks(blocks: blocks, style: ProseStyle(tone: tone, accent: accent), fade: fade,
                  blockIdOffset: blockIdOffset)
    }
  }
}

extension EnvironmentValues {
  /// --accent under accent = agent
  @Entry public var agentAccent: Color = Palette.brand
}

// MARK: - Streaming

/// Streaming fade signal handed down from `StreamProse`. Stays constant across pacing ticks so settled
/// paragraphs do not re-evaluate when only the tail grows; changes only on unpaced jumps (resume / catch-up /
/// rewrite) or when streaming finishes.
struct StreamFadeSignal: Equatable {
  var seedEpoch: Int
  var seeded: Bool
}

/// Per-paragraph fade state indexed from the start of the rendered glyphs (`0..<seen`), matching desktop
/// `glyphRuns.ts`. Counting rendered glyphs instead of subtracting from raw Markdown character offsets keeps
/// every earlier glyph's index stable across syntax (`**`, `` ` ``, link URLs, escapes) and inline code pads.
struct GlyphFade: Equatable {
  /// Rendered non-pad glyphs already revealed in this paragraph
  private(set) var seen = 0
  /// Active reveal steps for newly appended rendered glyphs (motion.css `sd-acpGlyph`)
  private(set) var reveals: [StreamReveal] = []

  /// Marks all `count` rendered glyphs as settled without fading (resume / catch-up / rewrite)
  mutating func seed(_ count: Int) {
    seen = max(0, count)
    reveals.removeAll()
  }

  /// Reconciles the rendered glyph count at `now`, recording a reveal for any newly appended suffix
  mutating func sync(count: Int, now: Double, duration: Double = Motion.streamChar) {
    let clamped = max(0, count)
    expire(now: now, duration: duration)
    if clamped > seen {
      reveals.append(StreamReveal(time: now, from: seen, to: clamped))
      seen = clamped
    } else if clamped < seen {
      seen = clamped
      reveals = reveals.compactMap { step in
        guard step.from < clamped else { return nil }
        return StreamReveal(time: step.time, from: step.from, to: min(step.to, clamped))
      }
    }
  }

  /// Drops reveal steps whose fade has finished
  mutating func expire(now: Double, duration: Double = Motion.streamChar) {
    reveals.removeAll { now - $0.time > duration }
  }

  /// Whether any glyph in `range` (`0`-based from paragraph start) is still fading
  func hasFading(in range: Range<Int>) -> Bool {
    reveals.contains { $0.from < range.upperBound && $0.to > range.lowerBound }
  }

  /// Opacity of the glyph at rendered index `index` (`0`-based from paragraph start); 1 once settled
  func opacity(at index: Int, now: Double, duration: Double = Motion.streamChar) -> Double {
    guard let step = reveals.last(where: { $0.from <= index && index < $0.to }) else { return 1 }
    let t = min(max((now - step.time) / duration, 0), 1)
    // CSS `ease`: cubic-bezier(.25, .1, .25, 1)
    return UnitCurve.bezier(startControlPoint: UnitPoint(x: 0.25, y: 0.1), endControlPoint: UnitPoint(x: 0.25, y: 1)).value(at: t)
  }
}

/// Reference wrapper held by `InlineText` so `InlineCodeRenderer.draw` can reconcile rendered glyph counts
/// without invalidating the view tree during layout or draw passes
final class GlyphFadeTracker {
  private(set) var fade = GlyphFade()
  private var initialized = false
  private var seedEpoch = 0
  private(set) var needsSeed = false
  private var lastContent = ""
  private var pendingFade = false

  /// Whether this paragraph has a new change waiting for its first draw or an unfinished glyph fade
  var isActive: Bool { !needsSeed && (pendingFade || !fade.reveals.isEmpty) }

  /// Updates tracking state from `InlineText.body` before `TimelineView` decides whether to run
  func prepare(content: String, signal: StreamFadeSignal) {
    if signal.seeded || (initialized && signal.seedEpoch != seedEpoch) {
      initialized = true
      seedEpoch = signal.seedEpoch
      lastContent = content
      needsSeed = true
      pendingFade = false
      fade.seed(fade.seen)
      return
    }
    if !initialized {
      initialized = true
      seedEpoch = signal.seedEpoch
      lastContent = content
      if !content.isEmpty { pendingFade = true }
      return
    }
    if content != lastContent {
      lastContent = content
      if !needsSeed { pendingFade = true }
    }
  }

  /// Called by `InlineCodeRenderer.draw` with the actual rendered non-pad glyph count
  func sync(glyphCount: Int, now: Double) {
    if needsSeed {
      fade.seed(glyphCount)
      needsSeed = false
      pendingFade = false
      return
    }
    guard now > 0 else { return }
    fade.sync(count: glyphCount, now: now)
    pendingFade = false
  }

  func expire(now: Double) {
    fade.expire(now: now)
    if fade.reveals.isEmpty { pendingFade = false }
  }

  func hasFading(in range: Range<Int>) -> Bool { fade.hasFading(in: range) }
  func opacity(at index: Int, now: Double) -> Double { fade.opacity(at: index, now: now) }
}

/// chat/Prose.tsx with streamMotion.ts: a streamed text is paced out at the agent's measured speed and every newly
/// shown glyph fades in. A text that mounts already complete renders at once; once the stream ends, the backlog
/// flushes and the last fades finish, the view settles into a static Prose. Motion off shows the received text.
public struct StreamProse: View {
  let markdown: String
  let streaming: Bool
  var tone: ProseTone
  /// The local images the engine read with the text, for its Markdown images
  var images: [ImageRef]?
  /// Top-level block id offset for a split live tail so surviving blocks keep their SwiftUI identity and fade trackers
  var blockIdOffset: Int = 0
  /// Continued ordered-list start number when a split cut landed mid-list
  var firstListStart: Int? = nil
  /// Optional callback invoked outside `body` with the UTF-8 byte length of settled, fully faded prefix text;
  /// returns the number of leading grapheme characters frozen out so the pacer drops them in lockstep.
  var onFreezePrefix: ((Int) -> Int)? = nil
  @Environment(\.motionEnabled) private var motion
  @Environment(\.quietEntrance) private var quiet
  @Environment(\.rowEnter) private var rowEnter
  @Environment(\.scrollGesture) private var gesture
  @State private var pacer: StreamPacer?
  @State private var parse = StreamParse()

  public init(markdown: String, tone: ProseTone = .reply, streaming: Bool) {
    self.markdown = markdown; self.streaming = streaming; self.tone = tone
    // Empty: the text arrives on appear. A state's initial value is built on every rebuild of the parent and kept only
    // the first time, and splitting a long text into graphemes for each throwaway copy cost every streamed chunk
    _pacer = State(initialValue: streaming ? StreamPacer(streaming: true) : nil)
  }

  public init(block: TextBlock, tone: ProseTone = .reply) {
    self.init(markdown: block.markdown, tone: tone, streaming: block.streaming == true)
    images = block.images
  }

  init(markdown: String, streaming: Bool, tone: ProseTone = .reply, images: [ImageRef]? = nil,
       blockIdOffset: Int = 0, firstListStart: Int? = nil, onFreezePrefix: ((Int) -> Int)? = nil) {
    self.markdown = markdown
    self.streaming = streaming
    self.tone = tone
    self.images = images
    self.blockIdOffset = blockIdOffset
    self.firstListStart = firstListStart
    self.onFreezePrefix = onFreezePrefix
    _pacer = State(initialValue: streaming ? StreamPacer(streaming: true) : nil)
  }

  public var body: some View {
    Group {
      if let pacer, motion {
        // The pacing ticks come from the task below; the tail paragraph runs its own frames while glyphs fade. A block
        // mounted quietly or after its entrance settled shows its existing text on the first frame, before `onAppear`
        let resume = pacer.target.isEmpty && (quiet || !rowEnter || gesture.active)
        let text = resume ? markdown : pacer.visible
        // The turn heading already indicates waiting before the first visible words. Stops at the first visible
        // character instead of trimming a copy of the whole text on every tick
        if !text.unicodeScalars.allSatisfy(CharacterSet.whitespacesAndNewlines.contains) {
          let signal = StreamFadeSignal(seedEpoch: pacer.skipEpoch, seeded: resume || quiet || pacer.seeded)
          Prose(blocks: adjusted(parse(text)), tone: tone, fade: signal, blockIdOffset: blockIdOffset)
        } else {
          // An empty Group is no view at all and would never start the task below; this holds it for the first tick
          Color.clear.frame(height: 0)
        }
      } else if streaming {
        Prose(blocks: adjusted(parse(markdown)), tone: tone, blockIdOffset: blockIdOffset)
      } else {
        Prose(blocks: adjusted(Markdown.parse(markdown.trimmingCharacters(in: .newlines))), tone: tone,
              blockIdOffset: blockIdOffset)
      }
    }
    .onAppear { receive(initial: true) }
    .onChange(of: markdown) { receive(initial: false) }
    .onChange(of: streaming) { receive(initial: false) }
    .task(id: pacer != nil && motion) { await pace() }
    .environment(\.markdownImages, images)
  }

  /// Applies `firstListStart` to the leading ordered list when a split cut landed in the middle of one
  private func adjusted(_ blocks: [MDBlock]) -> [MDBlock] {
    guard let start = firstListStart, !blocks.isEmpty,
          case .list(true, let raw, let items) = blocks[0], raw != start else { return blocks }
    var out = blocks
    out[0] = .list(ordered: true, start: start, items: items)
    return out
  }

  private func receive(initial: Bool) {
    guard motion, var next = pacer else {
      // Without pacing motion, all received text is immediately settled and eligible to freeze
      if streaming, let onFreezePrefix, markdown.utf8.count >= ReplyPieces.budget {
        if onFreezePrefix(markdown.utf8.count) > 0 { parse.reset() }
      }
      return
    }
    let skip = quiet || (initial && (!rowEnter || gesture.active))
    next.receive(markdown, streaming: streaming, now: Date.timeIntervalSinceReferenceDate, skip: skip)
    if skip, let onFreezePrefix, next.visible.utf8.count >= ReplyPieces.budget {
      let dropped = onFreezePrefix(next.visible.utf8.count)
      if dropped > 0 {
        next.dropPrefix(dropped)
        parse.reset()
      }
    }
    pacer = next
  }

  /// Ticks at STREAM_PACE.tick until the text has caught up, the stream ended and the last glyph faded in
  private func pace() async {
    guard motion, pacer != nil else { return }
    var last = Date.timeIntervalSinceReferenceDate
    // Track how far into the settled prefix we already scanned for newlines so `onFreezePrefix` only runs when a new
    // line enters the settled portion instead of re-parsing the tail every 32 ms
    var lastCheckedUTF8 = 0
    while !Task.isCancelled {
      try? await Task.sleep(for: .seconds(StreamPace.standard.tick))
      // Read after the sleep: text received meanwhile must not be overwritten by a stale copy
      guard !Task.isCancelled, var next = pacer else { return }
      let now = Date.timeIntervalSinceReferenceDate
      // Keep the pacer mounted a couple of frames past the glyph fade so the tail paragraph finishes smoothly
      // before switching to static Prose (matches webview RUN_FADE_MS = STREAM_DURATION_MS + 40)
      next.tick(now: now, dt: now - last, fade: Motion.streamChar + 0.04)
      last = now
      // Promote already-settled, fully faded prefix blocks into static thread rows so this live cell stays tiny
      if let onFreezePrefix {
        let settledChars = next.settledCount
        if settledChars >= 80 {
          let visible = next.visible
          let settledUTF8: Int
          if settledChars >= next.count {
            settledUTF8 = visible.utf8.count
          } else if let idx = visible.index(visible.startIndex, offsetBy: settledChars, limitedBy: visible.endIndex) {
            settledUTF8 = visible[..<idx].utf8.count
          } else {
            settledUTF8 = 0
          }
          if settledUTF8 < lastCheckedUTF8 { lastCheckedUTF8 = 0 }
          if settledUTF8 >= 64 && settledUTF8 > lastCheckedUTF8 {
            let utf8 = visible.utf8
            let from = utf8.index(utf8.startIndex, offsetBy: lastCheckedUTF8)
            let to = utf8.index(utf8.startIndex, offsetBy: settledUTF8)
            let hasNewline = utf8[from..<to].contains(0x0A)
            lastCheckedUTF8 = settledUTF8
            if hasNewline {
              let dropped = onFreezePrefix(settledUTF8)
              if dropped > 0 {
                next.dropPrefix(dropped)
                parse.reset()
                lastCheckedUTF8 = 0
              }
            }
          }
        }
      }
      if !next.streaming && !next.pending && next.reveals.isEmpty {
        pacer = nil
      } else if next != pacer {
        pacer = next
      }
    }
  }
}

/// A streamed text's parse, kept by its view between pacing ticks so only the unsettled tail is parsed again: parsing a
/// long reply whole on every tick was the largest cost of the app's own code in a profile of one streaming. A reference,
/// so the view's body advances it; it is only a cache, its result always that of a whole parse of the repaired text
final class StreamParse {
  private var stream = MarkdownStream()
  /// The last text parsed and its blocks. The body runs again with the same visible text whenever only the pacer's
  /// speed, a fade or the received tail moved, and each of those ran the whole repair again
  private var last: (text: String, blocks: [MDBlock])?
  /// Settled prefix for long unsplit streams (such as a 20K-char streaming thought): once a top-level prefix has
  /// balanced fences/math and ends on a blank-line boundary, `Remend.repair` runs only on the unsettled tail
  private var settledPrefix = ""
  private var settledBlocks: [MDBlock] = []

  func reset() {
    stream = MarkdownStream()
    last = nil
    settledPrefix = ""
    settledBlocks = []
  }

  func callAsFunction(_ text: String) -> [MDBlock] {
    if let last, last.text.utf8.count == text.utf8.count, last.text.utf8.elementsEqual(text.utf8) { return last.blocks }
    // Verify the cached prefix still matches; reset on any non-append rewrite
    if !settledPrefix.isEmpty {
      if text.utf8.count < settledPrefix.utf8.count || !text.utf8.starts(with: settledPrefix.utf8) {
        settledPrefix = ""
        settledBlocks = []
        stream = MarkdownStream()
      }
    }
    let tail = settledPrefix.isEmpty ? text : String(text[settledPrefix.endIndex...])
    let repairedTail = Remend.repair(tail).trimmingCharacters(in: .newlines)
    let tailBlocks = stream.parse(repairedTail)
    var combined = settledBlocks.isEmpty ? tailBlocks : (settledBlocks + tailBlocks)
    // For long unsplit streams (> 2 KB in the active tail), promote safe top-level blocks into `settledPrefix`
    // so `Remend.repair` stays bounded to the last block even in `ThoughtView` or non-hosted renders
    if tail.utf8.count > 2048, let promoted = Self.promoteSafePrefix(tail) {
      let prefixBlocks = Markdown.parse(Remend.repair(promoted.prefix).trimmingCharacters(in: .newlines))
      settledPrefix += promoted.prefix
      settledBlocks += prefixBlocks
      stream = MarkdownStream()
      let nextTail = String(tail[promoted.cut...])
      let nextTailBlocks = stream.parse(Remend.repair(nextTail).trimmingCharacters(in: .newlines))
      combined = settledBlocks + nextTailBlocks
    }
    last = (text, combined)
    return combined
  }

  /// Finds a safe block boundary in `tail` where fences (` ``` `), math (`$$`), and inline code ticks are closed
  /// and a subsequent block has already begun with at least one following line.
  private static func promoteSafePrefix(_ tail: String) -> (prefix: String, cut: String.Index)? {
    let layout = Markdown.parseLayout(tail)
    guard layout.blocks.count >= 3 else { return nil }
    var safe = 0
    while safe + 1 < layout.blocks.count && layout.starts[safe + 1] + 1 < layout.lineCount { safe += 1 }
    guard safe > 0 else { return nil }
    let cutByte = layout.lineOffsets[layout.starts[safe]]
    guard cutByte >= 512, cutByte < tail.utf8.count else { return nil }
    let cut = tail.utf8.index(tail.utf8.startIndex, offsetBy: cutByte)
    let prefix = String(tail[..<cut])
    // Ensure balanced fences / math / unescaped single delimiters so `Remend` state across the boundary is neutral
    guard Self.isRemendNeutral(prefix) else { return nil }
    return (prefix, cut)
  }

  private static func isRemendNeutral(_ s: String) -> Bool {
    var fences = 0
    var dollars = 0
    var ticks = 0
    var stars2 = 0
    var tildes2 = 0
    var openBrackets = 0
    let bytes = Array(s.utf8)
    var i = 0
    while i < bytes.count {
      let b = bytes[i]
      if b == 0x5C { i += 2; continue } // backslash escape
      if b == 0x60 { // `
        if i + 2 < bytes.count && bytes[i + 1] == 0x60 && bytes[i + 2] == 0x60 {
          fences += 1; i += 3; continue
        }
        if fences % 2 == 0 { ticks += 1 }
      } else if fences % 2 == 0 {
        if b == 0x24 && i + 1 < bytes.count && bytes[i + 1] == 0x24 && ticks % 2 == 0 {
          dollars += 1; i += 2; continue
        }
        if ticks % 2 == 0 {
          if b == 0x2A && i + 1 < bytes.count && bytes[i + 1] == 0x2A { stars2 += 1; i += 2; continue }
          if b == 0x7E && i + 1 < bytes.count && bytes[i + 1] == 0x7E { tildes2 += 1; i += 2; continue }
          if b == 0x5B { openBrackets += 1 }
          else if b == 0x5D { openBrackets = max(0, openBrackets - 1) }
        }
      }
      i += 1
    }
    return fences % 2 == 0 && dollars % 2 == 0 && ticks % 2 == 0 && stars2 % 2 == 0
      && tildes2 % 2 == 0 && openBrackets == 0
  }
}

/// Resolved type and ink for a prose tone
struct ProseStyle {
  var tone: ProseTone
  var accent: Color
  /// A quote reads its whole content in fg-2
  var inkOverride: Color?
  /// Both tones read at text-1: the webview's thought class asks for text-2, but streamdown's class merge drops
  /// `text-2` next to `text-fg-2`, so thoughts render at the inherited text-1 size and leading (LAB, Chrome)
  var body: TextStyle { WebType.text1 }
  var ink: Color { inkOverride ?? (tone == .reply ? Palette.fg1 : Palette.fg2) }
  var strongInk: Color { tone == .reply ? Palette.fgStrong : Palette.fg1 }
  /// List marker column: `padding-left: 1.2em`
  var listIndent: CGFloat { (body.size * 1.2).rounded() }

  func heading(_ level: Int) -> TextStyle {
    guard tone == .reply else { return body }
    let factor: CGFloat = level == 1 ? 1.35 : level == 2 ? 1.25 : level == 3 ? 1.15 : 1
    // line-height stays --text-1-lh
    return TextStyle(size: (WebType.text1.size * factor).rounded(), lineHeight: WebType.text1.lineHeight)
  }
}

/// How many blocks, items of one list or rows of one table a prose stack draws before it asks for more. A model
/// looping until its output limit wrote tens of thousands of paragraphs into one reply, and the thread lays a whole
/// turn out in one cell, so drawing them all froze the app or got it killed
enum ProseCap {
  static let page = 200
}

/// The "N more" line under a capped stack; each tap draws another page
struct ProseMore: View {
  let rest: Int
  let action: () -> Void

  var body: some View {
    Button(action: action) {
      Text(web: t("session.showMore", ["n": String(rest)]), paragraph: false)
        .textStyle(TypeScale.text2)
        .foregroundStyle(Palette.fg3)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }
}

struct ProseBlocks: View {
  let blocks: [MDBlock]
  let style: ProseStyle
  /// Streaming fade signal: passed to every block so an earlier paragraph finishes its own fade after a new
  /// block starts instead of snapping to full opacity, while settled paragraphs stay dormant
  var fade: StreamFadeSignal?
  /// Offset added to `ForEach` identity and `first` check when earlier blocks were frozen into separate rows
  var blockIdOffset: Int = 0
  @Environment(\.quoteProse) private var quote
  @Environment(\.deferQuoteMenu) private var deferQuoteMenu
  @Environment(\.rowHitTargets) private var hitTargets
  @Environment(\.insideQuotedBlock) private var insideQuotedBlock
  @State private var shown = ProseCap.page
  @State private var bucket = HitBucketToken()

  var body: some View {
    let prefixCount = min(blocks.count, shown)
    let trackHits = deferQuoteMenu && quote != nil && !insideQuotedBlock && hitTargets != nil
    if trackHits {
      let bucketKey = ObjectIdentifier(bucket)
      let onPlace: @Sendable ([CGRect]) -> Void = { [hitTargets] frames in
        var entries: [BlockHitEntry] = []
        entries.reserveCapacity(min(prefixCount, frames.count))
        for i in 0..<min(prefixCount, frames.count) {
          let block = blocks[i]
          guard block != .rule else { continue }
          entries.append(BlockHitEntry(quoteBlock: block, renderedBlock: block, localFrame: frames[i],
                                       offersSelection: RowHitTargets.offersSelection(block)))
        }
        MainActor.assumeIsolated { hitTargets?.setEntries(entries, for: bucketKey) }
      }
      BlockStackLayout(spacing: Metrics.gap, onPlace: onPlace) {
        blockChildren(prefixCount: prefixCount)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .onGeometryChange(for: CGPoint.self) { $0.frame(in: .named(RowHitSpace.name)).origin } action: { origin in
        hitTargets?.setOrigin(origin, for: bucketKey)
      }
      .onDisappear { hitTargets?.removeBucket(bucketKey) }
    } else {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        blockChildren(prefixCount: prefixCount)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
  }

  @ViewBuilder private func blockChildren(prefixCount: Int) -> some View {
    ForEach(blockIdOffset..<(blockIdOffset + prefixCount), id: \.self) { absolute in
      let block = blocks[absolute - blockIdOffset]
      ProseBlock(block: block, style: style, first: absolute == 0, fade: fade)
        #if !canImport(UIKit)
        .environment(\.quoteProse, nil)
        #endif
        .quoteMenu(block, quote, deferred: deferQuoteMenu)
    }
    if blocks.count > shown {
      ProseMore(rest: blocks.count - shown) { shown += ProseCap.page }
    }
  }
}

/// A row's run of a settled reply cut into thread rows (ReplyPieces.swift), stacked as ProseBlocks stacks a whole text:
/// whole blocks, and runs of a list's items numbered where they stand in the list
struct ProseRun: View {
  let segments: [ReplySegment]
  @Environment(\.agentAccent) private var accent
  @Environment(\.quoteProse) private var quote
  @Environment(\.deferQuoteMenu) private var deferQuoteMenu
  @Environment(\.rowHitTargets) private var hitTargets
  @State private var bucket = HitBucketToken()

  var body: some View {
    let style = ProseStyle(tone: .reply, accent: accent)
    let trackHits = deferQuoteMenu && quote != nil && hitTargets != nil
    if trackHits {
      let bucketKey = ObjectIdentifier(bucket)
      let onPlace: @Sendable ([CGRect]) -> Void = { [hitTargets] frames in
        var entries: [BlockHitEntry] = []
        entries.reserveCapacity(min(segments.count, frames.count))
        for i in 0..<min(segments.count, frames.count) {
          switch segments[i] {
          case let .block(block, _):
            guard block != .rule else { continue }
            entries.append(BlockHitEntry(quoteBlock: block, renderedBlock: block, localFrame: frames[i],
                                         offersSelection: RowHitTargets.offersSelection(block)))
          case let .items(list, ordered, start, items):
            let slice: MDBlock = .list(ordered: ordered, start: start, items: items)
            entries.append(BlockHitEntry(quoteBlock: list, renderedBlock: slice, localFrame: frames[i],
                                         offersSelection: true))
          }
        }
        MainActor.assumeIsolated { hitTargets?.setEntries(entries, for: bucketKey) }
      }
      BlockStackLayout(spacing: Metrics.gap, onPlace: onPlace) {
        segmentChildren(style: style)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      .onGeometryChange(for: CGPoint.self) { $0.frame(in: .named(RowHitSpace.name)).origin } action: { origin in
        hitTargets?.setOrigin(origin, for: bucketKey)
      }
      .onDisappear { hitTargets?.removeBucket(bucketKey) }
    } else {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        segmentChildren(style: style)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
  }

  @ViewBuilder private func segmentChildren(style: ProseStyle) -> some View {
    ForEach(Array(segments.enumerated()), id: \.offset) { _, segment in
      Group {
        switch segment {
        case let .block(block, index):
          ProseBlock(block: block, style: style, first: index == 0).quoteMenu(block, quote, deferred: deferQuoteMenu)
        case let .items(list, ordered, start, items):
          // Long-pressed, a run of the list still quotes the whole list, as the list drawn whole does
          ProseList(ordered: ordered, start: start, items: items, style: style).quoteMenu(list, quote, deferred: deferQuoteMenu)
        }
      }
      #if !canImport(UIKit)
      .environment(\.quoteProse, nil)
      #endif
    }
  }
}

struct ProseBlock: View {
  let block: MDBlock
  let style: ProseStyle
  var first: Bool
  /// Streaming fade signal while `StreamProse` is live; tables, code and math appear whole and take none
  var fade: StreamFadeSignal?

  var body: some View {
    switch block {
    case .paragraph(let inlines) where holdsImage(inlines):
      ImageParagraph(inlines: inlines, style: style, fade: fade)
    case .paragraph(let inlines):
      InlineText(inlines: inlines, style: style.body, ink: style.ink, strongInk: style.strongInk, accent: style.accent,
                 fade: fade)
    case .heading(let level, let inlines):
      InlineText(inlines: inlines, style: style.heading(level), ink: style.tone == .reply ? Palette.fgStrong : style.strongInk,
                 strongInk: style.tone == .reply ? Palette.fgStrong : style.strongInk, accent: style.accent, weight: .semibold,
                 fade: fade)
        // :not(:first-child) gets margin-top: var(--gap) on top of the block gap
        .padding(.top, first ? 0 : Metrics.gap)
    case .list(let ordered, let start, let items):
      ProseList(ordered: ordered, start: start, items: items, style: style, fade: fade)
    case .quote(let inner):
      ProseQuote(blocks: inner, style: style, fade: fade)
    case .code(let lang, let text):
      // A mermaid block the flowchart parser reads is drawn natively; any other diagram goes to mermaid itself in a web
      // view, and stays source if mermaid cannot read it either, as a failed render does in the webview
      if lang == "mermaid", let diagram = MermaidBlock(source: text) {
        diagram
      } else if lang == "mermaid" {
        MermaidWebBlock(source: text)
      } else {
        CodeSurface(text: text, lang: lang)
      }
    case .table(let align, let header, let rows):
      // Tables, code and math appear whole: a fade by distance from the end does not map onto their cells
      ProseTable(align: align, header: header, rows: rows, style: style)
    case .rule:
      Rectangle().fill(Palette.conversationLine).frame(height: 1).padding(.vertical, Metrics.gap)
    case .math(let tex):
      MathBlock(tex: tex, style: style)
    }
  }
}

// MARK: - Lists

struct ProseList: View {
  let ordered: Bool
  let start: Int
  let items: [[MDBlock]]
  let style: ProseStyle
  /// Streaming fade signal handed to each item's blocks so an item finishes fading after the next item starts
  var fade: StreamFadeSignal?
  @State private var shown = ProseCap.page

  var body: some View {
    // li + li: margin-top 2px
    VStack(alignment: .leading, spacing: Metrics.s0_5) {
      ForEach(Array(items.prefix(shown).enumerated()), id: \.offset) { index, item in
        HStack(alignment: .firstTextBaseline, spacing: 0) {
          marker(index)
            .frame(width: style.listIndent, alignment: .trailing)
          VStack(alignment: .leading, spacing: Metrics.s0_5) {
            ForEach(Array(item.enumerated()), id: \.offset) { i, block in
              ProseBlock(block: block, style: style, first: i == 0, fade: fade)
            }
          }
          .frame(maxWidth: .infinity, alignment: .leading)
        }
        // streamdown's li carries py-1
        .padding(.vertical, Metrics.s1)
      }
      if items.count > shown {
        ProseMore(rest: items.count - shown) { shown += ProseCap.page }
          .padding(.leading, style.listIndent)
      }
    }
  }

  /// `li::marker` in fg-3, outside the content box: "1." or a disc, then a space's width before the text
  @ViewBuilder func marker(_ index: Int) -> some View {
    let space = (style.body.size * 0.28).rounded()
    if ordered {
      Text(web: "\(start + index).")
        .textStyle(style.body)
        .foregroundStyle(Palette.fg3)
        .padding(.trailing, space)
        .fixedSize()
    } else {
      // Chrome's disc: a filled circle about a third of an em, centred on the x-height
      Text(web: "\u{2022}")
        .textStyle(style.body)
        .foregroundStyle(.clear)
        .overlay {
          Circle().fill(Palette.fg3)
            .frame(width: (style.body.size * 0.32).rounded(), height: (style.body.size * 0.32).rounded())
        }
        .padding(.trailing, space + (style.body.size * 0.18).rounded())
        .fixedSize()
    }
  }
}

// MARK: - Quote

struct ProseQuote: View {
  let blocks: [MDBlock]
  let style: ProseStyle
  var fade: StreamFadeSignal?

  var body: some View {
    var quiet = style
    quiet.inkOverride = Palette.fg2
    return ProseBlocks(blocks: blocks, style: quiet, fade: fade)
      .environment(\.insideQuotedBlock, true)
      .padding(.vertical, Metrics.gap)
      .padding(.horizontal, Metrics.pad)
      .background(alignment: .leading) {
        RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.lineStrong).frame(width: 2)
      }
  }
}

// MARK: - Code

extension EnvironmentValues {
  /// A code block's fill where the stock one clashes with its ground (a screenshot card's sheet); nil keeps --bg-code
  @Entry var codeFill: Color? = nil
}

/// CodeSurface: fenced code on the code background inside the conversation ring, scrolling sideways; coloured when the
/// fence names a known grammar (CodeBlock.tsx)
struct CodeSurface: View {
  let text: String
  var lang: String?
  @Environment(\.snapshot) private var snapshot
  @Environment(\.colorScheme) private var scheme
  @Environment(\.codeFill) private var fill
  @State private var colored: Colored?
  @State private var running = Running()

  /// A pass's tokens and the text they were made for
  struct Colored { var text: String; var language: String; var theme: CodeTheme; var lines: [[CodeToken]]; var at: ContinuousClock.Instant }
  struct Pass: Equatable { var text: String; var language: String?; var theme: CodeTheme; var landed: ContinuousClock.Instant? }
  /// Whether a pass is on: one at a time, its result kept even when the text has grown since
  final class Running { var on = false }
  /// CodeBlock.tsx STREAM_SETTLE_MS: a fence that streams asks again this long after its last pass landed
  private static let settle = Duration.milliseconds(120)

  var body: some View {
    let theme = CodeTheme(scheme)
    let language = fenceLanguage(lang)
    let usable = colored.flatMap { $0.language == language && $0.theme == theme ? $0 : nil }
      // The full-length render tokenizes in place, as its tasks may not run before the image is taken
      ?? (snapshot ? language.flatMap { l in
        SyntaxHighlighter.shared.tokenize(text, language: l, theme: theme).map { Colored(text: text, language: l, theme: theme, lines: $0, at: .now) }
      } : nil)
    Scroller(.horizontal) {
      Text(usable == nil ? CJKWidth.attributed(text, size: WebType.mono.size, paragraph: false)
           : codeSource(text, tokenized: usable?.text, lines: usable?.lines, size: WebType.mono.size))
        .textStyle(WebType.mono)
        .foregroundStyle(Palette.fg1)
        .fixedSize()
        .selectableText(text, style: WebType.mono)
        .padding(.horizontal, Metrics.pad)
        .padding(.vertical, Metrics.pad - 2)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    // The CSS border takes room inside the card; the stroke below does not
    .padding(Metrics.hairline)
    .background(fill ?? Palette.bgCode, in: RoundedRectangle(cornerRadius: Metrics.rLg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.conversationLine))
    // One pass at a time, off the main actor. A pass for a text the fence has since extended still lands (its lines
    // colour the text it covers), and its landing asks again for the text then: dropped by every chunk, a pass of a
    // streaming fence never landed
    .task(id: Pass(text: text, language: language, theme: theme, landed: colored?.at)) {
      guard !snapshot, let language, !running.on,
            !(colored?.text == text && colored?.language == language && colored?.theme == theme) else { return }
      if let last = colored, last.language == language, last.theme == theme, text.utf8.starts(with: last.text.utf8) {
        try? await Task.sleep(until: last.at + Self.settle)
        if Task.isCancelled { return }
      }
      running.on = true
      let source = text
      let lines = await Task.detached(priority: .utility) { SyntaxHighlighter.shared.tokenize(source, language: language, theme: theme) }.value
      running.on = false
      if let lines { colored = Colored(text: source, language: language, theme: theme, lines: lines, at: .now) }
    }
  }
}

// MARK: - Table

struct ProseTable: View {
  let align: [MDAlign]
  let header: [[MDInline]]
  let rows: [[[MDInline]]]
  let style: ProseStyle
  @State private var width: CGFloat = 0
  @State private var shown = ProseCap.page

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.gap) {
      grid
      if rows.count > shown {
        ProseMore(rest: rows.count - shown) { shown += ProseCap.page }
      }
    }
  }

  private var grid: some View {
    let drawn = rows.prefix(shown)
    return Scroller(.horizontal) {
      // Top-align cells across each row (`vertical-align: top`) without making cells vertically flexible:
      // `maxHeight: .infinity` on grid cells makes the whole Grid vertically flexible inside a horizontal ScrollView,
      // which stretches rows past the cell height UICollectionView measured and overflows into the next message.
      Grid(alignment: .topLeading, horizontalSpacing: 0, verticalSpacing: 0) {
        GridRow {
          ForEach(Array(header.enumerated()), id: \.offset) { i, cell in
            cellView(cell, column: i, head: true)
          }
        }
        .background(Palette.hover)
        if !drawn.isEmpty { rowRule }
        ForEach(Array(drawn.enumerated()), id: \.offset) { r, row in
          GridRow {
            ForEach(Array(row.enumerated()), id: \.offset) { i, cell in
              cellView(cell, column: i, head: false)
            }
          }
          if r < drawn.count - 1 { rowRule }
        }
      }
      // min-width: 100%
      .frame(minWidth: width, alignment: .leading)
    }
    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
    // The frame's 1 px border takes room around the scrollport
    .padding(Metrics.hairline)
    .clipShape(RoundedRectangle(cornerRadius: Metrics.rLg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.conversationLine))
  }

  /// border-bottom: 1px spanning every column without widening the grid
  private var rowRule: some View {
    Rectangle().fill(Palette.line).frame(height: Metrics.hairline).gridCellUnsizedAxes(.horizontal)
  }

  func cellView(_ cell: [MDInline], column: Int, head: Bool) -> some View {
    TableCellWidth(max: Metrics.tableCellMax) {
      InlineText(inlines: cell, style: WebType.text2, ink: head ? Palette.fg2 : style.ink, strongInk: style.strongInk,
                 accent: style.accent, weight: head ? .medium : .regular, expand: false)
    }
    .padding(.vertical, Metrics.gap)
    .padding(.horizontal, Metrics.pad)
    .frame(maxWidth: .infinity, alignment: Alignment(horizontal: alignment(column).horizontal, vertical: .top))
  }

  func alignment(_ column: Int) -> Alignment {
    switch column < align.count ? align[column] : .none {
    case .center: .center
    case .right: .trailing
    default: .leading
    }
  }
}

/// Caps a table cell's width inside a horizontal `ScrollView` so short columns keep their tight natural width while
/// long prose cells wrap cleanly instead of creating multi-thousand-point `Grid` backing textures.
private struct TableCellWidth: Layout {
  var max: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let child = subviews.first else { return .zero }
    let width = proposal.width.map { min($0, max) } ?? max
    return child.sizeThatFits(ProposedViewSize(width: width, height: nil))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    subviews.first?.place(at: bounds.origin, anchor: .topLeading,
                          proposal: ProposedViewSize(width: bounds.width, height: bounds.height))
  }
}

// MARK: - Math

// MathBlock (KaTeX-faithful `$$` display math) lives in MathTypeset.swift
