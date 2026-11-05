package cc.hotic.acpira.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.zIndex
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ModelSettings
import cc.hotic.acpira.logic.ModelVisibility
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.compactPresets
import cc.hotic.acpira.store.pickDefaultAgent
import cc.hotic.acpira.store.refreshControls
import cc.hotic.acpira.store.setAccountSwitch
import cc.hotic.acpira.store.setAgentEnabled
import cc.hotic.acpira.store.setAutoCompact
import cc.hotic.acpira.store.setCompactAt
import cc.hotic.acpira.store.setHidden
import cc.hotic.acpira.store.setPlanAutoApprove
import cc.hotic.acpira.store.setSteerQueued
import cc.hotic.acpira.store.moveAgent
import cc.hotic.acpira.store.settingsAgents
import kotlin.math.roundToInt

// apps/ios Settings/EngineSettings.swift: the engine's settings the phone offers (General.tsx, PluginsPage.tsx,
// AgentPage.tsx) as form rows: the agents (switches, order, the default), how sessions run (steering, compaction,
// account switching), and per agent the Plan-mode approval and which models its menus show. Model sources, keys, MCP
// and the rest stay on the computer: typing a provider key on the phone widens what a lost phone exposes.

/** The settings' hidden models (agent → control id → family keys), for the composers' model menus */
val LocalHiddenModels = compositionLocalOf<Map<String, Map<String, List<String>>>> { emptyMap() }

/** The agents' header: the default agent's card, then every agent, a tap opening its page and a long press dragging it */
@Composable
internal fun AgentsSections(model: AppModel) {
  val agents = model.settingsAgents
  // Switched-off agents are not offered; a default that was switched off reads as the agent new sessions fall back to
  FormSection(t("mobile.settings.agents")) {
    PickRow(
      t("settings.defaultAgent"), pickDefaultAgent(model.agents, model.settings.defaultAgent), agents.filter { it.disabled != true }.map { it.id },
      name = { id -> model.agent(id).name }, tag = "settings.defaultAgent", onPick = model::setDefaultAgent,
    )
  }
  FormSection(null, footer = t("mobile.settings.agents.desc")) { AgentList(model, agents) }
}

/**
 * Every agent laid out, as the system's Settings lists its apps: a row opens that agent's page (its on / off switch
 * included), touch and hold lifts it to drag into the order every list follows. The rows are one height, so the drop
 * slot is the offset in rows
 */
@Composable
private fun ColumnScope.AgentList(model: AppModel, agents: List<AgentInfo>) {
  var dragging by remember { mutableStateOf<String?>(null) }
  var offset by remember { mutableFloatStateOf(0f) }
  var rowPx by remember { mutableFloatStateOf(1f) }
  val haptics = LocalHapticFeedback.current
  val from = agents.indexOfFirst { it.id == dragging }
  val target = if (from < 0) -1 else (from + (offset / rowPx).roundToInt()).coerceIn(agents.indices)
  agents.forEachIndexed { i, a ->
    key(a.id) {
      val lifted = a.id == dragging
      // The rows between the lifted one and its slot step aside by one row
      val shift = when {
        from < 0 || lifted -> 0f
        i in (from + 1)..target -> -rowPx
        i in target until from -> rowPx
        else -> 0f
      }
      val slide by animateFloatAsState(shift, tween(if (model.motion) Motion.fade else 0), label = "agentRow")
      Column(
        Modifier.fillMaxWidth()
          .onSizeChanged { rowPx = it.height.toFloat().coerceAtLeast(1f) }
          .zIndex(if (lifted) 1f else 0f)
          .graphicsLayer { translationY = if (lifted) offset else slide }
          .then(if (lifted) Modifier.shadow(Metrics.barShadow).background(Palette.formRow) else Modifier)
          .clickable { model.push(Screen.AgentSettings(a.id)) }
          // Inside the click, so a drag's consumed moves cancel the tap
          .pointerInput(a.id) {
            detectDragGesturesAfterLongPress(
              onDragStart = {
                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                dragging = a.id
                offset = 0f
              },
              onDrag = { change, delta ->
                change.consume()
                val list = model.settingsAgents
                val at = list.indexOfFirst { it.id == a.id }
                offset = (offset + delta.y).coerceIn(-at * rowPx, (list.size - 1 - at) * rowPx)
              },
              onDragEnd = {
                val list = model.settingsAgents
                val at = list.indexOfFirst { it.id == a.id }
                if (at >= 0) model.moveAgent(at, (at + (offset / rowPx).roundToInt()).coerceIn(list.indices))
                dragging = null
                offset = 0f
              },
              onDragCancel = { dragging = null; offset = 0f },
            )
          }
          .testTag("settings.agent:${a.id}"),
      ) {
        // The rule rides with its row, so a lifted row carries no stray hairline over the others
        if (i > 0) RowRule(lead = true)
        Row(
          verticalAlignment = Alignment.CenterVertically,
          horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
          modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad),
        ) {
          VendorMark(a.id, a.name, Palette.fg2, size = Metrics.rowMark, fit = true)
          Text(a.name, color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
          if (a.disabled == true) Text(t("mobile.settings.agentOff"), color = Palette.fg2, fontSize = Type.text1)
          Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
        }
      }
    }
  }
}

/** How sessions run: the account menu first, the switches under it; the threshold stays under the switch it needs */
@Composable
internal fun SessionsSection(model: AppModel) {
  val s = model.settings
  FormSection(t("mobile.settings.sessions"), footer = t("mobile.settings.engine.desc")) {
    PickRow(
      t("settings.accountSwitch"), s.accountSwitch, listOf("off", "earliestReset", "mostRemaining", "listOrder"),
      name = { t("settings.accountSwitch.$it") }, tag = "settings.accountSwitch", onPick = model::setAccountSwitch,
    )
    RowRule()
    // Titles alone: the desktop's hints run to three lines at the phone's width
    SwitchRow(t("settings.steerQueued"), s.steerQueued, tag = "settings.steerQueued", onChange = model::setSteerQueued)
    RowRule()
    SwitchRow(t("settings.autoCompact"), s.autoCompact, tag = "settings.autoCompact", onChange = model::setAutoCompact)
    if (s.autoCompact) {
      RowRule()
      // Presets in the menu instead of a number field; a value set on the computer stays listed
      PickRow(
        t("settings.compactAt"), s.compactAtTokens.toString(), compactPresets(s.compactAtTokens).map { it.toString() },
        name = { n -> n.toInt().let { if (it == 0) t("settings.compactAt.model") else "${it / 1000}${t("settings.compactAt.unit")}" } },
        tag = "settings.compactAt", onPick = { model.setCompactAt(it.toInt()) },
      )
    }
  }
}

/** A switch row: the whole row flips it; [hint] a second line under the title, as the system words a switch */
@Composable
internal fun SwitchRow(label: String, on: Boolean, tag: String, enabled: Boolean = true, hint: String? = null, onChange: (Boolean) -> Unit) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    modifier = Modifier.fillMaxWidth().clickable(enabled = enabled) { onChange(!on) }.testTag(tag)
      .heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad, vertical = if (hint != null) Metrics.s2 else Metrics.s0_5),
  ) {
    Column(Modifier.weight(1f)) {
      Text(label, color = if (enabled) Palette.fg1 else Palette.fg3, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
      hint?.let { Text(it, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
    Switch(checked = on, onCheckedChange = onChange, enabled = enabled, colors = SwitchDefaults.colors())
  }
}

/** A pushed settings page: back, the title, [actions] at the bar's end, the form scrolling under the gesture bar */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun FormPage(model: AppModel, title: String, actions: @Composable () -> Unit = {}, top: @Composable () -> Unit = {}, content: @Composable ColumnScope.() -> Unit) {
  Scaffold(
    containerColor = Palette.formPage,
    topBar = {
      TopAppBar(
        title = { Text(title, fontSize = Type.barTitle, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        navigationIcon = {
          IconButton(onClick = { model.pop() }, modifier = Modifier.testTag("settings.back")) {
            Lucide("arrowLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.back"))
          }
        },
        actions = { actions() },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.formPage),
      )
    },
  ) { padding ->
    Column(Modifier.fillMaxSize().padding(top = padding.calculateTopPadding())) {
      top()
      Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = Metrics.pad).navigationBarsPadding().imePadding()
          .padding(bottom = Metrics.pad),
        verticalArrangement = Arrangement.spacedBy(Metrics.s6),
        content = content,
      )
    }
  }
}

/** One agent: on or off for new sessions, Plan-mode approval, and the models its menus show */
@Composable
fun AgentSettingsScreen(model: AppModel, id: String) {
  val agent = model.agent(id)
  val settings = model.settings
  val enabled = model.settingsAgents.filter { it.disabled != true }
  LaunchedEffect(id) { model.loadControls(id) }
  FormPage(model, agent.name) {
    FormCard {
      // The last agent on stays on: a new session needs one
      SwitchRow(
        t("settings.agent.enable"), agent.disabled != true, tag = "agent.enable", enabled = enabled.map { it.id } != listOf(id),
      ) { model.setAgentEnabled(id, it) }
    }
    FormSection(t("settings.agent.permissions"), footer = t("settings.planAutoApprove.desc", "agent" to agent.name)) {
      SwitchRow(t("settings.planAutoApprove"), id in settings.planAutoApprove, tag = "agent.planAutoApprove") { model.setPlanAutoApprove(id, it) }
    }
    FormSection(null, footer = t("settings.models.desc")) {
      val catalogs = ModelSettings.catalogs(model.agentControls[id])
      val count = catalogs?.takeIf { it.isNotEmpty() }?.let { ModelSettings.count(it, settings.hiddenOptions[id].orEmpty()) }
      FormRow(t("settings.models.selection"), Modifier.clickable { model.push(Screen.ModelSettings(id)) }.testTag("agent.models")) {
        count?.let { Text("${it.first}", color = Palette.fg2, fontSize = Type.text1) }
        Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
      }
    }
  }
}

/** The model menus' families with a switch each (AgentPage.tsx ModelsSection): a search, the all-models switch, then
 *  one group per control (official / custom models apart where the adapter says which is which) */
@Composable
fun ModelsSettingsScreen(model: AppModel, id: String) {
  val agent = model.agent(id)
  var query by remember { mutableStateOf("") }
  LaunchedEffect(id) { model.loadControls(id) }
  val hidden = model.settings.hiddenOptions[id].orEmpty()
  val catalogs = ModelSettings.catalogs(model.agentControls[id])
  FormPage(
    model, t("settings.models.selection"),
    // The desktop's refresh: the engine asks the agent again (a fresh probe), as the empty state says
    actions = {
      IconButton(onClick = { model.refreshControls(id) }, modifier = Modifier.testTag("models.refresh")) {
        Lucide("refreshCw", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.refresh"))
      }
    },
    top = { if (!catalogs.isNullOrEmpty()) SearchField(query) { query = it } },
  ) {
    when {
      catalogs == null -> FormCard { Note(t("settings.loading")) }
      catalogs.isEmpty() -> FormCard { Note(t("settings.models.none", "agent" to agent.name)) }
      else -> {
        if (query.isEmpty()) {
          val (shown, total) = ModelSettings.count(catalogs, hidden)
          FormSection(null, footer = t("settings.models.enabled", "count" to shown, "total" to total)) {
            SwitchRow(t("settings.models.all"), shown == total, tag = "models.all") { on ->
              for (c in catalogs) {
                val now = model.settings.hiddenOptions[id]?.get(c.control.id).orEmpty()
                model.setHidden(id, c.control.id, ModelVisibility.setAllVisible(c.families, now, on))
              }
            }
          }
        }
        val groups = ModelSettings.groups(catalogs, query)
        for (g in groups) {
          key(g.key) {
            FormSection(
              // A lone group is the page itself, whose title already names it
              if (groups.size > 1) g.title else null,
              footer = g.sources.takeIf { it.isNotEmpty() }?.joinToString(t("common.metaSep")),
            ) {
              g.rows.forEachIndexed { i, f ->
                if (i > 0) RowRule()
                val off = hidden[g.control.id].orEmpty()
                SwitchRow(
                  f.name, !ModelVisibility.hidden(f, off), tag = "models.family:${f.key}",
                  hint = ModelSettings.summary(f) ?: f.source,
                ) { on ->
                  val now = model.settings.hiddenOptions[id]?.get(g.control.id).orEmpty()
                  model.setHidden(id, g.control.id, ModelVisibility.setVisible(g.control.options, now, f.key, on))
                }
              }
            }
          }
        }
        if (query.isNotEmpty() && groups.isEmpty()) FormCard { Note(t("settings.models.noResults")) }
      }
    }
  }
}

/** A quiet line in a card: loading, nothing found */
@Composable
private fun Note(text: String) {
  Box(Modifier.fillMaxWidth().heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad, vertical = Metrics.s2), contentAlignment = Alignment.CenterStart) {
    Text(text, color = Palette.fg3, fontSize = Type.text1)
  }
}

/** The models page's search, under the bar as the system's searchable field sits */
@Composable
private fun SearchField(query: String, onQuery: (String) -> Unit) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    modifier = Modifier.padding(horizontal = Metrics.pad).padding(bottom = Metrics.s2).fillMaxWidth().height(Metrics.ctl)
      .background(Palette.formRow, RoundedCornerShape(Metrics.rCardRound)).padding(horizontal = Metrics.gap),
  ) {
    Lucide("search", Palette.fg3, size = Metrics.icon)
    Spacer(Modifier.width(Metrics.s2))
    Box(Modifier.weight(1f)) {
      if (query.isEmpty()) Text(t("settings.models.search"), color = Palette.fg3, fontSize = Type.text1, maxLines = 1)
      BasicTextField(
        value = query, onValueChange = onQuery, singleLine = true,
        textStyle = TextStyle(color = Palette.fg1, fontSize = Type.text1),
        cursorBrush = SolidColor(Palette.fg1),
        modifier = Modifier.fillMaxWidth().testTag("models.search"),
      )
    }
    if (query.isNotEmpty()) {
      Box(Modifier.clickable { onQuery("") }.padding(Metrics.s1)) { Lucide("x", Palette.fg3, size = Metrics.icon, contentDescription = t("panel.clear")) }
    }
  }
}
