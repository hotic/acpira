import AcpiraDesign
import SwiftUI

// Design study (2026-10-26): the phone shell's look split into orthogonal axes, so variants compare side by side
// instead of one redesign after another. Every axis defaults to the look that shipped before the study, so nothing
// changes until a variant is picked: live from Settings (stored in UserDefaults) or pinned for screenshots with
// `-lab <preset>` or `-lab header=float,margin=slim,user=bubble,…`. Once the owner picks, the losing branches are deleted and
// the winners become the only code path.

struct ShellStyle: Equatable, Sendable {
  /// The chat's top: the bar with a centred title (`bar`), floating glass buttons only (`float`), or glass back +
  /// a leading title + glass button group (`title`)
  enum Header: String, CaseIterable, Sendable { case bar, float, title }
  /// The transcript's side margins: the desktop's page + pad (prose 40 pt in), or a phone's (prose 20 pt in)
  enum Margin: String, CaseIterable, Sendable { case wide, slim }
  /// A sent prompt: the full-width card (`card`) or a right-aligned bubble (`bubble`)
  enum User: String, CaseIterable, Sendable { case card, bubble }
  /// A finished turn's fold head: the icon row (`row`) or one quiet caption (`quiet`)
  enum Process: String, CaseIterable, Sendable { case row, quiet }
  /// The copy / fork / stats row: under every reply, under the last one only, or never
  enum Actions: String, CaseIterable, Sendable { case every, last, none }
  /// The composer: the two-line card with the model's full name (`card`), one line with the model's mark (`pill`),
  /// or the field over a row with a short model chip (`stack`)
  enum Composer: String, CaseIterable, Sendable { case card, pill, stack }
  /// The shape language of controls and surfaces: rounded squares as on the desktop, or capsules and circles
  enum Shape: String, CaseIterable, Sendable { case square, round }
  /// The session list's top: the project bar with a search field (`bar`), or glass buttons over a large title
  enum ListHeader: String, CaseIterable, Sendable { case bar, large }
  /// A session row's lead: the mark on a tile, the bare mark on the title line, the bare mark larger and centred on
  /// both lines (`big`), or a state dot only
  enum ListRow: String, CaseIterable, Sendable { case tile, mark, big, dot }
  /// The list's groups: rows straight on the page (`plain`), or each group on a rounded card as Settings draws them
  enum ListBlock: String, CaseIterable, Sendable { case plain, card }
  /// Where a session row keeps its time: on every row (`full`), or only on rows no day caption dates (`quiet`: pinned
  /// rows and search hits). The second line is the same in both: the place on the left, the state on the right
  enum RowMeta: String, CaseIterable, Sendable { case full, quiet }
  /// Sessions in one run, or under pinned / today / earlier captions
  enum Grouping: String, CaseIterable, Sendable { case flat, time }
  /// A composer-shaped button at the list's bottom that starts a new session
  enum ListComposer: String, CaseIterable, Sendable { case none, pill }
  /// Menus, pickers, bars and the settings page: drawn by the app (`custom`), or the system's own (`system`): the
  /// navigation bar with its glass buttons, `Menu` for "…", model, attach, project and agent, a `Form` for settings
  enum Chrome: String, CaseIterable, Sendable { case custom, system }
  /// Under `chrome=system`, what the composer's `+` and model chips open: liquid beans squeezed out of the chip
  /// (`goo`), or the system's own menu with a submenu per setting (`menu`)
  enum Chips: String, CaseIterable, Sendable { case goo, menu }
  /// Reading sizes: the desktop's airy scale the port matched (`desk`), or the phone's own (`phone`: 17 pt prose,
  /// 14 pt code, larger glyphs and taller rows), see AcpiraDesign `Reading`
  enum Scale: String, CaseIterable, Sendable { case desk, phone }

  var header = Header.bar
  var margin = Margin.wide
  var user = User.card
  var process = Process.row
  var actions = Actions.every
  var composer = Composer.card
  var shape = Shape.square
  var listHeader = ListHeader.bar
  var listRow = ListRow.tile
  var listBlock = ListBlock.plain
  var rowMeta = RowMeta.full
  var grouping = Grouping.flat
  var listComposer = ListComposer.none
  var chrome = Chrome.custom
  var chips = Chips.goo
  var scale = Scale.desk

  /// Each axis as (key, title, options with their labels), in the order Settings lists them
  static let axes: [(key: String, title: String, options: [(id: String, label: String)])] = [
    ("header", "顶栏", [("bar", "标题栏"), ("float", "悬浮"), ("title", "标题+悬浮")]),
    ("margin", "页边距", [("wide", "宽"), ("slim", "窄")]),
    ("user", "用户消息", [("card", "通栏卡片"), ("bubble", "右侧气泡")]),
    ("process", "过程行", [("row", "图标行"), ("quiet", "一行灰字")]),
    ("actions", "回复操作", [("every", "每条"), ("last", "仅最后"), ("none", "隐藏")]),
    ("composer", "输入框", [("card", "卡片"), ("pill", "单行"), ("stack", "两行")]),
    ("shape", "形状", [("square", "圆角方"), ("round", "胶囊")]),
    ("listHeader", "列表顶部", [("bar", "项目栏"), ("large", "大标题")]),
    ("listRow", "会话行", [("tile", "图标块"), ("mark", "图标"), ("big", "大图标"), ("dot", "状态点")]),
    ("listBlock", "列表块", [("plain", "平铺"), ("card", "卡片")]),
    ("rowMeta", "行信息", [("full", "完整"), ("quiet", "精简")]),
    ("grouping", "分组", [("flat", "不分"), ("time", "按时间")]),
    ("listComposer", "列表底部", [("none", "无"), ("pill", "新会话条")]),
    ("chrome", "弹层", [("custom", "自绘"), ("system", "系统")]),
    ("chips", "输入框选项", [("goo", "豆子"), ("menu", "系统菜单")]),
    ("scale", "字号", [("desk", "桌面同款"), ("phone", "手机")]),
  ]

  /// Whole looks to start from; tweaking an axis afterwards leaves the preset
  static let presets: [(id: String, label: String, style: ShellStyle)] = [
    ("now", "现状", ShellStyle()),
    ("a", "甲 原生玻璃", ShellStyle(axes: "header=float,margin=slim,user=bubble,process=quiet,actions=last,composer=stack,shape=round,listHeader=large,listRow=mark,grouping=time,listComposer=pill")),
    ("b", "乙 方角克制", ShellStyle(axes: "header=title,margin=slim,user=bubble,process=quiet,actions=last,composer=stack,shape=square,listHeader=large,listRow=mark,grouping=time,listComposer=none")),
    ("c", "丙 极简", ShellStyle(axes: "header=float,margin=slim,user=bubble,process=quiet,actions=none,composer=pill,shape=round,listHeader=large,listRow=dot,grouping=time,listComposer=pill")),
    ("d", "丁 系统", ShellStyle(axes: "header=float,margin=slim,user=bubble,process=row,actions=every,composer=stack,shape=round,listHeader=large,listRow=mark,rowMeta=quiet,grouping=time,listComposer=none,chrome=system,scale=phone")),
  ]

  init() {}

  /// `a` (a preset id) or `header=float,margin=slim,user=bubble,…`; unknown keys and values are ignored
  init(code: String) {
    if let preset = Self.presets.first(where: { $0.id == code }) { self = preset.style; return }
    self.init(axes: code)
  }

  /// `key=value` pairs only. `presets` is built through this one: going through `init(code:)` would read
  /// `presets` while it is still being initialised, which re-enters its `dispatch_once` and traps at launch
  private init(axes code: String) {
    for pair in code.split(separator: ",") {
      let kv = pair.split(separator: "=", maxSplits: 1).map(String.init)
      if kv.count == 2 { set(kv[0], kv[1]) }
    }
  }

  var code: String { Self.axes.map { "\($0.key)=\(value($0.key))" }.joined(separator: ",") }

  /// The preset this style matches exactly, if any
  var preset: String? { Self.presets.first { $0.style == self }?.id }

  func value(_ key: String) -> String {
    switch key {
    case "header": header.rawValue
    case "margin": margin.rawValue
    case "user": user.rawValue
    case "process": process.rawValue
    case "actions": actions.rawValue
    case "composer": composer.rawValue
    case "shape": shape.rawValue
    case "listHeader": listHeader.rawValue
    case "listRow": listRow.rawValue
    case "listBlock": listBlock.rawValue
    case "rowMeta": rowMeta.rawValue
    case "grouping": grouping.rawValue
    case "listComposer": listComposer.rawValue
    case "chrome": chrome.rawValue
    case "chips": chips.rawValue
    case "scale": scale.rawValue
    default: ""
    }
  }

  mutating func set(_ key: String, _ raw: String) {
    switch key {
    case "header": header = Header(rawValue: raw) ?? header
    case "margin": margin = Margin(rawValue: raw) ?? margin
    case "user": user = User(rawValue: raw) ?? user
    case "process": process = Process(rawValue: raw) ?? process
    case "actions": actions = Actions(rawValue: raw) ?? actions
    case "composer": composer = Composer(rawValue: raw) ?? composer
    case "shape": shape = Shape(rawValue: raw) ?? shape
    case "listHeader": listHeader = ListHeader(rawValue: raw) ?? listHeader
    case "listRow": listRow = ListRow(rawValue: raw) ?? listRow
    case "listBlock": listBlock = ListBlock(rawValue: raw) ?? listBlock
    case "rowMeta": rowMeta = RowMeta(rawValue: raw) ?? rowMeta
    case "grouping": grouping = Grouping(rawValue: raw) ?? grouping
    case "listComposer": listComposer = ListComposer(rawValue: raw) ?? listComposer
    case "chrome": chrome = Chrome(rawValue: raw) ?? chrome
    case "chips": chips = Chips(rawValue: raw) ?? chips
    case "scale": scale = Scale(rawValue: raw) ?? scale
    default: break
    }
  }

  /// The shape of a control of the given side under this style's shape language
  func controlShape(_ radius: CGFloat = Metrics.rMd) -> AnyShape {
    shape == .round ? AnyShape(Capsule()) : AnyShape(RoundedRectangle(cornerRadius: radius))
  }
}

extension EnvironmentValues {
  @Entry var shellStyle = ShellStyle()
}

extension Prefs {
  /// The design study's stored style (`ShellStyle.code`)
  static let shellStyle = "acpira.lab.style"
}

// MARK: - Shared pieces

/// A floating glass control: a circle (or a rounded square under `square`) holding one glyph
struct GlassIconButton: View {
  let icon: LucideIcon
  let label: String
  var action: () -> Void
  @Environment(\.shellStyle) private var style

  var body: some View {
    Button(action: action) {
      Icon(icon, size: Metrics.glassIcon)
        .foregroundStyle(Palette.fg1)
        .frame(width: Metrics.glassCtl, height: Metrics.glassCtl)
        .contentShape(style.controlShape(Metrics.rLg))
    }
    .buttonStyle(.plain)
    .glassEffect(GlassStyle.control.interactive(), in: style.controlShape(Metrics.rLg))
    .accessibilityLabel(Text(web: label))
  }
}

/// Several glyph buttons sharing one glass capsule (new session and the menu, search and settings)
struct GlassGroup<Content: View>: View {
  @ViewBuilder var content: Content
  @Environment(\.shellStyle) private var style

  var body: some View {
    HStack(spacing: 0) { content }
      .glassEffect(GlassStyle.control.interactive(), in: style.controlShape(Metrics.rLg))
  }
}

/// A glyph inside a GlassGroup: the group's height, a little narrower than a lone glass button
struct GlassGroupItem: View {
  let icon: LucideIcon
  let label: String
  var open = false
  var action: () -> Void

  var body: some View {
    Button(action: action) {
      Icon(icon, size: Metrics.glassIcon)
        .foregroundStyle(open ? Palette.fgStrong : Palette.fg1)
        .frame(width: Metrics.glassGroupItem, height: Metrics.glassCtl)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityLabel(Text(web: label))
  }
}
