package cc.hotic.acpira.ui

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.ConnectionState
import cc.hotic.acpira.model.ConnectionView
import cc.hotic.acpira.model.SessionSection
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.projectName

/** A group's caption (置顶, 今天 …) */
@Composable
internal fun Caption(text: String) {
  Box(Modifier.fillMaxWidth().heightIn(min = Metrics.groupCaption).padding(horizontal = Metrics.pad).padding(bottom = Metrics.s1), contentAlignment = Alignment.BottomStart) {
    Text(text, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, fontWeight = FontWeight.Medium)
  }
}

/**
 * One session, full width, one fact per block: the agent's bare mark; the title, with the time at its right end where no
 * day caption says it, and the state's mark in a trailing slot on that line; under it the place (the project), and a
 * search's content hit. Inside a project's group the row is one line, a step in from the folder. [press] carries the
 * row's long press (its menu and drag)
 */
@Composable
internal fun SessionRow(
  session: SessionSummary,
  agentName: String,
  place: String?,
  time: String?,
  divider: Boolean,
  nested: Boolean,
  snippet: String?,
  terms: List<String>,
  active: Boolean,
  onSelect: () -> Unit,
  press: Modifier,
) {
  val line = Palette.line
  val fill = Palette.active
  val density = LocalDensity.current
  val ruleStart = with(density) { (Metrics.pad + (if (nested) Metrics.nest else Metrics.s0) + Metrics.lead + Metrics.gap).toPx() }
  val hair = with(density) { Metrics.hairline.toPx() }
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier
      .fillMaxWidth()
      // One line stands as tall as a project's header
      .heightIn(min = if (place == null && snippet == null) Metrics.sheetRow else Metrics.sessionRow)
      .background(Palette.bg0)
      // The sidebar's open chat: the desktop column's fill, rounded and held in from both edges (a square one ran into
      // the column's edge)
      .drawBehind {
        if (active) {
          val inset = Metrics.s2.toPx()
          val r = Metrics.rLg.toPx()
          drawRoundRect(fill, Offset(inset, 0f), androidx.compose.ui.geometry.Size(size.width - inset * 2, size.height), androidx.compose.ui.geometry.CornerRadius(r, r))
        }
      }
      .drawBehind { if (divider) drawRect(line, Offset(ruleStart, size.height - hair), androidx.compose.ui.geometry.Size(size.width - ruleStart, hair)) }
      .clickable(onClick = onSelect)
      .then(press)
      .padding(start = if (nested) Metrics.nest else Metrics.s0)
      .padding(horizontal = Metrics.pad, vertical = if (snippet == null) Metrics.s0 else Metrics.s2)
      .testTag("session:${session.id}"),
  ) {
    // The bare mark, its ink fitted; its column starts the slot, so the ink stands on the title's and captions' edge
    Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
      VendorMark(session.agent, agentName, Palette.fg2, size = Metrics.rowMark, fit = true)
    }
    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
      Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
          marked(session.title, terms), color = Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine,
          maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
        )
        if (time != null) {
          Spacer(Modifier.width(Metrics.s2))
          Text(time, color = Palette.fg3, fontSize = Type.text3, style = TextStyle(fontFeatureSettings = "tnum"), maxLines = 1)
        }
        // The state is its mark alone, as on the desktop; every row keeps the slot, so the times above line up
        Box(Modifier.padding(start = Metrics.gap - Metrics.s2).size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
          session.state?.takeIf { it != SessionState.unknown }?.let { StateMark(it, session.unread == true) }
        }
      }
      if (place != null) {
        Text(place, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
      }
      if (snippet != null) {
        Text(
          marked(snippet, terms), color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 2,
          overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("session.snippet:${session.id}"),
        )
      }
    }
  }
}

/**
 * A project's header in the by-project view (apps/ios ProjectHeader): the folder in the marks' column, the name and a
 * chevron turning down while open; folded, the project's most pressing state sits in the rows' state column. A folder on
 * an SSH host wears the globe and ends in its host's alias and connection dot. A tap folds it open or shut; its long
 * press opens its menu (a session in it, move to top, a section, copy path, off the list) and, moving on, drags it
 */
@Composable
internal fun ProjectHeader(
  model: AppModel,
  cwd: String,
  first: Boolean,
  open: Boolean,
  sessions: List<SessionSummary>,
  modifier: Modifier,
  onToggle: () -> Unit,
  onNewSection: () -> Unit,
  drag: ListDrag,
  onDrop: (ListDragItem, ListDropTarget) -> Unit,
) {
  val lists = model.lists
  var menu by remember { mutableStateOf(false) }
  val host = remoteAlias(cwd)
  val connection = host?.let { alias -> lists.connections.firstOrNull { it.alias == alias } }
  val turn by animateFloatAsState(if (open) CHEVRON_OPEN else 0f, tween(if (LocalMotion.current) MOTION_EASE_MS else 0), label = "chevron")
  Box(modifier) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
      modifier = Modifier
        .fillMaxWidth()
        .heightIn(min = Metrics.sheetRow)
        .clickable(onClick = onToggle)
        .semantics { heading(); onLongClick(t("session.more")) { menu = true; true } }
        .listDrag(drag, ListDragItem.Project(cwd), onLongPress = { menu = true }, onLift = { menu = false }, onDrop = onDrop) {
          ListDragPreview(projectName(cwd)) { if (host != null) RemoteFolderGlyph(false) else Lucide("folder", Palette.fg2) }
        }
        .padding(horizontal = Metrics.pad)
        .testTag("project:$cwd"),
    ) {
      // The folder in the top-level session marks' column (its own sessions sit a step in), shut or open with the fold
      Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
        Box(Modifier.width(Metrics.rowMark), contentAlignment = Alignment.Center) {
          if (host != null) Breathing(connection?.state == ConnectionState.connecting) { RemoteFolderGlyph(open) }
          else Lucide(if (open) "folderOpen" else "folder", Palette.fg3)
        }
      }
      Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
        Text(
          projectName(cwd), color = Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine, fontWeight = FontWeight.SemiBold,
          maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        Lucide("chevronRight", Palette.fg3, Modifier.rotate(turn), size = Metrics.chevron)
      }
      // The alias keeps its room; the project's name truncates first, as on the desktop
      if (host != null) HostTag(host, connection)
      // Open, each row shows its own state; the slot stays either way, as in the rows
      Box(Modifier.size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
        if (!open) pressing(sessions)?.let { (state, unread) -> StateMark(state, unread) }
      }
    }
    ProjectMenu(model, cwd, first, menu, onDismiss = { menu = false }, onNewSection = onNewSection)
  }
}

internal sealed interface SectionPrompt {
  data class Create(val project: String? = null) : SectionPrompt
  data class Rename(val section: SessionSection) : SectionPrompt
}

/** The project header's menu (the desktop project menu, less revealing the folder on the computer) */
@Composable
private fun ProjectMenu(model: AppModel, cwd: String, first: Boolean, expanded: Boolean, onDismiss: () -> Unit, onNewSection: () -> Unit) {
  val lists = model.lists
  // null: the top level; SECTIONS: the user's sections
  var level by remember { mutableStateOf<String?>(null) }
  val close = { onDismiss(); level = null }
  AppMenu(expanded = expanded, onDismiss = close) {
    val sections = lists.sections
    val current = sections.firstOrNull { cwd in it.projects }?.id
    when (level) {
      null -> {
        AppMenuItem(t("session.project.newSession"), onClick = { close(); model.open(Screen.New(cwd = cwd)) }, icon = { MenuGlyph("squarePen") }, modifier = Modifier.testTag("project.new"))
        AppMenuGap()
        if (!first) {
          AppMenuItem(t("mobile.project.moveTop"), onClick = { close(); lists.moveProjectToTop(cwd) }, icon = { MenuGlyph("arrowUp") }, modifier = Modifier.testTag("project.top"))
        }
        AppMenuItem(t("session.section.moveTo"), onClick = { level = SECTIONS }, icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("project.section"))
        AppMenuItem(t("session.project.copyPath"), onClick = { close(); lists.copyPath(cwd) }, icon = { MenuGlyph("copy") }, modifier = Modifier.testTag("project.copyPath"))
        AppMenuGap()
        AppMenuItem(t("session.project.remove"), onClick = { close(); lists.removeProject(cwd) }, icon = { MenuGlyph("minus") }, modifier = Modifier.testTag("project.remove"))
      }
      else -> {
        // The sections as a ticked choice, the folder's own ticked, then "New section…" and "Remove from section"
        AppMenuBack(t("session.section.moveTo"), onClick = { level = null })
        AppMenuGap()
        for (s in sections) {
          AppMenuItem(
            s.name.ifEmpty { t("session.section.untitled") }, checked = s.id == current,
            onClick = { close(); if (s.id != current) lists.setProjectSection(cwd, s.id) },
          )
        }
        if (sections.isNotEmpty()) AppMenuGap()
        AppMenuItem(t("session.section.newEllipsis"), checked = false, onClick = { close(); onNewSection() }, icon = { MenuGlyph("plus") }, modifier = Modifier.testTag("project.section.new"))
        if (current != null) {
          AppMenuItem(t("session.section.remove"), checked = false, onClick = { close(); lists.setProjectSection(cwd, null) }, icon = { MenuGlyph("minus") })
        }
      }
    }
  }
}

/**
 * The project's mark, ranked as the desktop's project rows rank it (SessionList.tsx STATE_RANK): running first, then
 * waiting, failed, unread, background work (lit when one of those sessions has a reply nobody has looked at)
 */
private fun pressing(sessions: List<SessionSummary>): Pair<SessionState, Boolean>? {
  val rank = listOf(SessionState.working, SessionState.waiting, SessionState.error, SessionState.unread, SessionState.background)
  val state = rank.firstOrNull { st -> sessions.any { it.state == st } } ?: return null
  return state to (state == SessionState.background && sessions.any { it.state == SessionState.background && it.unread == true })
}

/** RemoteFolderIcon.tsx: the folder with a small globe on its lower right corner, cut out of the folder under it */
@Composable
internal fun RemoteFolderGlyph(open: Boolean) {
  Box(Modifier.size(Metrics.icon)) {
    Lucide(if (open) "folderOpen" else "folder", Palette.fg3)
    Box(
      Modifier
        .align(Alignment.BottomEnd)
        .offset(Metrics.remoteOverhang, Metrics.remoteOverhang)
        .size(Metrics.remoteBadge)
        .background(Palette.bg0, CircleShape),
      contentAlignment = Alignment.Center,
    ) { Lucide("globe", Palette.fg3, size = Metrics.remoteGlobe) }
  }
}

/** A host folder's header end (SessionListSections.tsx GroupHeader): the host's alias, then its connection as a dot */
@Composable
private fun HostTag(alias: String, view: ConnectionView?) {
  Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
    Text(alias, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = Metrics.hostAlias))
    // No dot until the engine has said how the host is
    if (view != null) {
      val dot = when (view.state) {
        ConnectionState.connected -> Palette.ok
        ConnectionState.error -> Palette.warn
        ConnectionState.connecting -> Palette.fg3
        else -> Palette.fg3.copy(alpha = HOST_OFF_ALPHA)
      }
      Breathing(view.state == ConnectionState.connecting) { Box(Modifier.size(Metrics.hostDot).background(dot, CircleShape)) }
    }
  }
}

/** `remote-breathe`: [content] breathes while [on], unless motion is off */
@Composable
private fun Breathing(on: Boolean, content: @Composable () -> Unit) {
  if (!on || !LocalMotion.current) return content()
  val loop = rememberInfiniteTransition(label = "breathe")
  val level = loop.animateFloat(1f, BREATHE_LOW, infiniteRepeatable(tween(BREATHE_MS), RepeatMode.Reverse), label = "breatheLevel")
  // Read in the layer, so a breath composes nothing again
  Box(Modifier.graphicsLayer { alpha = level.value }) { content() }
}

/**
 * SessionListSections.tsx MoreRow, under an open project's latest few: how many the rest holds, the agents among them
 * and the most urgent state inside. A tap brings out the next page above it; with all of them out it reads "show less",
 * the chevron turned over, and a tap folds them away
 */
@Composable
internal fun MoreRow(rest: List<SessionSummary>, nameOf: (String) -> String, tag: String, modifier: Modifier = Modifier, nested: Boolean = true, onClick: () -> Unit) {
  val all = rest.isEmpty()
  val agents = rest.map { it.agent }.distinct().take(MORE_MARKS)
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = modifier
      .fillMaxWidth()
      .heightIn(min = Metrics.sheetRow)
      .clickable(onClick = onClick)
      .padding(start = if (nested) Metrics.nest else Metrics.s0)
      .padding(horizontal = Metrics.pad)
      .testTag(tag),
  ) {
    // The chevron in the sessions' mark column
    Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
      Box(Modifier.width(Metrics.rowMark), contentAlignment = Alignment.Center) {
        Lucide("chevronDown", Palette.fg3, Modifier.rotate(if (all) CHEVRON_TURNED else 0f), size = Metrics.chevron)
      }
    }
    Text(
      if (all) t("session.showLess") else t("session.showMore", "n" to rest.size), color = Palette.fg3, fontSize = Type.text2, maxLines = 1,
    )
    if (!all) {
      Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s1)) {
        for (agent in agents) VendorMark(agent, nameOf(agent), Palette.fg2, size = Metrics.moreMark, fit = true)
      }
    }
    Spacer(Modifier.weight(1f))
    Box(Modifier.size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
      pressing(rest)?.let { (state, unread) -> StateMark(state, unread) }
    }
  }
}

/**
 * The empty chats group's only row (SessionListSections.tsx EmptyProjectRow, flush): a faint plus in the marks' column
 * and "New chat"
 */
@Composable
internal fun NewChatRow(onClick: () -> Unit) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.sheetRow).clickable(onClick = onClick).padding(horizontal = Metrics.pad).testTag("list.newChat"),
  ) {
    Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
      Box(Modifier.width(Metrics.rowMark), contentAlignment = Alignment.Center) { Lucide("plus", Palette.fg3) }
    }
    Text(t("session.chat.new"), color = Palette.fg3, fontSize = Type.text2, maxLines = 1, modifier = Modifier.weight(1f))
  }
}

/**
 * The end of a pinned run: the desktop's short faded rule (SessionListSections.tsx `PinDivider`) instead of a pin on
 * each row
 */
@Composable
internal fun PinRule() {
  val line = Palette.line
  Box(
    Modifier
      .fillMaxWidth()
      .padding(horizontal = Metrics.pad, vertical = Metrics.s1)
      .height(Metrics.hairline)
      .background(Brush.horizontalGradient(listOf(Color.Transparent, line, Color.Transparent))),
  )
}

/**
 * The desktop list's state marks (packages/ui chat/SessionRows.tsx StateMark, motion in styles/motion.css), one slot
 * each so they share a centre line. Working: the spinning ring, which keeps turning with motion off. Waiting: an amber
 * dot beating like a heart. Error: a red dot that double-flashes every few seconds. Unread: a dot with a halo rippling
 * out of it. Background: the ring with a dot orbiting it slowly, lit with a dot inside once a reply came in meanwhile.
 * Motion off stops all but the ring on their resting frame
 */
@Composable
fun StateMark(state: SessionState, unread: Boolean) {
  // While the connection is down (or a new wire has not synced yet) a live state may be stale: the marks that move stop
  // on their resting frame and breathe instead, as the remote dot does (apps/ios StateMark `held`)
  val held = LocalSyncing.current && (state == SessionState.working || state == SessionState.waiting || state == SessionState.background)
  val motion = LocalMotion.current && !held
  val loop = rememberInfiniteTransition(label = "state")
  // The loops' values are read where a layer is set up, never while composing: read in composition, every mark on
  // screen was composed again for each frame of its loop
  Breathing(held) {
  Box(Modifier.size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
    when (state) {
      SessionState.working -> {
        // animate-spin: one turn a second, linear; it turns with motion off too, not while held
        val turn = loop.animateFloat(0f, FULL_TURN, infiniteRepeatable(tween(SPIN_MS, easing = LinearEasing)), label = "spin")
        Lucide("loaderCircle", Palette.fg2, Modifier.graphicsLayer { rotationZ = if (held) 0f else turn.value }, size = Metrics.stateRing)
      }
      SessionState.waiting -> {
        // acp-mark-beat: 0 / 100% 1, 20% 1.5, 40% 1.1, 55% 1.35, 75% 1, eased between keys
        val beat = loop.animateFloat(
          1f, 1f,
          infiniteRepeatable(keyframes {
            durationMillis = BEAT_MS
            1f at 0 using FastOutSlowInEasing
            1.5f at (BEAT_MS * 0.2f).toInt() using FastOutSlowInEasing
            1.1f at (BEAT_MS * 0.4f).toInt() using FastOutSlowInEasing
            1.35f at (BEAT_MS * 0.55f).toInt() using FastOutSlowInEasing
            1f at (BEAT_MS * 0.75f).toInt()
          }),
          label = "beat",
        )
        Box(
          Modifier.size(Metrics.stateDot).graphicsLayer {
            val by = if (motion) beat.value else 1f
            scaleX = by
            scaleY = by
          }.background(Palette.warn, CircleShape),
        )
      }
      SessionState.error -> {
        // acp-mark-flash: lit, then two dips to 0.2 at 70% and 82%, linear between keys
        val flash = loop.animateFloat(
          1f, 1f,
          infiniteRepeatable(keyframes {
            durationMillis = FLASH_MS
            1f at (FLASH_MS * 0.64f).toInt()
            FLASH_LOW at (FLASH_MS * 0.7f).toInt()
            1f at (FLASH_MS * 0.76f).toInt()
            FLASH_LOW at (FLASH_MS * 0.82f).toInt()
            1f at (FLASH_MS * 0.88f).toInt()
          }),
          label = "flash",
        )
        Box(Modifier.size(Metrics.stateDot).graphicsLayer { alpha = if (motion) flash.value else 1f }.background(Palette.danger, CircleShape))
      }
      SessionState.unread -> {
        // The halo: a copy of the dot growing to 2.6 times while it fades, ease-out over 2 s; invisible at rest
        if (motion) {
          val p = loop.animateFloat(0f, 1f, infiniteRepeatable(tween(HALO_MS, easing = { 1 - (1 - it) * (1 - it) })), label = "halo")
          Box(
            Modifier.size(Metrics.stateUnread).graphicsLayer {
              val by = 1 + HALO_GROWTH * p.value
              scaleX = by
              scaleY = by
              alpha = HALO_ALPHA * (1 - p.value)
            }.background(Palette.fgStrong, CircleShape),
          )
        }
        Box(Modifier.size(Metrics.stateUnread).background(Palette.fgStrong, CircleShape))
      }
      SessionState.background -> {
        val turn = loop.animateFloat(0f, FULL_TURN, infiniteRepeatable(tween(ORBIT_MS, easing = LinearEasing)), label = "orbit")
        val ring = if (unread) Palette.fgStrong else Palette.fg3
        Lucide("circle", ring, size = Metrics.stateRing)
        // The orbit rides the ring's own line (Lucide's circle: r 10 in a 24-unit box)
        Box(Modifier.size(Metrics.stateRing).graphicsLayer { rotationZ = if (motion) turn.value else 0f }, contentAlignment = Alignment.TopCenter) {
          Box(Modifier.offset(y = orbitTop(Metrics.stateRing, Metrics.stateOrbit)).size(Metrics.stateOrbit).background(if (unread) Palette.fgStrong else Palette.fg2, CircleShape))
        }
        if (unread) Box(Modifier.size(Metrics.stateInner).background(Palette.fgStrong, CircleShape))
      }
      SessionState.unknown -> {}
    }
  }
  }
}

/** The screens show what may be stale: the wire is down, or a new one has not synced yet (AppModel `current`) */
val LocalSyncing = androidx.compose.runtime.compositionLocalOf { false }

/** Where the orbiting dot's top sits in the ring's box: centred on the circle's line, r 10 of a 24-unit box */
private fun orbitTop(ring: Dp, dot: Dp): Dp = ring / 2 - ring * ORBIT_R / LUCIDE_BOX - dot / 2

/** The project menu's sections level */
private const val SECTIONS = "sections"
private const val MORE_MARKS = 3

private const val CHEVRON_TURNED = 180f
private const val FULL_TURN = 360f

private const val HOST_OFF_ALPHA = 0.4f
private const val BREATHE_LOW = 0.35f
private const val BREATHE_MS = 1200
private const val SPIN_MS = 1000
private const val BEAT_MS = 1400
private const val FLASH_MS = 3000
private const val FLASH_LOW = 0.2f
private const val HALO_MS = 2000
private const val HALO_GROWTH = 1.6f
private const val HALO_ALPHA = 0.6f
private const val ORBIT_MS = 3600
private const val ORBIT_R = 10f
private const val LUCIDE_BOX = 24f
