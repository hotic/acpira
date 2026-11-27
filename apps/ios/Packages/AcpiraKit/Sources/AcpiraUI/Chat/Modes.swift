import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import QuickLook
import SwiftUI
#if canImport(PhotosUI)
import PhotosUI
#endif
#if canImport(UIKit)
import UIKit
#endif

// MARK: - Mode

/// modeIcons.ts: the glyph is inferred from keywords in id + name; first match wins
func modeIcon(_ mode: SessionOption) -> LucideIcon {
  if mode.kind == "full_access" { return .shieldOff }
  if mode.kind == "auto_review" { return .sparkles }
  if mode.id == "auto_edit" || mode.id == "acceptEdits" { return .code }
  let key = "\(mode.id) \(mode.name)".lowercased()
  let rules: [(String, LucideIcon)] = [
    (#"bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe"#, .shieldOff),
    (#"debug"#, .bug),
    (#"plan"#, .listTodo),
    (#"\bask\b|chat|question"#, .messageCircleQuestionMark),
    (#"smart|\bauto\b"#, .sparkles),
    (#"code|edit|build"#, .code),
    (#"\bteam\b|multitask|orchestrat"#, .users),
    (#"agent|default|normal"#, .infinity),
  ]
  return rules.first { key.range(of: $0.0, options: .regularExpression) != nil }?.1 ?? .circleDashed
}

/// modeIcons.ts `modeLabel`: the short name for a mode the built-in integrations are known to send, matched on the
/// exact id (never the display name, so a user's own agent keeps its name), in the app's language and one length in
/// CJK; anything unknown shows the agent's own name
func modeLabel(_ mode: SessionOption) -> String {
  if mode.kind == "full_access" { return t("composer.modeShort.full") }
  if mode.kind == "auto_review" { return t("composer.modeShort.auto") }
  let short: [String: String] = [
    "default": "composer.access.ask", "agent": "composer.modeShort.agent", "build": "composer.modeShort.build",
    "plan": "composer.modeShort.plan", "raw": "composer.modeShort.raw", "team": "composer.modeShort.team", "acceptEdits": "composer.modeShort.autoEdit", "auto_edit": "composer.modeShort.autoEdit",
    "auto": "composer.modeShort.auto", "smart": "composer.modeShort.smart", "bypassPermissions": "composer.modeShort.full",
    "bypass": "composer.modeShort.full", "yolo": "composer.modeShort.full", "agent-full-access": "composer.modeShort.full",
    "read-only": "composer.modeShort.readOnly", "workspace-write": "composer.modeShort.workspace",
    "ask": "composer.modeShort.ask", "code": "composer.modeShort.code", "accept-edits": "composer.modeShort.code",
  ]
  guard let key = short[mode.id] else { return mode.name }
  return t(key)
}

/// modeIcons.ts `modeName`: a working mode where it is picked by name (the `/` list, the bar's pill), the bare name
/// without the bean labels' padding to one length; unknown modes keep the label
func modeName(_ mode: SessionOption) -> String {
  let names = ["plan": "composer.modeName.plan", "team": "composer.modeName.team", "raw": "composer.modeName.raw"]
  return names[mode.id].map { t($0) } ?? modeLabel(mode)
}

/// modeIcons.ts `modeHint`: one line on what the mode does, else the agent's own description
func modeHint(_ mode: SessionOption) -> String? {
  let hints = ["plan": "composer.modeHint.plan", "team": "composer.modeHint.team", "raw": "composer.modeHint.raw"]
  return hints[mode.id].map { t($0) } ?? mode.description
}
