package cc.hotic.acpira.ui

import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.SidecarProtocol
import cc.hotic.acpira.sync.ConnectionDiagnosis
import cc.hotic.acpira.sync.PairedComputer
import java.time.Instant
import java.time.temporal.ChronoUnit
import java.util.Locale

// Where a failed connection stopped, behind the failure page's "Details" (apps/ios Pairing/PairDiagnosis.swift): the
// versions on both sides, the relay, the connection's own words, and the whole of it as text to copy and send.

/** The report as a sheet with a form, laid out as the account sheet is */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PairDiagnosisSheet(computer: PairedComputer, diagnosis: ConnectionDiagnosis, onDismiss: () -> Unit) {
  val state = rememberModalBottomSheetState(skipPartiallyExpanded = false)
  val context = LocalContext.current
  val haptics = LocalHapticFeedback.current
  val app = remember { appVersion(context) }
  var copied by remember { mutableStateOf(false) }
  ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state, containerColor = Palette.formPage, dragHandle = null) {
    // A sheet is a window of its own: its test tags need the resource ids again
    Column(Modifier.fillMaxWidth().navigationBarsPadding().semantics { testTagsAsResourceId = true }.testTag("pair.details")) {
      Box(Modifier.fillMaxWidth().heightIn(min = Metrics.barCtl + Metrics.s2).padding(horizontal = Metrics.s1)) {
        IconButton(onClick = onDismiss, modifier = Modifier.align(Alignment.CenterStart).size(Metrics.barCtl).testTag("pair.details.close")) {
          Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
        }
        Text(
          t("mobile.pair.details"), color = Palette.fgStrong, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, maxLines = 1,
          modifier = Modifier.align(Alignment.Center),
        )
      }
      Column(
        Modifier.verticalScroll(rememberScrollState()).padding(horizontal = Metrics.pad).padding(top = Metrics.s2, bottom = Metrics.page),
        verticalArrangement = Arrangement.spacedBy(Metrics.s6),
      ) {
        FormCard {
          FormRow(t("mobile.pair.details.app")) { Text(app, color = Palette.fg2, fontSize = Type.text1) }
          RowRule()
          FormRow(t("connections.remoteVersion")) {
            Text(diagnosis.computerVersion ?: computer.version ?: "—", color = Palette.fg2, fontSize = Type.text1)
          }
          diagnosis.relayConnected?.let { relay ->
            RowRule()
            FormRow(t("mobile.pair.path.relay")) {
              Text(t(if (relay) "mobile.pair.connected" else "mobile.pair.state.offline"), color = Palette.fg2, fontSize = Type.text1)
            }
          }
        }
        FormCard {
          // The connection's own words, as they go into the report
          SelectionContainer {
            Text(
              diagnosis.detail, color = Palette.fg2, fontSize = Type.mono, lineHeight = Type.monoLine, fontFamily = FontFamily.Monospace,
              modifier = Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s3).testTag("pair.details.error"),
            )
          }
        }
        FormCard {
          FormRow(
            t(if (copied) "code.copied" else "common.copy"),
            Modifier.clickable {
              copyText(context, diagnosisReport(computer, diagnosis, app))
              haptics.play(Haptic.Copy)
              copied = true
            }.testTag("pair.details.copy"),
          ) {}
        }
      }
    }
  }
}

/** The app's version name and build, as the settings' About row shows it */
internal fun appVersion(context: Context): String =
  runCatching { context.packageManager.getPackageInfo(context.packageName, 0) }.getOrNull()
    ?.let { "${it.versionName} (${it.longVersionCode})" } ?: "0 (0)"

/**
 * The report as plain English text, the same in every language and on iOS (PairDiagnosisSheet.report): it is read by
 * whoever is asked for help
 */
internal fun diagnosisReport(computer: PairedComputer, d: ConnectionDiagnosis, app: String): String {
  var stopped = "Stopped: ${d.stop.name}"
  d.tookMs?.let { stopped += String.format(Locale.ROOT, " after %.1f s", it / MS_PER_S) }
  d.relayConnected?.let { stopped += if (it) ", relay connected" else ", relay not connected" }
  var theirs = "Computer: Acpira ${d.computerVersion ?: computer.version?.let { "$it (last seen)" } ?: "unknown"}"
  d.computerProtocol?.let { theirs += ", protocol $it" }
  return listOf(
    "Acpira app $app, protocol ${SidecarProtocol.VERSION}",
    theirs,
    stopped,
    "Error: ${d.detail}",
    // ISO 8601 in UTC without fractions, as the iOS report writes it
    "At: ${Instant.ofEpochMilli(d.at).truncatedTo(ChronoUnit.SECONDS)}",
  ).joinToString("\n")
}

private const val MS_PER_S = 1000.0
