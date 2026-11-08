package cc.hotic.acpira.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextRange
import androidx.compose.foundation.text.input.TextFieldState
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.MentionPersona
import cc.hotic.acpira.logic.PromptEdit
import cc.hotic.acpira.logic.SigilSpan
import cc.hotic.acpira.logic.SlashCommands
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.FileHit
import cc.hotic.acpira.model.SlashCommand
import kotlinx.coroutines.delay

// The composer's `/` and `@` half (apps/ios Chat/Composer.swift "Commands and mentions"): the tokens under the caret,
// the hits they raise, and what a pick writes. A command completes in place and goes out with the ordinary send (the
// agent runs it); a persona is summoned by name; a file leaves the text and rides along as an attachment.

/** The tokens under the caret and the picks on them, rebuilt with the field's value */
class ComposerTokens(
  private val state: TextFieldState,
  private val assist: ComposerAssist,
  private val staged: List<Staged>,
  private val onStaged: (List<Staged>) -> Unit,
  private val focusField: () -> Unit,
) {
  private val text = state.text.toString()
  /** The caret's offset, while nothing is selected */
  private val caret: Int? = state.selection.takeIf { it.collapsed }?.end?.coerceIn(0, text.length)

  val slashSpan: SigilSpan? = caret?.let { SlashCommands.commandAt(text, it) }

  val mentionSpan: SigilSpan? =
    if (assist.files == null && assist.personas.isEmpty()) null else caret?.let { SlashCommands.mentionAt(text, it) }

  /** The commands matching the `/` token, the list shown only with hits: without one the slash stays plain text */
  val slashHits: List<SlashCommand> = slashSpan?.let { SlashCommands.match(assist.commands, it.query) }.orEmpty()

  /** The input hint of the command the text names, while the list is not showing it */
  val commandHint: String? = if (slashHits.isEmpty()) SlashCommands.hint(assist.commands, text) else null

  /** A command completes the token in place (`/name `) */
  fun pickCommand(command: SlashCommand) {
    val span = slashSpan ?: return
    apply(SlashCommands.complete(text, span, caret ?: text.length, command.name))
  }

  /** A persona is summoned by name; a file leaves the text and rides along as an attachment */
  fun pickMention(hit: FileHit) {
    val span = mentionSpan ?: return
    val at = caret ?: text.length
    val persona = MentionPersona.of(hit, assist.personas)
    if (persona != null) return apply(SlashCommands.summon(text, persona.name, span.start, at - span.start))
    apply(SlashCommands.dropToken(text, span, at))
    if (staged.none { (it.draft as? Draft.File)?.uri == hit.uri }) onStaged(staged + Staged(Draft.File(hit.uri, hit.path), hit.path))
  }

  /** SummonMenu: `@name ` at the caret */
  fun summon(persona: MentionPersona) {
    apply(SlashCommands.summon(text, persona.name, caret ?: text.length))
    focusField()
  }

  /** The edited prompt, the caret after what was written */
  private fun apply(edit: PromptEdit) {
    state.edit {
      replace(0, length, edit.text)
      selection = TextRange(edit.caret.coerceIn(0, edit.text.length))
    }
  }
}

@Composable
fun rememberTokens(
  state: TextFieldState,
  assist: ComposerAssist,
  staged: List<Staged>,
  onStaged: (List<Staged>) -> Unit,
  focusField: () -> Unit,
): ComposerTokens {
  val focus by rememberUpdatedState(focusField)
  // Read here, so the tokens follow every keystroke and caret move
  val text = state.text
  val selection = state.selection
  val tokens = remember(text, selection, assist, staged, onStaged) { ComposerTokens(state, assist, staged, onStaged) { focus() } }
  // Mention.tsx useMentionHits: the query goes out a moment after the last keystroke, and the list closing drops
  // whatever is still on its way
  val query = tokens.mentionSpan?.query
  LaunchedEffect(query, assist.files) {
    val files = assist.files ?: return@LaunchedEffect
    if (query == null) return@LaunchedEffect files.close()
    delay(MENTION_DEBOUNCE_MS)
    files.search(query)
  }
  return tokens
}

/** The `/` list, else the `@` list (the personas, then the files the engine found for the query) */
@Composable
fun ColumnScope.Suggestions(tokens: ComposerTokens, assist: ComposerAssist, motion: Boolean) {
  val commands = tokens.slashHits
  val span = tokens.mentionSpan
  val mentions = span?.let { MentionPersona.hits(assist.personas, it.query) + assist.files?.hits.orEmpty() }.orEmpty()
  val answered = span != null && assist.files?.answered == span.query
  val showMentions = span != null && (mentions.isNotEmpty() || (answered && span.query.isNotEmpty()))
  // The last list shown stays while it leaves, so it does not empty out under its fade
  // (a plain holder, not state: writing it here must not ask for another composition)
  val last = remember { arrayOfNulls<@Composable () -> Unit>(1) }
  val now: (@Composable () -> Unit)? = when {
    commands.isNotEmpty() -> { { SuggestionList(commands, key = { it.name }, pick = tokens::pickCommand) { CommandRow(it) } } }
    showMentions -> {
      {
        SuggestionList(mentions, key = { it.uri }, pick = tokens::pickMention, empty = t("mention.noFiles")) { hit ->
          MentionRow(hit, MentionPersona.of(hit, assist.personas))
        }
      }
    }
    else -> null
  }
  if (now != null) last[0] = now
  SuggestionReveal(now != null, motion) { (now ?: last[0])?.invoke() }
}

/** The expand corner's button: the maximize glyph in the column the card keeps free for it */
@Composable
fun ExpandCorner(modifier: Modifier, onExpand: () -> Unit) {
  Box(
    modifier.width(Metrics.s3 * 2 + Metrics.icon).height(with(androidx.compose.ui.platform.LocalDensity.current) { Type.fieldLine.toDp() })
      .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onExpand)
      .testTag("composer.expand"),
    contentAlignment = Alignment.Center,
  ) { Lucide("maximize2", Palette.fg2, size = Metrics.icon, contentDescription = t("mobile.composer.expand")) }
}

/** Mention.tsx's wait before a query goes out */
private const val MENTION_DEBOUNCE_MS = 60L
