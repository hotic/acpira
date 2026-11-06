package cc.hotic.acpira.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.ProbeOutcome
import cc.hotic.acpira.model.Provider
import cc.hotic.acpira.model.ProviderAction
import cc.hotic.acpira.model.ProviderModel
import cc.hotic.acpira.model.ProviderProbe
import cc.hotic.acpira.model.endpointOf
import cc.hotic.acpira.model.looksFull
import cc.hotic.acpira.model.sameUrl
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import kotlinx.serialization.json.JsonObject

// The built-in agent's model sources (packages/ui settings/ProvidersSection.tsx, apps/ios Settings/ProvidersSettings.swift):
// the sources page with its model switches, a source's form and its add-model page. The engine on the computer keeps
// providers.json and the keys; the phone sees `hasKey` only, and every write goes through ModelSources' ordered queue.

/** The built-in agent page's row into its model sources; an engine without them leaves it greyed out with why */
@Composable
internal fun ProvidersRow(model: AppModel) {
  val supported = model.live && "providers" in model.engineFeatures
  LaunchedEffect(supported) { if (supported) model.sources.refresh() }
  FormSection(null, footer = if (model.live && !supported) t("mobile.outdated") else null) {
    FormRow(
      t("settings.providers"),
      Modifier.clickable(enabled = supported) { model.push(Screen.Providers) }.alpha(if (supported) 1f else OFF_ALPHA).testTag("agent.providers"),
    ) {
      model.sources.view?.let { Text("${it.providers.size}", color = Palette.fg2, fontSize = Type.text1) }
      Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
    }
  }
}

/**
 * Every source: Ollama / LM Studio servers found on the computer and not yet a source (added with one tap), then one
 * group per source (its state and address, its models with a switch each, Add model, Edit), then Add source
 */
@Composable
fun ProvidersScreen(model: AppModel) {
  val sources = model.sources
  var localProbe by remember { mutableStateOf<String?>(null) }
  LaunchedEffect(Unit) {
    sources.refresh()
    localProbe = sources.probe(ProviderProbe.Local)
  }
  // Add model asks the endpoint first (source id to probe id): its page opens only with an answer, a refused key or a
  // dead address opens the source's form with the error instead, so that page never waits and never fails
  var opening by remember { mutableStateOf<Pair<String, String>?>(null) }
  val answer = opening?.let { sources.probes[it.second] }
  LaunchedEffect(answer) {
    val id = opening?.first ?: return@LaunchedEffect
    when (answer) {
      null -> return@LaunchedEffect
      is ProbeOutcome.Models -> {
        sources.listed[id] = answer.models
        model.push(Screen.AddModels(id))
      }
      is ProbeOutcome.Failed -> model.push(Screen.ProviderForm(id, answer.error))
      else -> {}
    }
    opening = null
  }
  // A local server's add waits on its own write
  var addingLocal by remember { mutableStateOf<ProviderAction?>(null) }
  LaunchedEffect(sources.settled) { if (sources.settled?.action === addingLocal) addingLocal = null }

  val view = sources.view
  val providers = sources.providers
  val local = (localProbe?.let { sources.probes[it] } as? ProbeOutcome.Local)?.servers.orEmpty()
    .filter { s -> providers.none { sameUrl(it.baseUrl, s.baseUrl) } }
  FormPage(model, t("settings.providers")) {
    (sources.error ?: view?.error)?.let { ErrorLine(it, Modifier.padding(horizontal = Metrics.pad).testTag("providers.error")) }
    if (view == null) FormCard { Note(t("settings.loading")) }
    if (local.isNotEmpty()) {
      FormCard {
        local.forEachIndexed { i, s ->
          if (i > 0) RowRule()
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
            modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.formRowHeight).padding(start = Metrics.pad, end = Metrics.s1),
          ) {
            Column(Modifier.weight(1f).padding(vertical = Metrics.s2)) {
              Text(t("settings.providers.local", "name" to s.name), color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
              Text(s.baseUrl, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            TextButton(
              onClick = {
                val action = ProviderAction.Save(Provider.fromLocal(s))
                addingLocal = action
                sources.push(action)
              },
              enabled = addingLocal == null,
              modifier = Modifier.testTag("providers.local:${s.preset}"),
            ) { Text(t("settings.providers.local.add"), color = if (addingLocal == null) Palette.fg1 else Palette.fg3, fontSize = Type.text1) }
          }
        }
      }
    }
    if (view != null && providers.isEmpty()) FormCard { Note(t("settings.providers.none")) }
    for (p in providers) {
      key(p.id) {
        SourceGroup(
          p,
          opening = opening?.first == p.id,
          onToggle = { m, on -> sources.push(ProviderAction.Save(p.withModels(p.models.map { if (it.id == m.id) it.enabled(on) else it })), optimistic = true) },
          onRemove = { m -> sources.push(ProviderAction.Save(p.withModels(p.models.filter { it.id != m.id })), optimistic = true) },
          onAddModel = { if (opening == null) opening = p.id to sources.probe(ProviderProbe.Models(p)) },
          onEdit = { model.push(Screen.ProviderForm(p.id)) },
        )
      }
    }
    FormSection(null, footer = t("mobile.providers.desc")) {
      ActionRow(t("settings.providers.add"), "plus", tag = "providers.add") { model.push(Screen.ProviderForm(null)) }
    }
  }
}

/** One source: its state and address under its name, its models (a switch each, a long press to remove), then its actions */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun SourceGroup(
  p: Provider,
  opening: Boolean,
  onToggle: (ProviderModel, Boolean) -> Unit,
  onRemove: (ProviderModel) -> Unit,
  onAddModel: () -> Unit,
  onEdit: () -> Unit,
) {
  FormSection(p.title) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s2).testTag("providers.source:${p.id}")) {
      // Configuration, not connection health: a missing key is neutral, only an endpoint's answer says it needs one
      val (status, ok) = when {
        !p.enabled -> t("settings.providers.off") to false
        p.local -> t("settings.providers.keyNotNeeded") to true
        p.hasKey -> t("settings.providers.keySaved") to true
        else -> t("settings.providers.noKey") to false
      }
      Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
        Box(Modifier.size(Metrics.unreadDot).background(if (ok) Palette.ok else Palette.fg3, CircleShape))
        Text(status, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line)
      }
      Text(
        p.baseUrl, color = Palette.fg3, fontSize = Type.text4, lineHeight = Type.text4Line, fontFamily = FontFamily.Monospace,
        maxLines = 1, overflow = TextOverflow.Ellipsis,
      )
    }
    for (m in p.models) {
      key(m.id) {
        var menu by remember { mutableStateOf(false) }
        RowRule()
        Box {
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
            modifier = Modifier.fillMaxWidth()
              .combinedClickable(onClick = { onToggle(m, !m.enabled) }, onLongClick = { menu = true })
              .heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad, vertical = Metrics.s1)
              .testTag("providers.model:${p.id}/${m.id}"),
          ) {
            Column(Modifier.weight(1f)) {
              Text(m.name ?: m.id, color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
              if (m.name != null) Text(m.id, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Switch(checked = m.enabled, onCheckedChange = { onToggle(m, it) }, colors = SwitchDefaults.colors())
          }
          AppMenu(expanded = menu, onDismiss = { menu = false }) {
            AppMenuItem(
              t("common.remove"), onClick = { menu = false; onRemove(m) }, danger = true,
              icon = { MenuGlyph("trash2", danger = true) }, modifier = Modifier.testTag("providers.model.remove"),
            )
          }
        }
      }
    }
    RowRule()
    ActionRow(t("settings.providers.addModel"), "plus", tag = "providers.addModel:${p.id}", busy = opening, onClick = onAddModel)
    RowRule()
    ActionRow(t("settings.providers.editShort"), "pencil", tag = "providers.edit:${p.id}", onClick = onEdit)
  }
}

/** A row that does one thing: its glyph (a spinner while it waits on the computer) and its label */
@Composable
private fun ActionRow(label: String, icon: String, tag: String, busy: Boolean = false, danger: Boolean = false, onClick: () -> Unit) {
  val ink = if (danger) Palette.danger else Palette.fg1
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier.fillMaxWidth().clickable(enabled = !busy, onClick = onClick).heightIn(min = Metrics.formRowHeight)
      .padding(horizontal = Metrics.pad).testTag(tag),
  ) {
    Box(Modifier.size(Metrics.rowMark), contentAlignment = Alignment.Center) {
      if (busy) CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg2, strokeWidth = Metrics.pairCodeLine)
      else Lucide(icon, ink, size = Metrics.rowMark)
    }
    Text(label, color = ink, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
  }
}

/** Why something failed, in one red line */
@Composable
private fun ErrorLine(text: String, modifier: Modifier = Modifier) {
  Text(text, color = Palette.danger, fontSize = Type.text3, lineHeight = Type.text3Line, modifier = modifier)
}

/**
 * A source's form, a new one when [id] is null: name, API format, base URL with the request URL it makes, the key (none
 * for a local server). Saving first asks the endpoint for its model list whenever something that reaches it changed:
 * only an answer saves, so a refused key or a wrong address never gets in. A new source then opens its add-model page
 */
@Composable
fun ProviderFormScreen(model: AppModel, id: String?, opened: String? = null) {
  val sources = model.sources
  // The source as last seen: once removed it leaves the list, yet the form stays up until its write settles and pops it
  val seen = remember(id) { arrayOfNulls<Provider>(1) }
  val saved = id?.let { i -> sources.providers.firstOrNull { it.id == i } }?.also { seen[0] = it } ?: seen[0]
  if (id != null && saved == null) {
    FormPage(model, t("settings.providers")) { FormCard { Note(t("settings.loading")) } }
    return
  }
  var name by remember { mutableStateOf(saved?.name.orEmpty()) }
  var format by remember { mutableStateOf(saved?.format ?: "openai-chat") }
  var baseUrl by remember { mutableStateOf(saved?.baseUrl.orEmpty()) }
  // Whether the URL is used as typed: a saved source keeps its choice, a new one follows the URL's shape until the chip
  // under the field overrides it
  var fullOverride by remember { mutableStateOf(saved?.fullUrl) }
  val fullUrl = fullOverride ?: looksFull(baseUrl)
  var key by remember { mutableStateOf("") }
  var error by remember { mutableStateOf(opened) }
  // The models probe checking the draft, and the form's own write waiting for its answer
  var verifying by remember { mutableStateOf<String?>(null) }
  var waiting by remember { mutableStateOf<ProviderAction?>(null) }
  var before by remember { mutableStateOf<Set<String>?>(null) }
  var listedOnSave by remember { mutableStateOf<List<ProviderModel>>(emptyList()) }
  var removing by remember { mutableStateOf(false) }
  val local = saved?.local == true
  val busy = verifying != null || waiting != null
  val valid = Regex("""^https?://\S+$""").matches(baseUrl.trim())
  val typedKey = key.trim().ifEmpty { null }

  fun draft(): Provider {
    val base = saved?.let { Provider(JsonObject(it.json - "hasKey")) } ?: Provider.new("", "openai-chat", "", fullUrl = false)
    return base.with("name", name.trim()).with("format", format).with("baseUrl", baseUrl.trim()).with("fullUrl", fullUrl)
  }
  fun save(p: Provider, listed: List<ProviderModel>) {
    if (p.id.isEmpty()) before = sources.providers.map { it.id }.toSet()
    listedOnSave = listed
    val action = ProviderAction.Save(p, typedKey)
    waiting = action
    sources.push(action)
  }
  // Anything that reaches the endpoint changed: it is asked first
  val reaches = saved == null || typedKey != null || format != saved.format || baseUrl.trim() != saved.baseUrl || fullUrl != saved.fullUrl
  val submit = {
    if (valid && !busy) {
      error = null
      if (reaches) verifying = sources.probe(ProviderProbe.Models(draft(), typedKey)) else save(draft(), emptyList())
    }
  }
  val verified = verifying?.let { sources.probes[it] }
  LaunchedEffect(verified) {
    when (verified) {
      null -> return@LaunchedEffect
      is ProbeOutcome.Models -> save(draft(), verified.models)
      is ProbeOutcome.Failed -> error = verified.error
      else -> {}
    }
    verifying = null
  }
  LaunchedEffect(sources.settled) {
    val s = sources.settled ?: return@LaunchedEffect
    if (waiting == null || s.action !== waiting) return@LaunchedEffect
    waiting = null
    if (s.error != null) {
      error = s.error
      return@LaunchedEffect
    }
    val added = before?.let { b -> sources.providers.firstOrNull { it.id !in b } }
    model.pop()
    if (added != null) {
      sources.listed[added.id] = listedOnSave
      model.push(Screen.AddModels(added.id))
    }
  }
  // A field that reaches the endpoint changed: the last failure no longer applies
  val touch = { error = null }

  FormPage(
    model, saved?.let { t("settings.providers.edit", "name" to it.title) } ?: t("settings.providers.add"),
    actions = {
      TextButton(onClick = submit, enabled = valid && !busy, modifier = Modifier.testTag("provider.save")) {
        Box(contentAlignment = Alignment.Center) {
          // The label keeps its room under the spinner, so the bar does not shift
          Text(t("settings.providers.save"), color = if (valid && !busy) Palette.fg1 else Palette.fg3, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, modifier = Modifier.alpha(if (busy) 0f else 1f))
          if (busy) CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg2, strokeWidth = Metrics.pairCodeLine)
        }
      }
    },
  ) {
    FormSection(t("settings.providers.name")) {
      TextInput(name, { name = it }, t("settings.providers.name.placeholder"), tag = "provider.name")
    }
    FormCard {
      PickRow(
        t("settings.providers.format"), format, listOf("openai-chat", "openai-responses", "anthropic"),
        name = { t(FORMATS[it] ?: "settings.providers.format.openai") }, tag = "provider.format",
        onPick = { format = it; touch() },
      )
    }
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      FormSection(t("settings.providers.baseUrl")) {
        TextInput(
          baseUrl, { baseUrl = it; touch() }, t("settings.providers.baseUrl.placeholder"), tag = "provider.baseUrl",
          mono = true, keyboard = KeyboardType.Uri,
        )
      }
      // What the agent will call, so a root missing its /v1 shows before the first request fails; the chip switches
      // between appending the format's path and using the URL as typed
      if (baseUrl.isNotBlank()) {
        Row(
          verticalAlignment = Alignment.CenterVertically,
          horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
          modifier = Modifier.padding(horizontal = Metrics.pad),
        ) {
          Text(
            t("settings.providers.endpoint", "url" to endpointOf(baseUrl, format, fullUrl)), color = Palette.fg3, fontSize = Type.text3,
            lineHeight = Type.text3Line, modifier = Modifier.weight(1f),
          )
          Box(
            Modifier.clip(RoundedCornerShape(percent = 50)).background(Palette.chip)
              .clickable { fullOverride = !fullUrl; touch() }
              .padding(horizontal = Metrics.gap, vertical = Metrics.s1).testTag("provider.fullUrl"),
          ) { Text(t(if (fullUrl) "settings.providers.appendPath" else "settings.providers.asTyped"), color = Palette.fg2, fontSize = Type.text3, maxLines = 1) }
        }
      }
      if (local) error?.let { ErrorLine(it, Modifier.padding(horizontal = Metrics.pad).testTag("provider.error")) }
    }
    if (!local) {
      Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
        FormSection(t("settings.providers.key")) {
          TextInput(
            key, { key = it; touch() }, t(if (saved?.hasKey == true) "settings.providers.keyKept" else "settings.providers.key.placeholder"),
            tag = "provider.key", secret = true, onDone = submit,
          )
        }
        error?.let { ErrorLine(it, Modifier.padding(horizontal = Metrics.pad).testTag("provider.error")) }
      }
    }
    if (saved != null) {
      FormCard { ActionRow(t("common.remove"), "trash2", tag = "provider.remove", danger = true) { if (!busy) removing = true } }
    }
  }
  if (removing && saved != null) {
    AlertDialog(
      onDismissRequest = { removing = false },
      title = { Text(t("mobile.providers.removeConfirm", "name" to saved.title)) },
      confirmButton = {
        TextButton(onClick = {
          removing = false
          val action = ProviderAction.Delete(saved.id)
          waiting = action
          sources.push(action)
        }, modifier = Modifier.semantics { testTagsAsResourceId = true }.testTag("provider.remove.confirm")) { Text(t("common.remove"), color = Palette.danger) }
      },
      dismissButton = { TextButton(onClick = { removing = false }) { Text(t("mobile.cancel"), color = Palette.fg1) } },
    )
  }
}

/**
 * Add model: the models the source's endpoint listed when this page was asked for (it opens only with that answer),
 * filtered by the search; a tap adds one or takes an added one out again, each saving the source. An id typed that the
 * list does not hold can be added as is
 */
@Composable
fun AddModelsScreen(model: AppModel, id: String) {
  val sources = model.sources
  val p = sources.providers.firstOrNull { it.id == id }
  val listed = sources.listed[id].orEmpty()
  var query by remember { mutableStateOf("") }
  FormPage(
    model, t("settings.providers.addModel"),
    actions = {
      TextButton(onClick = { model.pop() }, modifier = Modifier.testTag("addModels.done")) {
        Text(t("settings.providers.addModel.done"), color = Palette.fg1, fontSize = Type.text1, fontWeight = FontWeight.SemiBold)
      }
    },
    top = { SearchField(query, placeholder = t("settings.providers.addModel.search"), tag = "addModels.search") { query = it } },
  ) {
    if (p == null) FormCard { Note(t("settings.loading")) }
    else {
      val typed = query.trim()
      val q = typed.lowercase()
      val shown = if (q.isEmpty()) listed else listed.filter { it.id.lowercase().contains(q) || it.name?.lowercase()?.contains(q) == true }
      val offerTyped = typed.isNotEmpty() && listed.none { it.id == typed } && p.models.none { it.id == typed }
      val toggle = { m: ProviderModel ->
        // The source as shown now, so quick taps build on each other while their writes queue
        val now = sources.providers.firstOrNull { it.id == id } ?: p
        val next = if (now.models.any { it.id == m.id }) now.models.filter { it.id != m.id } else now.models + m.enabled(true)
        sources.push(ProviderAction.Save(now.withModels(next)), optimistic = true)
      }
      FormSection(p.title, footer = t("settings.providers.addModel.count", "count" to p.models.size)) {
        if (offerTyped) {
          ActionRow(t("settings.providers.addModel.useId", "id" to typed), "plus", tag = "addModels.typed") {
            toggle(ProviderModel.new(typed))
            query = ""
          }
        }
        shown.forEachIndexed { i, m ->
          key(m.id) {
            if (i > 0 || offerTyped) RowRule()
            val taken = p.models.any { it.id == m.id }
            Row(
              verticalAlignment = Alignment.CenterVertically,
              horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
              modifier = Modifier.fillMaxWidth().clickable { toggle(m) }.heightIn(min = Metrics.formRowHeight)
                .padding(horizontal = Metrics.pad, vertical = Metrics.s1).testTag("addModels.model:${m.id}"),
            ) {
              Column(Modifier.weight(1f)) {
                Text(m.name ?: m.id, color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (m.name != null) Text(m.id, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
              }
              Box(Modifier.size(Metrics.icon), contentAlignment = Alignment.Center) { if (taken) Lucide("check", Palette.fg1, size = Metrics.icon) }
            }
          }
        }
        if (shown.isEmpty() && !offerTyped) Note(t(if (listed.isEmpty()) "settings.providers.addModel.typeId" else "settings.models.noResults"))
      }
    }
  }
}

/** The API formats the form offers, by wire value */
private val FORMATS = mapOf(
  "openai-chat" to "settings.providers.format.openai",
  "openai-responses" to "settings.providers.format.responses",
  "anthropic" to "settings.providers.format.anthropic",
)

/** A row the engine on the computer cannot serve yet */
private const val OFF_ALPHA = 0.4f
