import AcpiraModels
import Foundation
import JavaScriptCore
import SwiftUI
import Testing
#if canImport(UIKit)
import WebKit
#endif
@testable import AcpiraUI

// The engine settings the phone edits: the model menus' hidden families (cases from packages/shared/test/modelCatalog.test.ts
// and models.test.ts, so the phone and the desktop read one `hiddenOptions` alike) and the agent switches and order

@Suite @MainActor struct ModelVisibilityTests {
  private func options(_ names: [String]) -> [SessionOption] { names.map { SessionOption(id: $0, name: $0) } }
  private func sourced(_ id: String, _ name: String, _ source: String, _ kind: String) -> SessionOption {
    var o = SessionOption(id: id, name: name)
    o.source = SessionOption.Source(id: source, name: source, kind: kind)
    return o
  }

  @Test func familyKeysAreTheDesktopsJSONKeys() {
    let families = ModelCatalog.group([sourced("gw/k3", "K3", "asgard/gw", "custom"), SessionOption(id: "k2", name: "K2")])
    #expect(families.map(\.key) == [#"["asgard/gw","K3"]"#, "K2"])
    #expect(families[0].sourceKind == "custom")
  }

  @Test func newestReleaseOfEachSeriesComesFirst() {
    let families = ModelCatalog.group(options(["Claude Opus 5", "GLM-5.2", "Claude Opus 5.5", "Kimi K2.7", "GLM-5.3", "Kimi K3"]))
    #expect(ModelCatalog.prioritize(families).map(\.name) == ["Claude Opus 5.5", "GLM-5.3", "Kimi K3", "Claude Opus 5", "GLM-5.2", "Kimi K2.7"])
    #expect(ModelCatalog.prioritize(ModelCatalog.group(options(["Opus 5.9", "Automatic", "Opus 5.10", "Qwen 8B", "Qwen 32B"]))).map(\.name)
      == ["Opus 5.10", "Automatic", "Qwen 8B", "Qwen 32B", "Opus 5.9"])
    #expect(ModelCatalog.prioritize(ModelCatalog.group(options(["claude-opus-4-6", "claude-opus-4-8"])))[0].name == "claude-opus-4-8")
    let bySource = ModelCatalog.group([
      sourced("official-old", "Claude Opus 5", "official", "official"), sourced("gateway-new", "Claude Opus 5.5", "gateway", "custom"),
      sourced("official-new", "Claude Opus 5.5", "official", "official"),
    ])
    #expect(ModelCatalog.prioritize(bySource).map { $0.variants[0].id } == ["official-new", "gateway-new", "official-old"])
  }

  @Test func searchMatchesNameSourceAndVariantId() {
    let families = ModelCatalog.group([sourced("asgard/opus-max", "Claude Opus 5.5 Max", "asgard", "custom"), SessionOption(id: "auto", name: "Automatic")])
    #expect(ModelCatalog.filter(families, query: " ASGARD opus ").map(\.key) == [families[0].key])
    #expect(ModelCatalog.filter(families, query: "opus-max").map(\.key) == [families[0].key])
    #expect(ModelCatalog.filter(families, query: "unmatched").isEmpty)
    #expect(ModelCatalog.filter(families, query: "  ").count == 2)
  }

  @Test func bulkSwitchKeepsUnknownEntriesAndTheCurrentValue() {
    let choices = options(["Claude Opus 5 Low", "Claude Opus 5 Max", "GLM-5.3"])
    let families = ModelCatalog.group(choices)
    let hidden = ModelCatalog.setAllVisible(families, hidden: ["absent-model"], visible: false)
    #expect(families.allSatisfy { ModelCatalog.hidden($0, in: hidden) })
    #expect(ModelCatalog.visibleOptions(choices, hidden: hidden, current: choices[0].id) == [choices[0]])
    #expect(ModelCatalog.setAllVisible(families, hidden: hidden, visible: true) == ["absent-model"])
  }

  @Test func legacyNameOnlyEntriesSplitPerSource() {
    let kimi = [sourced("kimi-code/k3", "K3", "kimi-code", "official"), sourced("asgard/kimi-k3", "K3", "asgard", "custom")]
    let families = ModelCatalog.group(kimi)
    #expect(families.allSatisfy { ModelCatalog.hidden($0, in: ["K3"]) })
    let hidden = ModelCatalog.setVisible(kimi, hidden: ["K3"], key: families[1].key, show: true)
    #expect(hidden == [families[0].key])
    #expect(ModelCatalog.visibleOptions(kimi, hidden: hidden, current: nil).map(\.id) == ["asgard/kimi-k3"])
    #expect(ModelCatalog.setAllVisible(families, hidden: ["K3"], visible: true).isEmpty)
  }

  @Test func theModelsPageSplitsOfficialFromCustom() {
    var control = ConfigControl(id: "model", name: "Model", category: "model", type: "select",
                                options: [sourced("o/k3", "K3", "official", "official"), sourced("g/k3", "K3", "gw", "custom"), SessionOption(id: "auto", name: "Auto")])
    control.value = "o/k3"
    let effort = ConfigControl(id: "effort", name: "Effort", category: "thought_level", type: "select", options: options(["low", "high"]))
    // The built-in agent's sidekick pick and its levels are composer controls, not model families
    let sidekick = ConfigControl(id: "sidekick", name: "Sidekick", category: "_sidekick", type: "select", options: options(["none", "g/k3"]))
    let sidekickEffort = ConfigControl(id: "sidekick_effort", name: "Sidekick effort", category: "_sidekick_effort", type: "select", options: options(["low", "high"]))
    let catalogs = ModelSettings.catalogs([control, effort, sidekick, sidekickEffort]) ?? []
    #expect(catalogs.map(\.control.id) == ["model"])
    let groups = ModelSettings.groups(catalogs, query: "")
    #expect(groups.map(\.key) == ["model:official", "model:custom", "model:other"])
    #expect(groups.map { $0.rows.count } == [1, 1, 1])
    #expect(ModelSettings.count([control], hidden: ["model": [catalogs[0].families[0].key]])! == (shown: 2, total: 3))
  }
}

@Suite @MainActor struct EngineSettingsTests {
  private func model() -> AppModel {
    AppModel(options: LaunchOptions(fixture: "showcase", theme: nil, motion: false, locale: "en", screen: "list"))
  }

  @Test func agentsSwitchOffAndReorderLikeTheDesktop() throws {
    let m = model()
    let ids = m.agents.map(\.id)
    try #require(ids.count >= 2)
    m.setAgentEnabled(ids[0], false)
    #expect(m.settings?.disabledAgents == [ids[0]])
    #expect(m.agent(ids[0]).disabled == true)
    #expect(!launchable(m.agents).contains { $0.id == ids[0] })
    // The default that was switched off reads as the agent new sessions fall back to
    if let settings = m.settings, settings.defaultAgent == ids[0] {
      #expect(pickDefaultAgent(m.agents, settings.defaultAgent) != ids[0])
    }
    m.setAgentEnabled(ids[0], true)
    #expect(m.settings?.disabledAgents == [])
    m.moveAgents(from: IndexSet(integer: 0), to: ids.count)
    #expect(m.settings?.agentOrder == Array(ids.dropFirst()) + [ids[0]])
    #expect(m.agents.map(\.id).last == ids[0])
  }

  @Test func theLastAgentOnStaysOn() {
    let m = model()
    let ids = m.agents.map(\.id)
    for id in ids { m.setAgentEnabled(id, false) }
    #expect(m.agents.filter { $0.disabled != true }.map(\.id) == [ids.last])
  }

  @Test func hiddenModelsDropEmptyEntries() {
    let m = model()
    m.setHidden("claude", control: "model", ["a"])
    #expect(m.settings?.hiddenOptions == ["claude": ["model": ["a"]]])
    m.setHidden("claude", control: "model", [])
    #expect(m.settings?.hiddenOptions == [:])
  }
}

// The composer's model chip lists what the settings left visible, the model in use whatever its family
@Suite @MainActor struct HiddenModelChipTests {
  @Test func hiddenFamiliesLeaveTheChipButNotTheCurrentModel() {
    let control = ConfigControl(id: "model", name: "Model", category: "model", type: "select",
                                options: ["Kimi K3", "GLM-5.3", "Opus 5.5"].map { SessionOption(id: $0, name: $0) }, value: "GLM-5.3")
    let controls = SessionControls(options: [control])
    let all = ModelChoices(controls: controls, onSetConfig: { _, _ in }).families.map(\.name)
    let shown = ModelChoices(controls: controls, hidden: ["model": ["Kimi K3", "GLM-5.3"]], onSetConfig: { _, _ in }).families.map(\.name)
    #expect(all.count == 3)
    #expect(shown.sorted() == ["GLM-5.3", "Opus 5.5"])
  }
}

// The widget page: no network, the widget's own CSP and <base> dropped, the theme's variables on :root
@Suite @MainActor struct WidgetDocTests {
  @Test func theWidgetGetsOurHeadAndNoNetwork() {
    let doc = WidgetDoc.build(#"<meta http-equiv="Content-Security-Policy" content="default-src *"><base href="https://x.test/"><p class="text-muted-foreground">hi</p>"#, dark: true)
    #expect(doc.contains("default-src 'none'"))
    #expect(!doc.contains("default-src *"))
    #expect(!doc.contains("<base"))
    #expect(doc.contains(#"<html class="dark">"#))
    #expect(doc.contains("--muted-foreground: rgba("))
    #expect(doc.contains("--color-muted-foreground: var(--muted-foreground);"))
    #expect(!WidgetDoc.tailwindTheme.contains("--color-radius"))
    #expect(doc.hasSuffix(#"<p class="text-muted-foreground">hi</p>"#))
  }

  @Test func aThemeSwitchSetsTheSameVariables() {
    let light = WidgetDoc.theme(dark: false)
    #expect(light.contains("classList.toggle('dark', false)"))
    #expect(light.contains("colorScheme = 'light'"))
    // Every variable the page head declares is set again
    for (name, _) in WidgetDoc.vars(dark: false) { #expect(light.contains("\"\(name)\"")) }
  }

  @Test func bootstrapExposesSendPromptBehindActivationAndToken() {
    let ctx = JSContext()!
    ctx.evaluateScript("""
      var posted = [], active = false;
      var window = this, navigator = { userActivation: { get isActive() { return active; } } };
      var document = { addEventListener: function () {}, documentElement: { getBoundingClientRect: function () { return { height: 0 }; } } };
      function addEventListener() {}
      window.webkit = { messageHandlers: { acpiraWidget: { postMessage: function (m) { posted.push(m); } } } };
      """)
    ctx.evaluateScript(WidgetDoc.bootstrap(token: "tok-1"))
    #expect(ctx.exception == nil)

    // Without user activation (a script running at load), sendPrompt throws and posts nothing
    ctx.evaluateScript("var err = ''; try { window.acpira.sendPrompt('hi'); } catch (e) { err = e.message; }")
    #expect(ctx.evaluateScript("err")?.toString().contains("click or key") == true)
    #expect(ctx.evaluateScript("posted.length")?.toInt32() == 0)

    // Activated: validates input, then posts with the bootstrap's private token
    ctx.evaluateScript("active = true;")
    ctx.evaluateScript("err = ''; try { window.acpira.sendPrompt('   '); } catch (e) { err = e.name; }")
    #expect(ctx.evaluateScript("err")?.toString() == "TypeError")
    ctx.evaluateScript("err = ''; try { window.acpira.sendPrompt('x'.repeat(20001)); } catch (e) { err = e.name; }")
    #expect(ctx.evaluateScript("err")?.toString() == "RangeError")
    ctx.evaluateScript("window.acpira.sendPrompt('  Picked D  ');")
    #expect(ctx.evaluateScript("posted.length")?.toInt32() == 1)
    #expect(ctx.evaluateScript("posted[0].token")?.toString() == "tok-1")
    #expect(ctx.evaluateScript("posted[0].prompt")?.toString() == "  Picked D  ")
  }

  @Test func promptGateTrimsCapsAndThrottlesAndRespectsThreadRole() {
    var gate = WidgetPromptGate()
    let t0 = ContinuousClock.now
    #expect(gate.accept("   ", now: t0) == nil)
    #expect(gate.accept(String(repeating: "a", count: WidgetDoc.promptMax + 1), now: t0) == nil)
    #expect(gate.accept("  Picked D \n", now: t0) == "Picked D")
    // A second tap inside the gap drops; after the gap it goes through
    #expect(gate.accept("Again", now: t0 + .milliseconds(400)) == nil)
    #expect(gate.accept("Again", now: t0 + .milliseconds(1100)) == "Again")

    var main = EnvironmentValues()
    main.applyThread(SessionView(id: "s", agent: "claude", title: "", cwd: "", status: .ready), quiet: false)
    #expect(main.widgetCanSend)
    main.applyThread(SessionView(id: "s", agent: "claude", title: "", cwd: "", status: .readonly), quiet: false)
    #expect(!main.widgetCanSend)

    var inspector = EnvironmentValues()
    inspector.subagentTranscript = true
    inspector.applyThread(SessionView(id: "s", agent: "claude", title: "", cwd: "", status: .ready), quiet: false)
    #expect(!inspector.widgetCanSend)
  }

  #if canImport(UIKit)
  @Test func webViewReportsHeightAndBlocksUnactivatedOrForgedPrompts() async {
    var reported: CGFloat = 0
    var prompts: [String] = []
    let binding = Binding(get: { reported }, set: { reported = $0 })
    let html = WidgetDoc.build("""
      <div style="height: 48px">pick</div>
      <script>
        try { window.acpira.sendPrompt('auto'); } catch (_) {}
        window.webkit.messageHandlers.acpiraWidget.postMessage({ token: 'forged', prompt: 'forged' });
      </script>
      """, dark: true)
    let representable = WidgetWebView(html: html, dark: true, height: binding, onPrompt: { prompts.append($0) })
    let coordinator = representable.makeCoordinator()
    let config = WKWebViewConfiguration()
    config.websiteDataStore = .nonPersistent()
    config.userContentController.addUserScript(
      WKUserScript(source: WidgetDoc.bootstrap(token: coordinator.token), injectionTime: .atDocumentStart, forMainFrameOnly: true))
    config.userContentController.add(coordinator, name: "acpiraWidget")
    let web = WKWebView(frame: CGRect(x: 0, y: 0, width: 320, height: 200), configuration: config)
    web.navigationDelegate = coordinator
    coordinator.onPrompt = { prompts.append($0) }
    web.loadHTMLString(html, baseURL: nil)
    for _ in 0..<40 where reported == 0 {
      try? await Task.sleep(for: .milliseconds(50))
    }
    WidgetWebView.dismantleUIView(web, coordinator: coordinator)
    #expect(reported >= 48)
    #expect(prompts.isEmpty)
  }
  #endif
}

// Session links read the same as in TypeScript and Rust: every case of test/fixtures/session-links.json
@Suite @MainActor struct SessionLinkTests {
  struct Case: Decodable { var input: String; var link: String?; var ref: String? }

  @Test func everySharedCaseAgrees() throws {
    let data = try Data(contentsOf: ContractTests.root.appending(path: "test/fixtures/session-links.json"))
    let cases = try JSONDecoder().decode([Case].self, from: data)
    #expect(cases.count > 10)
    for c in cases {
      #expect(SessionLinks.id(ofLink: c.input) == c.link, "link \(c.input)")
      #expect(SessionLinks.ref(c.input) == c.ref, "ref \(c.input)")
    }
  }
}

@Suite @MainActor struct LinkOpeningTests {
  @Test func aSessionLinkOpensThatSession() throws {
    let m = AppModel(options: LaunchOptions(fixture: "showcase", theme: nil, motion: false, locale: "en", screen: "list"))
    let id = try #require(m.summaries.first?.id)
    #expect(m.openLink(try #require(URL(string: "vscode://hotic.acpira/session/\(id)"))))
    #expect(m.path.last == .chat(id))
    // Opening it again does not stack it twice
    #expect(m.openLink(try #require(URL(string: "acpira://session/\(id)"))))
    #expect(m.path.count == 1)
    #expect(!m.openLink(try #require(URL(string: "https://example.com/session/\(id)"))))
    #expect(m.openLink(try #require(URL(string: "acpira://session/nope"))))
    #expect(m.toast != nil && m.path.count == 1)
  }
}
