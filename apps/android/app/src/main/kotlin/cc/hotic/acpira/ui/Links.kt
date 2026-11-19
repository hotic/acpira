package cc.hotic.acpira.ui

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.openLink

/**
 * The links the screens open (a reply's `LinkAnnotation`s, anything through [LocalUriHandler]): the app's own first
 * ([AppModel.openLink], a session link opens that chat), a web page in the in-app browser ([openWebPage]), the rest
 * with the system's handler. Root provides it
 */
@Composable
fun rememberLinkHandler(model: AppModel): UriHandler {
  val system = LocalUriHandler.current
  val context = LocalContext.current
  val dark = LocalDark.current
  return remember(model, system, context, dark) {
    object : UriHandler {
      override fun openUri(uri: String) {
        if (!model.openLink(uri) && !openWebPage(context, uri, dark)) system.openUri(uri)
      }
    }
  }
}

/** A page a browser shows: the in-app browser takes http and https only */
fun isWebPage(url: String): Boolean = url.startsWith("http://", ignoreCase = true) || url.startsWith("https://", ignoreCase = true)

/**
 * A web link as a Custom Tab over the app (apps/ios Web/WebPage.swift): back returns to the chat. The intent is the
 * Custom Tabs protocol itself rather than `androidx.browser`, which only wraps these extras. A default browser
 * without Custom Tabs opens the page as a plain view intent. True when a browser took it
 */
fun openWebPage(context: Context, url: String, dark: Boolean): Boolean {
  if (!isWebPage(url)) return false
  val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url)).apply {
    // The session extra, even a null one, is what asks the browser for a Custom Tab
    putExtras(Bundle().apply { putBinder(EXTRA_SESSION, null) })
    // The app's theme, which may differ from the system's
    putExtra(EXTRA_COLOR_SCHEME, if (dark) COLOR_SCHEME_DARK else COLOR_SCHEME_LIGHT)
    putExtra(EXTRA_SHARE_STATE, SHARE_STATE_ON)
  }
  return runCatching { context.startActivity(intent) }.isSuccess
}

/** The system browser, or the app that claims the link */
fun openOutside(context: Context, url: String) {
  runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url))) }
}

// androidx.browser.customtabs.CustomTabsIntent's keys and values
private const val EXTRA_SESSION = "android.support.customtabs.extra.SESSION"
private const val EXTRA_COLOR_SCHEME = "androidx.browser.customtabs.extra.COLOR_SCHEME"
private const val COLOR_SCHEME_LIGHT = 1
private const val COLOR_SCHEME_DARK = 2
private const val EXTRA_SHARE_STATE = "androidx.browser.customtabs.extra.SHARE_STATE"
private const val SHARE_STATE_ON = 1
