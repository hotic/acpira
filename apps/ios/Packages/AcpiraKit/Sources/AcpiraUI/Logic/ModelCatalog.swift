import AcpiraDesign
import AcpiraModels
import Foundation

// shared/models.ts + shared/composerControls.ts: the model list rebuilt from flat option names (Devin flattens
// model × effort × Fast × 1M into one select), the native reasoning / Fast controls, and the chip's badges. The wire
// ids stay untouched; only the presentation is derived.

/// One option of a family: the wire id plus the dimensions parsed from its name
struct ModelVariant: Hashable, Sendable {
  var id: String
  var name: String
  /// None / Minimal / Low / Medium / High / XHigh / Max / Thinking; "" is the level-less (Standard) variant
  var effort: String
  var fast: Bool
  /// The 1M context variant
  var long: Bool
  /// Fusion only: the lead family (effort / fast describe it) and the sidekick label ("SWE-2 High")
  var lead: String?
  var sidekick: String?
}

/// A model family: one row of the panel's list, its variants the params underneath
struct ModelFamily: Hashable, Sendable {
  /// Namespaced identity, so equal display names from different providers stay apart
  var key: String
  var name: String
  var brand: String?
  /// The blurb every option of the family shares, if any
  var description: String?
  var source: String?
  var variants: [ModelVariant]
  /// Effort labels by strength, deduplicated
  var efforts: [String] = []
  var hasFast = false
  var hasLong = false
  /// Fusion only: lead families and sidekick labels in first-seen order
  var fusion: (leads: [String], sidekicks: [String])?

  static func == (a: ModelFamily, b: ModelFamily) -> Bool { a.key == b.key && a.variants == b.variants }
  func hash(into hasher: inout Hasher) { hasher.combine(key) }
}

enum ModelCatalog {
  static let fusion = "Fusion"
  private static let levels: [String: String] = [
    "none": "None", "minimal": "Minimal", "low": "Low", "medium": "Medium", "high": "High", "xhigh": "XHigh", "x-high": "XHigh", "max": "Max",
  ]
  private static let effortOrder = ["", "Thinking", "None", "Minimal", "Low", "Medium", "High", "XHigh", "Max"]
  static func rank(_ effort: String) -> Int { effortOrder.firstIndex(of: effort) ?? effortOrder.count }

  struct Parsed: Equatable {
    var family: String
    var effort: String
    var fast: Bool
    var long: Bool
    var lead: String?
    var sidekick: String?
  }

  /// "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)": lead GPT-6 Astra at High, sidekick
  /// "GPT-5.6 Luna High", Fast from either side (one pair-level switch on the wire)
  static func parseFusion(_ name: String) -> Parsed? {
    let trimmed = name.trimmingCharacters(in: .whitespaces)
    guard let m = trimmed.firstMatch(of: /^(?i)Fusion\s*\((.+?)\s+\+\s+(.+)\)$/) else { return nil }
    let lead = parse(String(m.1)), sk = parse(String(m.2))
    return Parsed(family: fusion, effort: lead.effort, fast: lead.fast || sk.fast, long: lead.long, lead: lead.family,
                  sidekick: [sk.family, sk.effort].filter { !$0.isEmpty }.joined(separator: " "))
  }

  /// `<family> [effort] [Thinking] [Fast] [1M]`
  static func parse(_ name: String) -> Parsed {
    if let fusion = parseFusion(name) { return fusion }
    var t = name.trimmingCharacters(in: .whitespaces).split(whereSeparator: \.isWhitespace).map(String.init)
    var fast = false, long = false, thinking = false, effort = ""
    // Fast and 1M may both trail, in either order
    while t.count > 1 {
      let last = t[t.count - 1].lowercased()
      if last == "fast" { fast = true; t.removeLast() } else if last == "1m" { long = true; t.removeLast() } else { break }
    }
    if t.count > 1, t[t.count - 1].lowercased() == "thinking" { thinking = true; t.removeLast() }
    let last = t.last?.lowercased() ?? ""
    if t.count > 1, let level = levels[last] {
      effort = level; t.removeLast()
    } else if t.count > 1, last == "no", thinking {
      effort = "None"; thinking = false; t.removeLast()
    }
    // "Claude Opus 4.6 Thinking" with no effort word: Thinking is the level
    if effort.isEmpty && thinking { effort = "Thinking" }
    return Parsed(family: t.joined(separator: " "), effort: effort, fast: fast, long: long)
  }

  static func group(_ options: [SessionOption]) -> [ModelFamily] {
    struct Base: Hashable { var namespace: String?; var family: String }
    struct Tuple: Hashable { var base: Base; var effort: String; var fast: Bool; var long: Bool; var lead: String?; var sidekick: String? }
    // With no provider metadata, equal parameter tuples are ambiguous, not interchangeable
    var tuples = Set<Tuple>(), ambiguous = Set<Base>()
    for o in options {
      let p = parse(o.name)
      let base = Base(namespace: o.source?.id ?? o.group?.id, family: p.family)
      let tuple = Tuple(base: base, effort: p.effort, fast: p.fast, long: p.long, lead: p.lead, sidekick: p.sidekick)
      if tuples.contains(tuple) { ambiguous.insert(base) }
      tuples.insert(tuple)
    }
    var order: [String] = []
    var map: [String: ModelFamily] = [:]
    for o in options {
      let p = parse(o.name)
      let namespace = o.source?.id ?? o.group?.id
      let separate = ambiguous.contains(Base(namespace: namespace, family: p.family))
      let source = o.source?.name ?? o.group?.name ?? (separate ? o.id : nil)
      let key = separate ? "\(namespace ?? "")\u{1}\(p.family)\u{1}\(o.id)" : namespace.map { "\($0)\u{1}\(p.family)" } ?? p.family
      var f = map[key] ?? {
        order.append(key)
        return ModelFamily(key: key, name: p.family, description: o.description, source: source, variants: [])
      }()
      // Fusion pairs are Devin's own routing, like Adaptive
      if f.brand == nil { f.brand = p.lead != nil ? "devin" : optionBrand(o) }
      if f.description != o.description { f.description = nil }
      if let lead = p.lead {
        var fu = f.fusion ?? ([], [])
        if !fu.leads.contains(lead) { fu.leads.append(lead) }
        if let sk = p.sidekick, !fu.sidekicks.contains(sk) { fu.sidekicks.append(sk) }
        f.fusion = fu
      }
      f.variants.append(ModelVariant(id: o.id, name: o.name, effort: p.effort, fast: p.fast, long: p.long, lead: p.lead, sidekick: p.lead != nil ? p.sidekick : nil))
      map[key] = f
    }
    return order.map { key in
      var f = map[key]!
      // The ranks read a copy: the sort mutates the family's variants
      let fusion = f.fusion
      let leadRank = { (v: ModelVariant) in fusion?.leads.firstIndex(of: v.lead ?? "") ?? 0 }
      let skRank = { (v: ModelVariant) in fusion?.sidekicks.firstIndex(of: v.sidekick ?? "") ?? 0 }
      f.variants.sort { a, b in
        let keys = [leadRank(a) - leadRank(b), rank(a.effort) - rank(b.effort), skRank(a) - skRank(b),
                    (a.fast ? 1 : 0) - (b.fast ? 1 : 0), (a.long ? 1 : 0) - (b.long ? 1 : 0)]
        return (keys.first { $0 != 0 } ?? 0) < 0
      }
      var seen = Set<String>()
      f.efforts = f.variants.map(\.effort).filter { seen.insert($0).inserted }.sorted { rank($0) < rank($1) }
      f.hasFast = f.variants.contains(where: \.fast)
      f.hasLong = f.variants.contains(where: \.long)
      return f
    }
  }

  /// Exact effort / Fast / 1M first, then dropping 1M, Fast, both; finally any variant at that effort
  static func findVariant(_ f: ModelFamily, effort: String, fast: Bool, long: Bool) -> ModelVariant? {
    func hit(_ fa: Bool, _ lo: Bool) -> ModelVariant? { f.variants.first { $0.effort == effort && $0.fast == fa && $0.long == lo } }
    return hit(fast, long) ?? hit(fast, false) ?? hit(false, long) ?? hit(false, false) ?? f.variants.first { $0.effort == effort }
  }

  /// The offered Fusion pair nearest to lead × effort × sidekick × fast
  static func findFusionVariant(_ f: ModelFamily, lead: String, effort: String, sidekick: String?, fast: Bool) -> ModelVariant? {
    let ofLead = f.variants.filter { $0.lead == lead }
    let pool = ofLead.isEmpty ? f.variants : ofLead
    func hit(_ e: String, _ sk: String?, _ fa: Bool) -> ModelVariant? {
      pool.first { $0.effort == e && (sk == nil || $0.sidekick == sk) && $0.fast == fa }
    }
    func at(_ e: String) -> ModelVariant? { hit(e, sidekick, fast) ?? hit(e, sidekick, false) ?? hit(e, nil, fast) ?? hit(e, nil, false) }
    var seen = Set<String>()
    let nearest = pool.map(\.effort).filter { $0 != effort && seen.insert($0).inserted }.sorted {
      let da = abs(rank($0) - rank(effort)), db = abs(rank($1) - rank(effort))
      return da != db ? da < db : rank($0) > rank($1)
    }
    return at(effort) ?? nearest.lazy.compactMap(at).first ?? pool.first
  }

  /// "GPT-6 Astra High + SWE-2 High Fast"
  static func fusionLabel(_ v: ModelVariant) -> String {
    let lead = [v.lead ?? "", v.effort].filter { !$0.isEmpty }.joined(separator: " ")
    let sk = [v.sidekick ?? "", v.fast ? "Fast" : ""].filter { !$0.isEmpty }.joined(separator: " ")
    return [lead, sk].filter { !$0.isEmpty }.joined(separator: " + ")
  }

  /// The vendor of an option: its wire id is the real identity ("asgard/kimi-k3"), the name only a label
  static func optionBrand(_ o: SessionOption) -> String? { VendorBrand.of(o.id) ?? VendorBrand.of(o.name) }

  /// A Standard / Thinking pair is a Thinking switch rather than two segments
  static func thinkingOnly(_ f: ModelFamily) -> Bool { f.efforts.count == 2 && f.efforts.contains("") && f.efforts.contains("Thinking") }
  static func variantScale(_ f: ModelFamily) -> Bool { f.efforts.count > 1 && !thinkingOnly(f) }
  static func fusionEfforts(_ f: ModelFamily, _ v: ModelVariant) -> [String] {
    let lead = v.lead ?? f.fusion?.leads.first ?? ""
    return f.efforts.filter { e in f.variants.contains { $0.lead == lead && $0.effort == e } }
  }
}

// MARK: - Native controls

/// composerControls.ts: reasoning levels, Fast and the badges, whatever the wire shape
enum ComposerControls {
  private static let levels = ["None", "Minimal", "Low", "Medium", "High", "XHigh", "Max", "Ultra", "Thinking"]

  private static func compact(_ value: String) -> String {
    value.lowercased().replacing(/(?:reasoning|thinking|effort|level)/, with: "").replacing(/[\s_-]/, with: "")
  }

  /// Presentation only: Low / Medium / High / XHigh / Max / Ultra; the wire ids stay
  static func effortLabel(_ option: SessionOption) -> String {
    func canonical(_ value: String) -> String? {
      let key = compact(value)
      return key == "extrahigh" ? "XHigh" : levels.first { $0.lowercased() == key }
    }
    return canonical(option.id) ?? canonical(option.name) ?? option.name
  }

  static func effortOptions(_ options: [SessionOption]) -> [SessionOption] {
    func rank(_ name: String) -> Int { levels.firstIndex(of: name) ?? levels.count }
    return options.map { var o = $0; o.name = effortLabel($0); return o }
      .enumerated().sorted { rank($0.element.name) != rank($1.element.name) ? rank($0.element.name) < rank($1.element.name) : $0.offset < $1.offset }
      .map(\.element)
  }

  static func isUltra(_ label: String?) -> Bool { label == "Ultra" }

  static func isReasoning(_ c: ConfigControl) -> Bool {
    c.category == "thought_level" || (c.category == nil && c.id.wholeMatch(of: /reasoning_effort|thought_level|thinking|thinking_level/) != nil)
  }

  /// Kimi folds thinking on / off into the effort select
  private static func toggleSide(_ o: SessionOption) -> String? {
    let key = compact(o.id).isEmpty ? compact(o.name) : compact(o.id)
    return key == "on" || key == "off" ? key : nil
  }

  /// Kimi keeps the previous model's leftover value: drop the stranger, keep the native set
  private static func nativeThoughtOptions(_ options: [SessionOption]) -> [SessionOption] {
    let toggles = options.filter { toggleSide($0) != nil }
    let efforts = options.filter { toggleSide($0) == nil }
    let ons = toggles.filter { toggleSide($0) == "on" }
    if toggles.contains(where: { toggleSide($0) == "off" }) { return options }
    if !ons.isEmpty && efforts.count > 1 { return efforts }
    if !ons.isEmpty && efforts.count == 1 { return ons }
    return options
  }

  struct Reasoning {
    var efforts: [SessionOption]
    var offId: String?
    var onId: String?
    var value: String?
    var off: Bool
  }

  static func presentReasoning(_ c: ConfigControl) -> Reasoning {
    let native = nativeThoughtOptions(c.options)
    let efforts = effortOptions(native.filter { toggleSide($0) == nil })
    let offId = native.first { toggleSide($0) == "off" }?.id
    let onId = native.first { toggleSide($0) == "on" }?.id
    let value = native.contains { $0.id == c.value } ? c.value : thoughtValue(native)
    return Reasoning(efforts: efforts, offId: offId, onId: onId, value: value, off: offId != nil && value == offId)
  }

  private static func thoughtValue(_ native: [SessionOption]) -> String? {
    let efforts = native.filter { toggleSide($0) == nil }
    return efforts.first { $0.id == "high" }?.id ?? efforts.first?.id ?? native.first { toggleSide($0) == "on" }?.id ?? native.first?.id
  }

  static func reasoningVisible(_ c: ConfigControl) -> Bool {
    let p = presentReasoning(c)
    return p.efforts.count > 1 || p.offId != nil
  }

  static func reasoningChip(_ c: ConfigControl) -> String? {
    let p = presentReasoning(c)
    return p.off ? nil : p.efforts.first { $0.id == p.value }?.name
  }

  static func isFast(_ c: ConfigControl) -> Bool {
    if c.type == "boolean" {
      return c.id.firstMatch(of: /(^|[\s_-])(?i)fast($|[\s_-])/) != nil || c.name.wholeMatch(of: /(?i)fast( mode)?/) != nil
    }
    return c.options.count == 2 && ["standard", "fast"].allSatisfy { id in c.options.contains { $0.id == id } }
  }

  static func fastOn(_ c: ConfigControl) -> Bool { c.value == (c.type == "boolean" ? "true" : "fast") }
  static func fastValue(_ c: ConfigControl, on: Bool) -> String { c.type == "boolean" ? String(on) : on ? "fast" : "standard" }

  static func modelConfigChip(_ c: ConfigControl) -> String? {
    if isFast(c) { return fastOn(c) ? "Fast" : nil }
    if c.type == "boolean" { return c.value == "true" ? c.name : nil }
    return c.options.first { $0.id == c.value }?.name
  }

  struct Tag: Hashable { var label: String; var ultra = false; var fast = false }

  /// The model chip's badges in the order they give way: effort (or Ultra), Fast, 1M, then the rest
  static func chipTags(_ cur: ModelFamily?, _ v: ModelVariant?, reasoning: [ConfigControl], modelConfig: [ConfigControl], standard: String) -> [Tag] {
    var tags: [Tag] = []
    if let cur, let v, v.lead != nil || cur.efforts.count > 1 || !v.effort.isEmpty || v.fast || v.long {
      let effort = !v.effort.isEmpty ? v.effort : (v.lead == nil && cur.efforts.count > 1 ? standard : "")
      if !effort.isEmpty { tags.append(Tag(label: effort)) }
      if v.fast { tags.append(Tag(label: "Fast", fast: true)) }
      if v.long { tags.append(Tag(label: "1M")) }
    }
    for c in reasoning { if let level = reasoningChip(c) { tags.append(Tag(label: level, ultra: isUltra(level))) } }
    for c in modelConfig { if let label = modelConfigChip(c) { tags.append(isFast(c) ? Tag(label: "Fast", fast: true) : Tag(label: label)) } }
    return tags
  }

  /// The built-in agent's sidekick select (custom category) and its level, the model panel's second side
  static let sidekickCategory = "_sidekick", sidekickEffortCategory = "_sidekick_effort"
  /// The sidekick's value for none
  static let noSidekick = "none"

  /// The toolbar's split: model selectors, reasoning, model parameters, working modes, the sidekick, the rest
  struct Split {
    var models: [ConfigControl] = []; var reasoning: [ConfigControl] = []; var modelConfig: [ConfigControl] = []; var collaboration: [ConfigControl] = []
    var sidekick: [ConfigControl] = []; var sidekickEffort: [ConfigControl] = []; var other: [ConfigControl] = []
  }

  static func split(_ options: [ConfigControl]) -> Split {
    var s = Split()
    for c in options {
      if isReasoning(c) { s.reasoning.append(c) }
      else if c.category == "collaboration_mode" && c.type != "boolean" { s.collaboration.append(c) }
      else if c.category == sidekickCategory && c.type != "boolean" { s.sidekick.append(c) }
      else if c.category == sidekickEffortCategory && c.type != "boolean" { s.sidekickEffort.append(c) }
      else if c.category == "model_config" { s.modelConfig.append(c) }
      else if c.type != "boolean" && (c.category == "model" || (c.category == nil && (c.id == "model" || ModelCatalog.group(c.options).count < c.options.count))) { s.models.append(c) }
      else { s.other.append(c) }
    }
    return s
  }
}
