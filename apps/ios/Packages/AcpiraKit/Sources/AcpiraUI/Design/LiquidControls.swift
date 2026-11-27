import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// MARK: - The composer's liquid controls

/// Attach as a liquid chip: the photo, camera and file sources squeeze out of `+` in a short column, each named beside
/// it; the `+` turns ×. The pasteboard's image joins them while there is one, and the summonable personas
/// (SummonMenu) last, as a bean whose panel lists them
struct LiquidAttach: View {
  @Binding var open: String?
  /// The bean whose panel is open (the summon list), 0 for none
  @Binding var stage: Int
  /// How far a panel may rise above the chip (`GooColumn.panelHeight`)
  var room: CGFloat = .infinity
  /// Stages the pasteboard's image; nil while it holds none
  var paste: (() -> Void)? = nil
  var personas: [MentionPersona] = []
  var onSummon: (MentionPersona) -> Void = { _ in }
  var onPick: (AttachSource) -> Void
  @Environment(\.motionEnabled) private var motion

  /// One bean: its glyph and name, and either what a tap does or the panel it turns into
  private struct Source {
    let key: String
    let icon: LucideIcon
    let label: String
    var pick: (() -> Void)? = nil
    var rows = 0
    var panel: AnyView? = nil
  }

  var body: some View {
    let sources = sources()
    let chip = CGSize(width: Metrics.barCtl, height: Metrics.barCtl)
    let offsets = GooColumn.offsets(chip: chip, widths: sources.map { _ in chip.width })
    GooChip(id: "attach", open: $open, stage: $stage, pieces: zip(sources, offsets).enumerated().map { k, item in
      let (source, offset) = item
      let h = GooColumn.panelHeight(rows: source.rows, room: room)
      return GooPiece(id: "attach.\(source.key)", name: source.label, offset: offset,
                      glyph: AnyView(Icon(source.icon, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)),
                      caption: AnyView(GooCaption(text: source.label)),
                      panel: source.panel.map { GooPanel(size: CGSize(width: Metrics.gooPanelW, height: h), content: $0) }) {
        if let pick = source.pick {
          withAnimation(motion ? Motion.gooSpring : nil) { open = nil }
          pick()
        } else {
          withAnimation(motion ? Motion.gooSpring : nil) { stage = stage == k + 1 ? 0 : k + 1 }
        }
      }
    }) {
      // × over the beans, ‹ over the summon panel, as the model chip
      ZStack {
        if open == "attach" && stage > 0 {
          Icon(.chevronLeft, size: Metrics.barIcon, strokeWidth: 1.75)
            .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
        } else {
          Icon(.plus, size: Metrics.barIcon, strokeWidth: 1.75)
            .rotationEffect(.degrees(open == "attach" ? 45 : 0))
            .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
        }
      }
      .foregroundStyle(Palette.fg1)
      .accessibilityLabel(Text(web: open == "attach" && stage > 0 ? t("common.back") : t("mobile.attach")))
    }
  }

  private func sources() -> [Source] {
    var out = [
      Source(key: "photos", icon: .image, label: t("mobile.attach.photos")) { onPick(.photos) },
      Source(key: "camera", icon: .camera, label: t("mobile.attach.camera")) { onPick(.camera) },
      Source(key: "files", icon: .paperclip, label: t("mobile.attach.files")) { onPick(.files) },
    ]
    if let paste { out.append(Source(key: "paste", icon: .clipboardPaste, label: t("desktop.menu.paste"), pick: paste)) }
    guard !personas.isEmpty else { return out }
    out.append(Source(key: "summon", icon: .atSign, label: t("summon.entry"), rows: personas.count,
                      panel: AnyView(GooList(current: nil) {
                        ForEach(personas) { persona in
                          GooRow(name: persona.name, checked: false) {
                            SummonMark(persona: persona, size: Metrics.iconCtl).foregroundStyle(Palette.fg1)
                          } trailing: {
                            if let cli = persona.cli {
                              Text(web: cli, paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3).lineLimit(1)
                            }
                          } action: {
                            withAnimation(motion ? Motion.gooSpring : nil) { open = nil; stage = 0 }
                            onSummon(persona)
                          }
                        }
                      })))
    return out
  }
}

/// The model button as one liquid chip (the model's mark, large, beside `+`). Opening it squeezes its beans out of
/// it in a column: the model, the reasoning level, Fast, the working mode (Codex's collaboration mode) and the agent's
/// modes, then the approval level, each with its value on its right, and the mark turns into × (the chip
/// has become the beans, as `+` does). A tapped bean stretches into its panel of choices and swallows the others; a
/// pick shrinks it back into the bean and leaves the beans out, so another setting can change next. Fast is one bean
/// whatever the agent's wire shape and flips on a tap, no panel; Thinking flips in place. The built-in agent's
/// sidekick is the last bean: a tap pulls the beans home and squeezes out the sidekick's own, its model, its level and
/// its Fast, each working as the lead's do; the chip (‹) trades them back
struct LiquidControls: View {
  let controls: SessionControls
  @Binding var open: String?
  /// The bean whose panel is open (1…), 0 for none
  @Binding var stage: Int
  /// `sidekick` while the sidekick's own beans are out, nil on the lead's
  @Binding var deeper: String?
  /// How far a panel may rise above the chip (`GooColumn.panelHeight`)
  var room: CGFloat = .infinity
  /// The built-in agent's sidekick is working a round
  var sidekickLive = false
  var onSetMode: (String) -> Void
  var onSetConfig: (String, String) -> Void
  @Environment(\.motionEnabled) private var motion
  @Environment(\.hiddenModels) private var hidden
  /// The beans tucked into the chip while they trade for the other level's
  @State private var held = false
  /// The CLI whose models the sidekick's model bean lists, once one was picked from its agent bean; nil: the picked
  /// sidekick's CLI
  @State private var browse: String?

  /// The value of `deeper` while the sidekick's beans are out
  static let sidekickLevel = "sidekick"

  var body: some View {
    let c = ModelChoices(controls: controls, hidden: hidden, onSetConfig: onSetConfig)
    let nested = deeper == Self.sidekickLevel && (c.fusion != nil || c.sidekick != nil)
    let beans = nested ? (c.fusion != nil ? [fusionModelBean(c, sidekick: true)] : sidekickBeans(c)) : beans(c)
    let inside = stage > 0 || deeper != nil
    if !beans.isEmpty {
      GooChip(id: "model", open: $open, stage: $stage, deeper: $deeper, held: held, leave: { enter(nil) },
              pieces: GooColumn.pieces(beans, room: room, motion: motion, stage: $stage)) {
        // The model's mark alone, large; Fast shows as a bolt on its shoulder. Open, it is the way back: × over the
        // beans, ‹ over a panel
        ZStack {
          if open == "model" {
            Icon(inside ? .chevronLeft : .x, size: Metrics.gooBack, strokeWidth: 1.75)
              .id(inside)
              .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
          } else {
            ChipMark(cur: c.cur, fast: c.fast?.on == true, sidekick: sidekickLive ? sidekickTint(c) : nil)
              .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
          }
        }
        .foregroundStyle(Palette.fg1)
        // The mark leaves at once (the first bean carries it up) and comes back as the last drop lands
        .animation(motion ? open == "model" ? Motion.gooSwap : Motion.gooSwapBack : nil, value: open == "model" ? (inside ? 2 : 1) : 0)
        .accessibilityLabel(Text(web: open == "model" ? t(inside ? "common.back" : "common.close") : c.cur?.name ?? t("stats.model")))
      }
    }
  }

  /// The live sidekick's colour on the chip: its model vendor's own, else the neutral ink; no dot while the select
  /// reads None (ModelPicker.tsx shows the live tail only with a sidekick picked)
  private func sidekickTint(_ c: ModelChoices) -> Color? {
    guard let picked = c.sidekick.flatMap({ sk in sk.options.first { $0.id == sk.value && $0.id != ComposerControls.noSidekick } }) else {
      return nil
    }
    return VendorBrand.color(ModelCatalog.optionBrand(picked)) ?? Palette.fg1
  }

  private func beans(_ c: ModelChoices) -> [GooBean] {
    let families = c.families
    let effort = c.effort
    var out: [GooBean] = []
    if c.hasModel, let cur = c.cur {
      out.append(GooBean(key: "model", title: t("stats.model"),
                      glyph: AnyView(VendorMark(brand: cur.brand, name: cur.name, size: Metrics.barIcon).foregroundStyle(Palette.fg1)),
                      caption: AnyView(GooCaption(text: cur.name)), rows: families.count,
                      panel: AnyView(GooList(current: cur.key) {
                        ForEach(families, id: \.key) { f in
                          GooRow(name: f.name, checked: f.key == cur.key) {
                            VendorMark(brand: f.brand, name: f.name, size: Metrics.iconCtl)
                          } action: {
                            c.pickFamily(f)
                            fold()
                          }
                          .id(f.key)
                        }
                      })))
    }
    if c.fusion != nil { out.append(fusionModelBean(c, sidekick: false)) }
    if c.hasEffort, let effort {
      let levels = effort.levels.count > 1 ? effort.levels : []
      let thinking = effort.thinking
      let name = thinking?.on == false ? t("mobile.thinking.off")
        : levels.first { $0.id == effort.value }?.name ?? (thinking != nil ? "Thinking" : "")
      out.append(GooBean(key: "effort", title: t("composer.effort"),
                      glyph: AnyView(Icon(.brain, size: Metrics.barIcon, strokeWidth: 1.75)
                        .foregroundStyle(thinking?.on == false ? Palette.fg3 : Palette.fg1)),
                      caption: AnyView(GooCaption(text: name)), rows: levels.count + (thinking != nil ? 1 : 0),
                      panel: AnyView(GooList(current: effort.value) {
                        if let thinking {
                          // ReasoningParams' Thinking switch (ModelParams' for a Standard / Thinking pair): flips in
                          // place, the panel stays out
                          GooRow(name: "Thinking", checked: thinking.on) {
                            Icon(.brain, size: Metrics.icon, strokeWidth: 1.75)
                          } trailing: {
                            Toggle("", isOn: Binding(get: { thinking.on }, set: { thinking.set($0) }))
                              .labelsHidden()
                              .tint(.green)
                              .disabled(thinking.disabled)
                          } action: { if !thinking.disabled { thinking.set(!thinking.on) } }
                        }
                        ForEach(levels, id: \.id) { level in
                          GooRow(name: level.name, checked: level.id == effort.value) { EmptyView() } action: {
                            effort.pick(level.id)
                            fold()
                          }
                          .id(level.id)
                        }
                      })))
    }
    if let fast = c.fast {
      // Every agent's Fast reads the same: the bolt and its name, lit on a tap, the panel never opens
      out.append(GooBean(key: "fast", title: "Fast",
                         glyph: AnyView(FastGlyph(on: fast.on, size: Metrics.barIcon)),
                         caption: AnyView(GooCaption(text: "Fast")),
                         tap: { if !fast.disabled { Haptics.play(.pick); withAnimation(motion ? Motion.quick : nil) { fast.set(!fast.on) } } }))
    }
    if let work = c.collaboration {
      let current = work.options.first { $0.id == work.value } ?? work.options.first
      out.append(GooBean(key: "work", title: work.name,
                         glyph: AnyView(Icon(current.map(modeIcon) ?? .circleDashed, size: Metrics.barIcon, strokeWidth: 1.75)
                           .foregroundStyle(Palette.fg1)),
                         caption: AnyView(GooCaption(text: current.map(workLabel) ?? work.name)), rows: work.options.count,
                         panel: AnyView(GooList(current: current?.id) {
                           ForEach(work.options) { o in
                             GooRow(name: workLabel(o), checked: o.id == current?.id) {
                               Icon(modeIcon(o), size: Metrics.icon, strokeWidth: 1.75)
                             } action: {
                               onSetConfig(work.id, o.id)
                               fold()
                             }
                             .id(o.id)
                           }
                         })))
    }
    if !c.modes.isEmpty { out.append(modeBean(c)) }
    if let access = c.access { out.append(accessBean(access)) }
    if let agent = c.agent {
      let current = agent.options.first { $0.id == agent.value } ?? agent.options.first
      out.append(GooBean(key: "agent", title: agent.name,
                         glyph: AnyView(Icon(current.map(modeIcon) ?? .circleDashed, size: Metrics.barIcon, strokeWidth: 1.75)
                           .foregroundStyle(Palette.fg1)),
                         caption: AnyView(GooCaption(text: current?.name ?? agent.name)), rows: agent.options.count,
                         panel: AnyView(GooList(current: current?.id) {
                           ForEach(agent.options) { o in
                             GooRow(name: o.name, checked: o.id == current?.id) {
                               Icon(modeIcon(o), size: Metrics.icon, strokeWidth: 1.75)
                             } action: {
                               onSetConfig(agent.id, o.id)
                               fold()
                             }
                             .id(o.id)
                           }
                         })))
    }
    if c.fusion != nil {
      let picked = SessionOption(id: c.curVar?.sidekick ?? "", name: c.curVar?.sidekick ?? "")
      out.append(GooBean(key: "sidekick", title: t("composer.sidekick"),
                         glyph: AnyView(SidekickMark(picked: picked).foregroundStyle(Palette.fg1)),
                         caption: AnyView(GooCaption(text: picked.name, note: t("composer.sidekick"))),
                         tap: { enter(Self.sidekickLevel) }))
    } else if let sk = c.sidekick { out.append(sidekickBean(sk, picked: c.sidekickCur, fast: c.sidekickFast)) }
    return out
  }

  /// Fusion uses the built-in sidekick's bean navigation, but every pick still writes the real pair's model id.
  /// Unavailable sidekicks stay in the list, disabled just as in the desktop's FusionParams.
  private func fusionModelBean(_ c: ModelChoices, sidekick: Bool) -> GooBean {
    let names = (sidekick ? c.fusion?.sidekicks : c.fusion?.leads) ?? []
    let current = (sidekick ? c.curVar?.sidekick : c.curVar?.lead) ?? names.first ?? ""
    let picked = SessionOption(id: current, name: current)
    let title = t(sidekick ? "composer.sidekick" : "composer.lead")
    let glyph = sidekick
      ? AnyView(SidekickMark(picked: picked).foregroundStyle(Palette.fg1))
      : AnyView(VendorMark(brand: ModelCatalog.optionBrand(picked), name: current, size: Metrics.barIcon).foregroundStyle(Palette.fg1))
    return GooBean(key: sidekick ? "sidekick.model" : "lead", title: title, glyph: glyph,
                   caption: AnyView(GooCaption(text: current, note: title)), rows: names.count,
                   panel: AnyView(GooList(current: current) {
                     ForEach(names, id: \.self) { name in
                       let available = !sidekick || c.offersSidekick(name)
                       GooRow(name: name, checked: name == current) {
                         VendorMark(brand: VendorBrand.of(name), name: name, size: Metrics.iconCtl)
                       } action: {
                         if sidekick { c.pickFusion(sidekick: name) } else { c.pickFusion(lead: name) }
                         fold()
                       }
                       .disabled(!available)
                       .opacity(available ? 1 : 0.4)
                       .id(name)
                     }
                   }))
  }

  /// The agent's modes, one row each, for an agent without a permission control of its own. Beside one (`accessBean`)
  /// there is no mode bean: the working modes come from the `/` list and show as the bar's pill (`ModePill`)
  private func modeBean(_ c: ModelChoices) -> GooBean {
    let modes = c.modes
    let mode = c.mode
    return GooBean(key: "mode", title: t("composer.mode"),
                   glyph: AnyView(Icon(mode.map(modeIcon) ?? .circleDashed, size: Metrics.barIcon, strokeWidth: 1.75)
                     .foregroundStyle(Palette.fg1)),
                   caption: AnyView(GooCaption(text: mode.map(modeLabel) ?? t("composer.mode"))),
                   rows: modes.count,
                   panel: AnyView(GooList(current: mode?.id) {
                     ForEach(modes) { m in
                       GooRow(name: modeLabel(m), checked: m.id == mode?.id) {
                         Icon(modeIcon(m), size: Metrics.icon, strokeWidth: 1.75)
                       } action: {
                         onSetMode(m.id)
                         fold()
                       }
                       .id(m.id)
                     }
                   }))
  }

  /// The approval level offered as a config option (the built-in agent's, Claude's): its own bean, the level in force
  /// beside it, and a panel with one row per level
  private func accessBean(_ access: ConfigControl) -> GooBean {
    let level = access.options.first { $0.id == access.value } ?? access.options.first
    return GooBean(key: "access", title: t("composer.access"),
                   glyph: AnyView(Icon(level.map(accessIcon) ?? .circleDashed, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)),
                   caption: AnyView(GooCaption(text: level.map(accessLabel) ?? access.name)), rows: access.options.count,
                   panel: AnyView(GooList(current: level?.id) {
                     ForEach(access.options) { o in
                       GooRow(name: accessLabel(o), checked: o.id == level?.id) {
                         Icon(accessIcon(o), size: Metrics.icon, strokeWidth: 1.75)
                       } action: {
                         onSetConfig(access.id, o.id)
                         fold()
                       }
                       .id(o.id)
                     }
                   }))
  }

  /// The built-in agent's sidekick, last so the lead's settings stay together: its mark with the plus badge (a faint
  /// fading plus for none), its model's name and what a sidekick does beside it. A tap trades the beans for the
  /// sidekick's own (`sidekickBeans`)
  private func sidekickBean(_ sk: ConfigControl, picked: ModelFamily?, fast: FastState?) -> GooBean {
    let pickedOpt = Self.picked(sk)
    // None yet, with models from two or more CLIs: the tap goes straight to the CLI list, not to a lone agent bean
    let toAgents = pickedOpt == nil && ComposerControls.sidekickClis(sk.options).count > 1
    return GooBean(key: "sidekick", title: t("composer.sidekick"),
                   glyph: AnyView(SidekickMark(picked: pickedOpt).foregroundStyle(Palette.fg1)),
                   caption: AnyView(GooCaption(text: picked?.name ?? pickedOpt?.name ?? t("composer.sidekickNone"), note: t("composer.sidekickHint"))),
                   tap: { enter(Self.sidekickLevel, opening: toAgents ? 1 : 0) })
  }

  /// The sidekick's own beans, as the lead's read: its model (families grouped identically to the lead, with None first
  /// for a single CLI), its reasoning level while the model has levels, and its Fast while the model has a fast tier,
  /// flipped on a tap
  private func sidekickBeans(_ c: ModelChoices) -> [GooBean] {
    guard let sk = c.sidekick else { return [] }
    let current = sk.value ?? ComposerControls.noSidekick
    let picked = Self.picked(sk)
    let pickedFamily = c.sidekickCur
    let clis = ComposerControls.sidekickClis(sk.options)
    var out: [GooBean] = []
    // Models from two or more CLIs (the host's select): the CLI is a bean of its own, first, as the desktop's rail is,
    // and the model bean lists one CLI's families; a group label in it then always names a channel, never a CLI
    let activeCli: ComposerControls.SidekickCli? = if clis.count > 1 {
      clis.first { $0.id == browse } ?? clis.first { $0.id == picked?.group?.id }
    } else {
      clis.first
    }
    if clis.count > 1 {
      let mine = clis.first { $0.id == picked?.group?.id }
      out.append(agentBean(sk, clis: clis, picked: mine))
    }
    let families: [ModelFamily]? = if clis.count > 1 {
      activeCli.map { ComposerControls.sidekickFamilies($0.options, cliId: $0.id) }
    } else if let only = clis.first {
      ComposerControls.sidekickFamilies(only.options, cliId: only.id)
    } else {
      ComposerControls.sidekickFamilies(sk.options)
    }
    if let families {
      let showNone = clis.count <= 1
      let groups = ComposerControls.byChannelFamilies(families)
      let currentKey = pickedFamily?.key ?? ComposerControls.noSidekick
      let noneRow = GooRow(name: t("composer.sidekickNone"), checked: picked == nil) {
        SidekickRowGlyph(option: nil, size: Metrics.iconCtl)
      } action: {
        onSetConfig(sk.id, ComposerControls.noSidekick)
        fold()
      }
      .id(ComposerControls.noSidekick)
      let familyRow = { (f: ModelFamily) in
        let checked = f.variants.contains { $0.id == current }
        let target = f.variants.first?.id ?? f.key
        return GooRow(name: f.name, checked: checked) {
          VendorMark(brand: f.brand, name: f.name, size: Metrics.iconCtl)
        } action: {
          onSetConfig(sk.id, target)
          fold()
        }
        .id(f.key)
      }
      out.append(GooBean(key: "sidekick.model", title: t("stats.model"),
                         // The badged mark, so this column reads as the sidekick's and not the lead's
                         glyph: AnyView(SidekickMark(picked: picked).foregroundStyle(Palette.fg1)),
                         caption: AnyView(GooCaption(text: pickedFamily?.name ?? picked?.name ?? t("composer.sidekickNone"))),
                         rows: families.count + (showNone ? 1 : 0),
                         labels: groups?.filter { !$0.channel.isEmpty }.count ?? 0,
                         panel: AnyView(GooList(current: currentKey) {
                           if showNone { noneRow }
                           if let groups {
                             ForEach(groups, id: \.channel) { g in
                               if !g.channel.isEmpty { GooSection(title: g.channel) }
                               ForEach(g.families, id: \.key) { familyRow($0) }
                             }
                           } else {
                             ForEach(families, id: \.key) { familyRow($0) }
                           }
                         })))
    }
    if picked != nil, let e = c.sidekickEffortScale, e.levels.count > 1 || e.thinking != nil {
      let levels = e.levels.count > 1 ? e.levels : []
      let level = e.thinking?.on == false ? t("mobile.thinking.off")
        : levels.first { $0.0 == e.value }?.1 ?? (e.thinking != nil ? "Thinking" : t("composer.effort"))
      out.append(GooBean(key: "sidekick.effort", title: t("composer.effort"),
                         glyph: AnyView(Icon(.brain, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)),
                         caption: AnyView(GooCaption(text: level)), rows: levels.count + (e.thinking != nil ? 1 : 0),
                         panel: AnyView(GooList(current: e.thinking?.on == false ? "off" : e.value) {
                           if let thinking = e.thinking {
                             GooRow(name: t("mobile.thinking.off"), checked: !thinking.on) { EmptyView() } action: {
                               thinking.set(false)
                               fold()
                             }
                             .id("off")
                             if levels.isEmpty {
                               GooRow(name: "Thinking", checked: thinking.on) { EmptyView() } action: {
                                 thinking.set(true)
                                 fold()
                               }
                               .id("on")
                             }
                           }
                           ForEach(levels, id: \.0) { l in
                             GooRow(name: l.1, checked: e.thinking?.on != false && l.0 == e.value) { EmptyView() } action: {
                               e.pick(l.0)
                               fold()
                             }
                             .id(l.0)
                           }
                         })))
    }
    if picked != nil, let fast = c.sidekickFast {
      out.append(GooBean(key: "sidekick.fast", title: "Fast",
                         glyph: AnyView(FastGlyph(on: fast.on, size: Metrics.barIcon)),
                         caption: AnyView(GooCaption(text: "Fast")),
                         tap: { if !fast.disabled { Haptics.play(.pick); withAnimation(motion ? Motion.quick : nil) { fast.set(!fast.on) } } }))
    }
    return out
  }

  /// The CLI the sidekick runs in: its mark and name (a faint fading plus and None while there is none), and a panel of
  /// None and every CLI. None turns the sidekick off; a CLI is no pick of its own, so it opens the model bean on that
  /// CLI's models
  private func agentBean(_ sk: ConfigControl, clis: [ComposerControls.SidekickCli], picked: ComposerControls.SidekickCli?) -> GooBean {
    let none = ComposerControls.noSidekick
    let glyph = picked.map { AnyView(VendorMark(brand: $0.id, name: $0.name, size: Metrics.barIcon).foregroundStyle(Palette.fg1)) }
      ?? AnyView(Icon(.circleFadingPlus, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg3))
    return GooBean(key: "sidekick.agent", title: t("common.agent"), glyph: glyph,
                   caption: AnyView(GooCaption(text: picked?.name ?? t("composer.sidekickNone"))), rows: clis.count + 1,
                   panel: AnyView(GooList(current: picked?.id ?? none) {
                     GooRow(name: t("composer.sidekickNone"), checked: picked == nil) {
                       SidekickRowGlyph(option: nil, size: Metrics.iconCtl)
                     } action: {
                       onSetConfig(sk.id, none)
                       browse = nil
                       fold()
                     }
                     .id(none)
                     ForEach(clis) { cli in
                       GooRow(name: cli.name, checked: cli.id == picked?.id) {
                         VendorMark(brand: cli.id, name: cli.name, size: Metrics.iconCtl)
                       } action: {
                         // The agent bean is first, the model bean second
                         withAnimation(motion ? Motion.gooSpring : nil) {
                           browse = cli.id
                           stage = 2
                         }
                       }
                       .id(cli.id)
                     }
                   }))
  }

  /// The sidekick picked, none while the select reads None
  private static func picked(_ sk: ConfigControl) -> SessionOption? {
    let current = sk.value ?? ComposerControls.noSidekick
    return sk.options.first { $0.id == current && $0.id != ComposerControls.noSidekick }
  }

  /// Trades the beans for another level's (`sidekick`, or nil for the lead's): they head home into the chip, the level
  /// changes while they are small there, and the new beans squeeze out, `opening` that bean's panel at once (1…).
  /// Without motion it swaps at once
  private func enter(_ level: String?, opening: Int = 0) {
    browse = nil
    guard motion else {
      stage = opening
      deeper = level
      return
    }
    withAnimation(Motion.gooSpring) {
      held = true
      stage = 0
    }
    Task { @MainActor in
      try? await Task.sleep(for: .seconds(Motion.gooLevelSwap))
      // Closed meanwhile (a tap outside): the level stays as it was, and the next open starts on the lead's beans
      guard open == "model" else { held = false; return }
      deeper = level
      withAnimation(Motion.gooSpring) {
        held = false
        stage = opening
      }
    }
  }

  /// A pick folds the panel back into its bean; the beans stay out for the next change
  private func fold() { withAnimation(motion ? Motion.gooSpring : nil) { stage = 0 } }
}
