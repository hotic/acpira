package cc.hotic.acpira.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition

/** Motion durations (packages/ui motion tokens): a page push and the toast's entrance */
object Motion {
  const val push = 280
  const val fade = 180
}

/**
 * The app's stack: the session list at the root, a chat, a new session or settings over it. The system back gesture
 * pops one screen, as the iOS app's edge swipe does
 */
@Composable
fun Root(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel? = null) {
  // The settings' hidden models reach every composer's model menu (ChatScreen.swift `.environment(\.hiddenModels)`)
  androidx.compose.runtime.CompositionLocalProvider(
    LocalPairing provides pairing,
    LocalHiddenModels provides model.settings.hiddenOptions,
    // The transcript's folds, rails and entrances read the motion switch (the webview's motion axis)
    cc.hotic.acpira.design.LocalMotion provides model.motion,
  ) { Stack(model, pairing) }
}

@Composable
private fun Stack(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel?) {
  BackHandler(enabled = model.path.isNotEmpty()) { model.pop() }
  // Test tags double as resource ids, so uiautomator and the touch-sim scripts find controls by name
  Box(Modifier.fillMaxSize().background(Palette.bg0).semantics { testTagsAsResourceId = true }) {
    val top: Screen? = model.path.lastOrNull()
    val depth = model.path.size
    AnimatedContent(
      targetState = top to depth,
      transitionSpec = {
        val forward = targetState.second >= initialState.second
        val d = tween<androidx.compose.ui.unit.IntOffset>(Motion.push)
        // Motion off (the settings switch): screens swap at once
        if (!model.motion) EnterTransition.None togetherWith ExitTransition.None
        else if (forward) slideInHorizontally(d) { it } togetherWith slideOutHorizontally(d) { -it / 4 }
        else slideInHorizontally(d) { -it / 4 } togetherWith slideOutHorizontally(d) { it }
      },
      contentKey = { (screen, _) -> screen?.let { key(it) } ?: "list" },
      label = "stack",
    ) { (screen, _) ->
      Surface(color = Palette.bg0, modifier = Modifier.fillMaxSize()) {
        when (screen) {
          null -> SessionListScreen(model)
          is Screen.Chat -> ChatScreen(model, screen.id)
          is Screen.New -> NewSessionScreen(model, screen)
          Screen.Settings -> SettingsScreen(model)
          is Screen.Computer -> if (pairing != null) ComputerScreen(model, pairing, screen.id)
          is Screen.AgentSettings -> AgentSettingsScreen(model, screen.id)
          is Screen.ModelSettings -> ModelsSettingsScreen(model, screen.id)
        }
      }
    }
    // The add-computer sheet, also for an `acpira://pair/…` link opened from outside; the first-run flow pairs in place
    if (pairing != null && pairing.addOpen && !pairing.onboarding) PairSheet(pairing)
    // The engine's and the app's own notices, over every screen
    val toast = model.toast
    val fade = if (model.motion) Motion.fade else 0
    AnimatedVisibility(
      visible = toast != null,
      enter = fadeIn(tween(fade)) + slideInVertically(tween(fade)) { -it / 2 },
      exit = fadeOut(tween(fade)) + slideOutVertically(tween(fade)) { -it / 2 },
      modifier = Modifier.align(Alignment.TopCenter).safeDrawingPadding().padding(top = Metrics.s2),
    ) {
      Surface(
        color = Palette.btn1Bg,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(Metrics.rCardRound),
        shadowElevation = Metrics.toastShadow,
      ) {
        Text(
          toast.orEmpty(), color = Palette.btn1Fg, fontSize = Type.text3,
          modifier = Modifier.padding(horizontal = Metrics.pad, vertical = Metrics.s2 + Metrics.s1),
        )
      }
    }
    if (pairing != null) Welcome(model, pairing)
  }
}

/**
 * The first-run flow over every screen, once the paired computers are known: for a build that pairs, launched bare,
 * with none paired yet and the flow never finished. A launch that finds a computer already paired never shows it, and
 * it does not come back, even with every computer removed (apps/ios RootView `startWelcome`)
 */
@Composable
private fun Welcome(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel) {
  val context = LocalContext.current
  val prefs = remember { context.getSharedPreferences(WELCOME_PREFS, android.content.Context.MODE_PRIVATE) }
  var welcoming by rememberSaveable { mutableStateOf(false) }
  val options = model.options
  LaunchedEffect(pairing.loaded) {
    if (!pairing.loaded || welcoming) return@LaunchedEffect
    if (options.welcome == null) {
      if (prefs.getBoolean(WELCOMED, false) || options.demo || options.engine != null) return@LaunchedEffect
      if (pairing.computers.isNotEmpty()) {
        prefs.edit().putBoolean(WELCOMED, true).apply()
        return@LaunchedEffect
      }
    }
    welcoming = true
  }
  // The add sheet stays down while the flow pairs in place, also after the activity is recreated
  SideEffect { pairing.onboarding = welcoming }
  val fade = if (model.motion) Motion.push else 0
  AnimatedVisibility(visible = welcoming, enter = fadeIn(tween(fade)), exit = fadeOut(tween(fade))) {
    WelcomeFlow(pairing, options.welcome, model.motion, onDone = {
      prefs.edit().putBoolean(WELCOMED, true).apply()
      welcoming = false
      pairing.onboarding = false
      // A pairing link the flow took leaves the add sheet asked for
      pairing.addOpen = false
    })
  }
}

private const val WELCOME_PREFS = "welcome"
private const val WELCOMED = "welcomed"

private fun key(screen: Screen): String = when (screen) {
  is Screen.Chat -> "chat:${screen.id}"
  is Screen.New -> "new"
  Screen.Settings -> "settings"
  is Screen.Computer -> "computer:${screen.id}"
  is Screen.AgentSettings -> "agent:${screen.id}"
  is Screen.ModelSettings -> "models:${screen.id}"
}
