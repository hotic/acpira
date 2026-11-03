package cc.hotic.acpira.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.FloatingActionButtonDefaults
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Connection
import cc.hotic.acpira.store.Screen
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** One captioned run of the list (置顶, 今天 …); dated runs leave the time off their rows, the caption says it */
private data class SessionGroup(val id: String, val caption: String?, val sessions: List<SessionSummary>, val dated: Boolean = false)

/**
 * The session list (apps/ios Sessions/SessionListScreen.swift, by time): pinned first, then today / yesterday / this
 * week / this month / older under captions. Material's centred top bar holds the view filter and settings; the search
 * field and the new-session button ride the bottom, where the iOS app keeps them
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionListScreen(model: AppModel) {
  var query by rememberSaveable { mutableStateOf("") }
  val now = model.now()
  val terms = query.trim().lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
  val listed = model.summaries.filter { it.archived != true }
    .filter { s -> terms.all { term -> listOf(s.title, model.agent(s.agent).name, model.placeName(s.cwd)).any { it.lowercase().contains(term) } } }
    .sortedByDescending { it.updatedAt }
  val groups = if (terms.isNotEmpty()) listOf(SessionGroup("all", null, listed)) else groups(listed, now)
  // Paired remote access: the followed computer's state takes the page until it has sessions to list. The bar sits over
  // both, so the frame giving way to the list moves nothing in it
  val pairing = LocalPairing.current
  val covered = pairing != null && covers(pairing, model)

  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      CenterAlignedTopAppBar(
        title = {
          if (pairing != null) ListBarTitle(pairing)
          else Text(t("mobile.home"), fontWeight = FontWeight.SemiBold, fontSize = Type.barTitle)
        },
        navigationIcon = {
          if (!covered) IconButton(onClick = {}, modifier = Modifier.testTag("list.view")) {
            Lucide("listFilter", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.view.group"))
          }
        },
        actions = {
          IconButton(onClick = { model.push(Screen.Settings) }, modifier = Modifier.testTag("list.settings")) {
            Lucide("settings", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.settings"))
          }
        },
        colors = TopAppBarDefaults.centerAlignedTopAppBarColors(containerColor = Palette.bg0, scrolledContainerColor = Palette.bg0),
      )
    },
    bottomBar = { if (!covered) ListBottomBar(query, { query = it }) { model.push(Screen.New()) } },
  ) { padding ->
    if (covered) {
      PairHeroList(pairing!!, Modifier.padding(padding))
    } else if (listed.isEmpty()) {
      EmptyList(model, Modifier.padding(padding))
    } else {
      LazyColumn(
        modifier = Modifier.fillMaxSize().testTag("list.sessions"),
        contentPadding = PaddingValues(top = padding.calculateTopPadding(), bottom = padding.calculateBottomPadding() + Metrics.s3),
      ) {
        for (group in groups) {
          group.caption?.let { caption -> item(key = "caption:${group.id}") { Caption(caption) } }
          items(group.sessions, key = { "${group.id}:${it.id}" }) { s ->
            SessionRow(
              session = s,
              agentName = model.agent(s.agent).name,
              place = model.placeName(s.cwd),
              time = if (group.dated) null else sessionTime(s.updatedAt, now),
              onSelect = { model.push(Screen.Chat(s.id)) },
              onPin = { model.togglePin(s.id) },
              onDelete = { model.delete(s.id) },
            )
          }
        }
      }
    }
  }
}

/** By time: pinned, then today / yesterday / this week / this month / older (sessionView.ts groupSessions) */
private fun groups(sessions: List<SessionSummary>, now: Instant): List<SessionGroup> {
  val out = ArrayList<SessionGroup>()
  val pinned = sessions.filter { it.pinned == true }
  if (pinned.isNotEmpty()) out += SessionGroup("pinned", t("session.group.pinned"), pinned)
  val buckets = sessions.filter { it.pinned != true }.groupBy { recency(it.updatedAt, now) }
  for (key in listOf("today", "yesterday", "week", "month", "older")) {
    buckets[key]?.let { out += SessionGroup(key, t("session.when.$key"), it, dated = true) }
  }
  return out
}

private fun parse(iso: String): Instant? = runCatching { Instant.parse(iso) }.getOrNull()

private fun recency(iso: String, now: Instant): String {
  val date = parse(iso) ?: return "older"
  val zone = ZoneId.systemDefault()
  val day = date.atZone(zone).toLocalDate()
  val today = now.atZone(zone).toLocalDate()
  if (day == today) return "today"
  if (day == today.minusDays(1)) return "yesterday"
  val days = (now.toEpochMilli() - date.toEpochMilli()) / 86_400_000.0
  return if (days < 7) "week" else if (days < 30) "month" else "older"
}

/** Today: the time of day; earlier: month/day, both in the UI locale */
fun sessionTime(iso: String, now: Instant): String {
  val date = parse(iso) ?: return ""
  val zone = ZoneId.systemDefault()
  val locale = Locale.forLanguageTag(L10n.locale)
  val local = date.atZone(zone)
  return if (local.toLocalDate() == now.atZone(zone).toLocalDate()) {
    local.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale))
  } else {
    local.format(DateTimeFormatter.ofPattern("M/d", locale))
  }
}

/** A group's caption (置顶, 今天 …) */
@Composable
private fun Caption(text: String) {
  Box(Modifier.fillMaxWidth().heightIn(min = Metrics.groupCaption).padding(horizontal = Metrics.pad).padding(bottom = Metrics.s1), contentAlignment = Alignment.BottomStart) {
    Text(text, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, fontWeight = FontWeight.Medium)
  }
}

/**
 * One session: the agent's bare mark, the title over its project, the time or the state mark at the end. A long press
 * opens its menu (pin, delete), the Android counterpart of the iOS context menu
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun SessionRow(
  session: SessionSummary,
  agentName: String,
  place: String?,
  time: String?,
  onSelect: () -> Unit,
  onPin: () -> Unit,
  onDelete: () -> Unit,
) {
  var menu by remember { mutableStateOf(false) }
  Box {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
      modifier = Modifier
        .fillMaxWidth()
        .heightIn(min = Metrics.sessionRow)
        .combinedClickable(onClick = onSelect, onLongClick = { menu = true })
        .padding(horizontal = Metrics.pad)
        .testTag("session:${session.id}"),
    ) {
      Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
        VendorMark(session.agent, agentName, Palette.fg2, size = Metrics.rowMark, fit = true)
      }
      Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
          Text(
            session.title, color = Palette.fgStrong, fontSize = Type.text1, lineHeight = Type.text2Line,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
          )
          if (time != null) {
            Spacer(Modifier.width(Metrics.s2))
            Text(time, color = Palette.fg3, fontSize = Type.text3)
          }
          Box(Modifier.padding(start = Metrics.gap - Metrics.s2).size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
            session.state?.takeIf { it != SessionState.unknown }?.let { StateMark(it, session.unread == true) }
          }
        }
        if (place != null) {
          Text(place, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
      }
    }
    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
      DropdownMenuItem(
        text = { Text(t(if (session.pinned == true) "common.unpin" else "common.pin")) },
        leadingIcon = { Lucide(if (session.pinned == true) "pinOff" else "pin", Palette.fg1) },
        onClick = { menu = false; onPin() },
      )
      DropdownMenuItem(
        text = { Text(t("common.delete"), color = Palette.danger) },
        leadingIcon = { Lucide("trash2", Palette.danger) },
        onClick = { menu = false; onDelete() },
      )
    }
  }
}

/** SessionList.tsx StateMark: working spins, waiting beats, error flashes, unread is a solid dot, background orbits */
@Composable
fun StateMark(state: SessionState, unread: Boolean) {
  val loop = rememberInfiniteTransition(label = "state")
  when (state) {
    SessionState.working -> {
      val turn by loop.animateFloat(0f, 360f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "spin")
      Lucide("loaderCircle", Palette.fg2, Modifier.rotate(turn), size = Metrics.stateRing)
    }
    SessionState.waiting -> {
      val beat by loop.animateFloat(1f, 1.25f, infiniteRepeatable(tween(700), RepeatMode.Reverse), label = "beat")
      Box(Modifier.size(Metrics.stateDot).scale(beat).background(Palette.warn, CircleShape))
    }
    SessionState.error -> Box(Modifier.size(Metrics.stateDot).background(Palette.danger, CircleShape))
    SessionState.unread -> Box(Modifier.size(Metrics.stateDot).background(Palette.fgStrong, CircleShape))
    SessionState.background -> {
      val turn by loop.animateFloat(0f, 360f, infiniteRepeatable(tween(3600, easing = LinearEasing)), label = "orbit")
      Box(contentAlignment = Alignment.Center) {
        Lucide("circle", if (unread) Palette.fgStrong else Palette.fg3, size = Metrics.stateRing)
        Box(Modifier.size(Metrics.stateRing).rotate(turn), contentAlignment = Alignment.TopCenter) {
          Box(Modifier.padding(top = Metrics.s0_5).size(Metrics.s1).background(if (unread) Palette.fgStrong else Palette.fg2, CircleShape))
        }
      }
    }
    SessionState.unknown -> {}
  }
}

/** The search field and the new-session button the list keeps at the bottom */
@Composable
private fun ListBottomBar(query: String, onQuery: (String) -> Unit, onNew: () -> Unit) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(horizontal = Metrics.pad, vertical = Metrics.s2),
  ) {
    Surface(
      color = Palette.bg1, shape = RoundedCornerShape(percent = 50), shadowElevation = Metrics.barShadow,
      modifier = Modifier.weight(1f).height(Metrics.barCtl),
    ) {
      Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(horizontal = Metrics.pad)) {
        Lucide("search", Palette.fg1, size = Metrics.iconCtl)
        Spacer(Modifier.width(Metrics.s2 + Metrics.s1))
        Box(Modifier.weight(1f)) {
          if (query.isEmpty()) Text(t("session.searchContent"), color = Palette.fg3, fontSize = Type.text1, maxLines = 1)
          BasicTextField(
            value = query, onValueChange = onQuery, singleLine = true,
            textStyle = TextStyle(color = Palette.fg1, fontSize = Type.text1),
            cursorBrush = SolidColor(Palette.fg1),
            modifier = Modifier.fillMaxWidth().testTag("list.search"),
          )
        }
      }
    }
    FloatingActionButton(
      onClick = onNew,
      containerColor = Palette.bg1, contentColor = Palette.fg1, shape = CircleShape,
      elevation = FloatingActionButtonDefaults.elevation(defaultElevation = Metrics.barShadow),
      modifier = Modifier.size(Metrics.barCtl).testTag("list.new"),
    ) {
      Lucide("squarePen", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.new"))
    }
  }
}

/** No sessions yet: what the list says, and where the engine stands */
@Composable
private fun EmptyList(model: AppModel, modifier: Modifier) {
  Column(modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
    val note = when (val c = model.connection) {
      Connection.Connecting -> t("connections.state.connecting")
      is Connection.Closed -> t("mobile.engine.disconnected")
      else -> t("session.none")
    }
    Text(note, color = Palette.fg3, fontSize = Type.text2, modifier = Modifier.testTag("list.empty"))
  }
}

