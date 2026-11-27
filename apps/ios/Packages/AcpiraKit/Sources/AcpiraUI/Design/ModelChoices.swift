import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// A sidekick choice's glyph, on its bean or in its list: its model's mark, or the faint fading plus for none
struct SidekickRowGlyph: View {
  var option: SessionOption?
  var size: CGFloat
  var body: some View {
    if let option {
      VendorMark(brand: ModelCatalog.optionBrand(option), name: option.name, size: size)
    } else {
      Icon(.circleFadingPlus, size: size, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
    }
  }
}

/// The sidekick's mark: its model's mark with the desktop's plus badge (SidekickPair.tsx `SidekickMark`) set into a
/// ring cut out of the mark's corner, or a faint fading plus while there is none, an empty slot that reads as "add
/// one" in any language. The badge's plus is cut out of it, so the glass shows through
struct SidekickMark: View {
  var picked: SessionOption?

  var body: some View {
    if let picked {
      let b = Metrics.sidekickBadge, n = Metrics.sidekickNotch
      VendorMark(brand: ModelCatalog.optionBrand(picked), name: picked.name, size: Metrics.barIcon)
        .mask {
          ZStack(alignment: .bottomTrailing) {
            Rectangle()
            Circle()
              .frame(width: b + n * 2, height: b + n * 2)
              .offset(x: n * 2, y: n * 2)
              .blendMode(.destinationOut)
          }
          .compositingGroup()
        }
        .overlay(alignment: .bottomTrailing) {
          ZStack {
            Circle()
            Icon(.plus, size: b, strokeWidth: 2).blendMode(.destinationOut)
          }
          .compositingGroup()
          .frame(width: b, height: b)
          .offset(x: n, y: n)
        }
    } else {
      Icon(.circleFadingPlus, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
    }
  }
}

/// modeIcons.ts `accessLabel`: the built-in agent's approval levels by id, any other level its short mode label
func accessLabel(_ level: SessionOption) -> String {
  let keys = ["ask": "composer.access.ask", "auto-edit": "composer.access.autoEdit", "full": "composer.access.full"]
  return keys[level.id].map { t($0) } ?? modeLabel(level)
}

/// modeIcons.ts `accessIcon`: the approval levels (the built-in agent's, and `auto` from the engine's split of Claude's
/// modes) as one shield family, so the bean reads as a scale; other levels keep their mode glyph
func accessIcon(_ level: SessionOption) -> LucideIcon {
  let icons: [String: LucideIcon] = ["ask": .shieldEllipsis, "auto-edit": .shieldCheck, "auto": .shieldHalf, "full": .shieldOff]
  return icons[level.id] ?? modeIcon(level)
}

/// A collaboration mode's name: Codex's `default` is its Build mode, not the permission default beside it
func workLabel(_ mode: SessionOption) -> String {
  mode.id == "default" ? t("composer.modeShort.build") : modeLabel(mode)
}

/// One Fast toggle per panel, whatever its source: an embedded variant, a Fusion pair, or a native model_config control
struct FastState {
  var on: Bool
  var disabled: Bool
  var set: (Bool) -> Void
}

/// Devin's Standard / Fast select or an adapter boolean as one toggle
func nativeFastState(_ c: ConfigControl, onChange: @escaping (String) -> Void) -> FastState {
  let on = ComposerControls.fastOn(c)
  let next = ComposerControls.fastValue(c, on: !on)
  return FastState(on: on, disabled: !c.options.contains { $0.id == next }) { _ in onChange(next) }
}

/// What the composer's model chip offers, read from the session's controls: the model families, the reasoning levels
/// with Fast, and the modes
struct ModelChoices {
  /// The reasoning levels on offer (the family's own scale, else the agent's thought_level), Fast beside them, and the
  /// Thinking switch where thinking can be turned off
  typealias Effort = (levels: [(id: String, name: String)], value: String, pick: (String) -> Void, fast: FastState?,
                      thinking: FastState?)

  let families: [ModelFamily]
  let cur: ModelFamily?
  let curVar: ModelVariant?
  /// The config option holding the model, when the agent offers one
  let modelId: String?
  let effort: Effort?
  /// The mode in force (the agent's first until one is set)
  let mode: SessionOption?
  /// The modes to choose between; none when the agent has only one
  let modes: [SessionOption]
  /// Codex's collaboration mode (Build / Plan), a working mode beside the permission modes
  let collaboration: ConfigControl?
  /// The approval level offered as a config option (the built-in agent's), its own bean after the modes
  let access: ConfigControl?
  /// Main-thread agent persona (e.g. Claude `.claude/agents`), shown only when custom agents are configured
  let agent: ConfigControl?
  /// The built-in agent's sidekick select, its level and its Fast (the last bean opens them as beans of their own)
  let sidekick: ConfigControl?
  let sidekickCur: ModelFamily?
  let sidekickCurVar: ModelVariant?
  let sidekickEffort: ConfigControl?
  let sidekickEffortScale: Effort?
  let sidekickFast: FastState?
  private let nativeFast: FastState?
  private let onSetConfig: (String, String) -> Void

  /// hidden: the agent's settings `hiddenOptions` (option id → hidden family keys); the model in use always stays
  init(controls: SessionControls, hidden: [String: [String]] = [:], onSetConfig: @escaping (String, String) -> Void) {
    let split = ComposerControls.split(controls.options)
    let model = split.models.first.map { Self.visible($0, hidden: hidden) }
    let families = model.map { ModelCatalog.grouped($0.options) } ?? []
    let cur = families.first { f in f.variants.contains { $0.id == model?.value } }
    let curVar = cur?.variants.first { $0.id == model?.value }
    self.families = families
    self.cur = cur
    self.curVar = curVar
    self.modelId = model?.id
    self.effort = Self.effort(split: split, cur: cur, curVar: curVar, modelId: model?.id, onSetConfig: onSetConfig)
    self.mode = controls.modes.first { $0.id == controls.modeId } ?? controls.modes.first
    // Beside a permission control the modes have no bean: the working ones are picked from the `/` list (Composer.tsx)
    self.modes = controls.modes.count > 1 && ComposerControls.workingModes(controls) == nil ? controls.modes : []
    self.collaboration = split.collaboration.first { $0.options.count > 1 }
    self.access = split.permission.first { $0.options.count > 1 }
    self.agent = split.agent.first { $0.options.count > 1 }
    let sidekick = Self.sidekick(split, hidden: hidden)
    let skCur = sidekick.flatMap { ComposerControls.findSidekickFamily($0.select.options, value: $0.select.value) }
    let nativeSkFast = split.sidekickFast.first.map { c in nativeFastState(c) { onSetConfig(c.id, $0) } }
    let skEffort = Self.sidekickEffort(select: sidekick?.select, cur: skCur?.family, curVar: skCur?.variant,
                                       nativeEffort: sidekick?.effort, nativeFast: nativeSkFast, onSetConfig: onSetConfig)
    self.sidekick = sidekick?.select
    self.sidekickCur = skCur?.family
    self.sidekickCurVar = skCur?.variant
    self.sidekickEffort = sidekick?.effort
    self.sidekickEffortScale = skEffort
    self.sidekickFast = (skEffort?.fast ?? nativeSkFast).flatMap { $0.disabled && !$0.on ? nil : $0 }
    self.nativeFast = split.modelConfig.first(where: ComposerControls.isFast).map { c in nativeFastState(c) { onSetConfig(c.id, $0) } }
    self.onSetConfig = onSetConfig
  }

  /// The models the settings left visible (ModelPicker.tsx visibleOptions)
  private static func visible(_ c: ConfigControl, hidden: [String: [String]], key: String? = nil) -> ConfigControl {
    var c = c
    c.options = ModelCatalog.visibleOptions(c.options, hidden: hidden[key ?? c.id], current: c.value)
    return c
  }

  /// The sidekick select, thinned as the lead's list is, and its level select. Cross-CLI host sidekicks are already
  /// filtered per CLI by the engine (`host_sidekick.rs`), so only a native single-agent sidekick takes the lead's hidden list
  static func sidekick(_ split: ComposerControls.Split, hidden: [String: [String]]) -> (select: ConfigControl, effort: ConfigControl?)? {
    split.sidekick.first.map { sk in
      let shown = ComposerControls.sidekickClis(sk.options).isEmpty
        ? visible(sk, hidden: hidden, key: split.models.first?.id)
        : sk
      return (shown, split.sidekickEffort.first)
    }
  }

  /// Which of the chip's beans apply, in the column's order: the model, the reasoning level, Fast, the working mode,
  /// the modes, the approval level
  var hasModel: Bool { cur != nil && modelId != nil }
  var hasEffort: Bool { effort.map { $0.levels.count > 1 || $0.thinking != nil } ?? false }
  var hasModes: Bool { !modes.isEmpty || access != nil || agent != nil }

  /// Fast, when the model in use can switch it at the level in force, or while it is on (the bolt on the chip says
  /// so). A family with Fast at other levels only offers none here, rather than a switch that cannot move
  var fast: FastState? { (effort?.fast ?? nativeFast).flatMap { $0.disabled && !$0.on ? nil : $0 } }

  var fusion: (leads: [String], sidekicks: [String])? { cur?.fusion }

  /// Keep the lead's effort and the other side where possible; the shared resolver picks the nearest offered pair.
  func pickFusion(lead: String? = nil, effort: String? = nil, sidekick: String? = nil, fast: Bool? = nil) {
    guard let f = cur, f.fusion != nil, let v = curVar, let modelId,
          let hit = ModelCatalog.findFusionVariant(f, lead: lead ?? v.lead ?? "", effort: effort ?? v.effort,
            sidekick: sidekick ?? v.sidekick, fast: fast ?? v.fast) else { return }
    onSetConfig(modelId, hit.id)
  }

  func offersSidekick(_ name: String) -> Bool {
    guard let f = cur, let v = curVar else { return false }
    return f.variants.contains { $0.lead == v.lead && $0.effort == v.effort && $0.sidekick == name }
  }

  private static func variantFast(family f: ModelFamily, variant v: ModelVariant, controlId: String,
                                  onSetConfig: @escaping (String, String) -> Void) -> FastState? {
    guard f.fusion == nil, f.hasFast else { return nil }
    func at(_ long: Bool) -> ModelVariant? { f.variants.first { $0.effort == v.effort && $0.fast == !v.fast && $0.long == long } }
    let hit = at(false) ?? at(v.long)
    return FastState(on: v.fast, disabled: hit == nil) { _ in if let hit { onSetConfig(controlId, hit.id) } }
  }

  private static func effort(split: ComposerControls.Split, cur: ModelFamily?, curVar: ModelVariant?, modelId: String?,
                             onSetConfig: @escaping (String, String) -> Void) -> Effort? {
    let nativeFast = split.modelConfig.first(where: ComposerControls.isFast).map { c in nativeFastState(c) { onSetConfig(c.id, $0) } }
    let variantFast = cur.flatMap { f in curVar.flatMap { v in modelId.flatMap { id in Self.variantFast(family: f, variant: v, controlId: id, onSetConfig: onSetConfig) } } }
    var fusionFast: FastState?
    if let f = cur, f.fusion != nil, let v = curVar, let modelId {
      let pick = { (e: String, fast: Bool) in
        if let hit = ModelCatalog.findFusionVariant(f, lead: v.lead ?? "", effort: e, sidekick: v.sidekick, fast: fast) {
          onSetConfig(modelId, hit.id)
        }
      }
      if f.hasFast {
        let can = f.variants.contains { $0.lead == v.lead && $0.effort == v.effort && $0.sidekick == v.sidekick && $0.fast != v.fast }
        fusionFast = FastState(on: v.fast, disabled: !can) { pick(v.effort, $0) }
      }
      let levels = ModelCatalog.fusionEfforts(f, v)
      if levels.count > 1 {
        return (levels.map { ($0, $0.isEmpty ? t("composer.standard") : $0) }, v.effort,
                { pick($0, v.fast) }, fusionFast ?? nativeFast, nil)
      }
    }
    let resolvedFast = fusionFast ?? variantFast ?? nativeFast
    if let f = cur, let v = curVar, let modelId, f.fusion == nil, ModelCatalog.variantScale(f) || ModelCatalog.thinkingOnly(f) {
      let pick = { (e: String) in
        if let hit = ModelCatalog.findVariant(f, effort: e, fast: v.fast, long: false) { onSetConfig(modelId, hit.id) }
      }
      // A Standard / Thinking pair is a switch, not two levels
      if ModelCatalog.thinkingOnly(f) {
        let on = v.effort == "Thinking"
        let thinking = FastState(on: on, disabled: ModelCatalog.findVariant(f, effort: on ? "" : "Thinking", fast: v.fast, long: false) == nil) {
          pick($0 ? "Thinking" : "")
        }
        return ([], v.effort, pick, resolvedFast, thinking)
      }
      return (f.efforts.map { ($0, $0.isEmpty ? t("composer.standard") : $0) }, v.effort, pick, resolvedFast, nil)
    }
    if let c = split.reasoning.first(where: ComposerControls.reasoningVisible) {
      let p = ComposerControls.presentReasoning(c)
      // Turned back on at High, else the first level, else the agent's own "on"
      let turnOn = p.efforts.first { $0.id == "high" }?.id ?? p.efforts.first?.id ?? p.onId
      let thinking = p.offId.map { off in
        FastState(on: !p.off, disabled: false) { on in
          if !on { onSetConfig(c.id, off) } else if let turnOn { onSetConfig(c.id, turnOn) }
        }
      }
      guard p.efforts.count > 1 || thinking != nil else {
        return (fusionFast ?? variantFast).map { ([], curVar?.effort ?? "", { _ in }, $0, nil) }
      }
      return (p.efforts.map { ($0.id, $0.name) }, p.off ? "" : p.value ?? "", { onSetConfig(c.id, $0) }, resolvedFast, thinking)
    }
    // A model with embedded Fast (Factory "Opus 5.5 Fast Mode") or pair-level Fast can have Fast without a reasoning bean
    return (fusionFast ?? variantFast).map { ([], curVar?.effort ?? "", { _ in }, $0, nil) }
  }

  /// Sidekick reasoning levels and Fast: embedded variants first (Devin effort / Fast, Factory Fast Mode), then native selects
  private static func sidekickEffort(select: ConfigControl?, cur: ModelFamily?, curVar: ModelVariant?,
                                     nativeEffort: ConfigControl?, nativeFast: FastState?,
                                     onSetConfig: @escaping (String, String) -> Void) -> Effort? {
    guard let select, let f = cur, let v = curVar else { return nil }
    let variantFast = Self.variantFast(family: f, variant: v, controlId: select.id, onSetConfig: onSetConfig)
    let resolvedFast = variantFast ?? nativeFast
    if f.fusion == nil, ModelCatalog.variantScale(f) || ModelCatalog.thinkingOnly(f) {
      let pick = { (e: String) in
        if let hit = ModelCatalog.findVariant(f, effort: e, fast: v.fast, long: false) { onSetConfig(select.id, hit.id) }
      }
      if ModelCatalog.thinkingOnly(f) {
        let on = v.effort == "Thinking"
        let thinking = FastState(on: on, disabled: ModelCatalog.findVariant(f, effort: on ? "" : "Thinking", fast: v.fast, long: false) == nil) {
          pick($0 ? "Thinking" : "")
        }
        return ([], v.effort, pick, resolvedFast, thinking)
      }
      return (f.efforts.map { ($0, $0.isEmpty ? t("composer.standard") : $0) }, v.effort, pick, resolvedFast, nil)
    }
    if let c = nativeEffort, ComposerControls.reasoningVisible(c) {
      let p = ComposerControls.presentReasoning(c)
      let turnOn = p.efforts.first { $0.id == "high" }?.id ?? p.efforts.first?.id ?? p.onId
      let thinking = p.offId.map { off in
        FastState(on: !p.off, disabled: false) { on in
          if !on { onSetConfig(c.id, off) } else if let turnOn { onSetConfig(c.id, turnOn) }
        }
      }
      if p.efforts.count > 1 || thinking != nil {
        return (p.efforts.map { ($0.id, $0.name) }, p.off ? "" : p.value ?? "", { onSetConfig(c.id, $0) }, resolvedFast, thinking)
      }
    }
    return variantFast.map { ([], v.effort, { _ in }, $0, nil) }
  }

  /// ModelPanel.pickFamily: into Fusion the model in use becomes the lead; out of it the lead's effort carries over
  func pickFamily(_ f: ModelFamily) {
    guard let modelId else { return }
    let next = f.fusion != nil
      ? ModelCatalog.findFusionVariant(f, lead: curVar?.lead ?? cur?.name ?? "", effort: curVar?.effort ?? "", sidekick: curVar?.sidekick, fast: curVar?.fast ?? false)
      : curVar.flatMap { ModelCatalog.findVariant(f, effort: $0.effort, fast: $0.fast, long: false) }
    if let id = (next ?? f.variants.first)?.id { onSetConfig(modelId, id) }
  }
}

/// The model chip at rest: the model's mark, large, with Fast as a bolt on its shoulder (a sliders glyph when the
/// agent names no model)
struct ChipMark: View {
  var cur: ModelFamily?
  var fast: Bool
  /// The sidekick's colour while it works a round: a dot in it breathes on the mark's foot, as the desktop chip lights
  /// its sidekick half (the bar has no sidekick half at rest); nil while it does not
  var sidekick: Color?
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    ZStack {
      Group {
        if let cur {
          VendorMark(brand: cur.brand, name: cur.name, size: Metrics.barIcon)
        } else {
          Icon(.slidersHorizontal, size: Metrics.barIcon, strokeWidth: 1.75)
        }
      }
      // Another model (a pick, or another agent on the new session page) crossfades its mark in place
      .id(cur?.key)
      .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
    }
    .overlay(alignment: .topTrailing) {
      if fast {
        FastGlyph(on: true, size: Metrics.chevron).offset(x: Metrics.s1, y: -Metrics.s1)
          .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
      }
    }
    // Its own curve: the new session page swaps agents inside the ring's no-animation transaction, and a pushed
    // answer comes with none, so the mark would otherwise blink from one model to the next
    .transaction(value: MarkKey(model: cur?.key, fast: fast)) {
      $0.disablesAnimations = false
      $0.animation = motion ? Motion.quick : nil
    }
    .overlay(alignment: .bottomTrailing) {
      if let sidekick { SidekickPulse(tint: sidekick).offset(x: Metrics.s1, y: Metrics.s1).transition(.scale.combined(with: .opacity)) }
    }
    .animation(Motion.quick, value: sidekick != nil)
  }

  private struct MarkKey: Equatable {
    var model: String?
    var fast: Bool
  }
}

/// The sidekick's live dot (`.sidekick-pulse`), ringed in the page colour so it stands off the mark; it holds still
/// with motion off
private struct SidekickPulse: View {
  let tint: Color
  @Environment(\.motionEnabled) private var motion
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @State private var low = false

  var body: some View {
    Circle()
      .fill(tint)
      .frame(width: Metrics.sidekickPulse, height: Metrics.sidekickPulse)
      .padding(Metrics.hairline)
      .background(Circle().fill(Palette.bg0))
      .opacity(low ? Motion.breatheLow : 1)
      .onAppear {
        guard motion && !reduceMotion else { return }
        withAnimation(Motion.breathe.repeatForever(autoreverses: true)) { low = true }
      }
      .accessibilityHidden(true)
  }
}
