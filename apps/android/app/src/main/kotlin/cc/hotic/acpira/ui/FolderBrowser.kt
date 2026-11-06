package cc.hotic.acpira.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.childPath
import cc.hotic.acpira.model.folderName
import cc.hotic.acpira.store.AppModel

/**
 * The folder menu's "Add folder": the computer's folder browser where its engine lists folders (helloOk.features
 * 'localDir'), else the typed path alone. [onAdded] gets the folder once the app model has added it
 */
@Composable
fun AddProject(model: AppModel, open: Boolean, onClose: () -> Unit, onAdded: (String) -> Unit) {
  val add: (String) -> Unit = { folder ->
    model.addProject(folder)
    onAdded(folder.trim())
    onClose()
  }
  if (model.live && "localDir" in model.engineFeatures) FolderBrowser(model, open, onAdd = add, onClose = onClose)
  else PathDialog(open, onAdd = add, onDismiss = onClose)
}

/**
 * The computer's folders, full screen over the page that opened it (apps/ios Sessions/FolderBrowser.swift): the folder
 * shown as the title with its path under it, its subfolders as rows (a tap goes in), the bar's back to the enclosing
 * folder, and the bottom button adding the folder shown. Paths are the engine's own strings, never parsed here: the
 * computer may be Windows
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun FolderBrowser(model: AppModel, open: Boolean, onAdd: (String) -> Unit, onClose: () -> Unit) {
  val folders = model.folders
  var typing by remember { mutableStateOf(false) }
  // A fresh browse starts at the user's home every time it opens
  LaunchedEffect(open) { if (open) folders.open(null) }
  val fade = if (model.motion) Motion.push else 0
  AnimatedVisibility(
    visible = open,
    enter = slideInVertically(tween(fade)) { it / 8 } + fadeIn(tween(fade)),
    exit = slideOutVertically(tween(fade)) { it / 8 } + fadeOut(tween(fade)),
  ) {
    val listing = folders.listing
    val parent = listing?.parent
    // The system back goes up a folder first, as the bar's back does; at a root it closes
    BackHandler(enabled = open) { if (parent != null && !folders.loading) folders.open(parent) else onClose() }
    Scaffold(
      containerColor = Palette.formPage,
      topBar = {
        TopAppBar(
          title = {
            Column {
              Text(
                listing?.path?.let(::folderName) ?: t("mobile.projects.add"), fontSize = Type.text1, fontWeight = FontWeight.SemiBold,
                maxLines = 1, overflow = TextOverflow.Ellipsis,
              )
              listing?.path?.let { Text(it, color = Palette.fg3, fontSize = Type.text4, lineHeight = Type.text4Line, maxLines = 1, overflow = TextOverflow.Ellipsis) }
            }
          },
          navigationIcon = {
            IconButton(onClick = { parent?.let(folders::open) }, enabled = parent != null, modifier = Modifier.testTag("folders.parent")) {
              Lucide("arrowLeft", if (parent != null) Palette.fg1 else Palette.fg3, size = Metrics.iconCtl, contentDescription = t("mobile.projects.parent"))
            }
          },
          actions = {
            IconButton(onClick = { typing = true }, modifier = Modifier.testTag("folders.typePath")) {
              Lucide("folderPen", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("mobile.projects.typePath"))
            }
            IconButton(onClick = onClose, modifier = Modifier.testTag("folders.close")) {
              Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
            }
          },
          colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.formPage),
        )
      },
      bottomBar = {
        Box(Modifier.fillMaxWidth().navigationBarsPadding().padding(horizontal = Metrics.pad, vertical = Metrics.s2), contentAlignment = Alignment.Center) {
          PrimaryPill(
            t("mobile.projects.addHere", "name" to (listing?.path?.let(::folderName) ?: "")),
            onClick = { listing?.path?.let(onAdd) },
            enabled = listing != null && listing.error == null && !folders.loading,
            modifier = Modifier.fillMaxWidth().testTag("folders.add"),
          )
        }
      },
    ) { padding ->
      Box(Modifier.fillMaxSize().padding(padding)) {
        when {
          // The first listing: a quiet spinner where the rows will be
          listing == null -> Box(Modifier.fillMaxWidth().padding(top = Metrics.page), contentAlignment = Alignment.Center) {
            if (folders.loading) CircularProgressIndicator(Modifier.size(Metrics.icon), color = Palette.fg3, strokeWidth = Metrics.pairCodeLine)
          }
          listing.error != null -> Text(
            listing.error.orEmpty(), color = Palette.danger, fontSize = Type.text3, lineHeight = Type.text3Line,
            modifier = Modifier.padding(Metrics.pad).testTag("folders.error"),
          )
          listing.entries.isEmpty() -> Text(
            t("mobile.projects.noFolders"), color = Palette.fg3, fontSize = Type.text1,
            modifier = Modifier.padding(Metrics.pad).testTag("folders.empty"),
          )
          else -> {
            val entries = listing.entries
            // A folder on its way keeps the one before shown, faded and still, so the page never blanks between steps
            LazyColumn(
              Modifier.fillMaxSize().alpha(if (folders.loading) LOADING_ALPHA else 1f),
              contentPadding = PaddingValues(horizontal = Metrics.pad, vertical = Metrics.s2),
            ) {
              itemsIndexed(entries, key = { _, e -> e.name }) { i, entry ->
                val shape = RoundedCornerShape(
                  topStart = if (i == 0) Metrics.rCardRound else Metrics.s0, topEnd = if (i == 0) Metrics.rCardRound else Metrics.s0,
                  bottomStart = if (i == entries.lastIndex) Metrics.rCardRound else Metrics.s0,
                  bottomEnd = if (i == entries.lastIndex) Metrics.rCardRound else Metrics.s0,
                )
                Column(Modifier.fillMaxWidth().clip(shape).background(Palette.formRow)) {
                  if (i > 0) RowRule(lead = true)
                  Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
                    modifier = Modifier.fillMaxWidth()
                      .clickable(enabled = !folders.loading) { folders.open(childPath(listing.path, entry.name)) }
                      .heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad)
                      .testTag("folders.entry:${entry.name}"),
                  ) {
                    // The icon set has no git folder glyph; a git folder reads as an open one
                    Lucide(if (entry.git == true) "folderOpen" else "folder", Palette.fg2, size = Metrics.rowMark)
                    Text(entry.name, color = Palette.fg1, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    Lucide("chevronRight", Palette.fg3, size = Metrics.chevron)
                  }
                }
              }
            }
          }
        }
      }
    }
  }
  PathDialog(typing, onAdd = { typing = false; onAdd(it) }, onDismiss = { typing = false })
}

/** A path typed by hand (apps/ios AddProjectAlert): a folder on the computer running the engine */
@Composable
private fun PathDialog(open: Boolean, onAdd: (String) -> Unit, onDismiss: () -> Unit) {
  if (!open) return
  var path by remember { mutableStateOf("") }
  val submit = { path.trim().takeIf { it.isNotEmpty() }?.let(onAdd) }
  AlertDialog(
    onDismissRequest = onDismiss,
    title = { Text(t("mobile.projects.add")) },
    text = {
      // A dialog is a window of its own: its tags need the resource-id switch again
      Column(Modifier.semantics { testTagsAsResourceId = true }, verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
        Text(t("mobile.projects.pathHint"), color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line)
        TextInput(path, { path = it }, t("mobile.projects.path"), tag = "folders.path", boxed = true, mono = true, onDone = { submit() })
      }
    },
    confirmButton = {
      TextButton(onClick = { submit() }, enabled = path.isNotBlank(), modifier = Modifier.testTag("folders.path.add")) {
        Text(t("mobile.add"), color = if (path.isNotBlank()) Palette.fg1 else Palette.fg3)
      }
    },
    dismissButton = { TextButton(onClick = onDismiss) { Text(t("mobile.cancel"), color = Palette.fg1) } },
  )
}

/** The listing on screen while the next folder loads */
private const val LOADING_ALPHA = 0.5f
