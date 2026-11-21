import AcpiraDesign
import AcpiraModels
import Foundation

// Mirror of packages/ui/usage/summary.ts and formatting helpers from packages/ui/usage/UsagePage.tsx: folds the
// engine's hour-bucketed rows into local days and hours, summary figures, trend columns and model / agent / project
// breakdowns. Pure functions taking an explicit Calendar and locale so unit tests stay deterministic.

enum UsagePeriod: String, CaseIterable, Hashable, Sendable {
  case days7 = "7"
  case days30 = "30"
  case all = "all"

  var label: String {
    switch self {
    case .days7: t("usagePage.period.days", ["n": 7])
    case .days30: t("usagePage.period.days", ["n": 30])
    case .all: t("usagePage.period.all")
    }
  }
}

enum UsageDim: String, CaseIterable, Hashable, Sendable {
  case model
  case agent
  case project

  var label: String {
    switch self {
    case .model: t("usagePage.dim.model")
    case .agent: t("usagePage.dim.agent")
    case .project: t("usagePage.dim.project")
    }
  }

  var moreKey: String { "usagePage.more.\(rawValue)" }
}

enum UsageStep: Equatable, Sendable {
  case hours
  case day
  case week
  case month
}

struct UsageModelLabel: Hashable, Sendable {
  var name: String
  var brand: String?
}

struct UsageModelLabels: Sendable {
  var keys: [String]
  var label: [String: UsageModelLabel]
  var projects: [String]
}

struct UsageDay: Sendable {
  var day: Int64
  var tokens: Int64
  var byModel: [String: Int64]
  var byAgent: [String: Int64]
  var byProject: [String: Int64]

  func tokens(for dim: UsageDim, key: String) -> Int64 {
    switch dim {
    case .model: byModel[key] ?? 0
    case .agent: byAgent[key] ?? 0
    case .project: byProject[key] ?? 0
    }
  }
}

struct UsagePart: Identifiable, Hashable, Sendable {
  var key: String
  var tokens: Int64
  var share: Double
  var id: String { key }
}

struct UsageSummary: Sendable {
  var tokens: Int64 = 0
  var workMs: Int64 = 0
  var longestMs: Int64 = 0
  var prompts: Int = 0
  var turns: Int = 0
  var tools: Int = 0
  var added: Int = 0
  var removed: Int = 0
  var sessions: Int = 0
  var modelParts: [UsagePart] = []
  var agentParts: [UsagePart] = []
  var projectParts: [UsagePart] = []

  func parts(for dim: UsageDim) -> [UsagePart] {
    switch dim {
    case .model: modelParts
    case .agent: agentParts
    case .project: projectParts
    }
  }
}

struct UsageTrendSpan: Equatable, Sendable {
  var start: Int64
  var end: Int64
}

struct UsageTrendColumns: Equatable, Sendable {
  var step: UsageStep
  var cols: [UsageTrendSpan]
}

private enum UsagePatterns {
  nonisolated(unsafe) static let effortSuffix = /[\s._-]+(minimal|low|medium|high|xhigh|max)$/
  nonisolated(unsafe) static let separators = /[\s._-]+/
  nonisolated(unsafe) static let leadingVendor = /^(claude|gpt|kimi)/
  nonisolated(unsafe) static let hanThenAscii = /([\u{4e00}-\u{9fff}])(?=[0-9A-Za-z])/
  nonisolated(unsafe) static let asciiThenHan = /([0-9A-Za-z])(?=[\u{4e00}-\u{9fff}])/
}

enum UsageStats {
  /// Sentinel project key for chat sessions directly under `chatRoot` (summary.ts CHATS)
  static let chatsKey = "\u{0000}chats"
  static let trendSpanHours = 4
  static let trendDaysMax = 31
  static let trendWeeksMax = 26
  static let dotSteps = 4
  static let keepRows = 10
  static let hourMs: Int64 = 3_600_000
  static let dayMs: Int64 = 86_400_000

  // MARK: - Calendar slots

  static func dayOf(_ ms: Int64, calendar: Calendar = .current) -> Int64 {
    let date = Date(timeIntervalSince1970: Double(ms) / 1000)
    return Int64((calendar.startOfDay(for: date).timeIntervalSince1970 * 1000).rounded())
  }

  static func addDays(_ day: Int64, _ n: Int, calendar: Calendar = .current) -> Int64 {
    let date = Date(timeIntervalSince1970: Double(day) / 1000)
    let comps = calendar.dateComponents([.year, .month, .day], from: date)
    let next = calendar.date(from: DateComponents(year: comps.year, month: comps.month, day: (comps.day ?? 1) + n)) ?? date
    return Int64((next.timeIntervalSince1970 * 1000).rounded())
  }

  static func hourOf(_ ms: Int64, calendar: Calendar = .current) -> Int64 {
    let date = Date(timeIntervalSince1970: Double(ms) / 1000)
    let comps = calendar.dateComponents([.year, .month, .day, .hour], from: date)
    let start = calendar.date(from: comps) ?? date
    return Int64((start.timeIntervalSince1970 * 1000).rounded())
  }

  static func addHours(_ hour: Int64, _ n: Int, calendar: Calendar = .current) -> Int64 {
    let date = Date(timeIntervalSince1970: Double(hour) / 1000)
    let comps = calendar.dateComponents([.year, .month, .day, .hour], from: date)
    let next = calendar.date(from: DateComponents(year: comps.year, month: comps.month, day: comps.day, hour: (comps.hour ?? 0) + n)) ?? date
    return Int64((next.timeIntervalSince1970 * 1000).rounded())
  }

  /// Monday = 0 … Sunday = 6 (matching `(getDay() + 6) % 7` in summary.ts)
  static func mondayOffset(_ day: Int64, calendar: Calendar = .current) -> Int {
    let weekday = calendar.component(.weekday, from: Date(timeIntervalSince1970: Double(day) / 1000))
    return (weekday + 5) % 7
  }

  // MARK: - Model and project grouping

  /// Folds agent spelling differences ("Opus 5.5", "Claude Opus 5.5", "claude-opus-5-5-medium") under one key
  static func modelKey(_ name: String) -> String {
    let afterSlash = name.lastIndex(of: "/").map { String(name[name.index(after: $0)...]) } ?? name
    var bare = afterSlash.lowercased()
    bare.replace(UsagePatterns.effortSuffix, with: "")
    bare.replace(UsagePatterns.separators, with: "")
    var stripped = bare
    stripped.replace(UsagePatterns.leadingVendor, with: "")
    return stripped.isEmpty ? bare : stripped
  }

  private static func nameRank(_ name: String) -> Int {
    let hasSpace = name.contains(where: \.isWhitespace)
    let hasUpper = name.contains(where: { $0.isASCII && $0.isUppercase })
    return (hasSpace && hasUpper ? 2 : 0) + (name.contains("/") ? 0 : 1)
  }

  /// True when `cwd` sits directly inside `chatRoot` (either separator), matching `sessionTree.ts isChatFolder`
  static func isChatFolder(_ chatRoot: String?, _ cwd: String) -> Bool {
    guard let root = chatRoot, !root.isEmpty, let cut = cwd.lastIndex(where: { $0 == "/" || $0 == "\\" }) else { return false }
    let parent = String(cwd[..<cut])
    let clean: (String) -> String = { $0.replacingOccurrences(of: "\\", with: "/").replacingOccurrences(of: "/+$", with: "", options: .regularExpression) }
    return clean(parent) == clean(root)
  }

  static func modelLabels(_ view: UsageView, chats: String?) -> UsageModelLabels {
    let projects = view.sessions.map { isChatFolder(chats, $0.cwd) ? chatsKey : $0.cwd }
    let keys = view.models.map { modelKey($0.name) }
    var label: [String: UsageModelLabel] = [:]
    for (i, m) in view.models.enumerated() {
      let k = keys[i]
      if let cur = label[k] {
        let curRank = nameRank(cur.name), nextRank = nameRank(m.name)
        if curRank > nextRank || (curRank == nextRank && cur.name.count >= m.name.count) { continue }
      }
      let shown = m.name.lastIndex(of: "/").map { String(m.name[m.name.index(after: $0)...]) } ?? m.name
      label[k] = UsageModelLabel(name: shown, brand: VendorBrand.of(m.id) ?? VendorBrand.of(m.name))
    }
    return UsageModelLabels(keys: keys, label: label, projects: projects)
  }

  static func keyOf(_ view: UsageView, _ models: UsageModelLabels, _ row: UsageRow, _ dim: UsageDim) -> String {
    switch dim {
    case .model:
      guard let idx = row.model, idx >= 0, idx < models.keys.count else { return "" }
      return models.keys[idx]
    case .agent:
      guard row.session >= 0, row.session < view.sessions.count else { return "" }
      return view.sessions[row.session].agent
    case .project:
      guard row.session >= 0, row.session < models.projects.count else { return "" }
      return models.projects[row.session]
    }
  }

  // MARK: - Folding and summaries

  static func daysOf(_ view: UsageView, _ models: UsageModelLabels, calendar: Calendar = .current) -> [Int64: UsageDay] {
    foldBy(view, models) { dayOf($0, calendar: calendar) }
  }

  static func hoursOf(_ view: UsageView, _ models: UsageModelLabels, calendar: Calendar = .current) -> [Int64: UsageDay] {
    foldBy(view, models) { hourOf($0, calendar: calendar) }
  }

  private static func foldBy(_ view: UsageView, _ models: UsageModelLabels, slot: (Int64) -> Int64) -> [Int64: UsageDay] {
    var out: [Int64: UsageDay] = [:]
    for r in view.rows {
      let bucket = slot(r.at)
      var entry = out[bucket] ?? UsageDay(day: bucket, tokens: 0, byModel: [:], byAgent: [:], byProject: [:])
      let tokens = r.tokens ?? 0
      entry.tokens += tokens
      entry.byModel[keyOf(view, models, r, .model), default: 0] += tokens
      entry.byAgent[keyOf(view, models, r, .agent), default: 0] += tokens
      entry.byProject[keyOf(view, models, r, .project), default: 0] += tokens
      out[bucket] = entry
    }
    return out
  }

  static func periodStart(_ period: UsagePeriod, today: Int64, calendar: Calendar = .current) -> Int64 {
    switch period {
    case .all: 0
    case .days7: addDays(today, -6, calendar: calendar)
    case .days30: addDays(today, -29, calendar: calendar)
    }
  }

  static func summarize(_ view: UsageView, _ models: UsageModelLabels, from: Int64) -> UsageSummary {
    var s = UsageSummary()
    var byModel: [String: Int64] = [:]
    var byAgent: [String: Int64] = [:]
    var byProject: [String: Int64] = [:]
    var sessions = Set<Int>()
    for r in view.rows {
      if r.at + hourMs <= from { continue }
      let tokens = r.tokens ?? 0
      s.tokens += tokens
      s.workMs += r.workMs ?? 0
      s.longestMs = max(s.longestMs, r.longestMs ?? 0)
      s.prompts += r.prompts ?? 0
      s.turns += r.turns ?? 0
      s.tools += r.tools ?? 0
      s.added += r.added ?? 0
      s.removed += r.removed ?? 0
      sessions.insert(r.session)
      byModel[keyOf(view, models, r, .model), default: 0] += tokens
      byAgent[keyOf(view, models, r, .agent), default: 0] += tokens
      byProject[keyOf(view, models, r, .project), default: 0] += tokens
    }
    s.sessions = sessions.count
    let rank: ([String: Int64]) -> [UsagePart] = { totals in
      totals.compactMap { k, v -> UsagePart? in
        guard v > 0, !k.isEmpty else { return nil }
        return UsagePart(key: k, tokens: v, share: s.tokens > 0 ? Double(v) / Double(s.tokens) : 0)
      }
      .sorted { a, b in a.tokens != b.tokens ? a.tokens > b.tokens : a.key < b.key }
    }
    s.modelParts = rank(byModel)
    s.agentParts = rank(byAgent)
    s.projectParts = rank(byProject)
    return s
  }

  static func trendColumns(_ period: UsagePeriod, today: Int64, first: Int64, now: Int64,
                           calendar: Calendar = .current) -> UsageTrendColumns {
    func every(_ step: UsageStep, from: Int64, next: (Int64) -> Int64, until: Int64) -> UsageTrendColumns {
      var cols: [UsageTrendSpan] = []
      var d = from
      while d <= until {
        let end = next(d)
        cols.append(UsageTrendSpan(start: d, end: end))
        if end <= d { break }
        d = end
      }
      if step != .hours, !cols.isEmpty {
        cols[cols.count - 1].end = min(cols[cols.count - 1].end, addDays(today, 1, calendar: calendar))
      }
      return UsageTrendColumns(step: step, cols: cols)
    }

    switch period {
    case .days7:
      return every(.hours, from: periodStart(.days7, today: today, calendar: calendar),
                   next: { addHours($0, trendSpanHours, calendar: calendar) }, until: now)
    case .days30:
      return every(.day, from: periodStart(.days30, today: today, calendar: calendar),
                   next: { addDays($0, 1, calendar: calendar) }, until: today)
    case .all:
      let span = Int((Double(today - first) / Double(dayMs)).rounded()) + 1
      if span <= trendDaysMax {
        return every(.day, from: first, next: { addDays($0, 1, calendar: calendar) }, until: today)
      }
      if span <= trendWeeksMax * 7 {
        let weekStart = addDays(first, -mondayOffset(first, calendar: calendar), calendar: calendar)
        return every(.week, from: weekStart, next: { addDays($0, 7, calendar: calendar) }, until: today)
      }
      let firstDate = Date(timeIntervalSince1970: Double(first) / 1000)
      let fComps = calendar.dateComponents([.year, .month], from: firstDate)
      let monthStart = Int64(((calendar.date(from: DateComponents(year: fComps.year, month: fComps.month, day: 1)) ?? firstDate)
        .timeIntervalSince1970 * 1000).rounded())
      return every(.month, from: monthStart, next: { d in
        let date = Date(timeIntervalSince1970: Double(d) / 1000)
        let c = calendar.dateComponents([.year, .month], from: date)
        let m = calendar.date(from: DateComponents(year: c.year, month: (c.month ?? 1) + 1, day: 1)) ?? date
        return Int64((m.timeIntervalSince1970 * 1000).rounded())
      }, until: today)
    }
  }

  static func firstDay(_ days: [Int64: UsageDay], today: Int64) -> Int64 {
    var first = today
    for d in days.values where d.tokens > 0 && d.day < first { first = d.day }
    return first
  }

  static func currentRun(_ days: [Int64: UsageDay], today: Int64, calendar: Calendar = .current) -> Int {
    var day = (days[today]?.tokens ?? 0) > 0 ? today : addDays(today, -1, calendar: calendar)
    var n = 0
    while (days[day]?.tokens ?? 0) > 0 {
      n += 1
      day = addDays(day, -1, calendar: calendar)
    }
    return n
  }

  // MARK: - Formatting

  /// Plain up to 9,999, else locale compact notation with at most 3 significant digits split into `(value, unit)`
  static func figure(_ n: Int64, localeId: String = L10n.locale) -> (value: String, unit: String?) {
    let locale = Locale(identifier: localeId)
    if abs(n) < 10_000 {
      return (n.formatted(.number.locale(locale)), nil)
    }
    let formatted = Double(n).formatted(.number.notation(.compactName).precision(.significantDigits(1...3)).locale(locale))
    guard let lastDigit = formatted.lastIndex(where: { $0.isASCII && $0.isNumber }) else {
      return (formatted, nil)
    }
    let value = String(formatted[...lastDigit]).trimmingCharacters(in: .whitespacesAndNewlines)
    let rest = String(formatted[formatted.index(after: lastDigit)...]).trimmingCharacters(in: .whitespacesAndNewlines)
    return (value, rest.isEmpty ? nil : rest)
  }

  /// Inserts Pangu spacing between Han characters and ASCII digits/letters for Chinese locales
  static func spaced(_ text: String, localeId: String = L10n.locale) -> String {
    guard localeId.hasPrefix("zh") else { return text }
    var out = text
    out.replace(UsagePatterns.hanThenAscii) { "\($0.1) " }
    out.replace(UsagePatterns.asciiThenHan) { "\($0.1) " }
    return out
  }

  /// Minutes under an hour, else hours to one decimal (whole hours past 100)
  static func duration(_ ms: Int64, localeId: String = L10n.locale) -> (value: String, unit: String) {
    let minutes = Int((Double(ms) / 60_000.0).rounded())
    if minutes < 60 { return ("\(minutes)", t("usagePage.unit.minutes")) }
    let hours = Double(minutes) / 60.0
    let digits = hours >= 100 ? 0 : 1
    let value = hours.formatted(.number.precision(.fractionLength(0...digits)).locale(Locale(identifier: localeId)))
    return (value, t("usagePage.unit.hours"))
  }

  /// Share to whole percent, a tenth under 1%, and "<0.1%" for a non-zero sliver
  static func percent(_ share: Double, localeId: String = L10n.locale) -> String {
    let locale = Locale(identifier: localeId)
    let digits = share > 0 && share < 0.01 ? 1 : 0
    let style = FloatingPointFormatStyle<Double>.Percent(locale: locale).precision(.fractionLength(0...digits))
    if share > 0 && share < 0.001 { return "<\(style.format(0.001))" }
    return style.format(floor(share * 1000) / 1000)
  }

  static func monthShort(_ ms: Int64, localeId: String = L10n.locale, calendar: Calendar = .current) -> String {
    let date = Date(timeIntervalSince1970: Double(ms) / 1000)
    if calendar.component(.month, from: date) == 1 {
      return "\(calendar.component(.year, from: date))"
    }
    let text = date.formatted(Date.FormatStyle(locale: Locale(identifier: localeId), calendar: calendar).month(.abbreviated))
    return spaced(text, localeId: localeId)
  }

  static func numericDate(_ ms: Int64, localeId: String = L10n.locale, calendar: Calendar = .current) -> String {
    let date = Date(timeIntervalSince1970: Double(ms) / 1000)
    let fmt = DateFormatter()
    fmt.locale = Locale(identifier: localeId)
    fmt.calendar = calendar
    fmt.setLocalizedDateFormatFromTemplate("Md")
    return fmt.string(from: date)
  }

  static func dayLabel(_ ms: Int64, localeId: String = L10n.locale, calendar: Calendar = .current) -> String {
    let date = Date(timeIntervalSince1970: Double(ms) / 1000)
    let weekday = date.formatted(Date.FormatStyle(locale: Locale(identifier: localeId), calendar: calendar).weekday(.abbreviated))
    return "\(numericDate(ms, localeId: localeId, calendar: calendar)) \(weekday)"
  }

  static func monthLong(_ ms: Int64, localeId: String = L10n.locale, calendar: Calendar = .current) -> String {
    let date = Date(timeIntervalSince1970: Double(ms) / 1000)
    let text = date.formatted(Date.FormatStyle(locale: Locale(identifier: localeId), calendar: calendar).year().month(.wide))
    return spaced(text, localeId: localeId)
  }

  static func weekRange(_ start: Int64, _ endInclusive: Int64, localeId: String = L10n.locale, calendar: Calendar = .current) -> String {
    "\(numericDate(start, localeId: localeId, calendar: calendar))–\(numericDate(endInclusive, localeId: localeId, calendar: calendar))"
  }

  // MARK: - Seeded demo view (mirror of lab/usage.preview.tsx)

  /// Deterministic sample history anchored to `now` for offline demo mode and simulator screenshots
  static func sampleView(now: Date, chats: String?, calendar: Calendar = .current) -> UsageView {
    var state: UInt32 = 20261121
    func rnd() -> Double {
      state = state &+ 0x6d2b79f5
      var z = state
      z = (z ^ (z >> 15)) &* (z | 1)
      z ^= z &+ ((z ^ (z >> 7)) &* (z | 61))
      return Double(z ^ (z >> 14)) / 4294967296.0
    }

    let root = "/Users/dev/projects/acpira"
    let work = "/Users/dev/projects/Work-Local"
    let pc = "ssh://asgard/srv/studio-pc"
    let keke = "/Users/dev/projects/keke"
    let chat = "\(chats ?? "/Users/dev/.acpira/chats")/quick-chat"

    let sessions = [
      UsageSession(agent: "claude", cwd: root),
      UsageSession(agent: "claude", cwd: work),
      UsageSession(agent: "codex", cwd: root),
      UsageSession(agent: "acpira", cwd: root),
      UsageSession(agent: "codex", cwd: pc),
      UsageSession(agent: "kimi", cwd: keke),
      UsageSession(agent: "acpira", cwd: chat),
    ]
    let models = [
      UsageModel(id: "opus", name: "Opus 5.5"),
      UsageModel(id: "sonnet", name: "Sonnet 5.5"),
      UsageModel(id: "gpt-6-astra", name: "6 Astra"),
      UsageModel(id: "asgard/deepseek-v4.1-flash", name: "DeepSeek V4.1 Flash"),
      UsageModel(id: "kimi-code/k3", name: "K3"),
    ]
    let modelFor = [0, 1, 2, 3, 2, 4, 3]

    let todayMs = dayOf(Int64((now.timeIntervalSince1970 * 1000).rounded()), calendar: calendar)
    let startMs = addDays(todayMs, -300, calendar: calendar)
    var rows: [UsageRow] = []

    for d in 0...300 {
      let dayMs = addDays(startMs, d, calendar: calendar)
      let weekday = calendar.component(.weekday, from: Date(timeIntervalSince1970: Double(dayMs) / 1000))
      let weekend = weekday == 1 || weekday == 7
      // Keep the most recent days active so the current streak reads cleanly in demo screenshots
      if d < 294, rnd() < (weekend ? 0.55 : 0.12) { continue }
      let ramp = 0.3 + (Double(d) / 300.0) * 0.9
      for h in 9..<24 {
        if rnd() < 0.55 { continue }
        let s = min(sessions.count - 1, Int(rnd() * Double(sessions.count)))
        let peak = (h >= 14 && h <= 17) ? 2.2 : 1.0
        let tokens = Int64((peak * ramp * (0.3 + rnd()) * 6_000_000.0).rounded())
        let at = addHours(dayMs, h, calendar: calendar)
        let turns = 1 + Int(rnd() * 6)
        let workMs = Int64((Double(turns) * (2 + rnd() * 9) * 60_000.0).rounded())
        let longestMs = Int64((Double(workMs) / Double(turns) * (1 + rnd())).rounded())
        let prompts = max(0, turns - (rnd() < 0.2 ? 1 : 0))
        rows.append(UsageRow(
          at: at, session: s, model: modelFor[s], tokens: tokens,
          input: Int64((Double(tokens) * 0.97).rounded()), output: Int64((Double(tokens) * 0.03).rounded()),
          turns: turns, prompts: prompts, workMs: workMs, longestMs: longestMs,
          tools: turns * 7, added: Int((rnd() * 300).rounded()), removed: Int((rnd() * 120).rounded())))
      }
    }
    return UsageView(sessions: sessions, models: models, rows: rows)
  }
}
