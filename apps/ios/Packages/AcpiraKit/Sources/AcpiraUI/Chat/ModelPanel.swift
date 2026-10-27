import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/ModelPicker.tsx: the lg model panel (family list, then the current family's params, native reasoning and model
// parameters as a small form), the flat option menu and the reasoning-only chip. Picking a family closes the panel;
// effort, Fast and Thinking apply at once and keep it open.

/// Only offer search once the list passes this length; short lists are scannable at a glance
private let searchFrom = 12

/// One Fast toggle per panel, whatever its source: an embedded variant, a Fusion pair, or a native model_config control
struct FastState {
  var on: Bool
  var disabled: Bool
  var set: (Bool) -> Void
}

struct ModelPanel: View {
  let families: [ModelFamily]
  var cur: ModelFamily?
  var curVar: ModelVariant?
  var reasoning: [ConfigControl] = []
  var modelConfig: [ConfigControl] = []
  var onSelect: (String) -> Void
  var onSetConfig: (String, String) -> Void
  @State private var query = ""
  @Environment(\.dismissPopup) private var dismiss

  private enum Owner { case embedded, reasoning, row }

  var body: some View {
    let shown = reasoning.filter(ComposerControls.reasoningVisible)
    let params: (family: ModelFamily, variant: ModelVariant)? = if let cur, let curVar, cur.variants.count > 1 { (cur, curVar) } else { nil }
    let nativeFast = modelConfig.first(where: ComposerControls.isFast)
    let embeddedFast = params.map { $0.family.hasFast } ?? false
    let fast: FastState? = if let params, embeddedFast {
      params.family.fusion != nil ? fusionFast(params.family, params.variant) : variantFast(params.family, params.variant)
    } else {
      nativeFast.map { c in nativeFastState(c) { onSetConfig(c.id, $0) } }
    }
    let usedNative = fast != nil && !embeddedFast ? nativeFast : nil
    // The Fast toggle sits in the first effort header; with no scale to sit on it stays a switch row
    let embeddedScale = params.map { p in
      p.family.fusion != nil ? ModelCatalog.fusionEfforts(p.family, p.variant).count > 1 : ModelCatalog.variantScale(p.family)
    } ?? false
    let reasoningScale = shown.firstIndex { ComposerControls.presentReasoning($0).efforts.count > 1 }
    let owner: Owner? = fast == nil ? nil : embeddedScale ? .embedded : reasoningScale != nil ? .reasoning : .row
    let toggle = fast.map { f in
      AnyView(HeaderToggle("Fast", pressed: f.on, onChange: f.set) { FastGlyph(on: f.on) }.disabled(f.disabled))
    }
    let rest = modelConfig.filter { $0.id != usedNative?.id }
    let hits = filtered

    VStack(alignment: .leading, spacing: 0) {
      if families.count >= searchFrom { PopupSearch(query: $query) }
      if hits.isEmpty {
        PopupEmpty()
      } else {
        PopupList(current: cur?.key) {
          ForEach(hits, id: \.key) { f in
            MenuOption(f.name, description: f.source, checked: f.key == cur?.key, checkSlot: cur != nil) {
              pickFamily(f)
            } mark: {
              VendorMark(brand: f.brand, name: f.name)
            }
            .accessibilityHint(Text(web: f.description ?? ""))
            .id(f.key)
          }
        }
      }
      if params != nil || !shown.isEmpty || !modelConfig.isEmpty {
        // mt-1 border-t pt-1.5: the form under the list
        VStack(alignment: .leading, spacing: Metrics.s0_5) {
          if let params {
            if params.family.fusion != nil {
              FusionParams(family: params.family, variant: params.variant, onSelect: onSelect, actions: owner == .embedded ? toggle : nil)
            } else {
              ModelParams(family: params.family, variant: params.variant, onSelect: onSelect, actions: owner == .embedded ? toggle : nil)
            }
          }
          ForEach(Array(shown.enumerated()), id: \.element.id) { i, c in
            ReasoningParams(control: c, onChange: { onSetConfig(c.id, $0) }, actions: owner == .reasoning && i == reasoningScale ? toggle : nil)
          }
          if owner == .row, let fast {
            SwitchRow(label: "Fast", checked: fast.on, onChange: fast.set).disabled(fast.disabled)
          }
          ForEach(rest) { c in ModelConfigParams(control: c) { onSetConfig(c.id, $0) } }
        }
        // The border sits outside the padding (border-box)
        .padding(.top, Metrics.s1_5 + Metrics.hairline)
        .overlay(alignment: .top) { Rectangle().fill(Palette.menuLine).frame(height: Metrics.hairline) }
        .padding(.top, Metrics.s1)
      }
    }
  }

  /// Families whose name (or source) holds the query
  private var filtered: [ModelFamily] {
    let q = query.trimmingCharacters(in: .whitespaces)
    guard !q.isEmpty else { return families }
    return families.filter { [$0.name, $0.source ?? ""].joined(separator: " ").localizedStandardContains(q) }
  }

  /// Into Fusion the model in use becomes the lead; out of it the lead's effort carries over. A family switch lands on
  /// the standard-context variant (1M variants are not offered)
  private func pickFamily(_ f: ModelFamily) {
    let next = f.fusion != nil
      ? ModelCatalog.findFusionVariant(f, lead: curVar?.lead ?? cur?.name ?? "", effort: curVar?.effort ?? "", sidekick: curVar?.sidekick, fast: curVar?.fast ?? false)
      : curVar.flatMap { ModelCatalog.findVariant(f, effort: $0.effort, fast: $0.fast, long: false) }
    if let id = (next ?? f.variants.first)?.id { onSelect(id) }
    dismiss()
  }

  /// Embedded Fast: the same effort with Fast flipped, standard context first
  private func variantFast(_ f: ModelFamily, _ v: ModelVariant) -> FastState {
    func at(_ long: Bool) -> ModelVariant? { f.variants.first { $0.effort == v.effort && $0.fast == !v.fast && $0.long == long } }
    let hit = at(false) ?? at(v.long)
    return FastState(on: v.fast, disabled: hit == nil) { _ in if let hit { onSelect(hit.id) } }
  }

  /// Fusion Fast is pair-level: offered when the same lead, effort and sidekick exist with Fast flipped
  private func fusionFast(_ f: ModelFamily, _ v: ModelVariant) -> FastState {
    let lead = v.lead ?? f.fusion?.leads.first ?? ""
    let can = f.variants.contains { $0.lead == lead && $0.effort == v.effort && $0.sidekick == v.sidekick && $0.fast == !v.fast }
    return FastState(on: v.fast, disabled: !can) { on in
      if let hit = ModelCatalog.findFusionVariant(f, lead: lead, effort: v.effort, sidekick: v.sidekick, fast: on) { onSelect(hit.id) }
    }
  }
}

/// Devin's Standard / Fast select or an adapter boolean as one toggle
func nativeFastState(_ c: ConfigControl, onChange: @escaping (String) -> Void) -> FastState {
  let on = ComposerControls.fastOn(c)
  let next = ComposerControls.fastValue(c, on: !on)
  return FastState(on: on, disabled: !c.options.contains { $0.id == next }) { _ in onChange(next) }
}

/// The current family's params: reasoning levels as one segmented scale, a lone Standard / Thinking pair as a switch
private struct ModelParams: View {
  let family: ModelFamily
  let variant: ModelVariant
  var onSelect: (String) -> Void
  var actions: AnyView?

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      if ModelCatalog.variantScale(family) {
        SegmentScale(label: t("composer.effort"),
                     options: family.efforts.map { .init(value: $0, label: $0.isEmpty ? t("composer.standard") : $0) },
                     value: variant.effort, onChange: pick, actions: actions)
      }
      if ModelCatalog.thinkingOnly(family) {
        let other = variant.effort == "Thinking" ? "" : "Thinking"
        SwitchRow(label: "Thinking", checked: variant.effort == "Thinking") { pick($0 ? "Thinking" : "") }
          .disabled(ModelCatalog.findVariant(family, effort: other, fast: variant.fast, long: false) == nil)
      }
    }
  }

  /// A level change keeps Fast where it can and the standard context
  private func pick(_ effort: String) {
    if let hit = ModelCatalog.findVariant(family, effort: effort, fast: variant.fast, long: false) { onSelect(hit.id) }
  }
}

/// A Fusion pair's controls: Lead (menu), Effort (what that lead offers), Sidekick (menu, the ones this lead cannot
/// take disabled); every change resolves to the nearest offered pair
private struct FusionParams: View {
  let family: ModelFamily
  let variant: ModelVariant
  var onSelect: (String) -> Void
  var actions: AnyView?

  var body: some View {
    let fusion = family.fusion ?? ([], [])
    let lead = variant.lead ?? fusion.leads.first ?? ""
    let efforts = ModelCatalog.fusionEfforts(family, variant)
    let ofLead = family.variants.filter { $0.lead == lead }
    VStack(alignment: .leading, spacing: 0) {
      SelectRow(label: t("composer.lead"), options: fusion.leads.map { .init(value: $0, label: $0) }, value: lead) { pick(lead: $0) }
      if efforts.count > 1 {
        SegmentScale(label: t("composer.effort"), options: efforts.map { .init(value: $0, label: $0.isEmpty ? t("composer.standard") : $0) },
                     value: variant.effort, onChange: { pick(effort: $0) }, actions: actions)
      }
      SelectRow(label: t("composer.sidekick"),
                options: fusion.sidekicks.map { s in .init(value: s, label: s, disabled: !ofLead.contains { $0.effort == variant.effort && $0.sidekick == s }) },
                value: variant.sidekick ?? "") { pick(sidekick: $0) }
    }
  }

  private func pick(lead: String? = nil, effort: String? = nil, sidekick: String? = nil) {
    let hit = ModelCatalog.findFusionVariant(family, lead: lead ?? variant.lead ?? family.fusion?.leads.first ?? "",
                                             effort: effort ?? variant.effort, sidekick: sidekick ?? variant.sidekick, fast: variant.fast)
    if let hit { onSelect(hit.id) }
  }
}

/// Native thought_level: a Thinking switch when the agent can turn it off, then the effort segments (Ultra tinted last)
struct ReasoningParams: View {
  let control: ConfigControl
  var onChange: (String) -> Void
  var actions: AnyView?

  var body: some View {
    let p = ComposerControls.presentReasoning(control)
    let turnOn = p.efforts.first { $0.id == "high" }?.id ?? p.efforts.first?.id ?? p.onId
    VStack(alignment: .leading, spacing: 0) {
      if let offId = p.offId {
        SwitchRow(label: "Thinking", checked: !p.off) { on in
          if on { if let turnOn { onChange(turnOn) } } else { onChange(offId) }
        }
      }
      if p.efforts.count > 1 {
        SegmentScale(label: t("composer.effort"),
                     options: p.efforts.map { .init(value: $0.id, label: $0.name, ultra: ComposerControls.isUltra($0.name)) },
                     value: p.off ? "" : p.value ?? "", onChange: onChange, actions: actions)
      }
    }
  }
}

/// Model parameters past Fast: a boolean as its switch, anything else as a select row with the agent's labels
private struct ModelConfigParams: View {
  let control: ConfigControl
  var onChange: (String) -> Void

  var body: some View {
    if ComposerControls.isFast(control) {
      let fast = nativeFastState(control, onChange: onChange)
      SwitchRow(label: "Fast", checked: fast.on, onChange: fast.set).disabled(fast.disabled)
    } else if control.type == "boolean" {
      SwitchRow(label: control.name, checked: control.value == "true") { onChange($0 ? "true" : "false") }
    } else {
      SelectRow(label: control.name, options: control.options.map { .init(value: $0.id, label: $0.name) }, value: control.value ?? "", onChange: onChange)
    }
  }
}

/// Field.tsx SelectRow: the label, the current value and a caret on a --row line; the values open as a system menu
/// on top of the panel (a second anchored popup would cover the one it belongs to)
struct SelectRow: View {
  struct Option: Hashable { var value: String; var label: String; var disabled = false }
  var label: String
  var options: [Option]
  var value: String
  var onChange: (String) -> Void

  var body: some View {
    Menu {
      ForEach(options, id: \.value) { o in
        Button { onChange(o.value) } label: {
          if o.value == value { Label(o.label, systemImage: "checkmark") } else { Text(verbatim: o.label) }
        }
        .disabled(o.disabled)
      }
    } label: {
      HStack(spacing: Metrics.s2) {
        Text(web: label, paragraph: false).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
        Text(web: options.first { $0.value == value }?.label ?? value, paragraph: false)
          .foregroundStyle(Palette.popFg2)
          .lineLimit(1)
        Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.popFg3)
      }
      .padding(.horizontal, Metrics.s2)
      .frame(minHeight: Metrics.row)
    }
    .buttonStyle(MenuOptionStyle())
  }
}

// MARK: - Flat options

/// OptionMenu: any other select as a quiet chip; vendor marks only when some option has a known brand. Long lists
/// get the search field
struct OptionChip: View {
  let control: ConfigControl
  var onSelect: (String) -> Void
  @State private var open = false

  var body: some View {
    let branded = control.options.contains { ModelCatalog.optionBrand($0) != nil }
    let cur = control.options.first { $0.id == control.value }
    Button { open.toggle() } label: {
      if branded, let cur, let brand = ModelCatalog.optionBrand(cur) {
        ChipLabel(cur.name, caret: false) { VendorMark(brand: brand, name: cur.name) } tags: { EmptyView() }
      } else {
        ChipLabel(cur?.name ?? control.name, caret: false) { EmptyView() } tags: { EmptyView() }
      }
    }
    .buttonStyle(ChipStyle(open: open))
    .popup(isPresented: $open, edge: .trailing, width: Metrics.popMd) {
      OptionMenu(control: control, branded: branded, onSelect: onSelect)
    }
  }
}

private struct OptionMenu: View {
  let control: ConfigControl
  var branded: Bool
  var onSelect: (String) -> Void
  @State private var query = ""
  @Environment(\.dismissPopup) private var dismiss

  var body: some View {
    let searchable = control.options.count >= searchFrom
    let q = query.trimmingCharacters(in: .whitespaces)
    let hits = q.isEmpty ? control.options : control.options.filter { [$0.name, $0.description ?? ""].joined(separator: " ").localizedStandardContains(q) }
    let checkSlot = control.options.contains { $0.id == control.value }
    VStack(alignment: .leading, spacing: 0) {
      if searchable { PopupSearch(query: $query) }
      if hits.isEmpty {
        PopupEmpty()
      } else {
        PopupList(current: control.value) {
          ForEach(hits) { o in
            Group {
              if branded {
                MenuOption(o.name, description: o.description, checked: o.id == control.value, checkSlot: checkSlot) {
                  onSelect(o.id); dismiss()
                } mark: {
                  VendorMark(brand: ModelCatalog.optionBrand(o), name: o.name)
                }
              } else {
                MenuOption(o.name, description: o.description, checked: o.id == control.value, checkSlot: checkSlot) {
                  onSelect(o.id); dismiss()
                }
              }
            }
            .id(o.id)
          }
        }
      }
    }
  }
}

/// A model-less agent's reasoning: the level as a quiet chip, the same field in its panel
struct ReasoningChip: View {
  let control: ConfigControl
  var onSelect: (String) -> Void
  @State private var open = false

  var body: some View {
    if ComposerControls.reasoningVisible(control) {
      Button { open.toggle() } label: {
        ChipLabel(ComposerControls.reasoningChip(control) ?? t("composer.effort"), caret: false) { EmptyView() } tags: { EmptyView() }
      }
      .buttonStyle(ChipStyle(open: open))
      .popup(isPresented: $open, edge: .trailing, width: Metrics.popLg) {
        ReasoningParams(control: control, onChange: onSelect)
      }
    }
  }
}
