import AcpiraModels
@testable import AcpiraUI
import Foundation
import Testing

@MainActor
struct UsageStatsTests {
  private static var utc: Calendar {
    var cal = Calendar(identifier: .gregorian)
    cal.timeZone = TimeZone(secondsFromGMT: 0)!
    return cal
  }

  @Test
  func mergesModelKeySpellings() {
    #expect(UsageStats.modelKey("Opus 5.5") == UsageStats.modelKey("Claude Opus 5.5"))
    #expect(UsageStats.modelKey("Opus 5.5") == UsageStats.modelKey("asgard/claude-opus-5-5-medium"))
    #expect(UsageStats.modelKey("GPT-6.1 Sol") == UsageStats.modelKey("6.1 Sol"))
    #expect(UsageStats.modelKey("kimi-code/k3") == UsageStats.modelKey("Kimi K3"))
    #expect(UsageStats.modelKey("claude") == "claude")
  }

  @Test
  func summarizesAndTracksStreak() {
    let cal = Self.utc
    let day0 = UsageStats.dayOf(1_761_590_400_000, calendar: cal) // 2025-10-27 00:00 UTC
    let day1 = UsageStats.addDays(day0, -1, calendar: cal)
    let day3 = UsageStats.addDays(day0, -3, calendar: cal)
    let view = UsageView(
      sessions: [
        UsageSession(agent: "claude", cwd: "/Users/dev/app"),
        UsageSession(agent: "codex", cwd: "/Users/dev/.acpira/chats/c1"),
      ],
      models: [
        UsageModel(id: "claude-opus-5-5", name: "claude-opus-5-5"),
        UsageModel(id: "opus", name: "Claude Opus 5.5"),
        UsageModel(id: "gpt-6", name: "6 Astra"),
      ],
      rows: [
        UsageRow(at: day3, session: 0, model: 0, tokens: 1000, workMs: 60_000, longestMs: 30_000),
        UsageRow(at: day1, session: 0, model: 1, tokens: 3000, workMs: 120_000, longestMs: 90_000),
        UsageRow(at: day0, session: 1, model: 2, tokens: 6000, workMs: 180_000, longestMs: 120_000),
      ]
    )
    let labels = UsageStats.modelLabels(view, chats: "/Users/dev/.acpira/chats")
    #expect(labels.label[UsageStats.modelKey("Opus 5.5")]?.name == "Claude Opus 5.5")
    #expect(labels.projects[1] == UsageStats.chatsKey)

    let days = UsageStats.daysOf(view, labels, calendar: cal)
    #expect(UsageStats.currentRun(days, today: day0, calendar: cal) == 2)

    let all = UsageStats.summarize(view, labels, from: 0)
    #expect(all.tokens == 10_000)
    #expect(all.longestMs == 120_000)
    #expect(all.modelParts.first?.tokens == 6000)
    #expect(all.projectParts.contains { $0.key == UsageStats.chatsKey })
  }

  @Test
  func selectsTrendStepByPeriodAndSpan() {
    let cal = Self.utc
    let today = UsageStats.dayOf(1_761_590_400_000, calendar: cal)
    let now = UsageStats.addHours(today, 12, calendar: cal)
    #expect(UsageStats.trendColumns(.days7, today: today, first: today, now: now, calendar: cal).step == .hours)
    #expect(UsageStats.trendColumns(.days30, today: today, first: today, now: now, calendar: cal).cols.count == 30)
    let sixtyDaysAgo = UsageStats.addDays(today, -60, calendar: cal)
    #expect(UsageStats.trendColumns(.all, today: today, first: sixtyDaysAgo, now: now, calendar: cal).step == .week)
    let yearAgo = UsageStats.addDays(today, -300, calendar: cal)
    #expect(UsageStats.trendColumns(.all, today: today, first: yearAgo, now: now, calendar: cal).step == .month)
  }

  @Test
  func formatsFiguresSpacingAndPercents() {
    #expect(UsageStats.figure(9_500, localeId: "en") == ("9,500", nil))
    let compactEn = UsageStats.figure(12_340, localeId: "en")
    #expect(compactEn.value == "12.3")
    #expect(compactEn.unit == "K")
    let compactZh = UsageStats.figure(762_000_000, localeId: "zh-CN")
    #expect(compactZh.value == "7.62")
    #expect(compactZh.unit == "亿")
    #expect(UsageStats.spaced("2026年10月", localeId: "zh-CN") == "2026 年 10 月")
    #expect(UsageStats.spaced("7.62亿", localeId: "zh-CN") == "7.62 亿")
    #expect(UsageStats.percent(0.0004, localeId: "en") == "<0.1%")
    #expect(UsageStats.percent(0.0056, localeId: "en") == "0.5%")
    #expect(UsageStats.percent(0.0099, localeId: "en") == "0.9%")
    #expect(UsageStats.percent(0.429, localeId: "en") == "43%")
  }
}
