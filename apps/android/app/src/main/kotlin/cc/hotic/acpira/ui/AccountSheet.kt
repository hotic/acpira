package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.TextUnit
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.avatar
import cc.hotic.acpira.design.popActive
import cc.hotic.acpira.design.quotaDanger
import cc.hotic.acpira.design.quotaOk
import cc.hotic.acpira.design.quotaTrack
import cc.hotic.acpira.design.quotaWarn
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AccountInfo
import cc.hotic.acpira.model.AccountQuota
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
          Column(section) {
            if (known) {
              AccountRow(local.label, plan = local.detail, caption = t("quota.officialAccount"), quota = local.quota, now = now,
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
              SwipeToRemove(onRemove = { onRemove(a.id) }) {
                Box(Modifier.background(Palette.formRow).clickable { onSelect(a.id); onDismiss() }.testTag("account:${a.id}")) {
                  AccountRow(a.label, caption = a.detail, checked = a.id == accountId, checkSlot = true, quota = a.quota, now = now)
                }
              }
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

/**
 * One login (NativeAccountRow): an avatar, the name with its plan as a tag over a caption, then the quota windows. The
 * tick keeps its slot in a list of logins, so picking one moves nothing
 */
@Composable
private fun AccountRow(
  title: String,
  plan: String? = null,
  caption: String? = null,
  checked: Boolean = false,
  checkSlot: Boolean = false,
  quota: AccountQuota? = null,
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
          plan?.let {
            Text(
              it, color = Palette.fg1, fontSize = Type.text4, fontWeight = FontWeight.Medium, maxLines = 1,
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
    if (quota != null) QuotaBars(quota, now, Type.text2, Palette.fg1)
    else status?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line) }
  }
}

/**
 * QuotaBars.tsx: per window a line (name, the time to its reset as a badge, what is left) over a tube whose fill turns
 * warn under 30 % and danger under 10 %
 */
@Composable
fun QuotaBars(quota: AccountQuota, now: () -> Instant, size: TextUnit = Type.text3, ink: Color = Palette.fg2) {
  // The countdowns move on by the minute
  val clock by produceState(now()) { while (true) { delay(MINUTE_MS); value = now() } }
  Column(Modifier.padding(top = Metrics.s2), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    for (w in quota.windows) QuotaBar(w, clock, size, ink)
    quota.onDemandBalanceUsd?.takeIf { it.isFinite() }?.let { balance ->
      Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
        Text(t("quota.onDemandBalance"), color = ink, fontSize = size, maxLines = 1, modifier = Modifier.weight(1f))
        Text(
          NumberFormat.getCurrencyInstance(Locale.forLanguageTag(L10n.locale)).apply { currency = Currency.getInstance("USD") }.format(balance),
          color = ink, fontSize = size,
        )
      }
    }
  }
}

@Composable
private fun QuotaBar(window: QuotaWindow, now: Instant, size: TextUnit, ink: Color) {
  val pct = (window.remaining * 100).roundToInt().coerceIn(0, 100)
  val resets = window.resetsAt?.let { runCatching { Instant.parse(it) }.getOrNull() }
  val reset = when {
    resets != null -> if (resets.isAfter(now)) QuotaTime.until(resets, now, compact = true) else t("quota.resetPending")
    else -> t(if (window.remaining >= 1) "quota.resetIdle" else "quota.resetUnknown")
  }
  val fill = when {
    window.remaining <= DANGER_SHARE -> Palette.quotaDanger
    window.remaining <= WARN_SHARE -> Palette.quotaWarn
    else -> Palette.quotaOk
  }
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.s1), modifier = Modifier.testTag("quota:${window.id}")) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      Text(QuotaTime.windowLabel(window.id), color = ink, fontSize = size, maxLines = 1, softWrap = false)
      // The badge takes the line's room and truncates in it, what is left stays flush right
      Box(Modifier.weight(1f)) {
        Text(
          reset, color = ink, fontSize = size, maxLines = 1, overflow = TextOverflow.Ellipsis,
          modifier = Modifier.background(Palette.popActive, RoundedCornerShape(Metrics.rSm)).padding(horizontal = Metrics.s1),
        )
      }
      Text(t("quota.left", "pct" to pct), color = ink, fontSize = size, maxLines = 1, softWrap = false, textAlign = TextAlign.End)
    }
    Box(Modifier.fillMaxWidth().height(Metrics.quotaTrack).clip(CircleShape).background(Palette.popActive)) {
      Box(Modifier.fillMaxWidth(pct / 100f).height(Metrics.quotaTrack).clip(CircleShape).background(fill))
    }
  }
}

/** The reset countdown's wording (QuotaBars.tsx fmtUntil): up to two units, compact units run together in CJK */
object QuotaTime {
  /** Known window ids translated, anything else shown as sent */
  fun windowLabel(id: String): String {
    val key = "quota.window.$id"
    return t(key).let { if (it == key) id else it }
  }

  fun until(date: Instant, now: Instant, compact: Boolean = false): String {
    val minutes = ceil(maxOf(0L, date.toEpochMilli() - now.toEpochMilli()) / MINUTE_MS.toDouble()).toInt()
    fun unit(name: String, n: Int) = t(if (compact) "quota.time.short.$name" else "quota.time.$name", "n" to n)
    val separator = if (compact && L10n.locale in setOf("zh-CN", "zh-TW", "ja")) "" else " "
    if (minutes < MINUTES_PER_HOUR) return unit("minutes", minutes)
    val hours = minutes / MINUTES_PER_HOUR
    if (hours < HOURS_PER_DAY) {
      return listOfNotNull(unit("hours", hours), (minutes % MINUTES_PER_HOUR).takeIf { it > 0 }?.let { unit("minutes", it) }).joinToString(separator)
    }
    return listOfNotNull(unit("days", hours / HOURS_PER_DAY), (hours % HOURS_PER_DAY).takeIf { it > 0 }?.let { unit("hours", it) }).joinToString(separator)
  }
}

private const val MINUTE_MS = 60_000L
private const val MINUTES_PER_HOUR = 60
private const val HOURS_PER_DAY = 24
private const val WARN_SHARE = 0.3
private const val DANGER_SHARE = 0.1
