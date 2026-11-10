package cc.hotic.acpira.ui

import android.content.ClipboardManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.PairCodeRows
import cc.hotic.acpira.store.PairLink
import cc.hotic.acpira.store.PairingModel
import cc.hotic.acpira.sync.NearbyComputer
import cc.hotic.acpira.sync.PairedComputer
import cc.hotic.acpira.sync.PairingError
import cc.hotic.acpira.sync.isPairingLink
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// The add-computer sheet (apps/ios Pairing/PairSheet.swift). Its form leads with the camera card, then the computers this
// network shows (picking one asks for the code on its card) and a pasted pairing link. A code scanned or a link pasted
// pairs at once; a link opened from outside the app asks first. Pairing, connecting, connected and a refusal share one
// connect frame that replaces the form in place, and the sheet goes once the new computer answers.

/** What the sheet shows */
private sealed interface PairPhase {
  data object Choose : PairPhase
  /** A computer found nearby, asking for its code */
  data class Code(val computer: NearbyComputer) : PairPhase
  /** A link from outside the app, waiting for a yes */
  data class Confirm(val link: String, val name: String) : PairPhase
  /** The link on its way to the computer */
  data class Pairing(val link: String, val name: String) : PairPhase
  /** Paired: the list follows it now, and the frame waits for its wire */
  data class Joined(val computer: PairedComputer) : PairPhase
  /** Refused or unanswered, with the way on: the same link again (unanswered), else back to the form */
  data class Failed(val name: String?, val message: String, val retryLink: String?) : PairPhase
}

/** The connect frame's one content key across its phases (PairFlow's AnimatedContent) */
private object PairStageKey

/** Android 17's local network permission: LAN dials and NSD browsing (a runtime grant from that release on) */
const val LOCAL_NETWORK = "android.permission.ACCESS_LOCAL_NETWORK"
private const val LOCAL_NETWORK_SDK = 37

fun localNetworkGranted(context: Context): Boolean =
  Build.VERSION.SDK_INT < LOCAL_NETWORK_SDK || ContextCompat.checkSelfPermission(context, LOCAL_NETWORK) == PackageManager.PERMISSION_GRANTED

/** How long the paired moment holds before the sheet goes, and a wire that came up later shows its path this long */
private const val PAIR_HOLD_MS = 2_400L
private const val PAIR_DONE_MS = 600L
/** How long a pasted text that is no pairing link says so in the paste row */
private const val PAIR_NOTICE_MS = 2_400L
private const val CODE_LENGTH = 6

/** The sheet's content in place instead of in a sheet: the first-run flow's pairing step (WelcomeFlow) */
class PairInline(val onBack: () -> Unit, val onDone: () -> Unit)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairSheet(pairing: PairingModel) {
  val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
  val scope = rememberCoroutineScope()
  ModalBottomSheet(
    onDismissRequest = { pairing.addOpen = false },
    sheetState = state,
    containerColor = Palette.formPage,
    dragHandle = null,
    modifier = Modifier.testTag("pair.sheet"),
  ) {
    PairFlow(pairing, inline = null, onClose = { scope.launch { state.hide() }.invokeOnCompletion { pairing.addOpen = false } })
  }
}

/**
 * The sheet's form and stages. [inline]: in place on the first-run flow's page, a large title over the computer's hint,
 * back to the flow instead of close, and the flow moves on where the sheet would go
 */
@Composable
fun PairFlow(pairing: PairingModel, inline: PairInline?, onClose: () -> Unit = {}) {
  val scope = rememberCoroutineScope()
  var phase by remember { mutableStateOf<PairPhase>(PairPhase.Choose) }
  var nearby by remember { mutableStateOf<List<NearbyComputer>>(emptyList()) }
  var browsing by remember { mutableStateOf(true) }
  // The code being paired with and its link: the very modules a scan read, else made from the link
  var code by remember { mutableStateOf<Pair<String, List<String>>?>(null) }

  /** The code to draw for a link: the rows a scan read, else the ones already shown for it, else made from its text */
  fun show(link: String, scanned: List<String>? = null) {
    if (scanned != null) code = link to scanned
    else if (code?.first != link) code = PairCodeRows.make(link)?.let { link to it }
  }

  /** Paired and done: the sheet goes, or the flow it stands in moves on */
  fun finish() {
    if (inline != null) inline.onDone() else onClose()
  }

  /** Pairs through a scanned, pasted or confirmed link; a scan hands over the modules it read */
  fun pair(link: String, scanned: List<String>? = null) {
    val name = try {
      pairing.service.computerName(link)
    } catch (e: Exception) {
      code = null
      phase = PairPhase.Failed(null, message(e, null), null)
      return
    }
    show(link, scanned)
    val asked = PairPhase.Pairing(link, name)
    phase = asked
    scope.launch {
      try {
        val computer = pairing.service.pair(link)
        if (phase != asked) return@launch
        phase = PairPhase.Joined(computer)
        pairing.paired(computer)
      } catch (e: CancellationException) {
        throw e
      } catch (e: Exception) {
        if (phase != asked) return@launch
        // Not answered: the same link may work once the computer or this network comes back
        val unanswered = e == PairingError.Unreachable || e == PairingError.NoRelay
        phase = PairPhase.Failed(name, message(e, name), if (unanswered) link else null)
      }
    }
  }

  // A link opened from outside while the sheet is up, or the one that raised it
  LaunchedEffect(pairing.pendingLink) {
    val link = pairing.pendingLink ?: return@LaunchedEffect
    pairing.pendingLink = null
    phase = try {
      PairPhase.Confirm(link, pairing.service.computerName(link)).also { show(link) }
    } catch (e: Exception) {
      code = null
      PairPhase.Failed(null, message(e, null), null)
    }
  }
  // Android 17 asks before the app reaches this network: browsing waits for the answer, so a refusal gets the system's
  // device picker once instead of both prompts at the same time
  val context = LocalContext.current
  var asked by remember { mutableStateOf(localNetworkGranted(context)) }
  val askNetwork = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { asked = true }
  LaunchedEffect(Unit) { if (!asked) askNetwork.launch(LOCAL_NETWORK) }
  // Browsing runs for the sheet's whole life, so coming back from the code page keeps the rows where they were
  LaunchedEffect(asked) {
    if (!asked) return@LaunchedEffect
    try {
      pairing.pinnedNearby?.let { nearby = listOf(it) }
      pairing.service.nearby().collect { found -> nearby = listOfNotNull(pairing.pinnedNearby) + found.filter { it.id != pairing.pinnedNearby?.id } }
    } finally {
      browsing = false
    }
  }

  val failed = phase is PairPhase.Failed
  BackHandler(enabled = phase is PairPhase.Code || failed) { phase = PairPhase.Choose }
  // In place, back from the form leaves the step
  BackHandler(enabled = inline != null && phase == PairPhase.Choose) { inline?.onBack() }
  // A sheet is a window of its own: its tags need the resource-id switch again for the scripts to find them
  Column(Modifier.fillMaxWidth().fillMaxHeight().navigationBarsPadding().imePadding().semantics { testTagsAsResourceId = true }) {
    SheetBar(
      title = when (phase) {
        // In place the form carries its own large title
        PairPhase.Choose -> if (inline == null) t("mobile.pair.add") else ""
        is PairPhase.Code -> t("mobile.pair.code.title")
        else -> ""
      },
      subtitle = (phase as? PairPhase.Code)?.computer?.name,
      onBack = when {
        phase is PairPhase.Code -> ({ phase = PairPhase.Choose })
        // In place: back from the form leaves the step, back from a failure goes to the form
        inline != null && phase == PairPhase.Choose -> inline.onBack
        inline != null && failed -> ({ phase = PairPhase.Choose })
        else -> null
      },
      onClose = if (inline == null) onClose else null,
    )
    AnimatedContent(
      targetState = phase,
      // The connect frame stays one view through confirm, pairing, paired and a failure, so its code dissolves and its
      // lines swap in place rather than the whole frame cross-fading at every phase
      contentKey = { if (it is PairPhase.Choose || it is PairPhase.Code) it::class else PairStageKey },
      transitionSpec = { fadeIn(tween(Motion.fade)) togetherWith fadeOut(tween(Motion.fade)) },
      label = "pairPhase",
      modifier = Modifier.weight(1f),
    ) { p ->
      when (p) {
        PairPhase.Choose -> PairAddForm(
          nearby = nearby, browsing = browsing,
          lead = if (inline != null) t("mobile.pair.connectTitle") to t("mobile.pair.connectHint") else null,
          onLink = { link, rows -> pair(link, rows) },
          // A code typed from a nearby computer's card has no picture to draw
          onNearby = { code = null; phase = PairPhase.Code(it) },
        )
        is PairPhase.Code -> PairCodePage(pairing, p.computer, onJoined = { phase = PairPhase.Joined(it) })
        else -> PairStage(pairing, p, code = code?.second, onFinish = ::finish, onConnect = { (p as? PairPhase.Confirm)?.let { pair(it.link) } },
          onRetry = { (p as? PairPhase.Failed)?.retryLink?.let { pair(it) } ?: run { phase = PairPhase.Choose } },
          onRescan = { phase = PairPhase.Choose })
      }
    }
  }
}

/** The sheet's bar: back (on the code page), the title with the computer's name under it, close (in a sheet) */
@Composable
private fun SheetBar(title: String, subtitle: String?, onBack: (() -> Unit)?, onClose: (() -> Unit)?) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.barCtl + Metrics.s2).padding(horizontal = Metrics.s1),
  ) {
    Box(Modifier.size(Metrics.barCtl), contentAlignment = Alignment.Center) {
      if (onBack != null) IconButton(onClick = onBack, modifier = Modifier.testTag("pair.back")) {
        Lucide("arrowLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.back"))
      }
    }
    Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
      Text(title, color = Palette.fgStrong, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, maxLines = 1)
      subtitle?.let { Text(it, color = Palette.fg3, fontSize = Type.text4, maxLines = 1) }
    }
    if (onClose != null) IconButton(onClick = onClose, modifier = Modifier.size(Metrics.barCtl).testTag("pair.close")) {
      Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
    } else Box(Modifier.size(Metrics.barCtl))
  }
}

/** The sheet's form: the camera card, the computers nearby, a pasted link; [lead] a large title and hint over them */
@Composable
private fun PairAddForm(
  nearby: List<NearbyComputer>,
  browsing: Boolean,
  lead: Pair<String, String>? = null,
  onLink: (String, List<String>?) -> Unit,
  onNearby: (NearbyComputer) -> Unit,
) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var notice by remember { mutableStateOf<String?>(null) }
  var noticeGen by remember { mutableIntStateOf(0) }
  Column(
    Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Metrics.pad),
    verticalArrangement = Arrangement.spacedBy(Metrics.s6),
  ) {
    lead?.let { (title, hint) ->
      Column(Modifier.padding(horizontal = Metrics.s1), verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
        Text(title, color = Palette.fgStrong, fontSize = Type.largeTitle, lineHeight = Type.largeTitleLine, fontWeight = FontWeight.Bold)
        Text(hint, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line)
      }
    }
    FormCard {
      Box(Modifier.fillMaxWidth().height(Metrics.pairScanCard).clip(RoundedCornerShape(Metrics.rCardRound))) {
        PairCameraCard(onLink = { link, rows -> if (isPairingLink(link)) onLink(link, rows) })
      }
    }
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2), modifier = Modifier.padding(horizontal = Metrics.pad)) {
        Text(t("mobile.pair.nearby"), color = Palette.fg3, fontSize = Type.text3, fontWeight = FontWeight.Medium)
        if (browsing) CircularProgressIndicator(Modifier.size(Metrics.chevron), color = Palette.fg3, strokeWidth = Metrics.pairCodeLine)
      }
      FormCard {
        if (nearby.isEmpty()) {
          FormRow(t("mobile.pair.nearby.none"), Modifier.testTag("pair.nearby.none")) {}
        }
        nearby.forEachIndexed { i, c ->
          if (i > 0) RowRule(lead = true)
          FormRow(c.name, Modifier.clickable { onNearby(c) }.testTag("pair.nearby:${c.id}"), lead = {
            Lucide(computerIcon(c.model), Palette.fg2, size = Metrics.rowMark)
          }) { Lucide("chevronRight", Palette.fg3, size = Metrics.chevron) }
        }
      }
    }
    FormCard {
      FormRow(notice ?: t("mobile.pair.paste"), Modifier.testTag("pair.paste"), lead = {
        Lucide("link", Palette.fg2, size = Metrics.rowMark)
      }) {
        OutlinedButton(
          onClick = {
            val text = clipboardText(context).trim()
            if (isPairingLink(text)) {
              onLink(text, null)
            } else {
              // Not a pairing link: the row says so for a moment, in place
              val gen = ++noticeGen
              notice = t("mobile.pair.paste.notLink")
              scope.launch {
                delay(PAIR_NOTICE_MS)
                if (gen == noticeGen) notice = null
              }
            }
          },
          modifier = Modifier.heightIn(min = Metrics.ctlSm).testTag("pair.paste.button"),
        ) { Text(t("desktop.menu.paste"), color = Palette.fg1, fontSize = Type.text3) }
      }
    }
    Spacer(Modifier.height(Metrics.page))
  }
}

private fun clipboardText(context: Context): String {
  val clip = context.getSystemService(ClipboardManager::class.java)?.primaryClip ?: return ""
  if (clip.itemCount == 0) return ""
  return clip.getItemAt(0).coerceToText(context)?.toString().orEmpty()
}

/**
 * The connect frame inside the sheet: the glyph, the computer's name, one sentence, one action. Paired is the moment
 * the computer answers the pairing, not its wire: the computer's card celebrates on that same answer, so the two screens
 * play it together while the wire comes up
 */
@Composable
private fun PairStage(
  pairing: PairingModel,
  phase: PairPhase,
  code: List<String>?,
  onFinish: () -> Unit,
  onConnect: () -> Unit = {},
  onRetry: () -> Unit = {},
  onRescan: () -> Unit = {},
) {
  val joined = (phase as? PairPhase.Joined)?.computer
  val followed = joined != null && pairing.selected?.id == joined.id
  val link = if (followed) pairing.link else null
  val pairedAt = remember(joined) { if (joined != null) System.nanoTime() else null }
  // The paired moment, with the computer's card celebrating the same answer; a refused or unanswered link
  val haptics = LocalHapticFeedback.current
  LaunchedEffect(joined != null) { if (joined != null) haptics.play(Haptic.Paired) }
  LaunchedEffect(phase is PairPhase.Failed) { if (phase is PairPhase.Failed) haptics.play(Haptic.Failed) }
  // Connected: the sheet goes once the paired moment has held as long as the card holds it; a wire that fails right
  // after pairing leaves the sheet too, and the list says what happened
  LaunchedEffect(joined, link is PairLink.Online, link is PairLink.Failed) {
    when {
      link is PairLink.Online -> {
        val held = pairedAt?.let { (System.nanoTime() - it) / 1_000_000 } ?: 0
        delay(maxOf(PAIR_HOLD_MS - held, PAIR_DONE_MS))
        onFinish()
      }
      link is PairLink.Failed -> onFinish()
    }
  }
  val name = when (phase) {
    is PairPhase.Confirm -> phase.name
    is PairPhase.Pairing -> phase.name
    is PairPhase.Joined -> phase.computer.name
    is PairPhase.Failed -> phase.name
    else -> null
  }
  val line = when (phase) {
    is PairPhase.Confirm -> t("mobile.pair.confirm")
    is PairPhase.Failed -> phase.message
    else -> t("mobile.pair.connecting")
  }
  val action = when (phase) {
    is PairPhase.Confirm -> t("mobile.pair.connect")
    is PairPhase.Failed -> if (phase.retryLink != null) t("mobile.pair.retry") else t("mobile.pair.scan.again")
    else -> null
  }
  PairHeroPlacement(box = if (code == null) Metrics.pairGlyphBox else Metrics.pairCode) {
    PairHero(
      icon = computerIcon(joined?.model),
      line = line,
      spin = phase is PairPhase.Pairing || phase is PairPhase.Joined,
      name = name,
      paired = joined != null,
      path = (link as? PairLink.Online)?.path,
      action = action,
      secondary = if (phase is PairPhase.Failed && phase.retryLink != null) t("mobile.pair.scan.again") else null,
      dim = phase is PairPhase.Failed,
      code = code,
      onAction = { if (phase is PairPhase.Confirm) onConnect() else onRetry() },
      onSecondary = onRescan,
    )
  }
}

/**
 * The six digits a nearby computer's pairing card shows (apps/ios PairCodePage.swift). One hidden field sits behind six
 * boxes, so typing advances on its own and a paste fills every box; the sixth digit sends the code. A refused code clears
 * the boxes and says why in a line that keeps its slot
 */
@Composable
private fun PairCodePage(pairing: PairingModel, computer: NearbyComputer, onJoined: (PairedComputer) -> Unit) {
  val scope = rememberCoroutineScope()
  val haptics = LocalHapticFeedback.current
  val focus = remember { FocusRequester() }
  var code by remember { mutableStateOf("") }
  var checking by remember { mutableStateOf(false) }
  var error by remember { mutableStateOf<String?>(null) }
  var focused by remember { mutableStateOf(false) }

  fun submit(digits: String) {
    if (checking) return
    checking = true
    error = null
    scope.launch {
      try {
        val paired = pairing.service.pair(computer, digits)
        checking = false
        onJoined(paired)
        pairing.paired(paired)
      } catch (e: CancellationException) {
        throw e
      } catch (e: Exception) {
        checking = false
        haptics.play(Haptic.Failed)
        error = codeMessage(e, computer.name)
        code = ""
      }
    }
  }

  LaunchedEffect(Unit) {
    // Focus once the page settled, so the keyboard does not ride in with the page
    delay(Motion.fade.toLong())
    runCatching { focus.requestFocus() }
  }
  Column(
    Modifier.fillMaxSize().padding(horizontal = Metrics.pad).padding(top = Metrics.page * 2),
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Text(t("mobile.pair.code.hint"), color = Palette.fg2, fontSize = Type.body, lineHeight = Type.bodyLine, textAlign = TextAlign.Center)
    Spacer(Modifier.height(Metrics.page))
    Box(contentAlignment = Alignment.Center) {
      BasicTextField(
        // The cursor pinned to the end: a pasted code then takes the next typed digit after it, not in front
        value = TextFieldValue(code, TextRange(code.length)),
        onValueChange = { typed ->
          // Typed while a code is checked: dropped, so the keyboard stays put
          if (checking) return@BasicTextField
          val digits = typed.text.filter { it.isDigit() }.take(CODE_LENGTH)
          code = digits
          if (digits.isNotEmpty()) error = null
          if (digits.length == CODE_LENGTH) submit(digits)
        },
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword),
        singleLine = true,
        // Kept in the layout but painted clear: a field that is not composed takes no focus
        textStyle = TextStyle(color = Color.Transparent),
        cursorBrush = androidx.compose.ui.graphics.SolidColor(Color.Transparent),
        modifier = Modifier.matchParentSize().focusRequester(focus).onFocusChanged { focused = it.isFocused }.testTag("pair.code.field"),
      )
      Row(Modifier.clickable { runCatching { focus.requestFocus() } }) {
        for (i in 0 until CODE_LENGTH) {
          if (i == CODE_LENGTH / 2) Spacer(Modifier.width(Metrics.pairCodeSplit - Metrics.s2))
          if (i > 0) Spacer(Modifier.width(Metrics.s2))
          val next = focused && !checking && i == code.length
          Box(
            Modifier.size(Metrics.pairCodeBox, Metrics.pairCodeBoxH).codeBox(next, Palette.line, Palette.formRow, Palette.fg1),
            contentAlignment = Alignment.Center,
          ) {
            code.getOrNull(i)?.let {
              Text(it.toString(), color = if (checking) Palette.fg2 else Palette.fgStrong, fontSize = Type.pairDigit, fontWeight = FontWeight.SemiBold)
            }
          }
        }
      }
    }
    // Checking, or why the last code was refused; the slot stays, so nothing under it moves
    Box(Modifier.fillMaxWidth().heightIn(min = Metrics.pairStatusSlot).padding(top = Metrics.gap), contentAlignment = Alignment.TopCenter) {
      if (checking) CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg2, strokeWidth = Metrics.pairCodeLine)
      else error?.let { Text(it, color = Palette.danger, fontSize = Type.text3, lineHeight = Type.text3Line, textAlign = TextAlign.Center, modifier = Modifier.testTag("pair.code.error")) }
    }
    // A paste fills every box, the sixth digit sending the code as typing does (apps/ios PairCodePage's PasteButton): a
    // quiet capsule in the code boxes' fill
    val context = LocalContext.current
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
      modifier = Modifier.padding(top = Metrics.s2).height(Metrics.ctlSm)
        .clip(RoundedCornerShape(percent = 50)).background(Palette.formRow)
        .clickable(enabled = !checking) {
          val digits = clipboardText(context).filter { it.isDigit() }.take(CODE_LENGTH)
          if (digits.isEmpty() || checking) return@clickable
          code = digits
          error = null
          if (digits.length == CODE_LENGTH) submit(digits)
        }
        .padding(horizontal = Metrics.pad).testTag("pair.code.paste"),
    ) {
      Lucide("clipboardPaste", Palette.fg1, size = Metrics.icon)
      Text(t("desktop.menu.paste"), color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.Medium)
    }
  }
}

/** Why a scanned or opened link did not pair: a stale code asks for the new one the computer's card shows by now */
private fun message(e: Throwable, name: String?): String {
  val named = "name" to (name ?: "")
  return when (e) {
    PairingError.Expired -> t("mobile.pair.scan.expired")
    PairingError.Used -> t("mobile.pair.scan.used")
    PairingError.Closed -> if (name != null) t("mobile.pair.closed", named) else t("mobile.pair.failed")
    PairingError.Unreachable -> if (name != null) t("mobile.pair.unreachable", named) else t("mobile.pair.failed")
    // No relay on one side: only the same Wi-Fi joins the two now
    PairingError.NoRelay -> if (name != null) t("mobile.pair.noRelay", named) else t("mobile.pair.failed")
    PairingError.Impostor -> t("mobile.pair.impostor")
    PairingError.InvalidLink -> t("mobile.pair.invalidLink")
    else -> t("mobile.pair.failed")
  }
}

/** Why a code was refused, in the code page's own words */
private fun codeMessage(e: Throwable, name: String): String = when (e) {
  PairingError.Wrong -> t("mobile.pair.code.wrong")
  PairingError.Expired -> t("mobile.pair.code.expired")
  PairingError.Used -> t("mobile.pair.code.used")
  PairingError.Closed -> t("mobile.pair.closed", "name" to name)
  // A code pairs over this network only, so no relay never stands in its way
  PairingError.Unreachable, PairingError.NoRelay -> t("mobile.pair.unreachable", "name" to name)
  PairingError.Impostor -> t("mobile.pair.impostor")
  else -> t("mobile.pair.failed")
}
