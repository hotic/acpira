package cc.hotic.acpira.store

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.SessionLinks

// Links the app opens itself (apps/ios Store/AppModel+Links.swift): a session link (`acpira://session/<id>` and the
// shells' forms, logic/SessionLinks.kt) in a reply, from a notification or from another app opens that session over the
// current screen, as the desktop's link handler does. Everything else goes to the system.

/** Opens [url] when it is the app's to open (a session link); false: the caller hands it to the system */
fun AppModel.openLink(url: String): Boolean {
  val id = SessionLinks.idOfLink(url) ?: return false
  if (path.lastOrNull() == Screen.Chat(id)) return true
  // The engine knows sessions this list may not show yet; the demo only its own
  if (!live && views[id] == null) {
    showToast(t("mobile.link.missing"))
    return true
  }
  push(Screen.Chat(id))
  return true
}
