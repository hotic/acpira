package cc.hotic.acpira.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.UriHandler
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.openLink

/**
 * The links the screens open (a reply's `LinkAnnotation`s, anything through [LocalUriHandler]): the app's own first
 * ([AppModel.openLink], a session link opens that chat), the rest with the system's handler. Root provides it
 */
@Composable
fun rememberLinkHandler(model: AppModel): UriHandler {
  val system = LocalUriHandler.current
  return remember(model, system) {
    object : UriHandler {
      override fun openUri(uri: String) {
        if (!model.openLink(uri)) system.openUri(uri)
      }
    }
  }
}
