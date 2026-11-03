package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen

/**
 * A new session (apps/ios Sessions/NewSessionScreen.swift): the agent's mark in the middle with its name and the folder
 * it starts in, both pickable; the first prompt from the composer starts it. No folder: a chat
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewSessionScreen(model: AppModel, screen: Screen.New) {
  val agents = model.startable
  // As the iOS app picks it (RootView destination): the asked agent, else the newest session's, else the first that can start
  var agent by rememberSaveable {
    mutableStateOf(screen.agent ?: model.summaries.maxByOrNull { it.updatedAt }?.agent ?: agents.firstOrNull()?.id ?: model.settings.defaultAgent)
  }
  // The latest project unless asked otherwise; a chat when the computer has none
  val projects = remember(model.summaries, model.projectOrder) {
    (model.projectOrder + model.summaries.sortedByDescending { it.updatedAt }.map { it.cwd }).filter { it.isNotEmpty() && !model.isChat(it) }.distinct()
  }
  var cwd by rememberSaveable { mutableStateOf(screen.cwd ?: projects.firstOrNull()) }
  var draft by rememberSaveable { mutableStateOf("") }
  val info = model.agent(agent)

  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      TopAppBar(
        title = { Text(t("session.new"), fontSize = Type.text1, fontWeight = FontWeight.SemiBold) },
        navigationIcon = {
          IconButton(onClick = { model.pop() }, modifier = Modifier.testTag("new.back")) {
            Lucide("arrowLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.back"))
          }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.bg0),
      )
    },
    bottomBar = {
      Composer(
        text = draft, onText = { draft = it }, mark = agent, markName = info.name, running = false,
        placeholder = t("mobile.new.placeholder"),
        onSend = {
          val text = draft
          draft = ""
          model.start(agent, cwd, text)
        },
        onStop = {},
      )
    },
  ) { padding ->
    Column(
      Modifier.fillMaxSize().padding(padding),
      horizontalAlignment = Alignment.CenterHorizontally,
      verticalArrangement = Arrangement.spacedBy(Metrics.gap, Alignment.CenterVertically),
    ) {
      var pickAgent by remember { mutableStateOf(false) }
      Box {
        Box(
          Modifier.size(Metrics.heroTile + Metrics.s3)
            .background(Palette.bg1, CircleShape)
            .border(Metrics.hairline, Palette.conversationLine, CircleShape)
            .clickable { pickAgent = true }
            .testTag("new.agent"),
          contentAlignment = Alignment.Center,
        ) {
          VendorMark(agent, info.name, Palette.fgStrong, size = Metrics.heroBare, fit = true)
        }
        DropdownMenu(expanded = pickAgent, onDismissRequest = { pickAgent = false }) {
          for (a in agents) {
            DropdownMenuItem(
              text = { Text(a.name) },
              leadingIcon = { VendorMark(a.id, a.name, Palette.fg1, size = Metrics.icon, fit = true) },
              trailingIcon = { if (a.id == agent) Lucide("check", Palette.fg1) },
              onClick = { agent = a.id; pickAgent = false },
              modifier = Modifier.testTag("agent:${a.id}"),
            )
          }
        }
      }
      Text(info.name, color = Palette.fgStrong, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = Metrics.s1))
      var pickFolder by remember { mutableStateOf(false) }
      Box {
        Row(
          verticalAlignment = Alignment.CenterVertically,
          horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
          modifier = Modifier
            .background(Palette.bg1, RoundedCornerShape(percent = 50))
            .border(Metrics.hairline, Palette.conversationLine, RoundedCornerShape(percent = 50))
            .clickable { pickFolder = true }
            .padding(horizontal = Metrics.pad, vertical = Metrics.s2)
            .testTag("new.folder"),
        ) {
          Lucide(if (cwd == null) "messageSquare" else "folder", Palette.fg2, size = Metrics.icon)
          Text(cwd?.let { model.placeName(it) } ?: t("session.group.chats"), color = Palette.fg2, fontSize = Type.text1)
        }
        DropdownMenu(expanded = pickFolder, onDismissRequest = { pickFolder = false }) {
          DropdownMenuItem(
            text = { Text(t("session.group.chats")) },
            leadingIcon = { Lucide("messageSquare", Palette.fg1) },
            onClick = { cwd = null; pickFolder = false },
          )
          for (p in projects) {
            DropdownMenuItem(
              text = { Text(model.placeName(p)) },
              leadingIcon = { Lucide("folder", Palette.fg1) },
              trailingIcon = { if (p == cwd) Lucide("check", Palette.fg1) },
              onClick = { cwd = p; pickFolder = false },
            )
          }
        }
      }
    }
  }
}
