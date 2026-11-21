import AcpiraDesign
import AcpiraModels
import SwiftUI

// Settings → Usage (packages/ui/usage/UsagePage.tsx, styles/usage.css): four all-time summary tiles, an activity dot
// lattice over the full history, a period-switched trend bar chart (7 days in 4-hour buckets, 30 days daily, all
// adaptive), and a breakdown card by model, agent or project. Tap or press-and-scrub inspects a day or column in place
// on the section's header line without shifting the charts below; tapping a breakdown row filters both charts to its share.

struct UsagePage: View {
  @Environment(AppModel.self) private var model
  @State private var period: UsagePeriod = .days30
  @State private var dim: UsageDim = .model
  @State private var focus: String?
  @State private var contentW: CGFloat = 0

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: Metrics.usageSectionGap) {
        if let view = model.usageView {
          let nowMs = Int64((model.now.timeIntervalSince1970 * 1000).rounded())
          let today = UsageStats.dayOf(nowMs)
          let nowHour = UsageStats.hourOf(nowMs)
          let labels = UsageStats.modelLabels(view, chats: model.chatRoot)
          let days = UsageStats.daysOf(view, labels)
          let hours = UsageStats.hoursOf(view, labels)
          let all = UsageStats.summarize(view, labels, from: 0)
          let current = UsageStats.summarize(view, labels, from: UsageStats.periodStart(period, today: today))
          let wide = contentW >= Metrics.usageTilesWide
          let pick: ((UsageDay) -> Int64)? = focus.map { key in { d in d.tokens(for: dim, key: key) } }

          UsageTiles(all: all, streak: UsageStats.currentRun(days, today: today), wide: wide)
          UsageYearDots(days: days, today: today, pick: pick, focusActive: focus != nil, labels: labels)
          UsageTrend(days: days, hours: hours, today: today, now: nowHour, period: $period, pick: pick,
                     focusActive: focus != nil, labels: labels)
          UsageBreakdownCard(dim: $dim, items: current.parts(for: dim), labels: labels, agents: model.agents,
                             focus: $focus, wide: wide)
        } else {
          HStack {
            Text(web: t("settings.loading"), paragraph: false)
              .textStyle(TypeScale.text3)
              .foregroundStyle(Palette.fg3)
            Spacer()
          }
          .padding(Metrics.pad)
          .background(Palette.formRow, in: RoundedRectangle(cornerRadius: Metrics.usageCardRadius, style: .continuous))
        }
      }
      .padding(.horizontal, Metrics.pad)
      .padding(.top, Metrics.s3)
      .padding(.bottom, Metrics.page)
      .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { contentW = $0 }
      .readableWidth()
    }
    .background(Palette.formPage.ignoresSafeArea())
    .navigationTitle(t("usagePage.title"))
    .navigationBarTitleDisplayModeInline()
    .onAppear { model.loadUsage() }
  }
}

// MARK: - Section head and segmented control

private struct UsageReading {
  var tokens: Int64
  var date: String?
  var topModel: String?
  var activeDays: Int?
}

/// One-line section header: title, inline reading (or inspected slot's tokens + crown + top model + trailing date), and
/// optional segmented control. Nothing wraps so inspection never pushes the chart underneath.
private struct UsageSectionHead<Action: View>: View {
  let title: String
  var reading: UsageReading?
  var count: Int?
  @ViewBuilder var action: () -> Action

  var body: some View {
    HStack(spacing: Metrics.s2) {
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
        Text(web: title, paragraph: false)
          .textStyle(TypeScale.text2, weight: .medium)
          .foregroundStyle(Palette.fg1)
          .fixedSize(horizontal: true, vertical: false)
          .layoutPriority(2)

        if let count {
          Text(verbatim: "\(count)")
            .textStyle(TypeScale.text4)
            .foregroundStyle(Palette.fg3)
            .monospacedDigit()
            .fixedSize(horizontal: true, vertical: false)
        } else if let reading {
          if let activeDays = reading.activeDays {
            Text(web: t("usagePage.period.days", ["n": activeDays]), paragraph: false)
              .textStyle(TypeScale.text4)
              .foregroundStyle(Palette.fg3)
              .monospacedDigit()
              .fixedSize(horizontal: true, vertical: false)
          } else {
            let formatted = UsageStats.figure(reading.tokens)
            let tokenText = reading.tokens > 0 || reading.date == nil
              ? UsageStats.spaced(formatted.value + (formatted.unit ?? ""))
              : t("usagePage.noUse")
            Text(verbatim: tokenText)
              .textStyle(TypeScale.text4)
              .foregroundStyle(reading.date != nil ? Palette.fg2 : Palette.fg3)
              .monospacedDigit()
              .fixedSize(horizontal: true, vertical: false)
              .layoutPriority(1.5)

            if let top = reading.topModel {
              HStack(spacing: Metrics.s1) {
                Icon(.crown, size: Metrics.usageCrown)
                  .foregroundStyle(Palette.fg3)
                Text(verbatim: top)
                  .textStyle(TypeScale.text4)
                  .foregroundStyle(Palette.fg3)
                  .lineLimit(1)
                  .truncationMode(.tail)
              }
              .frame(minWidth: 0)
            }
          }
        }

        Spacer(minLength: 0)

        if let date = reading?.date {
          Text(verbatim: date)
            .textStyle(TypeScale.text4)
            .foregroundStyle(Palette.fg3)
            .monospacedDigit()
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
            .layoutPriority(1)
        }
      }
      .frame(minWidth: 0, maxWidth: .infinity)

      action()
        .layoutPriority(2)
    }
    .padding(.horizontal, Metrics.s2)
  }
}

private struct UsageSegmented<Value: Hashable>: View {
  let items: [(value: Value, label: String)]
  @Binding var selection: Value
  var onPick: ((Value) -> Void)?

  var body: some View {
    HStack(spacing: Metrics.s0_5) {
      ForEach(Array(items.enumerated()), id: \.offset) { _, item in
        let active = item.value == selection
        Button {
          guard selection != item.value else { return }
          Haptics.play(.pick)
          selection = item.value
          onPick?(item.value)
        } label: {
          Text(web: item.label, paragraph: false)
            .textStyle(TypeScale.text4, weight: active ? .medium : .regular)
            .foregroundStyle(active ? Palette.fg1 : Palette.fg3)
            .lineLimit(1)
            .padding(.horizontal, Metrics.usageSegPadX)
            .frame(height: Metrics.usageSegH)
            .background {
              if active {
                RoundedRectangle(cornerRadius: Metrics.rSm, style: .continuous)
                  .fill(Palette.formRow)
                  .overlay(RoundedRectangle(cornerRadius: Metrics.rSm, style: .continuous).strokeBorder(Palette.line, lineWidth: Metrics.hairline))
              }
            }
        }
        .buttonStyle(.plain)
      }
    }
    .padding(Metrics.usageSegPad)
    .background(Palette.active, in: RoundedRectangle(cornerRadius: Metrics.rMd, style: .continuous))
  }
}

// MARK: - Block 1: Summary tiles

private struct UsageTileItem {
  var caption: String
  var value: String
  var unit: String?
}

private struct UsageTiles: View {
  let all: UsageSummary
  let streak: Int
  let wide: Bool

  private var items: [UsageTileItem] {
    let tokens = UsageStats.figure(all.tokens)
    let work = UsageStats.duration(all.workMs)
    let longest = UsageStats.duration(all.longestMs)
    return [
      UsageTileItem(caption: t("usagePage.fig.tokens"), value: tokens.value, unit: tokens.unit),
      UsageTileItem(caption: t("usagePage.fig.work"), value: work.value, unit: work.unit),
      UsageTileItem(caption: t("usagePage.fig.longest"), value: longest.value, unit: longest.unit),
      UsageTileItem(caption: t("usagePage.fig.streak"), value: "\(streak)", unit: t("usagePage.unit.days")),
    ]
  }

  var body: some View {
    let list = items
    VStack(spacing: 0) {
      if wide {
        HStack(spacing: 0) {
          ForEach(Array(list.enumerated()), id: \.offset) { idx, tile in
            if idx > 0 {
              Rectangle().fill(Palette.line).frame(width: Metrics.hairline)
                .padding(.vertical, Metrics.s3)
            }
            tileCell(tile)
          }
        }
      } else {
        HStack(spacing: 0) {
          tileCell(list[0])
          Rectangle().fill(Palette.line).frame(width: Metrics.hairline).padding(.top, Metrics.s3)
          tileCell(list[1])
        }
        Rectangle().fill(Palette.line).frame(height: Metrics.hairline).padding(.horizontal, Metrics.pad)
        HStack(spacing: 0) {
          tileCell(list[2])
          Rectangle().fill(Palette.line).frame(width: Metrics.hairline).padding(.bottom, Metrics.s3)
          tileCell(list[3])
        }
      }
    }
    .background(Palette.formRow, in: RoundedRectangle(cornerRadius: Metrics.usageCardRadius, style: .continuous))
  }

  private func tileCell(_ tile: UsageTileItem) -> some View {
    VStack(alignment: .leading, spacing: Metrics.s1_5) {
      Text(web: tile.caption, paragraph: false)
        .textStyle(TypeScale.text4)
        .foregroundStyle(Palette.fg3)
        .lineLimit(1)
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s0_5) {
        Text(verbatim: tile.value)
          .textStyle(TypeScale.usageFigure, weight: .semibold)
          .foregroundStyle(Palette.fg1)
          .monospacedDigit()
          .lineLimit(1)
        if let unit = tile.unit {
          Text(web: unit, paragraph: false)
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.fg2)
            .lineLimit(1)
        }
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.s3)
  }
}

// MARK: - Block 2: Activity dot lattice

private struct UsageYearDots: View {
  let days: [Int64: UsageDay]
  let today: Int64
  let pick: ((UsageDay) -> Int64)?
  let focusActive: Bool
  let labels: UsageModelLabels

  @State private var width: CGFloat = 0
  @State private var inspected: Int64?

  private func value(_ day: UsageDay?) -> Int64 {
    guard let day else { return 0 }
    return pick?(day) ?? day.tokens
  }

  private func topModel(_ day: UsageDay?) -> String? {
    guard !focusActive, let byModel = day?.byModel else { return nil }
    guard let best = byModel.filter({ !$0.key.isEmpty && $0.value > 0 }).max(by: { $0.value < $1.value })?.key else { return nil }
    return labels.label[best]?.name ?? best
  }

  var body: some View {
    let cal = Calendar.current
    let lastMonday = UsageStats.addDays(today, -UsageStats.mondayOffset(today, calendar: cal), calendar: cal)
    let cols = max(8, min(53, Int(floor(width / Metrics.usageDotPitchMin))))
    let gap = width > 0 ? max(Metrics.usageDotGapMin, (width / CGFloat(cols) * 0.2).rounded()) : Metrics.usageDotGapMin
    let pitch = cols > 0 ? (width + gap) / CGFloat(cols) : Metrics.usageDotPitchMin
    let size = max(1, pitch - gap)
    let corner = min(Metrics.usageDotCornerMax, size / 4)
    let height = 7 * size + 6 * gap
    let start = UsageStats.addDays(lastMonday, -(cols - 1) * 7, calendar: cal)

    let maxVal = max(Int64(1), days.values.reduce(Int64(0)) { acc, d in d.day >= start ? max(acc, value(d)) : acc })
    let activeDays = days.values.filter { value($0) > 0 }.count

    let reading: UsageReading = {
      if let day = inspected {
        let entry = days[day]
        return UsageReading(tokens: value(entry), date: UsageStats.dayLabel(day), topModel: topModel(entry))
      }
      return UsageReading(tokens: 0, activeDays: activeDays)
    }()

    let months: [(x: CGFloat, label: String, edge: Int)] = {
      guard width > 0 else { return [] }
      var out: [(x: CGFloat, label: String, edge: Int)] = []
      for c in 0..<cols {
        let weekEnd = UsageStats.addDays(start, c * 7 + 6, calendar: cal)
        let d = cal.component(.day, from: Date(timeIntervalSince1970: Double(weekEnd) / 1000))
        if d > 7 { continue }
        let x = CGFloat(c) * pitch
        if let prev = out.last, x - prev.x < Metrics.usageMonthGap { continue }
        let edge = x < Metrics.usageMonthEdge ? -1 : x > width - Metrics.usageMonthEdge ? 1 : 0
        out.append((x, UsageStats.monthShort(weekEnd, calendar: cal), edge))
      }
      return out
    }()

    let hitDay: (CGPoint) -> Int64? = { pt in
      guard width > 0, pitch > 0 else { return nil }
      let c = Int(floor((pt.x + gap / 2) / pitch))
      let k = Int(floor((pt.y + gap / 2) / pitch))
      guard c >= 0, c < cols, k >= 0, k < 7 else { return nil }
      let day = UsageStats.addDays(start, c * 7 + k, calendar: cal)
      return day <= today ? day : nil
    }

    VStack(alignment: .leading, spacing: Metrics.usageGroupHeadGap) {
      UsageSectionHead(title: t("usagePage.activity"), reading: reading) { EmptyView() }

      VStack(alignment: .leading, spacing: Metrics.s2) {
        Canvas { ctx, _ in
          guard width > 0 else { return }
          for c in 0..<cols {
            let x = CGFloat(c) * pitch
            for k in 0..<7 {
              let day = UsageStats.addDays(start, c * 7 + k, calendar: cal)
              if day > today { continue }
              let y = CGFloat(k) * pitch
              let rect = CGRect(x: x, y: y, width: size, height: size)
              let path = Path(roundedRect: rect, cornerRadius: corner)
              let v = value(days[day])
              if v > 0 {
                let step = max(1, Int(ceil(sqrt(Double(v) / Double(maxVal)) * Double(UsageStats.dotSteps))))
                ctx.fill(path, with: .color(Palette.usageInk.opacity(Double(step) / Double(UsageStats.dotSteps))))
              } else {
                ctx.fill(path, with: .color(Palette.usageCell))
              }
              if inspected == day {
                let focusRect = rect.insetBy(dx: -Metrics.usageFocusOutset, dy: -Metrics.usageFocusOutset)
                let focusPath = Path(roundedRect: focusRect, cornerRadius: corner + Metrics.usageFocusOutset)
                ctx.stroke(focusPath, with: .color(Palette.fg2), lineWidth: Metrics.hairline)
              }
            }
          }
        }
        .frame(height: height > 0 ? height : Metrics.usageTrendH)
        .contentShape(Rectangle())
        .gesture(
          SpatialTapGesture().onEnded { value in
            let next = hitDay(value.location)
            if next != nil && next == inspected {
              inspected = nil
            } else {
              if next != nil { Haptics.play(.pick) }
              inspected = next
            }
          }
        )
        .simultaneousGesture(
          LongPressGesture(minimumDuration: 0.16)
            .sequenced(before: DragGesture(minimumDistance: 0))
            .onChanged { value in
              if case .second(true, let drag?) = value {
                let next = hitDay(drag.location)
                if next != inspected {
                  if next != nil { Haptics.play(.pick) }
                  inspected = next
                }
              }
            }
        )

        ZStack(alignment: .topLeading) {
          Color.clear.frame(height: TypeScale.text4.lineHeight)
          ForEach(Array(months.enumerated()), id: \.offset) { _, m in
            Text(verbatim: m.label)
              .textStyle(TypeScale.text4)
              .foregroundStyle(Palette.fg3)
              .lineLimit(1)
              .fixedSize()
              .position(x: m.edge == -1 ? Metrics.usageMonthEdge : m.edge == 1 ? max(Metrics.usageMonthEdge, width - Metrics.usageMonthEdge) : m.x,
                        y: TypeScale.text4.lineHeight / 2)
          }
        }
      }
      .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
      .padding(.horizontal, Metrics.s2)
    }
  }
}

// MARK: - Block 3: Trend bar chart

private struct UsageTrendCol {
  var start: Int64
  var end: Int64
  var total: Int64
  var part: Int64
}

private struct UsageTrend: View {
  let days: [Int64: UsageDay]
  let hours: [Int64: UsageDay]
  let today: Int64
  let now: Int64
  @Binding var period: UsagePeriod
  let pick: ((UsageDay) -> Int64)?
  let focusActive: Bool
  let labels: UsageModelLabels

  @State private var width: CGFloat = 0
  @State private var inspectedIndex: Int?

  var body: some View {
    let cal = Calendar.current
    let layout = UsageStats.trendColumns(period, today: today, first: UsageStats.firstDay(days, today: today), now: now, calendar: cal)
    let fold = layout.step == .hours ? hours : days
    let advance: (Int64) -> Int64 = layout.step == .hours
      ? { UsageStats.addHours($0, 1, calendar: cal) }
      : { UsageStats.addDays($0, 1, calendar: cal) }

    let cols: [UsageTrendCol] = layout.cols.map { span in
      var total: Int64 = 0
      var part: Int64 = 0
      var d = span.start
      while d < span.end {
        if let u = fold[d] {
          total += u.tokens
          if let pick { part += pick(u) }
        }
        let next = advance(d)
        if next <= d { break }
        d = next
      }
      return UsageTrendCol(start: span.start, end: span.end, total: total, part: part)
    }

    let n = cols.count
    let h = Metrics.usageTrendH
    let bw = max(Metrics.usageBarMin, min(Metrics.usageBarMax, (width / CGFloat(max(1, n))) * 0.64))
    let pitch = n > 1 ? (width - bw) / CGFloat(n - 1) : 0
    let colX: (Int) -> CGFloat = { i in n > 1 ? CGFloat(i) * pitch : (width - bw) / 2 }
    let maxVal = max(Int64(1), cols.map(\.total).max() ?? 1)

    let safeIndex = inspectedIndex.flatMap { $0 >= 0 && $0 < n ? $0 : nil }
    let reading: UsageReading = {
      if let i = safeIndex {
        let c = cols[i]
        let date: String = switch layout.step {
        case .hours:
          "\(UsageStats.dayLabel(c.start, calendar: cal)) \(cal.component(.hour, from: Date(timeIntervalSince1970: Double(c.start) / 1000))):00"
        case .day:
          UsageStats.dayLabel(c.start, calendar: cal)
        case .week:
          UsageStats.weekRange(c.start, UsageStats.addDays(c.end, -1, calendar: cal), calendar: cal)
        case .month:
          UsageStats.monthLong(c.start, calendar: cal)
        }
        var top: String?
        if !focusActive {
          var byModel: [String: Int64] = [:]
          var d = c.start
          while d < c.end {
            for (k, v) in fold[d]?.byModel ?? [:] where !k.isEmpty { byModel[k, default: 0] += v }
            let next = advance(d)
            if next <= d { break }
            d = next
          }
          if let best = byModel.filter({ $0.value > 0 }).max(by: { $0.value < $1.value })?.key {
            top = labels.label[best]?.name ?? best
          }
        }
        return UsageReading(tokens: pick != nil ? c.part : c.total, date: date, topModel: top)
      }
      let sum = cols.reduce(Int64(0)) { $0 + (pick != nil ? $1.part : $1.total) }
      return UsageReading(tokens: sum)
    }()

    let ticks: [(x: CGFloat, label: String, edge: Int)] = {
      guard width > 0, n > 0 else { return [] }
      let tickLabel: (Int64) -> String = { ms in
        layout.step == .month ? UsageStats.monthShort(ms, calendar: cal) : UsageStats.numericDate(ms, calendar: cal)
      }
      var out: [(x: CGFloat, label: String, edge: Int)] = []
      if layout.step == .hours {
        let perDay = 24 / UsageStats.trendSpanHours
        for (i, c) in cols.enumerated() where cal.component(.hour, from: Date(timeIntervalSince1970: Double(c.start) / 1000)) == 0 {
          let mid = (colX(i) + colX(min(i + perDay - 1, n - 1)) + bw) / 2
          out.append((mid, tickLabel(c.start), 0))
        }
      } else {
        let every = layout.step == .day
          ? (width / CGFloat(max(1, n)) >= Metrics.usageLabelRoom ? 1 : 7)
          : max(1, Int(ceil(CGFloat(n) * Metrics.usageLabelRoom / max(1, width))))
        var i = n - 1
        while i >= 0 {
          let edge = n > 1 && i == 0 ? -1 : (n > 1 && i == n - 1 ? 1 : 0)
          out.append((colX(i) + bw / 2, tickLabel(cols[i].start), edge))
          i -= every
        }
      }
      return out
    }()

    let hitBar: (CGPoint) -> Int? = { pt in
      guard width > 0, n > 0 else { return nil }
      if n == 1 { return 0 }
      let raw = Int(((pt.x - bw / 2) / pitch).rounded())
      return max(0, min(n - 1, raw))
    }

    VStack(alignment: .leading, spacing: Metrics.usageGroupHeadGap) {
      UsageSectionHead(title: t("usagePage.trend"), reading: reading) {
        UsageSegmented(
          items: UsagePeriod.allCases.map { ($0, $0.label) },
          selection: $period,
          onPick: { _ in inspectedIndex = nil }
        )
      }

      VStack(alignment: .leading, spacing: Metrics.s2) {
        Canvas { ctx, size in
          guard width > 0 else { return }
          var baseline = Path()
          baseline.move(to: CGPoint(x: 0, y: size.height - 0.5))
          baseline.addLine(to: CGPoint(x: size.width, y: size.height - 0.5))
          ctx.stroke(baseline, with: .color(Palette.line), lineWidth: Metrics.hairline)

          func barPath(_ i: Int, _ v: Int64) -> Path? {
            guard v > 0 else { return nil }
            let bh = max(Metrics.usageBarMinH, (CGFloat(v) / CGFloat(maxVal)) * (size.height - Metrics.usageBarMinH))
            let x0 = colX(i)
            let r = min(Metrics.usageBarRadiusMax, bw / 2, bh)
            let yTop = size.height - bh
            var p = Path()
            p.move(to: CGPoint(x: x0, y: size.height))
            p.addLine(to: CGPoint(x: x0, y: yTop + r))
            p.addQuadCurve(to: CGPoint(x: x0 + r, y: yTop), control: CGPoint(x: x0, y: yTop))
            p.addLine(to: CGPoint(x: x0 + bw - r, y: yTop))
            p.addQuadCurve(to: CGPoint(x: x0 + bw, y: yTop + r), control: CGPoint(x: x0 + bw, y: yTop))
            p.addLine(to: CGPoint(x: x0 + bw, y: size.height))
            p.closeSubpath()
            return p
          }

          for (i, c) in cols.enumerated() {
            guard let whole = barPath(i, c.total) else { continue }
            let lit = safeIndex != nil ? safeIndex == i : pick == nil
            let fill = safeIndex == i ? Palette.usageInk : (lit ? Palette.usageBar : Palette.usageDim)
            ctx.fill(whole, with: .color(fill))
            if pick != nil, safeIndex == nil, let part = barPath(i, c.part) {
              ctx.fill(part, with: .color(Palette.usageInk))
            }
          }
        }
        .frame(height: h)
        .contentShape(Rectangle())
        .gesture(
          SpatialTapGesture().onEnded { value in
            let next = hitBar(value.location)
            if next != nil && next == inspectedIndex {
              inspectedIndex = nil
            } else {
              if next != nil { Haptics.play(.pick) }
              inspectedIndex = next
            }
          }
        )
        .simultaneousGesture(
          LongPressGesture(minimumDuration: 0.16)
            .sequenced(before: DragGesture(minimumDistance: 0))
            .onChanged { value in
              if case .second(true, let drag?) = value {
                let next = hitBar(drag.location)
                if next != inspectedIndex {
                  if next != nil { Haptics.play(.pick) }
                  inspectedIndex = next
                }
              }
            }
        )

        ZStack(alignment: .topLeading) {
          Color.clear.frame(height: TypeScale.text4.lineHeight)
          ForEach(Array(ticks.enumerated()), id: \.offset) { _, tick in
            Text(verbatim: tick.label)
              .textStyle(TypeScale.text4)
              .foregroundStyle(Palette.fg3)
              .monospacedDigit()
              .lineLimit(1)
              .fixedSize()
              .position(x: tick.edge == -1 ? Metrics.usageMonthEdge : tick.edge == 1 ? max(Metrics.usageMonthEdge, width - Metrics.usageMonthEdge) : tick.x,
                        y: TypeScale.text4.lineHeight / 2)
          }
        }
      }
      .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
      .padding(.horizontal, Metrics.s2)
    }
  }
}

// MARK: - Block 4: Breakdown list

private struct UsageBreakdownCard: View {
  @Binding var dim: UsageDim
  let items: [UsagePart]
  let labels: UsageModelLabels
  let agents: [AgentInfo]
  @Binding var focus: String?
  let wide: Bool

  @State private var expanded = false

  private func agentName(_ id: String) -> String {
    agents.first { $0.id == id }?.name ?? id
  }

  private func modelName(_ key: String) -> String {
    labels.label[key]?.name ?? key
  }

  private func projectLabel(_ key: String) -> String {
    if key == UsageStats.chatsKey { return t("session.group.chats") }
    return projectName(RemoteFolder.parse(key)?.path ?? key)
  }

  private func rowName(_ key: String) -> String {
    switch dim {
    case .model: modelName(key)
    case .agent: agentName(key)
    case .project: projectLabel(key)
    }
  }

  private func rowDetail(_ key: String) -> String? {
    guard dim == .project, key != UsageStats.chatsKey else { return nil }
    if let remote = RemoteFolder.parse(key) { return remote.alias }
    let label = projectLabel(key)
    let hasTwin = items.contains { $0.key != key && $0.key != UsageStats.chatsKey && projectLabel($0.key) == label }
    guard hasTwin, let cut = key.lastIndex(where: { $0 == "/" || $0 == "\\" }) else { return nil }
    return projectName(String(key[..<cut]))
  }

  @ViewBuilder
  private func rowMark(_ key: String) -> some View {
    switch dim {
    case .model:
      VendorMark(brand: labels.label[key]?.brand, name: modelName(key), size: Metrics.icon, fit: true)
        .foregroundStyle(Palette.fg2)
    case .agent:
      VendorMark(brand: key, name: agentName(key), size: Metrics.icon, fit: true)
        .foregroundStyle(Palette.fg2)
    case .project:
      if key == UsageStats.chatsKey {
        Icon(.messageSquare, size: Metrics.icon).foregroundStyle(Palette.fg2)
      } else if RemoteFolder.parse(key) != nil {
        RemoteFolderGlyph(open: false).foregroundStyle(Palette.fg2)
      } else {
        Icon(.folder, size: Metrics.icon).foregroundStyle(Palette.fg2)
      }
    }
  }

  var body: some View {
    let folds = items.count > UsageStats.keepRows + 1
    let head = folds ? Array(items.prefix(UsageStats.keepRows)) : items
    let rest = folds ? Array(items.dropFirst(UsageStats.keepRows)) : []
    let visible = expanded ? items : head
    let others = UsagePart(key: "*", tokens: rest.reduce(0) { $0 + $1.tokens }, share: rest.reduce(0) { $0 + $1.share })

    VStack(alignment: .leading, spacing: Metrics.usageGroupHeadGap) {
      UsageSectionHead(title: t("usagePage.breakdown"), count: items.count) {
        UsageSegmented(
          items: UsageDim.allCases.map { ($0, $0.label) },
          selection: $dim,
          onPick: { _ in
            focus = nil
            expanded = false
          }
        )
      }

      if !items.isEmpty {
        VStack(spacing: 0) {
          ForEach(Array(visible.enumerated()), id: \.element.id) { idx, part in
            if idx > 0 {
              Rectangle().fill(Palette.line).frame(height: Metrics.hairline).padding(.horizontal, Metrics.pad)
            }
            breakdownButton(part: part, isMore: false)
          }
          if folds && !expanded {
            Rectangle().fill(Palette.line).frame(height: Metrics.hairline).padding(.horizontal, Metrics.pad)
            breakdownButton(part: others, isMore: true, moreCount: rest.count)
          }
        }
        .background(Palette.formRow, in: RoundedRectangle(cornerRadius: Metrics.usageCardRadius, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: Metrics.usageCardRadius, style: .continuous))
      }
    }
  }

  private func breakdownButton(part: UsagePart, isMore: Bool, moreCount: Int = 0) -> some View {
    let dimmed = focus != nil && focus != part.key
    let formatted = UsageStats.figure(part.tokens)
    return Button {
      Haptics.play(.pick)
      if isMore {
        focus = nil
        withAnimation(Motion.open) { expanded = true }
      } else {
        focus = focus == part.key ? nil : part.key
      }
    } label: {
      Group {
        if wide {
          HStack(spacing: Metrics.gap) {
            markSlot(part.key, isMore: isMore)
            nameSlot(part.key, isMore: isMore, moreCount: moreCount)
              .frame(width: Metrics.usageNameWide, alignment: .leading)
            shareBar(share: part.share, muted: isMore)
            tokenSlot(formatted)
              .frame(width: Metrics.usageNumW, alignment: .trailing)
            percentSlot(part.share)
              .frame(width: Metrics.usagePctW, alignment: .trailing)
          }
        } else {
          HStack(alignment: .center, spacing: Metrics.gap) {
            markSlot(part.key, isMore: isMore)
            VStack(alignment: .leading, spacing: Metrics.s1_5) {
              HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
                nameSlot(part.key, isMore: isMore, moreCount: moreCount)
                Spacer(minLength: 0)
                tokenSlot(formatted)
                percentSlot(part.share)
                  .frame(width: Metrics.usagePctW, alignment: .trailing)
              }
              shareBar(share: part.share, muted: isMore)
            }
          }
        }
      }
      .padding(.horizontal, Metrics.pad)
      .padding(.vertical, Metrics.s3)
      .contentShape(Rectangle())
      .opacity(dimmed ? Metrics.usageDimRow : 1)
      .animation(Motion.quick, value: dimmed)
    }
    .buttonStyle(.plain)
  }

  private func markSlot(_ key: String, isMore: Bool) -> some View {
    Group {
      if isMore {
        Icon(.ellipsis, size: Metrics.icon).foregroundStyle(Palette.fg3)
      } else {
        rowMark(key)
      }
    }
    .frame(width: Metrics.lead, height: Metrics.lead)
  }

  private func nameSlot(_ key: String, isMore: Bool, moreCount: Int) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: Metrics.s1_5) {
      if isMore {
        Text(web: t(dim.moreKey, ["n": moreCount]), paragraph: false)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg2)
          .lineLimit(1)
      } else {
        Text(verbatim: rowName(key))
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg1)
          .lineLimit(1)
        if let detail = rowDetail(key) {
          Text(verbatim: detail)
            .textStyle(TypeScale.text4)
            .foregroundStyle(Palette.fg3)
            .lineLimit(1)
            .fixedSize(horizontal: true, vertical: false)
        }
      }
    }
    .frame(minWidth: 0, alignment: .leading)
  }

  private func shareBar(share: Double, muted: Bool) -> some View {
    GeometryReader { geo in
      ZStack(alignment: .leading) {
        Capsule().fill(Palette.usageTrack)
        Capsule()
          .fill(muted ? Palette.usageDim : Palette.usageInk)
          .frame(width: max(0, geo.size.width * min(1, max(0, share))))
      }
    }
    .frame(height: Metrics.usageTrackH)
  }

  private func tokenSlot(_ formatted: (value: String, unit: String?)) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: Metrics.s0_5) {
      Text(verbatim: formatted.value)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.fg1)
        .monospacedDigit()
      if let unit = formatted.unit {
        Text(verbatim: unit)
          .textStyle(TypeScale.text4)
          .foregroundStyle(Palette.fg3)
      }
    }
    .lineLimit(1)
    .fixedSize(horizontal: true, vertical: false)
  }

  private func percentSlot(_ share: Double) -> some View {
    Text(verbatim: UsageStats.percent(share))
      .textStyle(TypeScale.text4)
      .foregroundStyle(Palette.fg3)
      .monospacedDigit()
      .lineLimit(1)
  }
}
