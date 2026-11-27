package cc.hotic.acpira.ui

import android.content.Context
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import cc.hotic.acpira.design.GooMotion
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ComposerControls
import cc.hotic.acpira.logic.ModelChoices
import cc.hotic.acpira.logic.ModelVisibility
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.Usage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

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
  /** The CLI whose models the sidekick's model bean lists, once one was picked from its agent bean; null: the picked sidekick's */
  var browse by mutableStateOf<String?>(null)
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
   * level changes while they are small there, and the new beans squeeze out, `opening` that bean's panel at once (1…).
   * Without motion it swaps at once
   */
  fun enter(level: String?, opening: Int = 0) {
    stage = 0
    browse = null
    val scope = scope
    if (!motion || scope == null) { deeper = level; stage = opening; return }
    held = true
    swap?.cancel()
    swap = scope.launch {
      delay(GooMotion.levelSwapMs)
      // Closed meanwhile (a tap outside): the level stays as it was, and the next open starts on the chip's own beans
      if (open != null) { deeper = level; stage = opening }
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
  /** Group labels among the rows ([GooSection]), shorter than a row */
  val labels: Int = 0,
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
  // Beside a permission bean the working modes lead the `/` list, and the one in force is the bar's pill
  val working = remember(controls) { ComposerControls.workingModes(controls).orEmpty() }
  val pill = remember(controls) { ComposerControls.workingMode(controls) }
  val tokens = rememberTokens(prompt, assist, staged, onStaged, working, onSetMode) { focus.requestFocus(); keyboard?.show() }
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
  // The pill rides in the chips' group, so the field at rest starts after it
  var pillWidth by remember { mutableStateOf(Metrics.s0) }
  val lead = (if (modelBeans.isEmpty()) Metrics.barCtl else Metrics.barCtl * 2) + (if (pill != null) pillWidth else Metrics.s0)
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
          val rest = controls.modes.firstOrNull()
          if (pill != null && rest != null) {
            val density = LocalDensity.current
            ModePill(pill, Modifier.padding(horizontal = Metrics.s1).onSizeChanged { pillWidth = with(density) { it.width.toDp() } + Metrics.s1 * 2 }) {
              onSetMode(rest.id)
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
    // Cross-CLI host sidekicks are already filtered per CLI by the engine (`host_sidekick.rs`); only native single-agent
    // sidekicks share the lead model's hidden list here
    val split = if (hidden.isEmpty()) null else ComposerControls.split(controls.options, brand)
    val leadModelId = split?.models?.firstOrNull()?.id
    val modelIds = split?.models?.map { it.id }?.toSet().orEmpty()
    val sidekickIds = split?.sidekick?.filter { ComposerControls.sidekickClis(it.options).isEmpty() }?.map { it.id }?.toSet().orEmpty()
    val shown = if (modelIds.isEmpty() && sidekickIds.isEmpty()) controls else controls.copy(options = controls.options.map { c ->
      when (c.id) {
        in modelIds -> c.copy(options = ModelVisibility.visibleOptions(c.options, hidden[c.id], c.value))
        in sidekickIds -> c.copy(options = ModelVisibility.visibleOptions(c.options, leadModelId?.let { hidden[it] }, c.value))
        else -> c
      }
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
  if (goo.deeper == SIDEKICK_LEVEL && (c.fusion != null || c.sidekick != null)) return sidekickBeans(c, onSetConfig, fold, haptics, goo)
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
      glyph = { Lucide(mode?.let(::modeIcon) ?: "circleDashed", Palette.fg1, size = Metrics.barIcon) },
      rows = c.modes.size,
      panel = {
        GooList(c.modes.indexOfFirst { it.id == mode?.id }) {
          for (m in c.modes) {
            GooRow(modeLabel(m), m.id == mode?.id, glyph = { Lucide(modeIcon(m), Palette.fgStrong, size = Metrics.icon) }) {
              onSetMode(m.id); fold()
            }
          }
        }
      },
    )
  }
  c.access?.let { access ->
    val level = access.options.firstOrNull { it.id == access.value } ?: access.options.firstOrNull()
    out += Bean(
      "access", t("composer.access"), level?.let(::accessLabel) ?: access.name,
      glyph = { Lucide(level?.let(::accessIcon) ?: "circleDashed", Palette.fg1, size = Metrics.barIcon) },
      rows = access.options.size,
      panel = {
        GooList(access.options.indexOfFirst { it.id == level?.id }) {
          for (o in access.options) {
            GooRow(accessLabel(o), o.id == level?.id, glyph = { Lucide(accessIcon(o), Palette.fgStrong, size = Metrics.icon) }) {
              onSetConfig(access.id, o.id); fold()
            }
          }
        }
      },
    )
  }
  c.agent?.let { agent ->
    val current = agent.options.firstOrNull { it.id == agent.value } ?: agent.options.firstOrNull()
    out += Bean(
      "agent", agent.name, current?.name ?: agent.name,
      glyph = { Lucide(current?.let(::modeIcon) ?: "circleDashed", Palette.fg1, size = Metrics.barIcon) },
      rows = agent.options.size,
      panel = {
        GooList(agent.options.indexOfFirst { it.id == current?.id }) {
          for (o in agent.options) {
            GooRow(o.name, o.id == current?.id, glyph = { Lucide(modeIcon(o), Palette.fgStrong, size = Metrics.icon) }) {
              onSetConfig(agent.id, o.id); fold()
            }
          }
        }
      },
    )
  }
  sidekickBean(c) { opening -> goo.enter(SIDEKICK_LEVEL, opening) }?.let { out += it }
  return out
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

/** The field's line box (TypeScale.field), as a Dp for the bar's arithmetic */
private val FIELD_LINE = 24.dp
private const val MAX_LINES = 8
/** The lines from which the card offers the whole-screen editor (ChatGPT's expand corner) */
private const val EXPAND_LINES = 3
/** `+` turns into × by this many degrees */
private const val PLUS_TURN = 45f
/** The card's text under an open picker */
internal const val GOO_RECEDE = 0.35f
private const val COMPOSER_DAMPING = 0.86f
private const val COMPOSER_STIFFNESS = 520f
