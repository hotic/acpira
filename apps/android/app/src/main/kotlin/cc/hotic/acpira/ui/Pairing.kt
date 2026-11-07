package cc.hotic.acpira.ui

import cc.hotic.acpira.design.readableWidth
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.PairLink
import cc.hotic.acpira.store.PairingModel
import cc.hotic.acpira.store.pathLabel
import cc.hotic.acpira.sync.ConnectionError
import cc.hotic.acpira.sync.PairedComputer
import cc.hotic.acpira.sync.PathKind
import kotlinx.coroutines.launch
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

// The phone's side of paired remote access on screen (apps/ios Pairing/): the connect frame in the list's place while
// there is nothing to list, the bar title's menu of computers, and the settings' computers with one computer's page.

/** The pairing model; null while the app has no pairing service (a development engine, a demo, a build without it) */
val LocalPairing = staticCompositionLocalOf<PairingModel?> { null }

/** A computer's glyph from its `hw.model`: a Mac laptop, else a display (lucide has no desktop Mac) */
fun computerIcon(@Suppress("UNUSED_PARAMETER") model: String?): String = "laptop"

/**
 * The pairing screens' one centre piece, laid out as an empty state is: a computer's glyph, its name, one sentence and
 * one action (apps/ios PairHero.swift). Every case uses it, so moving between them swaps contents in a frame that stays.
 * Connected, the sentence gives way to the path the wire travels
 */
@Composable
fun PairHero(
  icon: String,
  line: String,
  modifier: Modifier = Modifier,
  spin: Boolean = false,
  name: String? = null,
  paired: Boolean = false,
  path: PathKind? = null,
  action: String? = null,
  secondary: String? = null,
  dim: Boolean = false,
  onAction: () -> Unit = {},
  onSecondary: () -> Unit = {},
) {
  Column(modifier.fillMaxWidth().padding(horizontal = Metrics.page), horizontalAlignment = Alignment.CenterHorizontally) {
    Box(Modifier.height(Metrics.pairGlyphBox), contentAlignment = Alignment.Center) {
      PairGlyph(icon, ink = if (paired) Palette.fg1 else if (dim) Palette.fg3 else Palette.fg2, checked = paired)
    }
    Spacer(Modifier.height(Metrics.page))
    if (name != null) {
      val title = if (paired) t("devices.paired", "name" to name) else name
      Text(
        title, color = Palette.fgStrong, fontSize = Type.pairName, lineHeight = Type.pairNameLine, fontWeight = FontWeight.SemiBold,
        maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("pair.name"),
      )
      Spacer(Modifier.height(Metrics.s2))
    }
    // The sentence, or the path's tag in its place; the slot keeps two lines so a longer one moves nothing under it
    Box(Modifier.fillMaxWidth().heightIn(min = Metrics.pairLineSlot), contentAlignment = Alignment.TopCenter) {
      AnimatedContent(targetState = path, transitionSpec = { fadeIn(tween(Motion.fade)) togetherWith fadeOut(tween(Motion.fade)) }, label = "pairLine") { p ->
        if (p != null) {
          PairPathTag(p)
        } else {
          Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
            if (spin) CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg2, strokeWidth = Metrics.pairCodeLine)
            Text(line, color = Palette.fg2, fontSize = Type.body, lineHeight = Type.bodyLine, textAlign = TextAlign.Center, modifier = Modifier.testTag("pair.line"))
          }
        }
      }
    }
    // The action's slot stays when there is none, so a phase without one moves nothing; it sits as far below the
    // sentence as the glyph sits above it
    Box(Modifier.padding(top = Metrics.page).heightIn(min = Metrics.barCtl), contentAlignment = Alignment.Center) {
      if (action != null) PrimaryPill(action, onAction, Modifier.testTag("pair.action"))
    }
    if (secondary != null) {
      TextButton(onClick = onSecondary, modifier = Modifier.heightIn(min = Metrics.barCtl).testTag("pair.secondary")) {
        Text(secondary, color = Palette.fg2, fontSize = Type.body, fontWeight = FontWeight.Medium)
      }
    }
  }
}

/** The computer's glyph; paired, a check on its corner */
@Composable
private fun PairGlyph(icon: String, ink: androidx.compose.ui.graphics.Color, checked: Boolean) {
  Box {
    Lucide(icon, ink, size = Metrics.pairGlyph)
    if (checked) {
      Box(
        Modifier.align(Alignment.BottomEnd).offset(x = Metrics.s2, y = Metrics.s1)
          .size(Metrics.lead).background(Palette.bg0, CircleShape).padding(Metrics.s0_5).background(Palette.ok, CircleShape),
        contentAlignment = Alignment.Center,
        // The tick stays white on the green disc in either theme
      ) { Lucide("check", androidx.compose.ui.graphics.Color.White, size = Metrics.chevron) }
    }
  }
}

/** The path a paired computer's wire travels: its name on the active fill (ui/Button ChipTag) */
@Composable
fun PairPathTag(path: PathKind) {
  Box(
    Modifier.height(Metrics.pairPathH).background(Palette.active, RoundedCornerShape(Metrics.rXs)).padding(horizontal = Metrics.pairPathX).testTag("pair.path"),
    contentAlignment = Alignment.Center,
  ) { Text(pathLabel(path), color = Palette.fg2, fontSize = Type.text4, maxLines = 1) }
}

/** The one primary action: a solid capsule, white in dark and black in light, like the composer's send */
@Composable
fun PrimaryPill(label: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true) {
  Button(
    onClick = onClick,
    enabled = enabled,
    colors = ButtonDefaults.buttonColors(containerColor = Palette.btn1Bg, contentColor = Palette.btn1Fg),
    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = Metrics.page + Metrics.s2),
    modifier = modifier.heightIn(min = Metrics.barCtl),
  ) { Text(label, fontSize = Type.body, fontWeight = FontWeight.SemiBold, maxLines = 1) }
}

/**
 * Puts the hero's glyph centre at the page's optical centre with everything else hanging under it, so a phase that
 * adds or drops a line keeps the glyph where it was
 */
@Composable
fun PairHeroPlacement(modifier: Modifier = Modifier, box: androidx.compose.ui.unit.Dp = Metrics.pairGlyphBox, content: @Composable () -> Unit) {
  BoxWithConstraints(modifier.fillMaxSize()) {
    val top = (maxHeight * Metrics.heroRise - box / 2).coerceAtLeast(Metrics.s0)
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
      Spacer(Modifier.height(top))
      content()
    }
  }
}

/**
 * Whether the list gives way to the connect frame: until the followed computer's lists are in (the wire is up well
 * before its init), and whenever its wire failed. While the computer the list shows connects again, its last lists stay
 */
fun covers(pairing: PairingModel, model: AppModel): Boolean {
  if (!pairing.loaded) return true
  val current = pairing.selected ?: return true
  if (pairing.link is PairLink.Failed) return true
  return !(model.computer == current.id && (model.synced || model.resumed))
}

/**
 * The page in the list's place while it has nothing to list: no computer yet, the followed one connecting, offline,
 * removed this phone, or refused the wire. One frame shows every case, so only the sentence changes between them
 */
@Composable
fun PairHeroList(pairing: PairingModel, modifier: Modifier = Modifier) {
  if (!pairing.loaded) return
  val current = pairing.selected
  PairHeroPlacement(modifier.testTag("pair.hero")) {
    if (current == null) {
      PairHero("laptop", t("mobile.pair.none"), action = t("mobile.pair.add"), onAction = { pairing.addOpen = true })
      return@PairHeroPlacement
    }
    val named = "name" to current.name
    val icon = computerIcon(current.model)
    val retry = { pairing.connectSelected() }
    when (val l = pairing.link) {
      is PairLink.Failed -> when (l.error) {
        ConnectionError.Offline -> PairHero(icon, t("mobile.pair.offline", named), action = t("mobile.pair.retry"), dim = true, onAction = retry)
        ConnectionError.Revoked -> PairHero(icon, t("mobile.pair.revoked", named), action = t("mobile.pair.again"), dim = true, onAction = { pairing.addOpen = true })
        is ConnectionError.Incompatible -> PairHero(icon, t("mobile.pair.incompatible", named), action = t("mobile.pair.retry"), dim = true, onAction = retry)
        // Online too until the engine's lists are in; a background close dials again on the way back
        ConnectionError.Suspended -> PairHero(icon, t("mobile.pair.connecting"), spin = true)
        is ConnectionError.Other -> PairHero(icon, t("mobile.pair.connectFailed", named), action = t("mobile.pair.retry"), dim = true, onAction = retry)
      }
      else -> PairHero(icon, t("mobile.pair.connecting"), spin = true)
    }
  }
}

/**
 * The list bar's centred title: the followed computer with its menu (every paired computer with how it is reached, the
 * followed one ticked, then add another); nothing while no computer is paired
 */
@Composable
fun ListBarTitle(pairing: PairingModel) {
  val current = pairing.selected ?: return
  var open by remember { mutableStateOf(false) }
  Box {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
      modifier = Modifier.clip(RoundedCornerShape(percent = 50)).clickable { open = true }.padding(horizontal = Metrics.s2, vertical = Metrics.s1).testTag("list.computer"),
    ) {
      Text(current.name, fontWeight = FontWeight.SemiBold, fontSize = Type.barTitle, maxLines = 1, overflow = TextOverflow.Ellipsis)
      Lucide("chevronDown", Palette.fg3, size = Metrics.icon)
    }
    AppMenu(expanded = open, onDismiss = { open = false }) {
      for (c in pairing.computers) {
        AppMenuItem(
          c.name,
          onClick = { open = false; pairing.select(c.id) },
          subtitle = pairing.status(c),
          checked = c.id == current.id,
          icon = { MenuGlyph(computerIcon(c.model)) },
          modifier = Modifier.testTag("list.computer:${c.id}"),
        )
      }
      AppMenuGap()
      AppMenuItem(
        t("mobile.pair.add"),
        onClick = { open = false; pairing.addOpen = true },
        icon = { MenuGlyph("plus") },
        modifier = Modifier.testTag("list.computer.add"),
      )
    }
  }
}

/** The settings' computers: each paired one with how it is reached right now, then add another */
@Composable
fun PairComputersSection(pairing: PairingModel, onOpen: (PairedComputer) -> Unit) {
  LaunchedEffect(pairing.loaded) { if (pairing.loaded) pairing.refreshPresence() }
  FormSection(t("mobile.pair.computers"), footer = t("mobile.pair.computers.desc")) {
    for (c in pairing.computers) {
      FormRow(c.name, Modifier.clickable { onOpen(c) }.testTag("settings.computer:${c.id}"), lead = {
        Box {
          Lucide(computerIcon(c.model), Palette.fg2, size = Metrics.rowMark)
          // Online: a dot on the glyph's corner, the same place on every row whatever the value's width
          if (pairing.online(c) == true) {
            Box(
              Modifier.align(Alignment.BottomEnd).offset(x = Metrics.rowDot / 2, y = Metrics.rowDot / 2)
                .background(Palette.formRow, CircleShape).padding(Metrics.pairDotCut).size(Metrics.rowDot).background(Palette.ok, CircleShape),
            )
          }
        }
      }) {
        pairing.status(c)?.let { Text(it, color = Palette.fg2, fontSize = Type.text1) }
        Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
      }
      RowRule(lead = true)
    }
    FormRow(t("mobile.pair.add"), Modifier.clickable { pairing.addOpen = true }.testTag("settings.computer.add"), lead = {
      Lucide("plus", Palette.fg2, size = Metrics.rowMark)
    }) {}
  }
}

/** One paired computer: how it is reached and since when, and removing it */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ComputerScreen(model: AppModel, pairing: PairingModel, id: String) {
  val computer = pairing.computers.firstOrNull { it.id == id }
  val scope = rememberCoroutineScope()
  var confirming by remember { mutableStateOf(false) }
  LaunchedEffect(id) { pairing.refreshPresence() }
  Scaffold(
    containerColor = Palette.formPage,
    topBar = {
      TopAppBar(
        title = { Text(computer?.name.orEmpty(), fontSize = Type.barTitle, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        navigationIcon = {
          IconButton(onClick = { model.pop() }, modifier = Modifier.testTag("computer.back")) {
            Lucide("arrowLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.back"))
          }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.formPage),
      )
    },
  ) { padding ->
    if (computer == null) return@Scaffold
    Column(
      // The page runs under the gesture bar; only the content keeps clear of it, so it never ends in a hard cut above the bar
      Modifier.fillMaxSize().padding(top = padding.calculateTopPadding()).verticalScroll(rememberScrollState()).readableWidth().padding(horizontal = Metrics.pad).navigationBarsPadding().padding(bottom = Metrics.pad),
      verticalArrangement = Arrangement.spacedBy(Metrics.s6),
    ) {
      FormCard {
        FormRow(t("mobile.pair.connection")) {
          val status = pairing.status(computer)
          if (status != null) Text(status, color = Palette.fg2, fontSize = Type.text1, modifier = Modifier.testTag("computer.status"))
          else CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg2, strokeWidth = Metrics.pairCodeLine)
        }
        RowRule()
        FormRow(t("mobile.pair.pairedAt")) {
          val date = Instant.ofEpochMilli(computer.pairedAt).atZone(ZoneId.systemDefault())
            .format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(Locale.forLanguageTag(L10n.locale)))
          Text(date, color = Palette.fg2, fontSize = Type.text1)
        }
      }
      FormSection(null, footer = t("mobile.pair.remove.desc")) {
        Row(
          verticalAlignment = Alignment.CenterVertically,
          modifier = Modifier.fillMaxWidth().clickable { confirming = true }.heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad).testTag("computer.remove"),
        ) { Text(t("mobile.pair.remove"), color = Palette.danger, fontSize = Type.text1) }
      }
    }
  }
  if (confirming && computer != null) {
    AlertDialog(
      onDismissRequest = { confirming = false },
      title = { Text(t("mobile.pair.remove.confirm", "name" to computer.name)) },
      confirmButton = {
        TextButton(onClick = {
          confirming = false
          scope.launch {
            pairing.remove(computer.id)
            model.pop()
          }
        }, modifier = Modifier.testTag("computer.remove.confirm")) { Text(t("common.remove"), color = Palette.danger) }
      },
      dismissButton = { TextButton(onClick = { confirming = false }) { Text(t("mobile.cancel"), color = Palette.fg1) } },
    )
  }
}

/** A digit box's outline, lit for the box the next digit goes in */
fun Modifier.codeBox(lit: Boolean, line: androidx.compose.ui.graphics.Color, fill: androidx.compose.ui.graphics.Color, litLine: androidx.compose.ui.graphics.Color) =
  this.background(fill, RoundedCornerShape(Metrics.rLg)).border(Metrics.pairCodeLine, if (lit) litLine else line, RoundedCornerShape(Metrics.rLg))
