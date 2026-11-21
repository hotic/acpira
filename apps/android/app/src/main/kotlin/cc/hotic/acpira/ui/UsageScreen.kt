package cc.hotic.acpira.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.UsageDay
import cc.hotic.acpira.logic.UsageDim
import cc.hotic.acpira.logic.UsageFigure
import cc.hotic.acpira.logic.UsageModelLabels
import cc.hotic.acpira.logic.UsagePart
import cc.hotic.acpira.logic.UsagePeriod
import cc.hotic.acpira.logic.UsageStats
import cc.hotic.acpira.logic.UsageStep
import cc.hotic.acpira.logic.UsageSummary
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.RemoteFolder
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.projectName
import java.time.Instant
import java.time.ZoneId
import kotlin.math.ceil
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.sqrt

// Settings → Usage (packages/ui/usage/UsagePage.tsx, apps/ios Settings/UsagePage.swift): four all-time summary tiles,
// an activity dot lattice over the full history, a period-switched trend bar chart, and a breakdown card by model,
// agent or project.

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun UsageScreen(model: AppModel) {
  LaunchedEffect(Unit) { model.loadUsage() }
  var period by remember { mutableStateOf(UsagePeriod.Days30) }
  var dim by remember { mutableStateOf(UsageDim.Model) }
  var focus by remember { mutableStateOf<String?>(null) }

  Scaffold(
    containerColor = Palette.formPage,
    topBar = {
      TopAppBar(
        title = { Text(t("usagePage.title"), fontSize = Type.barTitle, fontWeight = FontWeight.SemiBold) },
        navigationIcon = { LeadButton("usage.back") { model.pop() } },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.formPage),
      )
    },
  ) { padding ->
    BoxWithConstraints(
      Modifier.fillMaxSize().padding(top = padding.calculateTopPadding()).verticalScroll(rememberScrollState())
        .readableWidth().padding(horizontal = Metrics.pad).navigationBarsPadding().padding(bottom = Metrics.page),
    ) {
      val wide = maxWidth >= Metrics.usageTilesWide
      val view = model.usageView
      Column(
        Modifier.fillMaxWidth().padding(top = Metrics.s2),
        verticalArrangement = Arrangement.spacedBy(Metrics.usageSectionGap),
      ) {
        if (view != null) {
          val context = LocalContext.current
          val nowMs = remember(view) { model.now().toEpochMilli() }
          val zone = remember { ZoneId.systemDefault() }
          val today = remember(nowMs, zone) { UsageStats.dayOf(nowMs, zone) }
          val nowHour = remember(nowMs, zone) { UsageStats.hourOf(nowMs, zone) }
          val labels = remember(view, model.chatRoot) {
            UsageStats.modelLabels(view, model.chatRoot) { IconStore.brandOf(context, it) }
          }
          val days = remember(view, labels, zone) { UsageStats.daysOf(view, labels, zone) }
          val hours = remember(view, labels, zone) { UsageStats.hoursOf(view, labels, zone) }
          val all = remember(view, labels) { UsageStats.summarize(view, labels, 0L) }
          val current = remember(view, labels, period, today, zone) {
            UsageStats.summarize(view, labels, UsageStats.periodStart(period, today, zone))
          }
          val streak = remember(days, today, zone) { UsageStats.currentRun(days, today, zone) }
          val pick: ((UsageDay) -> Long)? = remember(focus, dim) {
            focus?.let { k -> { d: UsageDay -> d.tokensFor(dim, k) } }
          }

          UsageTilesCard(all, streak, wide)
          UsageYearDots(days, today, pick, focus != null, labels, zone)
          UsageTrend(days, hours, today, nowHour, period, { period = it }, pick, focus != null, labels, zone)
          UsageBreakdownCard(
            dim = dim,
            onDim = { dim = it; focus = null },
            items = current.parts(dim),
            labels = labels,
            agents = model.agents,
            focus = focus,
            onFocus = { focus = it },
            wide = wide,
          )
        } else {
          FormCard {
            Box(Modifier.fillMaxWidth().padding(Metrics.pad)) {
              Text(t("settings.loading"), color = Palette.fg3, fontSize = Type.text3)
            }
          }
        }
      }
    }
  }
}

private val TNUM = TextStyle(fontFeatureSettings = "tnum")

private data class UsageReading(
  val tokens: Long = 0L,
  val date: String? = null,
  val topModel: String? = null,
  val activeDays: Int? = null,
)

@Composable
private fun UsageSectionHead(
  title: String,
  reading: UsageReading? = null,
  count: Int? = null,
  action: (@Composable () -> Unit)? = null,
) {
  Row(
    Modifier.fillMaxWidth().padding(horizontal = Metrics.s2),
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    Row(
      Modifier.weight(1f),
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    ) {
      Text(title, color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 1, softWrap = false)
      when {
        count != null -> {
          Text(
            "$count",
            color = Palette.fg3,
            fontSize = Type.text4,
            style = TNUM,
            maxLines = 1,
            softWrap = false,
          )
        }
        reading != null && reading.activeDays != null -> {
          Text(
            t("usagePage.period.days", "n" to reading.activeDays),
            color = Palette.fg3,
            fontSize = Type.text4,
            style = TNUM,
            maxLines = 1,
            softWrap = false,
          )
        }
        reading != null -> {
          val fig = UsageStats.figure(reading.tokens)
          val tokenText = if (reading.tokens > 0L || reading.date == null) {
            UsageStats.spaced(fig.value + (fig.unit ?: ""))
          } else {
            t("usagePage.noUse")
          }
          Text(
            tokenText,
            color = if (reading.date != null) Palette.fg2 else Palette.fg3,
            fontSize = Type.text4,
            style = TNUM,
            maxLines = 1,
            softWrap = false,
          )
          if (reading.topModel != null) {
            Row(
              Modifier.weight(1f, fill = false),
              verticalAlignment = Alignment.CenterVertically,
              horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
            ) {
              Lucide("crown", Palette.fg3, size = Metrics.usageCrown)
              Text(
                reading.topModel,
                color = Palette.fg3,
                fontSize = Type.text4,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
              )
            }
          }
        }
      }
      Spacer(Modifier.weight(1f))
      if (reading?.date != null) {
        Text(
          reading.date,
          color = Palette.fg3,
          fontSize = Type.text4,
          style = TNUM,
          maxLines = 1,
          softWrap = false,
        )
      }
    }
    action?.invoke()
  }
}

@Composable
private fun <T> UsageSegmented(
  items: List<Pair<T, String>>,
  selected: T,
  onSelect: (T) -> Unit,
  tagPrefix: String,
) {
  val haptic = LocalHapticFeedback.current
  Row(
    Modifier
      .clip(RoundedCornerShape(Metrics.rMd))
      .background(Palette.active)
      .padding(Metrics.usageSegPad),
    horizontalArrangement = Arrangement.spacedBy(Metrics.s0_5),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    items.forEachIndexed { idx, (value, label) ->
      val active = value == selected
      val shape = RoundedCornerShape(Metrics.rSm)
      Box(
        Modifier
          .height(Metrics.usageSegH)
          .clip(shape)
          .then(
            if (active) Modifier.background(Palette.formRow, shape).border(Metrics.hairline, Palette.line, shape)
            else Modifier,
          )
          .clickable {
            if (!active) {
              haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
              onSelect(value)
            }
          }
          .padding(horizontal = Metrics.usageSegPadX)
          .testTag("$tagPrefix.$idx"),
        contentAlignment = Alignment.Center,
      ) {
        Text(
          label,
          color = if (active) Palette.fg1 else Palette.fg3,
          fontSize = Type.text4,
          fontWeight = if (active) FontWeight.Medium else FontWeight.Normal,
          maxLines = 1,
          softWrap = false,
        )
      }
    }
  }
}

private data class TileItem(val caption: String, val value: String, val unit: String?)

@Composable
private fun UsageTilesCard(all: UsageSummary, streak: Int, wide: Boolean) {
  val tokens = UsageStats.figure(all.tokens)
  val work = UsageStats.duration(all.workMs)
  val longest = UsageStats.duration(all.longestMs)
  val items = listOf(
    TileItem(t("usagePage.fig.tokens"), tokens.value, tokens.unit),
    TileItem(t("usagePage.fig.work"), work.first, work.second),
    TileItem(t("usagePage.fig.longest"), longest.first, longest.second),
    TileItem(t("usagePage.fig.streak"), "$streak", t("usagePage.unit.days")),
  )
  FormCard {
    if (wide) {
      Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
        items.forEachIndexed { idx, tile ->
          if (idx > 0) Box(Modifier.width(Metrics.hairline).fillMaxHeight().padding(vertical = Metrics.s3).background(Palette.line))
          TileCell(tile, Modifier.weight(1f))
        }
      }
    } else {
      Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
        TileCell(items[0], Modifier.weight(1f))
        Box(Modifier.width(Metrics.hairline).fillMaxHeight().padding(top = Metrics.s3).background(Palette.line))
        TileCell(items[1], Modifier.weight(1f))
      }
      HorizontalDivider(color = Palette.line, modifier = Modifier.padding(horizontal = Metrics.pad))
      Row(Modifier.fillMaxWidth().height(IntrinsicSize.Min)) {
        TileCell(items[2], Modifier.weight(1f))
        Box(Modifier.width(Metrics.hairline).fillMaxHeight().padding(bottom = Metrics.s3).background(Palette.line))
        TileCell(items[3], Modifier.weight(1f))
      }
    }
  }
}

@Composable
private fun TileCell(tile: TileItem, modifier: Modifier = Modifier) {
  Column(
    modifier.padding(horizontal = Metrics.pad, vertical = Metrics.s3),
    verticalArrangement = Arrangement.spacedBy(Metrics.s1_5),
  ) {
    Text(tile.caption, color = Palette.fg3, fontSize = Type.text4, maxLines = 1, overflow = TextOverflow.Ellipsis)
    Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
      Text(
        tile.value,
        color = Palette.fg1,
        fontSize = Type.usageFigure,
        lineHeight = Type.usageFigureLine,
        fontWeight = FontWeight.SemiBold,
        style = TNUM,
        maxLines = 1,
        softWrap = false,
        modifier = Modifier.alignByBaseline(),
      )
      if (tile.unit != null) {
        Text(
          tile.unit,
          color = Palette.fg2,
          fontSize = Type.text3,
          maxLines = 1,
          softWrap = false,
          modifier = Modifier.alignByBaseline(),
        )
      }
    }
  }
}

@Composable
private fun UsageYearDots(
  days: Map<Long, UsageDay>,
  today: Long,
  pick: ((UsageDay) -> Long)?,
  focusActive: Boolean,
  labels: UsageModelLabels,
  zone: ZoneId,
) {
  var inspected by remember { mutableStateOf<Long?>(null) }
  val haptic = LocalHapticFeedback.current
  val density = LocalDensity.current

  fun valueOf(d: UsageDay?): Long = d?.let { pick?.invoke(it) ?: it.tokens } ?: 0L
  fun topModel(d: UsageDay?): String? {
    if (focusActive || d == null) return null
    val best = d.byModel.entries.filter { it.key.isNotEmpty() && it.value > 0L }.maxByOrNull { it.value }?.key ?: return null
    return labels.label[best]?.name ?: best
  }

  val activeDays = remember(days, pick) { days.values.count { valueOf(it) > 0L } }
  val reading = inspected?.let { day ->
    val entry = days[day]
    UsageReading(tokens = valueOf(entry), date = UsageStats.dayLabel(day, zone = zone), topModel = topModel(entry))
  } ?: UsageReading(activeDays = activeDays)

  val ink = Palette.usageInk
  val cell = Palette.usageCell
  val strokeColor = Palette.fg2

  Column(verticalArrangement = Arrangement.spacedBy(Metrics.usageGroupHeadGap)) {
    UsageSectionHead(t("usagePage.activity"), reading = reading)
    BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Metrics.s2)) {
      val widthPx = with(density) { maxWidth.toPx() }
      val minPitchPx = with(density) { Metrics.usageDotPitchMin.toPx() }
      val minGapPx = with(density) { Metrics.usageDotGapMin.toPx() }
      val maxCornerPx = with(density) { Metrics.usageDotCornerMax.toPx() }
      val monthGapPx = with(density) { Metrics.usageMonthGap.toPx() }
      val monthEdgePx = with(density) { Metrics.usageMonthEdge.toPx() }
      val outsetPx = with(density) { Metrics.usageFocusOutset.toPx() }
      val hairlinePx = with(density) { Metrics.hairline.toPx() }

      val lastMonday = UsageStats.addDays(today, -UsageStats.mondayOffset(today, zone), zone)
      val cols = max(8, min(53, floor(widthPx / minPitchPx).toInt()))
      val gap = if (widthPx > 0f) max(minGapPx, (widthPx / cols * 0.2f).roundToInt().toFloat()) else minGapPx
      val pitch = if (cols > 0) (widthPx + gap) / cols else minPitchPx
      val size = max(1f, pitch - gap)
      val corner = min(maxCornerPx, size / 4f)
      val heightPx = 7f * size + 6f * gap
      val heightDp = with(density) { heightPx.toDp() }
      val start = UsageStats.addDays(lastMonday, -(cols - 1) * 7, zone)
      val maxVal = remember(days, start, pick) {
        max(1L, days.values.fold(0L) { acc, d -> if (d.day >= start) max(acc, valueOf(d)) else acc })
      }

      val months = remember(cols, start, widthPx, pitch) {
        val out = ArrayList<Triple<Float, String, Int>>()
        for (c in 0 until cols) {
          val weekEnd = UsageStats.addDays(start, c * 7 + 6, zone)
          val dom = Instant.ofEpochMilli(weekEnd).atZone(zone).dayOfMonth
          if (dom > 7) continue
          val x = c * pitch
          val prev = out.lastOrNull()
          if (prev != null && x - prev.first < monthGapPx) continue
          val edge = if (x < monthEdgePx) -1 else if (x > widthPx - monthEdgePx) 1 else 0
          out.add(Triple(x, UsageStats.monthShort(weekEnd, zone = zone), edge))
        }
        out
      }

      fun hitDay(pos: Offset): Long? {
        if (widthPx <= 0f || pitch <= 0f) return null
        val c = floor((pos.x + gap / 2f) / pitch).toInt()
        val k = floor((pos.y + gap / 2f) / pitch).toInt()
        if (c !in 0 until cols || k !in 0 until 7) return null
        val day = UsageStats.addDays(start, c * 7 + k, zone)
        return if (day <= today) day else null
      }

      Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
        Canvas(
          Modifier
            .fillMaxWidth()
            .height(heightDp)
            .pointerInput(start, cols, pitch, gap, today) {
              detectTapGestures { pos ->
                val next = hitDay(pos)
                if (next != null && next == inspected) {
                  inspected = null
                } else {
                  if (next != null) haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                  inspected = next
                }
              }
            }
            .pointerInput(start, cols, pitch, gap, today) {
              detectHorizontalDragGestures { change, _ ->
                val next = hitDay(change.position)
                if (next != inspected) {
                  if (next != null) haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                  inspected = next
                }
              }
            },
        ) {
          for (c in 0 until cols) {
            val x = c * pitch
            for (k in 0 until 7) {
              val day = UsageStats.addDays(start, c * 7 + k, zone)
              if (day > today) continue
              val y = k * pitch
              val v = valueOf(days[day])
              val fill = if (v > 0L) {
                val step = max(1, ceil(sqrt(v.toDouble() / maxVal.toDouble()) * UsageStats.DOT_STEPS).toInt())
                ink.copy(alpha = ink.alpha * (step.toFloat() / UsageStats.DOT_STEPS))
              } else {
                cell
              }
              drawRoundRect(fill, Offset(x, y), Size(size, size), CornerRadius(corner, corner))
              if (inspected == day) {
                drawRoundRect(
                  strokeColor,
                  Offset(x - outsetPx, y - outsetPx),
                  Size(size + outsetPx * 2f, size + outsetPx * 2f),
                  CornerRadius(corner + outsetPx, corner + outsetPx),
                  style = Stroke(width = hairlinePx),
                )
              }
            }
          }
        }
        AxisLabelsRow(months, widthPx)
      }
    }
  }
}

private data class TrendCol(val start: Long, val end: Long, val total: Long, val part: Long)

@Composable
private fun UsageTrend(
  days: Map<Long, UsageDay>,
  hours: Map<Long, UsageDay>,
  today: Long,
  now: Long,
  period: UsagePeriod,
  onPeriod: (UsagePeriod) -> Unit,
  pick: ((UsageDay) -> Long)?,
  focusActive: Boolean,
  labels: UsageModelLabels,
  zone: ZoneId,
) {
  var inspected by remember { mutableStateOf<Int?>(null) }
  val haptic = LocalHapticFeedback.current
  val density = LocalDensity.current

  val layout = remember(period, today, days, now, zone) {
    UsageStats.trendColumns(period, today, UsageStats.firstDay(days, today), now, zone)
  }
  val fold = if (layout.step == UsageStep.Hours) hours else days
  val advance: (Long) -> Long = remember(layout.step, zone) {
    if (layout.step == UsageStep.Hours) { d -> UsageStats.addHours(d, 1, zone) }
    else { d -> UsageStats.addDays(d, 1, zone) }
  }
  val cols = remember(layout, fold, pick) {
    layout.cols.map { span ->
      var total = 0L
      var part = 0L
      var d = span.start
      while (d < span.end) {
        val u = fold[d]
        if (u != null) {
          total += u.tokens
          if (pick != null) part += pick(u)
        }
        val next = advance(d)
        if (next <= d) break
        d = next
      }
      TrendCol(span.start, span.end, total, part)
    }
  }
  val n = cols.size
  val safeIdx = inspected?.takeIf { it in 0 until n }

  val reading = remember(safeIdx, cols, layout.step, focusActive, pick, fold, labels, zone) {
    if (safeIdx != null) {
      val c = cols[safeIdx]
      val date = when (layout.step) {
        UsageStep.Hours -> {
          val hr = Instant.ofEpochMilli(c.start).atZone(zone).hour
          "${UsageStats.dayLabel(c.start, zone = zone)} $hr:00"
        }
        UsageStep.Day -> UsageStats.dayLabel(c.start, zone = zone)
        UsageStep.Week -> UsageStats.weekRange(c.start, UsageStats.addDays(c.end, -1, zone), zone)
        UsageStep.Month -> UsageStats.monthLong(c.start, zone = zone)
      }
      var top: String? = null
      if (!focusActive) {
        val byModel = HashMap<String, Long>()
        var d = c.start
        while (d < c.end) {
          fold[d]?.byModel?.forEach { (k, v) -> if (k.isNotEmpty()) byModel[k] = (byModel[k] ?: 0L) + v }
          val next = advance(d)
          if (next <= d) break
          d = next
        }
        top = byModel.entries.filter { it.value > 0L }.maxByOrNull { it.value }?.key?.let { labels.label[it]?.name ?: it }
      }
      UsageReading(tokens = if (pick != null) c.part else c.total, date = date, topModel = top)
    } else {
      UsageReading(tokens = cols.sumOf { if (pick != null) it.part else it.total })
    }
  }

  val ink = Palette.usageInk
  val bar = Palette.usageBar
  val dim = Palette.usageDim
  val line = Palette.line

  Column(verticalArrangement = Arrangement.spacedBy(Metrics.usageGroupHeadGap)) {
    UsageSectionHead(t("usagePage.trend"), reading = reading) {
      UsageSegmented(
        items = UsagePeriod.entries.map { it to it.label() },
        selected = period,
        onSelect = { inspected = null; onPeriod(it) },
        tagPrefix = "usage.period",
      )
    }
    BoxWithConstraints(Modifier.fillMaxWidth().padding(horizontal = Metrics.s2)) {
      val widthPx = with(density) { maxWidth.toPx() }
      val barMaxPx = with(density) { Metrics.usageBarMax.toPx() }
      val barMinPx = with(density) { Metrics.usageBarMin.toPx() }
      val barMinHPx = with(density) { Metrics.usageBarMinH.toPx() }
      val barRadMaxPx = with(density) { Metrics.usageBarRadiusMax.toPx() }
      val labelRoomPx = with(density) { Metrics.usageLabelRoom.toPx() }
      val hairlinePx = with(density) { Metrics.hairline.toPx() }

      val bw = max(barMinPx, min(barMaxPx, (widthPx / max(1, n)) * 0.64f))
      val pitch = if (n > 1) (widthPx - bw) / (n - 1) else 0f
      fun colX(i: Int) = if (n > 1) i * pitch else (widthPx - bw) / 2f
      val maxVal = max(1L, cols.maxOfOrNull { it.total } ?: 1L)

      val ticks = remember(cols, layout.step, widthPx, bw, pitch, zone) {
        val out = ArrayList<Triple<Float, String, Int>>()
        fun tickLabel(ms: Long) = if (layout.step == UsageStep.Month) UsageStats.monthShort(ms, zone = zone) else UsageStats.numericDate(ms, zone)
        if (layout.step == UsageStep.Hours) {
          val perDay = 24 / UsageStats.TREND_SPAN_HOURS
          cols.forEachIndexed { i, c ->
            if (Instant.ofEpochMilli(c.start).atZone(zone).hour == 0) {
              val mid = (colX(i) + colX(min(i + perDay - 1, n - 1)) + bw) / 2f
              out.add(Triple(mid, tickLabel(c.start), 0))
            }
          }
        } else {
          val every = if (layout.step == UsageStep.Day) {
            if (widthPx / max(1, n) >= labelRoomPx) 1 else 7
          } else {
            max(1, ceil((n * labelRoomPx) / max(1f, widthPx)).toInt())
          }
          var i = n - 1
          while (i >= 0) {
            val edge = if (n > 1 && i == 0) -1 else if (n > 1 && i == n - 1) 1 else 0
            out.add(Triple(colX(i) + bw / 2f, tickLabel(cols[i].start), edge))
            i -= every
          }
        }
        out
      }

      fun hitBar(pos: Offset): Int? {
        if (widthPx <= 0f || n == 0) return null
        if (n == 1) return 0
        return max(0, min(n - 1, ((pos.x - bw / 2f) / pitch).roundToInt()))
      }

      Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
        Canvas(
          Modifier
            .fillMaxWidth()
            .height(Metrics.usageTrendH)
            .pointerInput(n, pitch, bw) {
              detectTapGestures { pos ->
                val next = hitBar(pos)
                if (next != null && next == inspected) {
                  inspected = null
                } else {
                  if (next != null) haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                  inspected = next
                }
              }
            }
            .pointerInput(n, pitch, bw) {
              detectHorizontalDragGestures { change, _ ->
                val next = hitBar(change.position)
                if (next != inspected) {
                  if (next != null) haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                  inspected = next
                }
              }
            },
        ) {
          val h = size.height
          drawLine(line, Offset(0f, h - hairlinePx / 2f), Offset(size.width, h - hairlinePx / 2f), strokeWidth = hairlinePx)

          fun drawBar(i: Int, v: Long, color: androidx.compose.ui.graphics.Color) {
            if (v <= 0L) return
            val bh = max(barMinHPx, (v.toFloat() / maxVal.toFloat()) * (h - barMinHPx))
            val x0 = colX(i)
            val r = min(barRadMaxPx, min(bw / 2f, bh))
            val yTop = h - bh
            val path = Path().apply {
              moveTo(x0, h)
              lineTo(x0, yTop + r)
              quadraticTo(x0, yTop, x0 + r, yTop)
              lineTo(x0 + bw - r, yTop)
              quadraticTo(x0 + bw, yTop, x0 + bw, yTop + r)
              lineTo(x0 + bw, h)
              close()
            }
            drawPath(path, color)
          }

          cols.forEachIndexed { i, c ->
            if (c.total <= 0L) return@forEachIndexed
            val lit = if (safeIdx != null) safeIdx == i else pick == null
            val fill = if (safeIdx == i) ink else if (lit) bar else dim
            drawBar(i, c.total, fill)
            if (pick != null && safeIdx == null && c.part > 0L) {
              drawBar(i, c.part, ink)
            }
          }
        }
        AxisLabelsRow(ticks, widthPx)
      }
    }
  }
}

@Composable
private fun AxisLabelsRow(items: List<Triple<Float, String, Int>>, widthPx: Float) {
  val density = LocalDensity.current
  val edgePx = with(density) { Metrics.usageMonthEdge.toPx() }
  Box(Modifier.fillMaxWidth().height(with(density) { Type.text4Line.toDp() })) {
    items.forEach { (x, label, edge) ->
      val align = when (edge) {
        -1 -> Alignment.CenterStart
        1 -> Alignment.CenterEnd
        else -> Alignment.CenterStart
      }
      val offsetPx = when (edge) {
        -1 -> 0
        1 -> 0
        else -> (x - edgePx).coerceIn(0f, max(0f, widthPx - edgePx * 2f)).roundToInt()
      }
      Text(
        label,
        color = Palette.fg3,
        fontSize = Type.text4,
        style = TNUM,
        maxLines = 1,
        softWrap = false,
        modifier = Modifier
          .align(align)
          .then(if (edge == 0) Modifier.offset { IntOffset(offsetPx, 0) } else Modifier),
      )
    }
  }
}

@Composable
private fun UsageBreakdownCard(
  dim: UsageDim,
  onDim: (UsageDim) -> Unit,
  items: List<UsagePart>,
  labels: UsageModelLabels,
  agents: List<AgentInfo>,
  focus: String?,
  onFocus: (String?) -> Unit,
  wide: Boolean,
) {
  var expanded by remember(dim) { mutableStateOf(false) }
  val haptic = LocalHapticFeedback.current

  fun agentName(id: String) = agents.firstOrNull { it.id == id }?.name ?: id
  fun modelName(key: String) = labels.label[key]?.name ?: key
  fun projectLabel(key: String) = if (key == UsageStats.CHATS_KEY) t("session.group.chats") else projectName(RemoteFolder.parse(key)?.path ?: key)
  fun rowName(key: String) = when (dim) {
    UsageDim.Model -> modelName(key)
    UsageDim.Agent -> agentName(key)
    UsageDim.Project -> projectLabel(key)
  }
  fun rowDetail(key: String): String? {
    if (dim != UsageDim.Project || key == UsageStats.CHATS_KEY) return null
    RemoteFolder.parse(key)?.let { return it.alias }
    val label = projectLabel(key)
    val twin = items.any { it.key != key && it.key != UsageStats.CHATS_KEY && projectLabel(it.key) == label }
    if (!twin) return null
    val cut = max(key.lastIndexOf('/'), key.lastIndexOf('\\'))
    return if (cut > 0) projectName(key.substring(0, cut)) else null
  }

  val folds = items.size > UsageStats.KEEP_ROWS + 1
  val head = if (folds) items.take(UsageStats.KEEP_ROWS) else items
  val rest = if (folds) items.drop(UsageStats.KEEP_ROWS) else emptyList()
  val others = UsagePart("*", rest.sumOf { it.tokens }, rest.sumOf { it.share })

  Column(verticalArrangement = Arrangement.spacedBy(Metrics.usageGroupHeadGap)) {
    UsageSectionHead(t("usagePage.breakdown"), count = items.size) {
      UsageSegmented(
        items = UsageDim.entries.map { it to it.label() },
        selected = dim,
        onSelect = onDim,
        tagPrefix = "usage.dim",
      )
    }

    if (items.isNotEmpty()) {
      FormCard {
        head.forEachIndexed { idx, part ->
          if (idx > 0) HorizontalDivider(color = Palette.line, modifier = Modifier.padding(horizontal = Metrics.pad))
          BreakdownRow(
            dim = dim, part = part, name = rowName(part.key), detail = rowDetail(part.key),
            brand = if (dim == UsageDim.Model) labels.label[part.key]?.brand else part.key,
            isMore = false, dimmed = focus != null && focus != part.key, wide = wide,
            onClick = {
              haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
              onFocus(if (focus == part.key) null else part.key)
            },
          )
        }
        if (folds) {
          HorizontalDivider(color = Palette.line, modifier = Modifier.padding(horizontal = Metrics.pad))
          if (expanded) {
            val firstRest = rest.first()
            BreakdownRow(
              dim = dim, part = firstRest, name = rowName(firstRest.key), detail = rowDetail(firstRest.key),
              brand = if (dim == UsageDim.Model) labels.label[firstRest.key]?.brand else firstRest.key,
              isMore = false, dimmed = focus != null && focus != firstRest.key, wide = wide,
              onClick = {
                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                onFocus(if (focus == firstRest.key) null else firstRest.key)
              },
            )
          } else {
            BreakdownRow(
              dim = dim, part = others, name = t(dim.moreKey(), "n" to rest.size), detail = null,
              brand = null, isMore = true, dimmed = focus != null, wide = wide,
              onClick = {
                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                onFocus(null)
                expanded = true
              },
            )
          }
          AnimatedVisibility(visible = expanded, enter = expandVertically() + fadeIn()) {
            Column {
              rest.drop(1).forEach { part ->
                HorizontalDivider(color = Palette.line, modifier = Modifier.padding(horizontal = Metrics.pad))
                BreakdownRow(
                  dim = dim, part = part, name = rowName(part.key), detail = rowDetail(part.key),
                  brand = if (dim == UsageDim.Model) labels.label[part.key]?.brand else part.key,
                  isMore = false, dimmed = focus != null && focus != part.key, wide = wide,
                  onClick = {
                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    onFocus(if (focus == part.key) null else part.key)
                  },
                )
              }
            }
          }
        }
      }
    }
  }
}

@Composable
private fun BreakdownRow(
  dim: UsageDim,
  part: UsagePart,
  name: String,
  detail: String?,
  brand: String?,
  isMore: Boolean,
  dimmed: Boolean,
  wide: Boolean,
  onClick: () -> Unit,
) {
  val fig = UsageStats.figure(part.tokens)
  val alpha = if (dimmed) Metrics.usageDimRow else 1f

  Box(Modifier.fillMaxWidth().alpha(alpha).clickable(onClick = onClick).padding(horizontal = Metrics.pad, vertical = Metrics.s3)) {
    if (wide) {
      Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.gap)) {
        BreakdownMark(dim, part.key, name, brand, isMore)
        BreakdownName(name, detail, isMore, Modifier.width(Metrics.usageNameWide))
        ShareBar(part.share, isMore, Modifier.weight(1f))
        TokenText(fig, Modifier.width(Metrics.usageNumW))
        PercentText(part.share, Modifier.width(Metrics.usagePctW))
      }
    } else {
      Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.gap)) {
        BreakdownMark(dim, part.key, name, brand, isMore)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
          Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
            BreakdownName(name, detail, isMore, Modifier.weight(1f))
            TokenText(fig)
            PercentText(part.share, Modifier.width(Metrics.usagePctW))
          }
          ShareBar(part.share, isMore, Modifier.fillMaxWidth())
        }
      }
    }
  }
}

@Composable
private fun BreakdownMark(dim: UsageDim, key: String, name: String, brand: String?, isMore: Boolean) {
  Box(Modifier.size(Metrics.lead), contentAlignment = Alignment.Center) {
    when {
      isMore -> Lucide("ellipsis", Palette.fg3, size = Metrics.icon)
      dim == UsageDim.Model -> VendorMark(brand, name, Palette.fg2, size = Metrics.icon, fit = true)
      dim == UsageDim.Agent -> VendorMark(key, name, Palette.fg2, size = Metrics.icon, fit = true)
      key == UsageStats.CHATS_KEY -> Lucide("messageSquare", Palette.fg2, size = Metrics.icon)
      RemoteFolder.parse(key) != null -> RemoteFolderGlyph(false)
      else -> Lucide("folder", Palette.fg2, size = Metrics.icon)
    }
  }
}

@Composable
private fun BreakdownName(name: String, detail: String?, isMore: Boolean, modifier: Modifier = Modifier) {
  Row(modifier, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
    Text(
      name,
      color = if (isMore) Palette.fg2 else Palette.fg1,
      fontSize = Type.text3,
      maxLines = 1,
      overflow = TextOverflow.Ellipsis,
      modifier = Modifier.weight(1f, fill = false),
    )
    if (!isMore && detail != null) {
      Text(detail, color = Palette.fg3, fontSize = Type.text4, maxLines = 1, softWrap = false)
    }
  }
}

@Composable
private fun ShareBar(share: Double, muted: Boolean, modifier: Modifier = Modifier) {
  val clamped = share.toFloat().coerceIn(0f, 1f)
  Box(modifier.height(Metrics.usageTrackH).clip(CircleShape).background(Palette.usageTrack)) {
    Box(
      Modifier
        .fillMaxHeight()
        .fillMaxWidth(clamped)
        .clip(CircleShape)
        .background(if (muted) Palette.usageDim else Palette.usageInk),
    )
  }
}

@Composable
private fun TokenText(fig: UsageFigure, modifier: Modifier = Modifier) {
  Row(
    modifier,
    horizontalArrangement = Arrangement.End,
  ) {
    Text(fig.value, color = Palette.fg1, fontSize = Type.text3, style = TNUM, maxLines = 1, softWrap = false, modifier = Modifier.alignByBaseline())
    if (fig.unit != null) {
      Spacer(Modifier.width(Metrics.s0_5))
      Text(fig.unit, color = Palette.fg3, fontSize = Type.text4, maxLines = 1, softWrap = false, modifier = Modifier.alignByBaseline())
    }
  }
}

@Composable
private fun PercentText(share: Double, modifier: Modifier = Modifier) {
  Text(
    UsageStats.percent(share),
    color = Palette.fg3,
    fontSize = Type.text4,
    style = TNUM,
    textAlign = TextAlign.End,
    maxLines = 1,
    softWrap = false,
    modifier = modifier,
  )
}
