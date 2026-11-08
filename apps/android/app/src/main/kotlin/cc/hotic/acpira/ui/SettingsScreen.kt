package cc.hotic.acpira.ui

import cc.hotic.acpira.design.readableWidth
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.width
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.PerfHud
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.store.AppModel

/**
 * Settings (apps/ios Settings/SettingsScreen.swift): grouped sections on the form page. The look of the app (theme,
 * language, motion), the agents (the default one, the list), the engine connection and the version. Pickers are
 * dropdown menus on the row, the motion switch keeps the system's own colours
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(model: AppModel) {
  Scaffold(
    containerColor = Palette.formPage,
    topBar = {
      TopAppBar(
        title = { Text(t("session.settings"), fontSize = Type.barTitle, fontWeight = FontWeight.SemiBold) },
        navigationIcon = {
          LeadButton("settings.back") { model.pop() }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.formPage),
      )
    },
  ) { padding ->
    Column(
      // The page runs under the gesture bar; only the content keeps clear of it, so it never ends in a hard cut above the bar
      Modifier.fillMaxSize().padding(top = padding.calculateTopPadding()).verticalScroll(rememberScrollState()).readableWidth().padding(horizontal = Metrics.pad).navigationBarsPadding().padding(bottom = Metrics.pad),
      verticalArrangement = Arrangement.spacedBy(Metrics.s6),
    ) {
      FormSection(t("settings.appearance.title")) {
        PickRow(
          t("settings.theme"), model.theme, listOf("system", "light", "dark"),
          name = { t(if (it == "system") "mobile.theme.system" else "settings.theme.$it") },
          tag = "settings.theme", onPick = model::pickTheme,
        )
        RowRule()
        PickRow(
          t("settings.language"), model.language, listOf("auto") + L10n.locales,
          name = { if (it == "auto") t("mobile.language.system") else t("settings.language.$it") },
          tag = "settings.language", onPick = model::pickLanguage,
        )
        RowRule()
        FormRow(t("settings.motion"), Modifier.clickable { model.pickMotion(!model.motion) }.testTag("settings.motion")) {
          Switch(checked = model.motion, onCheckedChange = model::pickMotion, colors = SwitchDefaults.colors())
        }
      }
      // The engine's own settings, shared with the computer (AgentSettings.kt)
      AgentsSections(model)
      SessionsSection(model)
      // Paired remote access: the computers in place of the engine row
      val pairing = LocalPairing.current
      if (pairing != null) PairComputersSection(pairing) { model.push(cc.hotic.acpira.store.Screen.Computer(it.id)) }
      // Notices come from a live wire's list (store/Attention.kt): none for the demo
      if (pairing != null || model.live) NotificationsSection()
      // One fact: the state, with its dot; what it means goes in the footnote
      else FormSection(t("mobile.engine"), footer = if (model.live) null else t("mobile.engine.desc")) {
        val live = model.connection == cc.hotic.acpira.store.Connection.Live
        FormRow(t("mobile.engine.status"), Modifier.testTag("settings.engine")) {
          Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
            Box(Modifier.size(Metrics.unreadDot).background(if (live) Palette.ok else Palette.fg3, CircleShape))
            Text(t(if (live) "connections.state.connected" else "connections.state.off"), color = Palette.fg2, fontSize = Type.text1)
          }
        }
      }
      FormSection(t("mobile.about")) {
        val context = LocalContext.current
        val version = remember {
          runCatching { context.packageManager.getPackageInfo(context.packageName, 0) }.getOrNull()
            ?.let { "${it.versionName} (${it.longVersionCode})" }.orEmpty()
        }
        FormRow(t("settings.fact.version")) { Text(version, color = Palette.fg2, fontSize = Type.text1) }
        RowRule()
        // Developer tools sit under the version, one tap away, as the system's own Developer page does
        FormRow(t("mobile.dev"), Modifier.clickable { model.push(cc.hotic.acpira.store.Screen.Developer) }.testTag("settings.developer")) {
          Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
        }
      }
    }
  }
}

/**
 * Notifications from the followed computer (apps/ios NotificationsSection): the system's word on them, one tap to ask
 * or to change it in the system's settings. Sound, banners and Do Not Disturb are the system's, so the app keeps no
 * switches of its own
 */
@Composable
private fun NotificationsSection() {
  val context = LocalContext.current
  val prefs = remember { context.getSharedPreferences("notify", android.content.Context.MODE_PRIVATE) }
  var state by remember { mutableStateOf(notifyState(context, prefs)) }
  val ask = androidx.activity.compose.rememberLauncherForActivityResult(
    androidx.activity.result.contract.ActivityResultContracts.RequestPermission(),
  ) { state = notifyState(context, prefs) }
  // Back from the system's settings, where it may have changed
  val owner = androidx.compose.ui.platform.LocalLifecycleOwner.current
  androidx.compose.runtime.DisposableEffect(owner) {
    val watch = androidx.lifecycle.LifecycleEventObserver { _, event ->
      if (event == androidx.lifecycle.Lifecycle.Event.ON_RESUME) state = notifyState(context, prefs)
    }
    owner.lifecycle.addObserver(watch)
    onDispose { owner.lifecycle.removeObserver(watch) }
  }
  FormSection(null, footer = t("mobile.notify.desc")) {
    FormRow(
      t("mobile.notify"),
      Modifier.clickable {
        if (state == "mobile.notify.ask" && android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
          prefs.edit().putBoolean("asked", true).apply()
          ask.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        } else {
          val open = android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
            .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName)
          runCatching { context.startActivity(open) }
        }
      }.testTag("settings.notify"),
    ) {
      Text(t(state), color = Palette.fg2, fontSize = Type.text1)
    }
  }
}

/** On, off (refused, or switched off in the system's settings), or not asked yet */
private fun notifyState(context: android.content.Context, prefs: android.content.SharedPreferences): String = when {
  cc.hotic.acpira.store.Attention.allowed(context) -> "mobile.notify.on"
  android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU && !prefs.getBoolean("asked", false) -> "mobile.notify.ask"
  else -> "mobile.notify.off"
}

/** Developer (apps/ios DeveloperSettingsPage): the on-screen frame-rate and hitch overlay's switches ([PerfHud]) */
@Composable
fun DeveloperSettingsScreen(model: AppModel) {
  val context = LocalContext.current
  val prefs = remember { context.getSharedPreferences(PerfHud.PREFS, android.content.Context.MODE_PRIVATE) }
  var fps by remember { mutableStateOf(PerfHud.isOn(prefs, PerfHud.FPS)) }
  var hitches by remember { mutableStateOf(PerfHud.isOn(prefs, PerfHud.HITCHES)) }
  // The overlay follows the stored switches itself
  FormPage(model, t("mobile.dev")) {
    FormSection(null, footer = t("mobile.dev.desc")) {
      SwitchRow(t("mobile.dev.fps"), fps, tag = "developer.fps") {
        fps = it
        PerfHud.set(prefs, PerfHud.FPS, it)
      }
      RowRule()
      SwitchRow(t("mobile.dev.hitches"), hitches, tag = "developer.hitches") {
        hitches = it
        PerfHud.set(prefs, PerfHud.HITCHES, it)
      }
    }
  }
}

/** A titled group: the caption, the card, then a footnote under it */
@Composable
internal fun FormSection(title: String?, footer: String? = null, content: @Composable ColumnScope.() -> Unit) {
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
    title?.let { Text(it, color = Palette.fg3, fontSize = Type.text3, fontWeight = FontWeight.Medium, modifier = Modifier.padding(horizontal = Metrics.pad)) }
    FormCard(content)
    footer?.let {
      Text(it, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, modifier = Modifier.padding(horizontal = Metrics.pad))
    }
  }
}

@Composable
internal fun FormCard(content: @Composable ColumnScope.() -> Unit) {
  // Clipped, so a pressed first or last row lights the card's rounded corners rather than a square over them
  Column(Modifier.fillMaxWidth().clip(RoundedCornerShape(Metrics.rCardRound)).background(Palette.formRow), content = content)
}

/** The hairline between rows, from the text's start (past the mark with [lead]) to the card's end */
@Composable
internal fun RowRule(lead: Boolean = false) {
  HorizontalDivider(color = Palette.line, modifier = Modifier.padding(start = if (lead) Metrics.pad + Metrics.rowMark + Metrics.gap else Metrics.pad))
}

/** A label with its value or control at the end; [lead] a glyph before the label, in the mark's slot */
@Composable
internal fun FormRow(label: String, modifier: Modifier = Modifier, lead: (@Composable () -> Unit)? = null, value: @Composable () -> Unit) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    modifier = Modifier.fillMaxWidth().then(modifier).heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad),
  ) {
    if (lead != null) {
      lead()
      Spacer(Modifier.width(Metrics.gap - Metrics.s2))
    }
    Text(label, color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
    value()
  }
}

/** A row whose value opens a menu of [options] at the row's end; the picked one carries a tick */
@Composable
internal fun PickRow(label: String, value: String, options: List<String>, name: (String) -> String, tag: String, onPick: (String) -> Unit) {
  var open by remember { mutableStateOf(false) }
  FormRow(label, Modifier.clickable { open = true }.testTag(tag)) {
    Box {
      Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1)) {
        Text(name(value), color = Palette.fg2, fontSize = Type.text1)
        Lucide("chevronsUpDown", Palette.fg3, size = Metrics.chevron)
      }
      AppMenu(expanded = open, onDismiss = { open = false }) {
        for (o in options) {
          AppMenuItem(
            name(o),
            onClick = { open = false; if (o != value) onPick(o) },
            checked = o == value,
            modifier = Modifier.testTag("$tag:$o"),
          )
        }
      }
    }
  }
}
