package cc.hotic.acpira.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.layout
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import androidx.compose.ui.zIndex
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import cc.hotic.acpira.design.GooMotion
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.popFg3
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ComposerControls
import cc.hotic.acpira.logic.ModelChoices
import cc.hotic.acpira.logic.ModelVisibility
import cc.hotic.acpira.logic.UsageBreakdown
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.Usage
import cc.hotic.acpira.store.MediaSource
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * An attachment waiting in the composer: what goes on the wire, its name, and a thumbnail for an image. [source] is the
 * sent prompt's own attachment an edit took back in: it is drawn and opened from that (a stored photo, a stored text,
 * a selection), while the edit sends it back by its index
 */
class Staged(val draft: Draft, val name: String, val thumb: ImageBitmap? = null, val source: Attachment? = null)

/**
 * The open liquid picker over the composer (apps/ios Design/Goo.swift): which chip's beans are out (`attach` or
 * `model`), whose panel is open (1…, 0 for none), and the beans the open chip offers. The composer fills it, the
 * screen draws it over its page ([GooLayer]), under the composer's bar
 */
@Stable
class Goo {
  var open by mutableStateOf<String?>(null)
  var stage by mutableIntStateOf(0)
  /** The chips' frames on the screen, so the beans rise out of the chip that opened them */
  var chips by mutableStateOf<Map<String, Rect>>(emptyMap())
  var beans by mutableStateOf<List<Bean>>(emptyList())
  /** The chip opened last, which a closing picker goes home into */
  var last by mutableStateOf("model")
  /** The level of beans out instead of the chip's own ([SIDEKICK_LEVEL]), one more level to step back from */
  var deeper by mutableStateOf<String?>(null)
  /** The beans are tucked into the chip while they trade for another level's ([enter]) */
  var held by mutableStateOf(false)
  private var swap: Job? = null

  fun toggle(id: String) {
    // Opening starts on the chip's own beans; closing leaves a deeper level as it is, so it shrinks home unchanged
    if (open == id) back() else { open = id; stage = 0; deeper = null }
  }

  /** One level back: an open panel into its bean, a deeper level's beans for the chip's own, else the beans into the chip */
  fun back() {
    when {
      stage > 0 -> stage = 0
      deeper != null -> enter(null)
      else -> close()
    }
  }

  /**
   * Trades the beans for another level's (`sidekick`, or null for the chip's own): they head home into the chip, the
   * level changes while they are small there, and the new beans squeeze out. Without motion it swaps at once
   */
  fun enter(level: String?) {
    stage = 0
    val scope = scope
    if (!motion || scope == null) { deeper = level; return }
    held = true
    swap?.cancel()
    swap = scope.launch {
      delay(GooMotion.levelSwapMs)
      // Closed meanwhile (a tap outside): the level stays as it was, and the next open starts on the chip's own beans
      if (open != null) deeper = level
      held = false
    }
  }

  /** Where [enter] runs its trade, and whether it animates; the composer binds them */
  internal var scope: CoroutineScope? = null
  internal var motion = true

  fun close() {
    open = null
    stage = 0
  }
}

/**
 * One bean a chip pulls out: its glyph, the value beside it, and either a panel of choices or an action in place
 * (Fast flips, an attach source opens its picker)
 */
class Bean(
  val key: String,
  val title: String,
  val caption: String,
  val glyph: @Composable () -> Unit,
  val rows: Int = 0,
  val panel: (@Composable () -> Unit)? = null,
  val tap: (() -> Unit)? = null,
  /** A line under the value saying what the setting is (the sidekick's), wrapped to the room beside the bean */
  val note: String? = null,
)

/**
 * The phone composer (apps/ios Chat/Composer.swift). At rest one bar: `+` and the model's mark at its start, the field,
 * the context ring and send at its end (dim with nothing to send, stop while a turn runs). Typing (focused, or a draft
 * waiting) the bar grows into one card: the field across the whole width over a row of the chips and send, so the text
 * is never squeezed between controls. Nothing comes or goes between the two: the chips keep the bottom-left corner and
 * send the bottom-right one, the card grows up around them, and the field stays the same view throughout, so focus
 * holds. `+` and the mark are liquid chips whose beans [GooLayer] draws over the page
 */
@Composable
fun Composer(
  text: String,
  onText: (String) -> Unit,
  staged: List<Staged>,
  onStaged: (List<Staged>) -> Unit,
  controls: SessionControls,
  /** The agent's mark, for a session whose controls name no model */
  agentMark: String,
  agentName: String,
  running: Boolean,
  usage: Usage?,
  placeholder: String,
  goo: Goo,
  motion: Boolean,
  onSend: () -> Unit,
  onStop: () -> Unit,
  onSetMode: (String) -> Unit,
  onSetConfig: (String, String) -> Unit,
  onToast: (String) -> Unit,
  modifier: Modifier = Modifier,
  /** The `/` commands, the `@` personas and file search (ui/Suggestions.kt) */
  assist: ComposerAssist = ComposerAssist(),
  /** The built-in agent's sidekick is working a round: its colour breathes on the model chip */
  sidekickLive: Boolean = false,
  /** The context ring's card: the auto-compact threshold it marks, and compaction when the agent offers `/compact` */
  usageCard: UsageCardActions = UsageCardActions(),
  /** The link is resyncing: stop rests unlit, as the run it would stop may be over (Composer.swift SendButton) */
  syncing: Boolean = false,
) {
  val context = LocalContext.current
  val haptics = LocalHapticFeedback.current
  val filled = text.isNotBlank() || staged.isNotEmpty()
  var focused by remember { mutableStateOf(false) }
  val expanded = focused || filled
  val focus = remember { FocusRequester() }
  val keyboard = LocalSoftwareKeyboardController.current
  // The field's text with its caret, the `/` and `@` tokens under it, and the lists they raise (ui/ComposerTokens.kt)
  val prompt = rememberPromptState(text, onText)
  val longPaste = rememberLongPaste(staged, onStaged, onToast)
  val tokens = rememberTokens(prompt, assist, staged, onStaged) { focus.requestFocus(); keyboard?.show() }
  // The whole-screen editor is open (ExpandedComposer), and the field's laid-out lines, from which it is offered
  var fullEditor by remember { mutableStateOf(false) }
  var lines by remember { mutableIntStateOf(1) }
  var stripHeight by remember { mutableIntStateOf(0) }
  val offersExpand = expanded && lines >= EXPAND_LINES
  val choices = rememberChoices(controls, LocalHiddenModels.current[agentMark].orEmpty(), onSetConfig)
  // The sidekick level's trade runs on the composer's scope
  val gooScope = rememberCoroutineScope()
  SideEffect { goo.scope = gooScope; goo.motion = motion }
  val modelBeans = modelBeans(choices, onSetMode, onSetConfig, goo)
  val attach = attachBeans(staged, onStaged, goo, onToast, paste = rememberPasteReady()) + summonBeans(assist.personas, goo, tokens::summon)
  // The open chip's beans, for the screen's layer; a closing picker keeps the beans it goes home with
  val open = goo.open
  SideEffect {
    when (open) {
      "attach" -> goo.beans = attach
      "model" -> goo.beans = modelBeans
    }
    if (open != null) goo.last = open
  }

  // The card's shape and the field's insets move with one curve, as the iOS card grows around its corners
  fun anim() = if (motion) spring<Dp>(dampingRatio = COMPOSER_DAMPING, stiffness = COMPOSER_STIFFNESS) else snap()
  val lead = if (modelBeans.isEmpty()) Metrics.barCtl else Metrics.barCtl * 2
  val line = (Metrics.barCtl - FIELD_LINE) / 2
  val corner by animateDpAsState(if (expanded) Metrics.rComposer else Metrics.barCtl / 2, anim(), label = "corner")
  val padStart by animateDpAsState(if (expanded) Metrics.s3 + Metrics.s1 * 2 else lead + Metrics.s1, anim(), label = "start")
  val ring = usage?.takeIf { it.size > 0 }
  // In the card the text stops short of the expand corner's column, so its arrival never rewraps the text
  val padEnd by animateDpAsState(
    if (expanded) Metrics.s3 + Metrics.icon + Metrics.s1 else Metrics.barCtl + Metrics.s1 + (if (ring != null) Metrics.ctlSm else Metrics.s0), anim(), label = "end",
  )
  val padBottom by animateDpAsState(if (expanded) Metrics.s1 + Metrics.barCtl + Metrics.s1 else line, anim(), label = "bottom")
  val springInset by animateDpAsState(if (expanded) Metrics.s1 else Metrics.s0, anim(), label = "inset")
  // The spring overshoots below zero on its way back to s0, which padding refuses
  val inset = springInset.coerceAtLeast(Metrics.s0)
  val shape = RoundedCornerShape(corner)

  ComposerColumn(modifier, suggestions = { Suggestions(tokens, assist, motion) }) {
    Surface(
      color = Palette.bg1,
      shape = shape,
      shadowElevation = Metrics.barShadow,
      // Handed over to the list's bar across a push or pop while it rests as the bar (ui/BarHandoff.kt)
      modifier = Modifier.handoffComposer(atRest = !expanded).fillMaxWidth().border(Metrics.hairline, Palette.line, shape),
    ) {
      Box(
        Modifier
          .handoffFace(HandoffPart.Composer)
          .fillMaxWidth()
          .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {
            focus.requestFocus()
            keyboard?.show()
          },
      ) {
        Column(Modifier.fillMaxWidth().alpha(if (goo.open != null) GOO_RECEDE else 1f).blur(if (goo.open != null) Metrics.s1 else Metrics.s0)) {
          if (staged.isNotEmpty()) StagedStrip(staged, onStaged, Modifier.padding(start = Metrics.s3 + Metrics.s1, top = Metrics.s2).onSizeChanged { stripHeight = it.height })
          Box(Modifier.fillMaxWidth().padding(start = padStart, end = padEnd, top = if (staged.isEmpty()) line else Metrics.s2, bottom = padBottom)) {
            if (text.isEmpty()) {
              // The one hint for the bar and the card, so opening the card only carries it up with the card's top
              Text(
                placeholder, color = Palette.barHint, fontSize = Type.field, lineHeight = Type.fieldLine, fontWeight = FontWeight.Medium,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
              )
            }
            // The card's cap in lines, fewer where the keyboard leaves little room (LayoutRoom.lines)
            PromptField(
              prompt,
              commands = assist.commands.map { it.name }, summons = assist.personas.map { it.name },
              onLongPaste = longPaste,
              maxLines = if (expanded) composerLines(MAX_LINES) else 1,
              below = tokens.commandHint,
              onLines = { lines = it },
              fieldModifier = Modifier
                .focusRequester(focus)
                .onFocusChanged { focused = it.isFocused }
                .testTag("composer.field"),
            )
          }
        }
        // The chips' row at the start, inset from the card's corner as send is from its own
        Row(Modifier.align(Alignment.BottomStart).padding(start = inset, bottom = inset)) {
          Chip("attach", goo, Modifier.testTag("composer.attach")) {
            AttachGlyph(goo, motion)
          }
          if (modelBeans.isNotEmpty()) {
            Chip("model", goo, Modifier.testTag("composer.model")) {
              ModelGlyph(goo, choices, agentMark, agentName, sidekickLive, motion)
            }
          }
        }
        // The expand corner at the card's top trailing side, centred on the first line under the strip
        if (offersExpand) {
          ExpandCorner(
            Modifier.align(Alignment.TopEnd).padding(top = with(LocalDensity.current) { stripHeight.toDp() } + (if (staged.isEmpty()) line else Metrics.s2))
              .alpha(if (goo.open != null) GOO_RECEDE else 1f),
          ) { fullEditor = true }
        }
        Row(
          Modifier.align(Alignment.BottomEnd).padding(end = inset, bottom = inset).alpha(if (goo.open != null) GOO_RECEDE else 1f),
          verticalAlignment = Alignment.CenterVertically,
        ) {
          if (ring != null) ContextRing(ring, motion, usageCard, running)
          // A staged video still compressing holds the draft back, and the button does not stop the turn meanwhile
          val waiting = VideoJobs.pending(staged)
          SendButton(running = running && !waiting, filled = filled && !waiting, motion = motion, syncing = syncing) {
            if (!waiting) {
              if (running && !filled) { haptics.play(Haptic.Stop); onStop() } else { haptics.play(Haptic.Send); onSend() }
            }
          }
        }
      }
    }
  }
  AttachPickers(goo, onStaged, staged, onToast, context)
  if (fullEditor) {
    ExpandedComposer(
      text, onText, staged, onStaged, assist, motion,
      onSend = { fullEditor = false; haptics.play(Haptic.Send); onSend() },
      // Collapsing gives the card its focus back
      onCollapse = { fullEditor = false; focus.requestFocus() },
      onToast = onToast,
    )
  }
}

/** A liquid chip: the bar-tall slot its glyph sits in; a tap opens its beans, or steps back one level */
@Composable
private fun Chip(id: String, goo: Goo, modifier: Modifier, glyph: @Composable () -> Unit) {
  Box(
    modifier
      .size(Metrics.barCtl)
      .clip(CircleShape)
      .clickable { goo.toggle(id) }
      .onGloballyPositioned { goo.chips = goo.chips + (id to it.boundsInRoot()) },
    contentAlignment = Alignment.Center,
  ) { glyph() }
}

/** `+`, turned × while its beans are out, ‹ over a panel */
@Composable
private fun AttachGlyph(goo: Goo, motion: Boolean) {
  val open = goo.open == "attach"
  val turn by animateFloatAsState(if (open) PLUS_TURN else 0f, if (motion) spring() else snap(), label = "plus")
  if (open && goo.stage > 0) Lucide("chevronLeft", Palette.fg1, size = Metrics.barIcon)
  else Lucide("plus", Palette.fg1, Modifier.rotate(turn), size = Metrics.barIcon, contentDescription = t("mobile.attach"))
}

/**
 * The model's mark alone, large, with Fast as a bolt on its shoulder and, while the sidekick works a round, a dot in its
 * colour breathing on the mark's foot; × over the beans, ‹ over a panel or the sidekick's beans
 */
@Composable
private fun ModelGlyph(goo: Goo, choices: ModelChoices, agentMark: String, agentName: String, sidekickLive: Boolean, motion: Boolean) {
  if (goo.open == "model") {
    val inside = goo.stage > 0 || goo.deeper != null
    Lucide(if (inside) "chevronLeft" else "x", Palette.fg1, size = Metrics.gooBack, contentDescription = t(if (inside) "common.back" else "common.close"))
    return
  }
  val sidekick = if (sidekickLive) sidekickTint(choices) else null
  Box {
    if (sidekick != null) {
      SidekickPulse(sidekick, motion, Modifier.align(Alignment.BottomEnd).layout { m, c ->
        val p = m.measure(c)
        layout(p.width, p.height) { p.place(Metrics.s1.roundToPx(), Metrics.s1.roundToPx()) }
      }.zIndex(1f).testTag("composer.sidekickLive"))
    }
    val cur = choices.cur
    if (cur != null) VendorMark(cur.brand, cur.name, Palette.fg1, size = Metrics.barIcon, fit = true)
    else VendorMark(agentMark, agentName, Palette.fg1, size = Metrics.barIcon, fit = true)
    if (choices.fast?.on == true) {
      Box(Modifier.align(Alignment.TopEnd).layout { m, c ->
        val p = m.measure(c)
        layout(p.width, p.height) { p.place(Metrics.s1.roundToPx(), -Metrics.s1.roundToPx()) }
      }) { FastGlyph(true, Metrics.chevron) }
    }
  }
}

/** The Fast bolt, in --fast while Fast is on */
@Composable
internal fun FastGlyph(on: Boolean, size: Dp) {
  Lucide("zap", if (on) Palette.fast else Palette.fg1, size = size)
}

/**
 * The session's controls as the model chip reads them, rebuilt only when they change; [hidden] (the agent's settings
 * `hiddenOptions`, control id → family keys) thins the model menus, the model in use always stays (Goo.swift `visible`)
 */
@Composable
private fun rememberChoices(controls: SessionControls, hidden: Map<String, List<String>>, onSetConfig: (String, String) -> Unit): ModelChoices {
  val context = LocalContext.current
  val standard = t("composer.standard")
  return remember(controls, hidden, standard) {
    val brand = { o: SessionOption -> brandOf(context, o) }
    // The sidekick's list is thinned as the lead's is
    val models = if (hidden.isEmpty()) emptySet() else ComposerControls.split(controls.options, brand).let { s -> s.models + s.sidekick }.map { it.id }.toSet()
    val shown = if (models.isEmpty()) controls else controls.copy(options = controls.options.map { c ->
      if (c.id in models) c.copy(options = ModelVisibility.visibleOptions(c.options, hidden[c.id], c.value)) else c
    })
    ModelChoices(shown, brand, standard) { id, value -> onSetConfig(id, value) }
  }
}

/** The vendor of an option: its wire id is the real identity ("asgard/kimi-k3"), the name only a label */
internal fun brandOf(context: Context, o: SessionOption): String? = IconStore.brandOf(context, o.id) ?: IconStore.brandOf(context, o.name)

/**
 * The model chip's beans (Goo.swift LiquidControls): the model, a Fusion pair's lead, the reasoning level, Fast, the
 * working mode, the agent's modes, the approval level, then the sidekick, each with its value beside it. A pick folds
 * the panel back into its bean. While the sidekick's level is out ([Goo.deeper]) the beans are the sidekick's own
 */
@Composable
private fun modelBeans(c: ModelChoices, onSetMode: (String) -> Unit, onSetConfig: (String, String) -> Unit, goo: Goo): List<Bean> {
  val fold = { goo.stage = 0 }
  val haptics = LocalHapticFeedback.current
  if (goo.deeper == SIDEKICK_LEVEL && (c.fusion != null || c.sidekick != null)) return sidekickBeans(c, onSetConfig, fold, haptics)
  val out = mutableListOf<Bean>()
  val cur = c.cur
  if (c.hasModel && cur != null) {
    out += Bean(
      "model", t("stats.model"), cur.name,
      glyph = { VendorMark(cur.brand, cur.name, Palette.fg1, size = Metrics.barIcon, fit = true) },
      rows = c.families.size,
      panel = {
        GooList(c.families.indexOfFirst { it.key == cur.key }) {
          for (f in c.families) {
            GooRow(f.name, f.key == cur.key, glyph = { VendorMark(f.brand, f.name, Palette.fgStrong, size = Metrics.iconCtl, fit = true) }) {
              c.pickFamily(f); fold()
            }
          }
        }
      },
    )
  }
  if (c.fusion != null) out += fusionModelBean(c, sidekick = false, fold)
  val effort = c.effort
  if (c.hasEffort && effort != null) {
    val levels = if (effort.levels.size > 1) effort.levels else emptyList()
    val thinking = effort.thinking
    val name = if (thinking?.on == false) t("mobile.thinking.off")
      else levels.firstOrNull { it.first == effort.value }?.second ?: if (thinking != null) "Thinking" else ""
    out += Bean(
      "effort", t("composer.effort"), name,
      glyph = { Lucide("brain", if (thinking?.on == false) Palette.fg3 else Palette.fg1, size = Metrics.barIcon) },
      rows = levels.size + (if (thinking != null) 1 else 0),
      panel = {
        GooList(levels.indexOfFirst { it.first == effort.value }) {
          if (thinking != null) {
            // The Thinking switch flips in place; the panel stays out
            GooRow("Thinking", thinking.on, glyph = { Lucide("brain", Palette.fgStrong, size = Metrics.icon) }, trailing = {
              Switch(checked = thinking.on, onCheckedChange = { thinking.set(it) }, enabled = !thinking.disabled)
            }) { if (!thinking.disabled) thinking.set(!thinking.on) }
          }
          for ((id, label) in levels) GooRow(label, id == effort.value) { effort.pick(id); fold() }
        }
      },
    )
  }
  c.fast?.let { fast ->
    // Every agent's Fast reads the same: the bolt and its name, lit on a tap, no panel
    out += Bean("fast", "Fast", "Fast", glyph = { FastGlyph(fast.on, Metrics.barIcon) }, tap = {
      if (!fast.disabled) { haptics.play(Haptic.Pick); fast.set(!fast.on) }
    })
  }
  c.collaboration?.let { work ->
    val current = work.options.firstOrNull { it.id == work.value } ?: work.options.firstOrNull()
    out += Bean(
      "work", work.name, current?.let(::workLabel) ?: work.name,
      glyph = { Lucide(current?.let(::modeIcon) ?: "circleDashed", Palette.fg1, size = Metrics.barIcon) },
      rows = work.options.size,
      panel = {
        GooList(work.options.indexOfFirst { it.id == current?.id }) {
          for (o in work.options) {
            GooRow(workLabel(o), o.id == current?.id, glyph = { Lucide(modeIcon(o), Palette.fgStrong, size = Metrics.icon) }) {
              onSetConfig(work.id, o.id); fold()
            }
          }
        }
      },
    )
  }
  if (c.modes.isNotEmpty()) {
    val mode = c.mode
    out += Bean(
      "mode", t("composer.mode"), mode?.let(::modeLabel) ?: t("composer.mode"),
      glyph = { Lucide(mode?.let(::modeIcon) ?: "circleDashed", if (mode?.kind == "full_access") Palette.warn else Palette.fg1, size = Metrics.barIcon) },
      rows = c.modes.size,
      panel = {
        GooList(c.modes.indexOfFirst { it.id == mode?.id }) {
          for (m in c.modes) {
            GooRow(modeLabel(m), m.id == mode?.id, glyph = {
              Lucide(modeIcon(m), if (m.kind == "full_access") Palette.warn else Palette.fgStrong, size = Metrics.icon)
            }) { onSetMode(m.id); fold() }
          }
        }
      },
    )
  }
  c.access?.let { access ->
    val level = access.options.firstOrNull { it.id == access.value } ?: access.options.firstOrNull()
    out += Bean(
      "access", t("composer.access"), level?.let(::accessLabel) ?: access.name,
      glyph = { Lucide(level?.let(::modeIcon) ?: "circleDashed", Palette.fg1, size = Metrics.barIcon) },
      rows = access.options.size,
      panel = {
        GooList(access.options.indexOfFirst { it.id == level?.id }) {
          for (o in access.options) {
            GooRow(accessLabel(o), o.id == level?.id, glyph = { Lucide(modeIcon(o), Palette.fgStrong, size = Metrics.icon) }) {
              onSetConfig(access.id, o.id); fold()
            }
          }
        }
      },
    )
  }
  sidekickBean(c) { goo.enter(SIDEKICK_LEVEL) }?.let { out += it }
  return out
}

/** Where an attach bean sends the reader: the photo picker, the camera, the file picker */
private enum class AttachSource { Photos, Camera, Files, Paste }

/** The source a bean asked for, picked up by [AttachPickers] */
private val pendingSource = mutableStateOf<AttachSource?>(null)

/** Attach as a liquid chip: the photo, camera and file sources squeeze out of `+`, each named beside it */
@Composable
private fun attachBeans(staged: List<Staged>, onStaged: (List<Staged>) -> Unit, goo: Goo, onToast: (String) -> Unit, paste: Boolean): List<Bean> {
  fun source(key: String, icon: String, label: String, s: AttachSource) = Bean(
    key, label, label,
    glyph = { Lucide(icon, Palette.fg1, size = Metrics.barIcon) },
    tap = { goo.close(); pendingSource.value = s },
  )
  return listOf(
    source("photos", "image", t("mobile.attach.photos"), AttachSource.Photos),
    source("camera", "camera", t("mobile.attach.camera"), AttachSource.Camera),
    source("files", "paperclip", t("mobile.attach.files"), AttachSource.Files),
  ) + listOfNotNull(
    // While the clipboard holds an image, `+` offers to paste it (Goo.swift sources)
    if (paste) source("paste", "clipboardPaste", t("desktop.menu.paste"), AttachSource.Paste) else null,
  )
}

/**
 * The system pickers behind the attach beans: photos through the photo picker (no permission), the camera into a
 * file this app shares with it, any file through the document picker. Images are cut to [IMAGE_SIDE] px as JPEG
 * (AttachmentImage.thumbnail); files travel whole, inside the one message, so the draft stays under [MESSAGE_BYTES]
 */
@Composable
private fun AttachPickers(goo: Goo, onStaged: (List<Staged>) -> Unit, staged: List<Staged>, onToast: (String) -> Unit, context: Context) {
  val scope = rememberCoroutineScope()
  val current by androidx.compose.runtime.rememberUpdatedState(staged)
  val add: (List<Staged>) -> Unit = { more ->
    val next = current + more
    if (next.sumOf { draftBytes(it.draft) } > MESSAGE_BYTES) onToast(t("mobile.video.tooBig", "mb" to MESSAGE_BYTES / 4 * 3 / (1 shl 20)))
    else onStaged(next)
  }
  // A picked video swaps its pending tile for the ready one, or drops it when its copy failed or is too large to go
  val replace: (String, Staged?) -> Unit = { uri, ready ->
    val at = current.indexOfFirst { (it.draft as? Draft.File)?.uri == uri }
    if (at >= 0) {
      val without = current.filterIndexed { j, _ -> j != at }
      val next = if (ready == null) without else current.toMutableList().also { it[at] = ready }
      if (ready != null && next.sumOf { draftBytes(it.draft) } > MESSAGE_BYTES) {
        onToast(t("mobile.video.tooBig", "mb" to MESSAGE_BYTES / 4 * 3 / (1 shl 20)))
        onStaged(without)
      } else onStaged(next)
    }
  }
  // Videos take their tile at once and fill it as they compress; photos arrive as they load (VideoAttach.swift)
  val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_PHOTOS)) { uris ->
    val (videos, images) = uris.partition { context.contentResolver.getType(it)?.startsWith("video/") == true }
    val pending = ArrayList<Staged>()
    for (v in videos) VideoJobs.start(context, v, stage = { pending += it }, replace = replace, failed = onToast)
    if (pending.isNotEmpty()) onStaged(current + pending)
    if (images.isNotEmpty()) scope.launch { add(images.mapNotNull { stageImage(context, it) }) }
  }
  var shot by remember { mutableStateOf<Uri?>(null) }
  val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { ok ->
    val uri = shot
    if (ok && uri != null) scope.launch { stageImage(context, uri)?.let { add(listOf(it)) } }
  }
  val takePicture = {
    val dir = File(context.cacheDir, "camera").apply { mkdirs() }
    val file = File(dir, "photo-${System.currentTimeMillis()}.jpg")
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
    shot = uri
    camera.launch(uri)
  }
  // The app declares CAMERA (pairing's scanner), so the system camera only opens once it is granted
  val cameraGrant = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
    if (granted) takePicture() else onToast(t("mobile.pair.camera.denied"))
  }
  val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
    scope.launch { add(uris.mapNotNull { stageFile(context, it) }) }
  }
  val source by pendingSource
  LaunchedEffect(source) {
    when (source ?: return@LaunchedEffect) {
      AttachSource.Photos -> photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
      AttachSource.Camera -> {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) takePicture()
        else cameraGrant.launch(Manifest.permission.CAMERA)
      }
      AttachSource.Files -> files.launch(arrayOf("*/*"))
      // The clipboard's image, staged as a picked photo is
      AttachSource.Paste -> clipboardImage(context)?.let { uri -> scope.launch { stageImage(context, uri)?.let { add(listOf(it)) } } }
    }
    pendingSource.value = null
  }
}

private fun draftBytes(d: Draft): Int = when (d) {
  is Draft.Image -> d.data.length
  is Draft.Text -> d.text.length
  is Draft.File -> d.data?.length ?: 0
  is Draft.Quote -> d.text.length
}

private fun displayName(context: Context, uri: Uri): String? =
  context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
    if (c.moveToFirst()) c.getString(0) else null
  }

/** A picked photo as the composer sends it: the longest side cut to [IMAGE_SIDE] px, JPEG, base64 */
private suspend fun stageImage(context: Context, uri: Uri): Staged? = withContext(Dispatchers.IO) {
  runCatching {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
      val scale = min(1f, IMAGE_SIDE.toFloat() / max(info.size.width, info.size.height))
      decoder.setTargetSize(max(1, (info.size.width * scale).toInt()), max(1, (info.size.height * scale).toInt()))
      decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
    }
    val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, JPEG_QUALITY, it) }.toByteArray()
    val name = displayName(context, uri) ?: "image.jpg"
    Staged(Draft.Image("image/jpeg", Base64.encodeToString(bytes, Base64.NO_WRAP), name), name, bitmap.asImageBitmap())
  }.getOrNull()
}

/** A picked file with its bytes: the engine stages it on the computer and links the staged path */
private suspend fun stageFile(context: Context, uri: Uri): Staged? = withContext(Dispatchers.IO) {
  runCatching {
    val bytes = context.contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: return@runCatching null
    val name = displayName(context, uri) ?: uri.lastPathSegment ?: "file"
    Staged(Draft.File(uri.toString(), name, Base64.encodeToString(bytes, Base64.NO_WRAP)), name)
  }.getOrNull()
}

/**
 * The staged attachments inside the card over the text (Composer.swift StagedAttachments): image tiles, video tiles and
 * file chips, each with its remove mark. A tile opens its picture in the Lightbox, a video plays, a pasted text shows
 * whole, a file picked on the phone opens in the phone's viewer and a file on the computer in the chat's file page.
 * [onNavigate] runs before a picture or a file opens (the whole-screen editor gets out of the way); a video and a
 * pasted text open in dialogs over it
 */
@Composable
internal fun StagedStrip(staged: List<Staged>, onStaged: (List<Staged>) -> Unit, modifier: Modifier, onNavigate: (() -> Unit)? = null) {
  val context = LocalContext.current
  val media = LocalMedia.current
  var playing by remember { mutableStateOf<Pair<File, String>?>(null) }
  var reading by remember { mutableStateOf<Staged?>(null) }
  playing?.let { (file, name) -> VideoPlayer(file, name) { playing = null } }
  reading?.let { s -> StagedText(s) { reading = null } }
  val opener = rememberFileOpener(onNavigate)
  Row(
    modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(end = Metrics.s3),
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    // Quotes from the transcript lead as one annotations chip (ui/Quotes.kt)
    QuoteChip(staged, onStaged)
    staged.forEachIndexed { i, s ->
      if (s.draft is Draft.Quote) return@forEachIndexed
      Box {
        val thumb = s.thumb
        val tile = Modifier.size(Metrics.stagedTile).clip(RoundedCornerShape(Metrics.rLg)).border(Metrics.hairline, Palette.line, RoundedCornerShape(Metrics.rLg))
        // A sent prompt's own photo, taken in for editing, still lives in the engine's store
        val stored = (s.source as? Attachment.Image)?.let { MediaSource.stored(it.blob, media.session) }
        // A video is a tile like a photo's from the moment it is picked: while it compresses, the original's first frame
        // waits under a spinner, one view throughout (Composer.swift staged strip)
        val file = s.draft as? Draft.File
        val progress = file?.let { VideoJobs.progress[it.uri] }
        val video = file?.takeIf { s.source !is Attachment.Selection }?.let {
          if (progress != null) Uri.parse(it.uri).path?.let { path -> VideoSource.Local(File(path)) }
          else VideoSource.of(context, it.uri, s.name, media.session, media.loader)
        }
        if (video != null) {
          // The player is a dialog of its own over the editor, so the editor stays (iOS leaves only for the file page):
          // collapsing it first took this strip, and the player with it, out of composition
          VideoAttachmentTile(video, s.name, side = Metrics.stagedTile, progress = progress, still = VideoJobs.stills[file.uri], corner = Metrics.rLg) {
            playing = it to s.name
          }
        } else if (thumb != null) {
          val image = s.draft as? Draft.Image
          Image(
            thumb, s.name, contentScale = ContentScale.Crop,
            modifier = tile.clickable {
              // The photo's own pixels, decoded once: the Lightbox shows it alone, as it is not in the session yet
              val source = image?.let { MediaSource.Inline(it.data) }
              val pixels = source?.let { media.loader.peek(it) } ?: return@clickable
              onNavigate?.invoke()
              media.lightbox?.open(LightboxItem(pixels, s.name, source))
            }.testTag("composer.staged.image"),
          )
        } else if (stored != null) {
          val glyph = @Composable {
            Box(Modifier.size(Metrics.stagedTile).background(Palette.chip, RoundedCornerShape(Metrics.rLg)), contentAlignment = Alignment.Center) {
              Lucide("image", Palette.fg3, size = Metrics.icon)
            }
          }
          MediaImage(
            stored,
            content = { pixels ->
              Picture(
                pixels,
                tile.clickable { onNavigate?.invoke(); media.lightbox?.open(LightboxItem(pixels, s.name, stored)) }.testTag("composer.staged.image"),
                contentScale = ContentScale.Crop,
                description = t("common.previewImage", "name" to s.name.ifEmpty { t("common.image") }),
              )
            },
            placeholder = glyph,
            failure = glyph,
          )
        } else {
          val open: (() -> Unit)? = when {
            s.draft is Draft.Text || s.source is Attachment.Text -> ({ reading = s })
            s.source is Attachment.Selection -> (s.source as Attachment.Selection).let { sel -> opener(sel.uri, sel.startLine) }
            file != null -> opener(file.uri, null)
            else -> null
          }
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5),
            modifier = Modifier.height(Metrics.stagedTile).widthIn(max = Metrics.stagedFileW)
              .background(Palette.chip, RoundedCornerShape(Metrics.rLg)).clip(RoundedCornerShape(Metrics.rLg))
              .then(if (open != null) Modifier.clickable(onClick = open) else Modifier)
              .padding(horizontal = Metrics.s3).testTag("composer.staged.file"),
          ) {
            Lucide(if (s.source is Attachment.Selection) "textSelect" else "fileText", Palette.fg2, size = Metrics.icon)
            Text(stagedLabel(s), color = Palette.fg1, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
          }
        }
        Box(
          Modifier.align(Alignment.TopEnd).padding(Metrics.s1).size(Metrics.stagedRemove)
            .background(Palette.scrim, CircleShape).clip(CircleShape).clickable {
              (s.draft as? Draft.File)?.let { VideoJobs.cancel(it.uri) }
              onStaged(staged.filterIndexed { j, _ -> j != i })
            },
          contentAlignment = Alignment.Center,
        ) { Lucide("x", Palette.fgStrong, size = Metrics.chevron, contentDescription = t("common.remove")) }
      }
    }
  }
}

/** A chip's name: a selection taken back from a sent prompt names its lines, as the transcript's pill does */
private fun stagedLabel(s: Staged): String = when (val a = s.source) {
  is Attachment.Selection -> "${a.name} ${if (a.startLine == a.endLine) "L${a.startLine}" else "L${a.startLine}-${a.endLine}"}"
  else -> s.name
}

/** A staged text's whole contents: the draft's own text, or a sent prompt's stored text read back */
@Composable
private fun StagedText(s: Staged, onDismiss: () -> Unit) {
  val draft = s.draft as? Draft.Text
  val source = s.source as? Attachment.Text
  TextAttachmentSheet(s.name, text = draft?.takeIf { source == null }?.text, blob = source?.blob, onDismiss = onDismiss)
}

/**
 * Send / stop in one: a solid circle set into the bar's end, lit (the btn-1 fill) once there is something to send or a
 * run to stop, a quiet chip fill otherwise. The bar-tall slot keeps the bar's layout
 */
@Composable
internal fun SendButton(running: Boolean, filled: Boolean, motion: Boolean, syncing: Boolean = false, action: () -> Unit) {
  val on = (running && !syncing) || filled
  val fill by animateColorAsState(if (on) Palette.btn1Bg else Palette.chip, if (motion) spring() else snap(), label = "send")
  val ink = if (on) Palette.btn1Fg else Palette.sendInk
  val stop = running && !filled
  Box(
    Modifier.size(Metrics.barCtl).clip(CircleShape).clickable(enabled = on, onClick = action).testTag(if (stop) "composer.stop" else "composer.send"),
    contentAlignment = Alignment.Center,
  ) {
    Box(Modifier.size(Metrics.barCtl - Metrics.s1 * 2).background(fill, CircleShape), contentAlignment = Alignment.Center) {
      if (stop) Box(Modifier.size(Metrics.barIcon / 2).background(ink, RoundedCornerShape(Metrics.s0_5)))
      else Lucide("arrowUp", ink, size = Metrics.iconCtl, contentDescription = t("composer.send"))
    }
  }
}

/** What the ring's card offers beyond the figures (ContextUsage.swift): the threshold it marks, and compaction */
class UsageCardActions(
  /** The auto-compact threshold in tokens (settings `compactAtTokens` while auto-compact is on), null for none */
  val compactAt: Int? = null,
  /** The agent advertises `/compact` */
  val canCompact: Boolean = false,
  val onCompact: () -> Unit = {},
)

/**
 * The context window ring beside send (Chat/ContextUsage.swift): how much of the window the conversation fills, a dim
 * sliver once a compaction left the figure stale; a tap opens the card with the share, the figures and the threshold
 */
@Composable
private fun ContextRing(usage: Usage, motion: Boolean, actions: UsageCardActions, running: Boolean) {
  val stale = usage.compactedFrom != null
  val fill by animateFloatAsState(UsageBreakdown.fill(usage), if (motion) spring() else snap(), label = "ring")
  var open by remember { mutableStateOf(false) }
  val ink = Palette.fg2
  val pct = if (usage.size > 0) (min(1f, usage.used.toFloat() / usage.size) * 100).toInt() else 0
  val aria = if (stale) t("usage.compactedStale") else t("usage.usedPct", "pct" to pct)
  Box(
    Modifier.width(Metrics.ctlSm).height(Metrics.barCtl).clip(CircleShape).clickable { open = !open }
      .semantics { contentDescription = aria }.testTag("composer.context"),
    contentAlignment = Alignment.Center,
  ) {
    Canvas(Modifier.size(Metrics.icon).padding(Metrics.icon / RING_PARTS)) {
      val stroke = (Metrics.icon / RING_PARTS).toPx()
      drawCircle(ink.copy(alpha = ink.alpha * RING_TRACK), style = Stroke(stroke))
      drawArc(ink.copy(alpha = if (stale) ink.alpha * STALE_DIM else ink.alpha), -90f, 360f * fill, false, style = Stroke(stroke, cap = StrokeCap.Round))
    }
    if (open) {
      Popup(
        alignment = Alignment.BottomEnd,
        offset = with(LocalDensity.current) { IntOffset(0, -(Metrics.barCtl + Metrics.s2).roundToPx()) },
        onDismissRequest = { open = false },
        properties = PopupProperties(focusable = true),
      ) { UsageCard(usage, actions, running) { open = false } }
    }
  }
}

/**
 * The ring's card (UsagePanel): the title, the share used and the figures, the bar with the threshold's tick and number
 * in its tone, the line saying what that tone means (or that a compaction left the figure stale), and compaction, held
 * while a turn runs
 */
@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
private fun UsageCard(usage: Usage, actions: UsageCardActions, running: Boolean, onClose: () -> Unit) {
  val stale = usage.compactedFrom != null
  val fill = UsageBreakdown.fill(usage)
  val pct = if (usage.size > 0) (min(1f, usage.used.toFloat() / usage.size) * 100).toInt() else 0
  val compactAt = actions.compactAt
  val panel = UsageBreakdown.panel(usage, compactAt, actions.canCompact, running)
  val tabular = TextStyle(fontFeatureSettings = "tnum")
  Column(
    Modifier.width(Metrics.popLg).background(Palette.menuBg, RoundedCornerShape(Metrics.rGooPanel))
      .border(Metrics.hairline, Palette.menuLine, RoundedCornerShape(Metrics.rGooPanel)).padding(Metrics.s3)
      // A popup is a window of its own: its test tags need the resource-id switch again
      .semantics { testTagsAsResourceId = true }
      .testTag("usage.card"),
    verticalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    Text(t("usage.title"), color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.Medium)
    Row(verticalAlignment = Alignment.CenterVertically) {
      Text(if (stale) t("usage.compactedStale") else t("usage.usedPctShort", "pct" to pct), color = Palette.fg2, fontSize = Type.text3, style = tabular, modifier = Modifier.weight(1f))
      Text(
        if (stale) t("usage.beforeCompact", "n" to number(usage.compactedFrom ?: 0)) else "${number(usage.used)} / ${number(usage.size)}",
        color = Palette.fg3, fontSize = Type.text3, style = tabular,
      )
    }
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      BoxWithConstraints(Modifier.weight(1f).height(Metrics.s1_5).clip(CircleShape).background(Palette.active)) {
        Box(Modifier.fillMaxWidth(fill).height(Metrics.s1_5).clip(CircleShape).background(Palette.fg2.copy(alpha = if (stale) STALE_DIM else 1f)))
        if (compactAt != null && usage.size > 0) {
          // The threshold's tick, kept inside the bar's end
          val at = maxWidth * min(compactAt.toFloat() / usage.size, TICK_MAX)
          Box(Modifier.padding(start = at).width(Metrics.s0_5).height(Metrics.s1_5).background(tickInk(panel.tone)).testTag("usage.tick"))
        }
      }
      if (compactAt != null) {
        val budget = t("usage.budget", "n" to number(compactAt))
        Text(
          number(compactAt), color = numberInk(panel.tone), fontSize = Type.text3, style = tabular,
          modifier = Modifier.alpha(if (panel.tone == UsageBreakdown.Tone.Unsupported) UNSUPPORTED_DIM else 1f)
            .semantics { contentDescription = budget }.testTag("usage.budget"),
        )
      }
    }
    val note = if (stale) "usage.compactedStaleHint" else panel.note
    if (note != null) Text(t(note), color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, modifier = Modifier.testTag("usage.note"))
    if (actions.canCompact) {
      CardButton(
        t("usage.compact"), primary = false, enabled = !running,
        modifier = Modifier.padding(top = Metrics.s1).testTag("usage.compact"),
        onClick = { actions.onCompact(); onClose() },
        lead = { Lucide("shrink", if (running) Palette.popFg3 else Palette.fg1, size = Metrics.icon) },
      )
    }
  }
}

/** The threshold's tick: plain while it holds, the accent once it will compact, warn when the window limits it */
@Composable
private fun tickInk(tone: UsageBreakdown.Tone) = when (tone) {
  UsageBreakdown.Tone.Ok -> Palette.fg1
  UsageBreakdown.Tone.Pending -> Palette.brand
  UsageBreakdown.Tone.Limited -> Palette.warn
  UsageBreakdown.Tone.Unsupported -> Palette.fg3
}

/** The threshold's number beside the bar, in the same tone */
@Composable
private fun numberInk(tone: UsageBreakdown.Tone) = when (tone) {
  UsageBreakdown.Tone.Ok, UsageBreakdown.Tone.Unsupported -> Palette.fg3
  UsageBreakdown.Tone.Pending -> Palette.brand
  UsageBreakdown.Tone.Limited -> Palette.warn
}

/** Numbers in the app's language, as the webview's Intl.NumberFormat(locale) */
private fun number(n: Int): String =
  java.text.NumberFormat.getIntegerInstance(java.util.Locale.forLanguageTag(cc.hotic.acpira.i18n.L10n.locale)).format(n)

/**
 * What the screen draws over an open picker (Goo.swift gooLayer): the beans rising out of their chip with their values
 * beside them, and a bean's panel resting on the column's bottom-left corner. Drawn over the whole screen, the composer
 * included (its card recedes meanwhile), so a tall card never hides a bean; the page under it dims (gooRecede). A tap
 * on the open chip goes one level back, anywhere else closes
 */
@Composable
fun GooLayer(goo: Goo, motion: Boolean) {
  val open = goo.open != null
  fun <T> spec() = if (motion) spring<T>(dampingRatio = GOO_DAMPING, stiffness = GOO_STIFFNESS) else snap()
  // Held beans go home into the chip while they trade for another level's
  val main by animateFloatAsState(if (open && !goo.held) 1f else 0f, spec(), label = "goo")
  if (main <= GONE && !open) return
  val chip = goo.chips[goo.open ?: goo.last] ?: return
  var origin by remember { mutableStateOf(Offset.Zero) }
  var height by remember { mutableStateOf(0f) }
  val beans = goo.beans
  val density = LocalDensity.current
  Box(
    Modifier.fillMaxSize()
      .onGloballyPositioned { origin = it.boundsInRoot().topLeft; height = it.size.height.toFloat() }
      // Over the composer too, so it takes every tap while open: on the open chip one level back (the chip's × or ‹),
      // anywhere else everything closes
      .then(
        if (open) Modifier.pointerInput(goo) {
          detectTapGestures { at ->
            val opener = goo.open?.let { goo.chips[it] }
            if (opener != null && opener.contains(at + origin)) goo.back() else goo.close()
          }
        } else Modifier,
      ),
  ) {
    val left = chip.left - origin.x
    val bottom = chip.top - origin.y - with(density) { Metrics.gooGap.toPx() }
    val step = with(density) { (Metrics.barCtl + Metrics.gooGap).toPx() }
    val room = bottom - with(density) { (Metrics.barCtl * 2).toPx() }
    // A short window (a phone on its side) has no room for a tall column: the beans that would rise past the safe top
    // start another column beside the first, clear of its captions
    val ceiling = max(0f, WindowInsets.safeDrawing.getTop(density) - origin.y) + with(density) { Metrics.gooGap.toPx() }
    val perColumn = max(1, ((bottom - ceiling - with(density) { Metrics.barCtl.toPx() }) / step).toInt() + 1)
    val columnW = with(density) { (Metrics.barCtl + Metrics.s2 + Metrics.gooAside + Metrics.s1 * 2 + Metrics.gooGap).toPx() }
    val n = beans.size
    val span = min(STAGGER * max(0, n - 1), STAGGER_MAX)
    val panels = beans.mapIndexed { i, _ -> animateFloatAsState(if (goo.open != null && goo.stage == i + 1) 1f else 0f, spec(), label = "panel$i") }
    val opened = panels.maxOfOrNull { it.value } ?: 0f
    beans.forEachIndexed { i, bean ->
      val lag = if (n > 1) span * i / (n - 1) else 0f
      val t = ((main.coerceAtMost(1f) - lag) / (1 - span)).coerceIn(0f, 1f) + max(0f, main - 1)
      val rest = bottom - step * (i % perColumn)
      val x = left + columnW * (i / perColumn) * t
      val y = chip.center.y - origin.y + (rest - with(density) { Metrics.barCtl.toPx() } / 2 - (chip.center.y - origin.y)) * t
      val beanLive = t > ARRIVED && opened < UNCOVERED
      // Every bean goes into an open panel, its own too: a see-through panel would show it under the rows
      val swallowed = opened.coerceIn(0f, 1f)
      // The bean
      Row(
        Modifier.layout { m, c ->
          val p = m.measure(c.copy(minWidth = 0, minHeight = 0))
          layout(c.maxWidth, c.maxHeight) { p.place(x.toInt(), (y - p.height / 2).toInt()) }
        }.graphicsLayer {
          val s = BEAN_SEED + (1 - BEAN_SEED) * t
          scaleX = s * (1 - swallowed); scaleY = s * (1 - swallowed)
          alpha = t.coerceIn(0f, 1f) * (1 - swallowed)
          transformOrigin = TransformOrigin(0.5f / (1 + 0f), 0.5f)
        },
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Box(
          Modifier.size(Metrics.barCtl).background(Palette.menuBg, CircleShape).border(Metrics.hairline, Palette.menuLine, CircleShape)
            .clip(CircleShape).clickable(enabled = beanLive) { if (bean.tap != null) bean.tap.invoke() else goo.stage = if (goo.stage == i + 1) 0 else i + 1 }
            .testTag("goo.${bean.key}"),
          contentAlignment = Alignment.Center,
        ) { bean.glyph() }
        // The value beside it, last out and first back
        val caption = ((t - CAPTION_FROM) / (1 - CAPTION_FROM)).coerceIn(0f, 1f) * (1 - (opened / CAPTION_OUT).coerceIn(0f, 1f))
        // A soft glow of the page's colour round the letters, so the value floats over whatever lies behind it with no
        // patch of its own (iOS blurs a page-coloured plate until its edges are gone)
        val glow = TextStyle(shadow = Shadow(Palette.bg0, blurRadius = with(density) { Metrics.s2.toPx() }))
        Column(
          Modifier.padding(start = Metrics.s2).widthIn(max = Metrics.gooAside).alpha(caption)
            .padding(horizontal = Metrics.s1)
            .clickable(enabled = beanLive, interactionSource = remember { MutableInteractionSource() }, indication = null) {
              if (bean.tap != null) bean.tap.invoke() else goo.stage = if (goo.stage == i + 1) 0 else i + 1
            },
        ) {
          Text(bean.caption, color = Palette.fgStrong, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis, style = glow)
          // What the setting is, under its value (the sidekick's)
          bean.note?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 2, overflow = TextOverflow.Ellipsis, style = glow) }
        }
      }
      // Its panel, grown out of the column's bottom-left corner
      val panel = bean.panel
      val s = panels[i].value
      if (panel != null && s > GONE) {
        val h = panelHeight(bean.rows, room, density)
        Box(
          Modifier.layout { m, c ->
            val p = m.measure(androidx.compose.ui.unit.Constraints.fixed(Metrics.gooPanelW.roundToPx(), h.toInt()))
            layout(c.maxWidth, c.maxHeight) { p.place(left.toInt(), (bottom - h).toInt()) }
          }.graphicsLayer {
            transformOrigin = TransformOrigin(0f, 1f)
            val k = PANEL_SEED + (1 - PANEL_SEED) * s
            scaleX = k; scaleY = k
            alpha = s.coerceIn(0f, 1f)
          }.background(Palette.menuBg, PANEL_SHAPE).border(Metrics.hairline, Palette.menuLine, PANEL_SHAPE)
            .clip(PANEL_SHAPE)
            .then(if (s > ARRIVED) Modifier.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {} else Modifier)
            .padding(vertical = Metrics.s1_5),
        ) { panel() }
      }
    }
  }
}

/**
 * The page under an open picker blurs as well as dims, as Messages does under `+`, so a caption never reads against the
 * words behind it (a dim alone left the transcript legible between the beans)
 */
fun Modifier.gooRecede(goo: Goo, motion: Boolean): Modifier = composed {
  val open = goo.open != null
  val radius by animateDpAsState(if (open) Metrics.gooBlur else Metrics.s0, if (motion) spring() else snap(), label = "recede")
  val dim by animateFloatAsState(if (open) 1f else 0f, if (motion) spring(dampingRatio = GOO_DAMPING, stiffness = GOO_STIFFNESS) else snap(), label = "dim")
  // The dim is drawn here, on the page under the picker's layer
  val tone = Palette.gooDim
  val dimmed = drawWithContent {
    drawContent()
    if (dim > 0f) drawRect(tone.copy(alpha = tone.alpha * dim.coerceIn(0f, 1f)))
  }
  (if (radius > Metrics.s0) blur(radius) else this).then(dimmed)
}

/**
 * A surface floating over the receded page (the docked to-do card) steps back the way the composer's own face does:
 * faint and soft, without a second dim, which would print its box as a darker band
 */
fun Modifier.gooAside(goo: Goo, motion: Boolean): Modifier = composed {
  val open = goo.open != null
  val radius by animateDpAsState(if (open) Metrics.s1 else Metrics.s0, if (motion) spring() else snap(), label = "aside")
  val alpha by animateFloatAsState(if (open) GOO_RECEDE else 1f, if (motion) spring() else snap(), label = "asideAlpha")
  (if (radius > Metrics.s0) blur(radius) else this).alpha(alpha)
}

/** A bean's panel: its rows up to [Metrics.gooPanelRows], no taller than the room above the chip */
private fun panelHeight(rows: Int, room: Float, density: androidx.compose.ui.unit.Density): Float = with(density) {
  val pad = (Metrics.s1_5 * 2).toPx()
  val row = Metrics.gooRow.toPx()
  val natural = min(rows.toFloat(), Metrics.gooPanelRows) * row + pad
  max(row + pad, min(natural, room))
}

/** A bean's panel: its rows in a scroll that opens on the current one */
@Composable
internal fun GooList(current: Int, content: @Composable () -> Unit) {
  val scroll = rememberScrollState()
  val density = LocalDensity.current
  LaunchedEffect(Unit) {
    if (current > 0) scroll.scrollTo(with(density) { (Metrics.gooRow * current - Metrics.gooRow * 2).roundToPx() }.coerceAtLeast(0))
  }
  Column(Modifier.fillMaxWidth().verticalScroll(scroll)) { content() }
}

/** One choice in a bean's panel: glyph, name, and the check (or a trailing control such as Thinking's switch) */
@Composable
internal fun GooRow(
  name: String,
  checked: Boolean,
  glyph: (@Composable () -> Unit)? = null,
  trailing: (@Composable () -> Unit)? = null,
  /** A choice listed but not offered: dimmed, and a tap does nothing */
  enabled: Boolean = true,
  action: () -> Unit,
) {
  val haptics = LocalHapticFeedback.current
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    modifier = Modifier.fillMaxWidth().height(Metrics.gooRow).alpha(if (enabled) 1f else GooMotion.unavailable)
      // Every row is a choice (a Thinking row's own switch plays the platform's feedback instead)
      .clickable(enabled = enabled) { haptics.play(Haptic.Pick); action() }.padding(horizontal = Metrics.gooPadX),
  ) {
    glyph?.invoke()
    Text(
      name, color = if (checked) Palette.fgStrong else Palette.fg2, fontSize = Type.text2,
      fontWeight = if (checked) FontWeight.SemiBold else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis,
      modifier = Modifier.weight(1f),
    )
    if (trailing != null) trailing()
    else Lucide("check", Palette.fgStrong, Modifier.alpha(if (checked) 1f else 0f), size = Metrics.icon)
  }
}

/** modeIcons.ts `modeIcon`: a mode's glyph from its kind, id and name */
fun modeIcon(mode: SessionOption): String {
  if (mode.kind == "full_access") return "shieldOff"
  if (mode.kind == "auto_review") return "sparkles"
  if (mode.id == "auto_edit" || mode.id == "acceptEdits") return "code"
  val key = "${mode.id} ${mode.name}".lowercase()
  val rules = listOf(
    "bypass|yolo|auto[\\s-]?accept|full[\\s-]?access|unsafe" to "shieldOff",
    "debug" to "bug",
    "plan" to "listTodo",
    "\\bask\\b|chat|question" to "messageCircleQuestionMark",
    "smart|\\bauto\\b" to "sparkles",
    "code|edit|build" to "code",
    "\\bteam\\b|multitask|orchestrat" to "users",
    "agent|default|normal" to "infinity",
  )
  return rules.firstOrNull { Regex(it.first).containsMatchIn(key) }?.second ?: "circleDashed"
}

/**
 * modeIcons.ts `modeLabel`: the short name for a mode the built-in integrations are known to send, matched on the exact
 * id, in the app's language; anything unknown shows the agent's own name
 */
fun modeLabel(mode: SessionOption): String {
  if (mode.kind == "full_access") return t("composer.modeShort.full")
  if (mode.kind == "auto_review") return t("composer.modeShort.auto")
  val short = mapOf(
    "default" to "default", "agent" to "agent", "build" to "build", "plan" to "plan", "raw" to "raw", "team" to "team",
    "acceptEdits" to "autoEdit", "auto_edit" to "autoEdit", "auto" to "auto", "smart" to "smart", "bypassPermissions" to "full",
    "bypass" to "full", "yolo" to "full", "agent-full-access" to "full", "read-only" to "readOnly",
    "workspace-write" to "workspace", "ask" to "ask", "code" to "code", "accept-edits" to "code",
  )
  return short[mode.id]?.let { t("composer.modeShort.$it") } ?: mode.name
}

/** Codex's collaboration mode: its `default` is the Build mode, not the permission default beside it */
private fun workLabel(mode: SessionOption): String = if (mode.id == "default") t("composer.modeShort.build") else modeLabel(mode)

/** The built-in agent's approval levels by id, any other level its short mode label */
private fun accessLabel(level: SessionOption): String {
  val keys = mapOf("ask" to "composer.access.ask", "auto-edit" to "composer.access.autoEdit", "full" to "composer.access.full")
  return keys[level.id]?.let { t(it) } ?: modeLabel(level)
}

private val PANEL_SHAPE = RoundedCornerShape(Metrics.rGooPanel)
/** The field's line box (TypeScale.field), as a Dp for the bar's arithmetic */
private val FIELD_LINE = 24.dp
private const val MAX_LINES = 8
/** The lines from which the card offers the whole-screen editor (ChatGPT's expand corner) */
private const val EXPAND_LINES = 3
/** `+` turns into × by this many degrees */
private const val PLUS_TURN = 45f
/** The card's text under an open picker */
private const val GOO_RECEDE = 0.35f
private const val COMPOSER_DAMPING = 0.86f
private const val COMPOSER_STIFFNESS = 520f
private const val GOO_DAMPING = 0.78f
private const val GOO_STIFFNESS = 380f
/** Under this the picker is closed and nothing is drawn */
private const val GONE = 0.001f
/** The beans' lags: each starts this share after the one before, spread over at most STAGGER_MAX */
private const val STAGGER = 0.06f
private const val STAGGER_MAX = 0.2f
/** A bean leaves the chip at this share of its size; a panel grows from this share of its own */
private const val BEAN_SEED = 0.35f
private const val PANEL_SEED = 0.2f
/** Taps land once a bean has arrived, and on the beans only once no panel is left over them */
private const val ARRIVED = 0.9f
private const val UNCOVERED = 0.05f
/** Captions come last on the way out and go as a panel starts */
private const val CAPTION_FROM = 0.7f
private const val CAPTION_OUT = 0.1f
/** The ring: its stroke and inset are an eighth of the glyph box, the track a quarter of the ink */
private const val RING_PARTS = 8
private const val RING_TRACK = 0.25f
/** A compacted window's sliver and bar are dimmed to this */
private const val STALE_DIM = 0.5f
/** The threshold's tick stays inside the bar's end; its number dims while the agent cannot compact */
private const val TICK_MAX = 0.99f
private const val UNSUPPORTED_DIM = 0.6f
private const val MAX_PHOTOS = 4
/** AttachmentImage.thumbnail: the longest side and the JPEG quality */
private const val IMAGE_SIDE = 1024
private const val JPEG_QUALITY = 80
/** AttachmentFile.messageBytes: one message on the paired link holds at most this much base64 */
private const val MESSAGE_BYTES = 60 * (1 shl 20)
