package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.avatar
import cc.hotic.acpira.design.popActive
import cc.hotic.acpira.design.quotaDanger
import cc.hotic.acpira.design.quotaOk
import cc.hotic.acpira.design.quotaReset
import cc.hotic.acpira.design.quotaStale
import cc.hotic.acpira.design.quotaTrack
import cc.hotic.acpira.design.quotaWarn
import cc.hotic.acpira.design.quotaWide
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AccountInfo
import cc.hotic.acpira.model.AccountQuota
import cc.hotic.acpira.model.AccountQuotaIssue
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.QuotaWindow
import kotlinx.coroutines.delay
import java.text.NumberFormat
import java.time.Instant
import java.util.Currency
import java.util.Locale
import kotlin.math.ceil
import kotlin.math.roundToInt

// The account sheet (apps/ios Design/NativeChrome.swift NativeAccountSheet, Chat/AccountQuota.swift QuotaBars): the
// agent's logins with their allowance windows, or the CLI's own account; a pick switches the session's login.

/** True when the agent has something for the account sheet: stored logins (Devin) or the CLI's own account (Grok, Kimi) */
fun hasAccountMenu(agent: AgentInfo): Boolean = agent.accounts == true || agent.localAccount != null

/**
 * The accounts as a sheet with a form (NativeAccountSheet): the CLI's own account with its allowance, or the stored logins
 * with a tick on the session's, each with its quota windows; a swipe removes one. The quotas are refreshed as the sheet
 * comes up ([onAppear]), as the desktop does when an account list comes into view
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AccountSheet(
  agent: AgentInfo,
  accounts: List<AccountInfo>,
  accountId: String?,
  now: () -> Instant,
  onSelect: (String) -> Unit,
  onRemove: (String) -> Unit,
  onAppear: () -> Unit,
  onDismiss: () -> Unit,
) {
  val state = rememberModalBottomSheetState(skipPartiallyExpanded = false)
  LaunchedEffect(Unit) { onAppear() }
  ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state, containerColor = Palette.formPage, dragHandle = null) {
    Column(Modifier.fillMaxWidth().navigationBarsPadding().semantics { testTagsAsResourceId = true }.testTag("account.sheet")) {
      // The bar: close, and the title with the agent whose logins these are under it
      Box(Modifier.fillMaxWidth().heightIn(min = Metrics.barCtl + Metrics.s2).padding(horizontal = Metrics.s1)) {
        IconButton(onClick = onDismiss, modifier = Modifier.align(Alignment.CenterStart).size(Metrics.barCtl).testTag("account.close")) {
          Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
        }
        Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
          Text(t("common.account"), color = Palette.fgStrong, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, maxLines = 1)
          Text(agent.name, color = Palette.fg3, fontSize = Type.text4, lineHeight = Type.text4Line, maxLines = 1)
        }
      }
      Column(
        Modifier.verticalScroll(rememberScrollState()).padding(horizontal = Metrics.pad).padding(top = Metrics.s2, bottom = Metrics.page),
        verticalArrangement = Arrangement.spacedBy(Metrics.s2),
      ) {
        val local = agent.localAccount
        val section = Modifier.fillMaxWidth().clip(RoundedCornerShape(Metrics.rLg)).background(Palette.formRow)
        if (local != null) {
          // Before a login is found the label is only the product name the bar already shows
          val known = local.label != agent.name || local.detail != null
          val stale = local.status != "ready" && local.status != "loading"
          Column(section) {
            if (known) {
              AccountRow(local.label, plan = local.detail, caption = t("quota.officialAccount"), quota = local.quota, stale = stale, now = now,
                status = if (local.quota == null) t("quota.status.${local.status}") else null)
            } else {
              AccountRow(if (local.status == "login_required") t("composer.notLoggedIn") else t("quota.officialAccount"), caption = t("quota.status.${local.status}"), now = now)
            }
          }
          if (local.quota != null) Footnote(t("quota.local.desc"))
        } else {
          Column(section) {
            if (accounts.isEmpty()) AccountRow(t("settings.agent.accounts.none"), caption = t("composer.noAccounts"), now = now)
            accounts.forEachIndexed { i, a ->
              if (i > 0) Box(Modifier.fillMaxWidth().padding(start = Metrics.pad + Metrics.avatar + Metrics.s3).height(Metrics.hairline).background(Palette.line))
              // A plugin's account carries its tag, with the plugin named under it; a failed read replaces the tag and dims last-known bars
              val issue = savedAccountIssue(agent, a)
              val issueStatus = if (a.quota == null) a.quotaIssue?.takeIf { it != AccountQuotaIssue.unknown }?.let { t("quota.status.${it.name}") } else null
              val row = @Composable {
                Box(Modifier.background(Palette.formRow).clickable { onSelect(a.id); onDismiss() }.testTag("account:${a.id}")) {
                  AccountRow(
                    a.label,
                    source = if (issue == null) a.badge else null,
                    issue = issue,
                    caption = if (a.badge == null) a.detail else a.plugin,
                    checked = a.id == accountId,
                    checkSlot = true,
                    quota = a.quota,
                    stale = issue != null,
                    status = issueStatus,
                    now = now,
                  )
                }
              }
              if (a.removable == false) row() else SwipeToRemove(onRemove = { onRemove(a.id) }, content = row)
            }
          }
          if (agent.credentialsLocked == true) Footnote(t("notice.locked.short"))
        }
      }
    }
  }
}

/** A row the user swipes toward the start to remove, the delete mark showing under it */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SwipeToRemove(onRemove: () -> Unit, content: @Composable () -> Unit) {
  val state = rememberSwipeToDismissBoxState(confirmValueChange = { value ->
    if (value == SwipeToDismissBoxValue.EndToStart) onRemove()
    value != SwipeToDismissBoxValue.EndToStart
  })
  SwipeToDismissBox(
    state = state,
    enableDismissFromStartToEnd = false,
    backgroundContent = {
      Box(Modifier.fillMaxWidth().fillMaxHeight().background(Palette.danger).padding(horizontal = Metrics.pad), contentAlignment = Alignment.CenterEnd) {
        Lucide("trash2", Color.White, size = Metrics.iconCtl, contentDescription = t("common.delete"))
      }
    },
  ) { content() }
}

@Composable
private fun Footnote(text: String) {
  Text(text, color = Palette.fg3, fontSize = Type.text4, lineHeight = Type.text4Line, modifier = Modifier.padding(horizontal = Metrics.pad))
}

private data class SavedAccountIssue(val text: String, val warn: Boolean)

private fun savedAccountIssue(agent: AgentInfo, account: AccountInfo): SavedAccountIssue? {
  if (agent.credentialsLocked == true && account.plugin == null) {
    return SavedAccountIssue(t("quota.short.locked"), warn = true)
  }
  val issue = account.quotaIssue?.takeIf { it != AccountQuotaIssue.unknown } ?: return null
  return SavedAccountIssue(t("quota.short.${issue.name}"), warn = issue == AccountQuotaIssue.expired)
}

/**
 * One login (NativeAccountRow): an avatar, the name with its plan/source/issue tag over a caption, then the desktop-
 * aligned quota bars. The tick keeps its slot in a list of logins, so picking one moves nothing
 */
@Composable
private fun AccountRow(
  title: String,
  plan: String? = null,
  source: String? = null,
  issue: SavedAccountIssue? = null,
  caption: String? = null,
  checked: Boolean = false,
  checkSlot: Boolean = false,
  quota: AccountQuota? = null,
  stale: Boolean = false,
  status: String? = null,
  now: () -> Instant,
) {
  Column(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s3), verticalArrangement = Arrangement.spacedBy(Metrics.s3)) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s3)) {
      Box(Modifier.size(Metrics.avatar).background(Palette.popActive, CircleShape), contentAlignment = Alignment.Center) {
        Lucide("userRound", Palette.fg2, size = Metrics.barIcon)
      }
      Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
          Text(title, color = Palette.fgStrong, fontSize = Type.body, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.MiddleEllipsis, modifier = Modifier.weight(1f, fill = false))
          when {
            issue != null -> Text(
              issue.text,
              color = if (issue.warn) Palette.warn else Palette.fg3,
              fontSize = Type.text4,
              fontWeight = FontWeight.Medium,
              maxLines = 1,
              modifier = Modifier
                .background(if (issue.warn) Palette.warn.copy(alpha = 0.15f) else Palette.popActive, CircleShape)
                .padding(horizontal = Metrics.s2, vertical = Metrics.s0_5),
            )
            source != null -> Text(
              source, color = Palette.accountSource, fontSize = Type.text4, maxLines = 1, overflow = TextOverflow.Ellipsis,
              modifier = Modifier.background(Palette.accountSource.copy(alpha = 0.1f), CircleShape)
                .border(Metrics.hairline, Palette.accountSource.copy(alpha = 0.25f), CircleShape)
                .padding(horizontal = Metrics.s2, vertical = Metrics.s0_5),
            )
            plan != null -> Text(
              plan, color = Palette.fg1, fontSize = Type.text4, fontWeight = FontWeight.Medium, maxLines = 1,
              modifier = Modifier.background(Palette.popActive, CircleShape).padding(horizontal = Metrics.s2, vertical = Metrics.s0_5),
            )
          }
        }
        caption?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line) }
      }
      if (checkSlot) Box(Modifier.size(Metrics.iconCtl), contentAlignment = Alignment.Center) {
        if (checked) Lucide("check", Palette.ok, size = Metrics.iconCtl)
      }
    }
    if (quota != null) QuotaBars(quota, now, stale = stale)
    else status?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line) }
  }
}

/**
 * QuotaBars.tsx: per window a metadata line over a slim tube whose fill turns warn under 30 % and danger under 10 %
 * (grey while stale). Windows in label pairs share one aligned row: the label once, then the two windows side by side
 */
@Composable
fun QuotaBars(
  quota: AccountQuota,
  now: () -> Instant,
  stale: Boolean = false,
  size: TextUnit = Type.text4,
  lineHeight: TextUnit = Type.text4Line,
) {
  // The countdowns move on by the minute, on the latest clock: the loop outlives a model swap (Rows.kt LiveElapsed)
  val latest by rememberUpdatedState(now)
  val clock by produceState(now()) { while (true) { delay(MINUTE_MS); value = latest() } }
  val balance = quota.onDemandBalanceUsd?.takeIf { it.isFinite() }
  if (quota.windows.isEmpty() && balance == null) return
  val pairs = QuotaTime.labelPairs(quota.windows)
  val pctInk = if (stale) Palette.fg3 else Palette.fg2
  val labelInk = if (stale) Palette.fg3 else Palette.fg1
  BoxWithConstraints(Modifier.fillMaxWidth()) {
    val wide = maxWidth >= Metrics.quotaWide
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      when {
        pairs != null -> PairedQuotaGrid(horizontalGap = Metrics.gap, verticalGap = Metrics.s2) {
          for ((a, b) in pairs) {
            Text(QuotaTime.windowLabel(a.id), color = labelInk, fontSize = size, lineHeight = lineHeight, maxLines = 1, softWrap = false)
            QuotaBar(a, clock, stale = stale, bare = true, wide = wide, size = size, lineHeight = lineHeight, labelInk = labelInk, pctInk = pctInk)
            QuotaBar(b, clock, stale = stale, bare = true, wide = wide, size = size, lineHeight = lineHeight, labelInk = labelInk, pctInk = pctInk)
          }
        }
        wide && quota.windows.size > 1 -> Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
          for (chunk in quota.windows.chunked(2)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Metrics.gap)) {
              QuotaBar(chunk[0], clock, stale = stale, bare = false, wide = wide, size = size, lineHeight = lineHeight, labelInk = labelInk, pctInk = pctInk, modifier = Modifier.weight(1f))
              if (chunk.size > 1) {
                QuotaBar(chunk[1], clock, stale = stale, bare = false, wide = wide, size = size, lineHeight = lineHeight, labelInk = labelInk, pctInk = pctInk, modifier = Modifier.weight(1f))
              } else {
                Spacer(Modifier.weight(1f))
              }
            }
          }
        }
        else -> Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
          for (w in quota.windows) {
            QuotaBar(w, clock, stale = stale, bare = false, wide = wide, size = size, lineHeight = lineHeight, labelInk = labelInk, pctInk = pctInk)
          }
        }
      }
      balance?.let {
        Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
          Text(t("quota.onDemandBalance"), color = pctInk, fontSize = size, lineHeight = lineHeight, maxLines = 1, modifier = Modifier.weight(1f))
          Text(
            NumberFormat.getCurrencyInstance(Locale.forLanguageTag(L10n.locale)).apply { currency = Currency.getInstance("USD") }.format(it),
            color = pctInk, fontSize = size, lineHeight = lineHeight,
          )
        }
      }
    }
  }
}

/** Three-column grid (`grid-cols-[auto_minmax(0,1fr)_minmax(0,1fr)]`) with vertically centred row labels */
@Composable
private fun PairedQuotaGrid(horizontalGap: Dp, verticalGap: Dp, content: @Composable () -> Unit) {
  Layout(content = content, modifier = Modifier.fillMaxWidth()) { measurables, constraints ->
    val hGap = horizontalGap.roundToPx()
    val vGap = verticalGap.roundToPx()
    val rows = measurables.size / 3
    val labels = (0 until rows).map { r ->
      measurables[r * 3].measure(Constraints(minWidth = 0, maxWidth = constraints.maxWidth))
    }
    val labelW = labels.maxOfOrNull { it.width } ?: 0
    val barW = ((constraints.maxWidth - labelW - hGap * 2) / 2).coerceAtLeast(0)
    val barConstraints = Constraints.fixedWidth(barW)
    val firstBars = (0 until rows).map { r -> measurables[r * 3 + 1].measure(barConstraints) }
    val secondBars = (0 until rows).map { r -> measurables[r * 3 + 2].measure(barConstraints) }
    val rowHeights = (0 until rows).map { r -> maxOf(labels[r].height, firstBars[r].height, secondBars[r].height) }
    val totalHeight = rowHeights.sum() + (rows - 1).coerceAtLeast(0) * vGap
    layout(constraints.maxWidth, totalHeight) {
      var y = 0
      for (r in 0 until rows) {
        val h = rowHeights[r]
        labels[r].placeRelative(0, y + (h - labels[r].height) / 2)
        firstBars[r].placeRelative(labelW + hGap, y + (h - firstBars[r].height) / 2)
        secondBars[r].placeRelative(labelW + hGap + barW + hGap, y + (h - secondBars[r].height) / 2)
        y += h + vGap
      }
    }
  }
}

@Composable
private fun QuotaBar(
  window: QuotaWindow,
  now: Instant,
  stale: Boolean,
  bare: Boolean,
  wide: Boolean,
  size: TextUnit,
  lineHeight: TextUnit,
  labelInk: Color,
  pctInk: Color,
  modifier: Modifier = Modifier,
) {
  val pct = (window.remaining * 100).roundToInt().coerceIn(0, 100)
  val reset = QuotaTime.resetText(window, now, bare = bare, wide = wide)
  val fill = when {
    stale -> Palette.quotaStale
    window.remaining <= DANGER_SHARE -> Palette.quotaDanger
    window.remaining <= WARN_SHARE -> Palette.quotaWarn
    else -> Palette.quotaOk
  }
  val aria = QuotaTime.accessibilityLabel(window, now)
  Column(
    verticalArrangement = Arrangement.spacedBy(Metrics.s1),
    modifier = modifier.fillMaxWidth().testTag("quota:${window.id}").clearAndSetSemantics { contentDescription = aria },
  ) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(if (bare && !wide) Metrics.s1 else Metrics.s2),
    ) {
      if (!bare) {
        Text(QuotaTime.windowLabel(window.id), color = labelInk, fontSize = size, lineHeight = lineHeight, maxLines = 1, softWrap = false)
      }
      Text(
        reset,
        color = Palette.quotaReset,
        fontSize = size,
        lineHeight = lineHeight,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier.weight(1f),
      )
      Text("$pct%", color = pctInk, fontSize = size, lineHeight = lineHeight, maxLines = 1, softWrap = false, textAlign = TextAlign.End)
    }
    Box(Modifier.fillMaxWidth().height(Metrics.quotaTrack).clip(CircleShape).background(Palette.popActive)) {
      Box(Modifier.fillMaxWidth(pct / 100f).height(Metrics.quotaTrack).clip(CircleShape).background(fill))
    }
  }
}

/** The reset countdown's wording and window pairing (QuotaBars.tsx): up to two units, compact units run together in CJK */
object QuotaTime {
  const val NO_RESET = "—"

  /** Known window ids translated, anything else shown as sent */
  fun windowLabel(id: String): String {
    val key = "quota.window.$id"
    return t(key).let { if (it == key) id else it }
  }

  /** The windows two by two when every such pair shares its label; null for any other list (QuotaBars.tsx labelPairs) */
  fun labelPairs(windows: List<QuotaWindow>): List<Pair<QuotaWindow, QuotaWindow>>? {
    if (windows.isEmpty() || windows.size % 2 != 0) return null
    val pairs = ArrayList<Pair<QuotaWindow, QuotaWindow>>(windows.size / 2)
    for (i in windows.indices step 2) {
      val a = windows[i]
      val b = windows[i + 1]
      if (windowLabel(a.id) != windowLabel(b.id)) return null
      pairs.add(a to b)
    }
    return pairs
  }

  fun until(date: Instant, now: Instant, compact: Boolean = false, largestOnly: Boolean = false): String {
    val minutes = ceil(maxOf(0L, date.toEpochMilli() - now.toEpochMilli()) / MINUTE_MS.toDouble()).toInt()
    fun unit(name: String, n: Int) = t(if (compact) "quota.time.short.$name" else "quota.time.$name", "n" to n)
    val separator = if (compact && L10n.locale in setOf("zh-CN", "zh-TW", "ja")) "" else " "
    if (minutes < MINUTES_PER_HOUR) return unit("minutes", minutes)
    val hours = minutes / MINUTES_PER_HOUR
    if (hours < HOURS_PER_DAY) {
      return listOfNotNull(unit("hours", hours), (minutes % MINUTES_PER_HOUR).takeIf { !largestOnly && it > 0 }?.let { unit("minutes", it) }).joinToString(separator)
    }
    return listOfNotNull(unit("days", hours / HOURS_PER_DAY), (hours % HOURS_PER_DAY).takeIf { !largestOnly && it > 0 }?.let { unit("hours", it) }).joinToString(separator)
  }

  fun resetLabel(window: QuotaWindow, now: Instant): String {
    val resets = window.resetsAt?.let { runCatching { Instant.parse(it) }.getOrNull() }
      ?: return t(if (window.remaining >= 1) "quota.resetIdle" else "quota.resetUnknown")
    if (!resets.isAfter(now)) return t("quota.resetPending")
    return t("quota.reset", "time" to until(resets, now))
  }

  fun resetText(window: QuotaWindow, now: Instant, bare: Boolean, wide: Boolean): String {
    val resets = window.resetsAt?.let { runCatching { Instant.parse(it) }.getOrNull() }
    val ahead = resets != null && resets.isAfter(now)
    if (bare && !wide) return if (ahead) until(resets, now, compact = true, largestOnly = true) else NO_RESET
    return if (ahead) until(resets, now, compact = true) else resetLabel(window, now)
  }

  fun accessibilityLabel(window: QuotaWindow, now: Instant): String {
    val pct = (window.remaining * 100).roundToInt().coerceIn(0, 100)
    return "${t("quota.aria", "window" to windowLabel(window.id), "pct" to pct)}${t("common.metaSep")}${resetLabel(window, now)}"
  }
}

private const val MINUTE_MS = 60_000L
private const val MINUTES_PER_HOUR = 60
private const val HOURS_PER_DAY = 24
private const val WARN_SHARE = 0.3
private const val DANGER_SHARE = 0.1
