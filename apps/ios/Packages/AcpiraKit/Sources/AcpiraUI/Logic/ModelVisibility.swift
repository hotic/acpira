import AcpiraModels
import Foundation

// shared/models.ts (familyHidden, setFamilyVisible, visibleOptions) and shared/modelCatalog.ts: which model families an
// agent's menus offer. The hidden lists live in the engine's settings (`hiddenOptions`, agent → control id → family
// keys), shared with the desktop, so the keys and the legacy forms are the TS ones.

extension ModelCatalog {
  /// Preferences stored before the family had a source: its bare name, or the `Provider/Name` Pi and OpenCode used to show
  static func legacyKeys(_ f: ModelFamily) -> [String] { f.source.map { [f.name, "\($0)/\(f.name)"] } ?? [f.name] }

  static func hidden(_ f: ModelFamily, in hidden: [String]) -> Bool {
    hidden.contains(f.key) || legacyKeys(f).contains(where: hidden.contains)
  }

  /// One family switched; legacy name-only entries become the family's key first, so its siblings keep their state
  static func setVisible(_ options: [SessionOption], hidden: [String], key: String, show: Bool) -> [String] {
    var next = hidden
    for f in group(options) {
      for k in legacyKeys(f) where hidden.contains(k) {
        next.removeAll { $0 == k }
        if !next.contains(f.key) { next.append(f.key) }
      }
    }
    next.removeAll { $0 == key }
    if !show { next.append(key) }
    return next
  }

  /// The options a menu offers: hidden families dropped, the current value always kept, and a list that would hide
  /// everything shows everything
  static func visibleOptions(_ options: [SessionOption], hidden: [String]?, current: String?) -> [SessionOption] {
    guard let hidden, !hidden.isEmpty else { return options }
    let off = Set(group(options).filter { Self.hidden($0, in: hidden) }.flatMap { $0.variants.map(\.id) })
    let kept = options.filter { $0.id == current || !off.contains($0.id) }
    return kept.isEmpty ? options : kept
  }

  /// Every listed family switched at once, rows a search or the preview limit leaves out included
  static func setAllVisible(_ families: [ModelFamily], hidden: [String], visible: Bool) -> [String] {
    var next = hidden
    for f in families {
      next.removeAll { $0 == f.name || $0 == f.key }
      if !visible { next.append(f.key) }
    }
    return next
  }

  static func filter(_ families: [ModelFamily], query: String) -> [ModelFamily] {
    let terms = query.lowercased().split(whereSeparator: \.isWhitespace).map(String.init)
    guard !terms.isEmpty else { return families }
    return families.filter { f in
      let text = ([f.name, f.source ?? ""] + f.variants.flatMap { [$0.id, $0.name] }).joined(separator: " ").lowercased()
      return terms.allSatisfy(text.contains)
    }
  }

  /// Each series' newest release first, then the older ones; unrelated series keep the agent's order
  static func prioritize(_ families: [ModelFamily]) -> [ModelFamily] {
    var order: [String] = []
    var buckets: [String: [(family: ModelFamily, version: [Int], at: Int)]] = [:]
    for (at, f) in families.enumerated() {
      let (series, version) = release(f)
      if buckets[series] == nil { order.append(series) }
      buckets[series, default: []].append((f, version, at))
    }
    let sorted = order.map { series in
      buckets[series]!.sorted { l, r in
        for i in 0..<max(l.version.count, r.version.count) {
          let d = (r.version[safe: i] ?? 0) - (l.version[safe: i] ?? 0)
          if d != 0 { return d < 0 }
        }
        return l.at < r.at
      }
    }
    return sorted.compactMap(\.first?.family) + sorted.flatMap { $0.dropFirst().map(\.family) }
  }

  /// A family's series (its name with the version cut out, per source) and version; a date or a size suffix is no
  /// version, so such a family is a series of its own
  private static func release(_ f: ModelFamily) -> (String, [Int]) {
    let own = (f.key, [Int]())
    guard let match = f.name.firstMatch(of: /\d+(?:[.-]\d+)*/) else { return own }
    let number = String(match.output)
    let rest = f.name[match.range.upperBound...]
    if number.wholeMatch(of: /\d{6,}/) != nil || rest.prefixMatch(of: /(?i)[bkmt]\b/) != nil { return own }
    let series = (f.name[..<match.range.lowerBound] + " " + rest).lowercased()
      .replacing(/[\s._-]+/, with: " ").trimmingCharacters(in: .whitespaces)
    return (jsonKey([f.sourceKind, f.source, series]), number.split(whereSeparator: { $0 == "." || $0 == "-" }).compactMap { Int($0) })
  }
}
