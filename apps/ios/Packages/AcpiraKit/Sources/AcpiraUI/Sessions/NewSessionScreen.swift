import AcpiraDesign
import AcpiraModels
import SwiftUI

// A new session before its first prompt: the agent's mark and the project in the middle of an empty thread, and the
// composer. The two choices a session is born with (which agent CLI runs it and which folder it works in) are made in
// the middle: the bare mark rings itself with the other agents, a glass capsule under the name opens the folders.
// Sending the first prompt creates the session and replaces this screen with its chat.

struct NewSessionScreen: View {
  @Environment(AppModel.self) private var model
  @State private var draft: SessionView
  @State private var text = ""
  @State private var staged: [Attachment] = []
  @State private var homeInset: CGFloat = 0
  /// A model or mode was picked here, so the engine's later answer for the agent's controls no longer replaces them
  @State private var picked = false
  /// The agent ring is out: the page dims under it and a tap anywhere folds it
  @State private var ring = false
  /// The hero's bottom and the composer's top on the page, so a composer grown into the hero hides it
  @State private var heroBottom: CGFloat = 0
  @State private var composerTop = CGFloat.infinity
  /// A composer picker is out: its beans and panels rise into the hero's place, so the hero steps aside meanwhile
  @State private var picking = false
  @Environment(\.motionEnabled) private var motion
  /// The list hand-over: its stand-in takes the composer's place while the page comes from the list or goes back to it
  /// (BarHandoff.swift)
  @Environment(BarHandoff.self) private var handoff: BarHandoff?
  /// `-draft TEXT`: the composer pre-filled, as in a chat
  @Environment(\.initialDraft) private var initialDraft

  init(agent: AgentId, cwd: String, controls: SessionControls) {
    _draft = State(initialValue: SessionView(id: "draft", agent: agent, title: t("session.new"), cwd: cwd, controls: controls))
  }

  var body: some View {
    let agent = model.agent(draft.agent)
    // A long prompt (or the keyboard) brings the composer up to the hero: it fades rather than show through the glass
    let covered = composerTop < heroBottom + Metrics.s3
    return VStack(spacing: 0) {
      // The hero sits at the page's optical centre (`heroRise`) and the composer lies over the page's bottom, so the
      // composer growing (the attach tray, a long prompt) never moves the hero
      GeometryReader { page in
        ZStack {
          if ring {
            Palette.gooDim
              .ignoresSafeArea()
              .contentShape(Rectangle())
              .onTapGesture { ring = false }
              .transition(.opacity)
          }
          NewSessionHero(agent: agent, cwd: draft.cwd, agents: model.agents,
                         recent: model.projects, onAgent: pickAgent, onProject: { draft.cwd = $0 },
                         onAdd: { model.addProject($0); draft.cwd = $0 }, ring: $ring)
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.page)).maxY } action: { heroBottom = $0 }
            .opacity(covered || picking ? 0 : 1)
            .allowsHitTesting(!covered)
            .animation(motion ? .easeOut(duration: Motion.durOpen) : nil, value: covered)
            // Gone as the beans leave the chip, back only as they get home (the dim's own timing)
            .animation(motion ? picking ? .easeOut(duration: Motion.durOpen) : Motion.gooDimOut : nil, value: picking)
            .position(x: page.size.width / 2, y: page.size.height * Metrics.heroRise)
        }
        // The dim lifts with an ease-in, held while the bubbles go home (the composer pickers' dim)
        .animation(ring ? .easeOut(duration: Motion.durOpen) : Motion.gooDimOut, value: ring)
      }
      .overlay(alignment: .bottom) {
        // The composer rests where the list's system bottom bar does
        DockSeat(homeInset: homeInset) {
          Composer(session: draft, text: $text, staged: $staged, onSend: start,
                   onSetMode: { picked = true; draft.controls.modeId = $0 }, onSetConfig: setConfig,
                   placeholder: t("mobile.new.placeholder"))
            .opacity(handoff?.hidden == draft.id ? 0 : 1)
            #if os(iOS)
            .background { BarHandoffProbe(session: draft, placeholder: t("mobile.new.placeholder")) }
            #endif
            .padding(.horizontal, Metrics.pad)
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.page)).minY } action: { composerTop = $0 }
        }
      }
    }
    .coordinateSpace(.named(Self.page))
    .onPreferenceChange(ComposerPickerKey.self) { picking = $0 }
    .onAppear { if let initialDraft, text.isEmpty { text = initialDraft } }
    .ignoresSafeArea(.container, edges: .bottom)
    .modifier(HomeInset(inset: $homeInset))
    // The agent's models as the engine knows them (its probe or its newest session), so the model chip is there
    // before the first prompt; they fill the draft once they arrive unless something was picked meanwhile. Asked
    // again on every new wire: an answer the lost one never brought left the page without a model chip for good
    .task(id: ControlsAsk(agent: draft.agent, wire: ObjectIdentifier(model))) { model.loadControls(draft.agent) }
    .onChange(of: model.agentControls[draft.agent]) { _, _ in
      guard !picked else { return }
      let known = model.controls(for: draft.agent)
      if known != draft.controls { draft.controls = known }
    }
    // The composer's liquid pickers, drawn over the whole screen so every piece takes taps
    .gooLayer()
    .background(Palette.bg0)
    .modifier(SystemTitle(title: t("session.new")))
    .environment(\.agentAccent, Palette.accent(for: draft.agent))
    .environment(\.hiddenModels, model.settings?.hiddenOptions[draft.agent] ?? [:])
    .environment(\.sessionActions, SessionActions(toast: { model.show($0, icon: $1) }))
    // Staged photos open in a Lightbox, as in a chat (the draft has no blobs of its own: they are inline pixels)
    .modifier(MediaLayer(session: draft, loader: model.media))
  }

  /// The page's coordinate space, for the hero and the composer to meet in
  nonisolated private static let page = "new.page"

  /// What the controls are asked for: the agent, on this wire's app model (every wire brings a new one)
  private struct ControlsAsk: Equatable {
    var agent: AgentId
    var wire: ObjectIdentifier
  }

  /// Another agent: its own controls (models and modes differ per CLI)
  private func pickAgent(_ id: AgentId) {
    draft.agent = id
    draft.controls = model.controls(for: id)
    picked = false
  }

  private func setConfig(_ control: String, _ value: String) {
    guard let at = draft.controls.options.firstIndex(where: { $0.id == control }) else { return }
    picked = true
    draft.controls.options[at].value = value
  }

  /// The first prompt creates the session; its chat takes this screen's place in the stack
  private func start(_ prompt: String, _ attachments: [Attachment]) {
    guard !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return }
    // With an engine the chat comes once the engine has made the session (AppModel+Engine.swift)
    guard let id = model.newSession(agent: draft.agent, cwd: draft.cwd, controls: draft.controls, text: prompt, attachments: attachments) else { return }
    if model.path.isEmpty { model.path = [.chat(id)] } else { model.path[model.path.count - 1] = .chat(id) }
  }
}

/// The empty thread's centre: the agent's bare mark, which rings itself with the other agents (AgentRing), its name,
/// and the folder as a glass capsule opening the recent projects' system menu
struct NewSessionHero: View {
  let agent: AgentInfo
  let cwd: String
  var agents: [AgentInfo] = []
  var recent: [String] = []
  var onAgent: (AgentId) -> Void = { _ in }
  var onProject: (String) -> Void = { _ in }
  var onAdd: (String) -> Void = { _ in }
  @Binding var ring: Bool
  /// The name and the folder are hidden while the ring is out and until AgentRing lets them back
  @State private var veil = false
  @State private var adding = false
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    VStack(spacing: Metrics.s2) {
      AgentRing(agent: agent, agents: agents, open: $ring, veil: $veil, onSelect: onAgent)
        .padding(.bottom, Metrics.s3)
      VStack(spacing: Metrics.s3) {
        name
          // A new agent's name is a new text, swapped at once while hidden. A `.transaction` with no animation here
          // also stopped the group's offset from reaching it, so the name jumped up as the ring opened
          .id(agent.id)
          .transition(.identity)
        Menu {
          NativeProjectMenu(cwd: cwd, recent: recent, onSelect: onProject, onAdd: { adding = true })
        } label: {
          HStack(spacing: Metrics.s1_5) {
            Icon(.folder, size: Metrics.icon)
            Text(web: folderName(cwd), paragraph: false).textStyle(TypeScale.text2)
          }
          .foregroundStyle(Palette.fg2)
          .padding(.horizontal, Metrics.s3)
          .padding(.vertical, Metrics.s1_5)
          .glassEffect(GlassStyle.control.interactive(), in: Capsule())
        }
        .menuStyle(.button)
        // Keep local before remote and Add last, even when the system opens the menu upward.
        .menuOrder(.fixed)
        .buttonStyle(.plain)
        .modifier(AddProjectAlert(isPresented: $adding, onAdd: onAdd))
      }
      // The name and the folder step back while the ring is out and rise back from under the drop once the picked
      // mark has landed, or once the bubbles are home
      .opacity(veil || ring ? 0 : 1)
      .offset(y: veil || ring ? -Metrics.s3 : 0)
      .animation(motion ? (veil || ring ? Motion.ringHeroAway : Motion.ringHeroBack) : nil, value: veil || ring)
    }
    .padding(.horizontal, Metrics.page)
    .task {
      // Screenshots and recordings: `-popup ring` puts the ring out a beat after the first frame (`-popup ring.pick`
      // then picks an agent, AgentRing)
      guard initialPopup == "ring" || initialPopup == "ring.pick" else { return }
      try? await Task.sleep(for: .seconds(1.2))
      ring = true
    }
  }

  private var name: some View {
    Text(web: agent.name, paragraph: false)
      .textStyle(TypeScale.field, weight: .semibold)
      .foregroundStyle(Palette.fgStrong)
  }
}

/// The agent's mark in a round drop of Liquid Glass with the other launchable agents on a ring around it, all one body
/// of glass. Tapping the drop buds the bubbles off it one after another, clockwise from the top; a tap outside melts
/// them back. A pick sends the picked bubble into the drop while the others fold home: its mark grows to the centre's
/// size as the old mark shrinks away under it, and the picked agent takes the centre where its mark has landed. The
/// drop stays: clearing it after a pick, even with the name rising in the same move, read as two things at once. An
/// agent that is not installed stays on the ring, dimmed
struct AgentRing: View {
  let agent: AgentInfo
  let agents: [AgentInfo]
  @Binding var open: Bool
  /// Holds the hero's name and folder back until the picked mark has landed or the bubbles are home
  @Binding var veil: Bool
  var onSelect: (AgentId) -> Void
  /// The bubble on its way into the centre after a pick, until its agent takes the centre
  @State private var picking: AgentId?
  /// Where the picked bubble sat on the ring, from the centre: its mark flies in from there
  @State private var flightFrom = CGSize.zero
  /// A missing agent's mark, faded as a disabled row's
  private static let missingInk: Double = 0.35
  @Environment(\.motionEnabled) private var motion
  @Environment(\.initialPopup) private var initialPopup

  var body: some View {
    let others = launchable(agents).filter { $0.id != agent.id }
    GlassEffectContainer(spacing: Metrics.s6) {
      ZStack {
        ForEach(Array(others.enumerated()), id: \.element.id) { i, a in
          bubble(a, at: i, of: others.count)
        }
        hub
      }
    }
    .frame(width: Metrics.heroTile, height: Metrics.heroTile)
    // The picked agent's mark, over all the glass: as the bubble's own content it went under the drop's glass the
    // moment the two met, and the centre stood empty until the swap
    .overlay {
      ZStack {
        // Looked up among all agents: it is the centre's own agent for the last part of the flight
        if let id = picking, let a = agents.first(where: { $0.id == id }) {
          VendorMark(brand: a.id, name: a.name, size: Metrics.heroBare)
            .foregroundStyle(Palette.fgStrong)
            .transition(.asymmetric(insertion: .modifier(active: RingFlight(offset: flightFrom, scale: Metrics.ringMark / Metrics.heroBare),
                                                         identity: RingFlight(offset: .zero, scale: 1)),
                                    removal: .identity))
            .allowsHitTesting(false)
        }
      }
      // Its own clock: the screen's dim animation (keyed on the ring) would otherwise run the flight on an ease-in,
      // and the mark trailed the bubble's glass into the drop
      .transaction { $0.animation = motion ? Motion.ringFlight : nil }
    }
    .task(id: open) {
      if open {
        veil = true
        // Recordings: `-popup ring.pick` picks the first installed agent on the ring once it has settled
        guard initialPopup == "ring.pick" else { return }
        try? await Task.sleep(for: .seconds(2))
        if let i = others.firstIndex(where: { $0.available != false }) { pick(others[i], from: spot(i, of: others.count)) }
      } else if veil, picking == nil {
        // A plain close; after a pick the name comes back as the mark lands (`pick`)
        if motion { try? await Task.sleep(for: .seconds(Motion.ringHeroHold)) }
        veil = false
      }
    }
  }

  /// The ring's radius: close round the drop for a few agents, wider as there are more, so neighbours keep
  /// `ringPitch` between their centres and their captions clear each other
  private func radius(_ count: Int) -> CGFloat {
    max(Metrics.ringRadiusMin, Metrics.ringPitch * CGFloat(count) / (2 * .pi))
  }

  /// Bubble `i` of `n` on the ring, from the centre, clockwise from the top
  private func spot(_ i: Int, of n: Int) -> CGSize {
    let angle = -Double.pi / 2 + 2 * Double.pi * Double(i) / Double(max(n, 1))
    let r = radius(n)
    return CGSize(width: cos(angle) * r, height: sin(angle) * r)
  }

  /// The centre: the mark in its drop of glass. During a pick the old mark shrinks away under the arriving one
  private var hub: some View {
    Button { if picking == nil { open.toggle() } } label: {
      VendorMark(brand: agent.id, name: agent.name, size: Metrics.heroBare)
        .foregroundStyle(Palette.fgStrong)
        .opacity(picking == nil ? 1 : 0)
        .scaleEffect(picking == nil ? 1 : Metrics.gooSeed)
        .animation(motion ? Motion.ringFlight : nil, value: picking)
        .frame(width: Metrics.heroTile, height: Metrics.heroTile)
        .contentShape(Circle())
    }
    .buttonStyle(.plain)
    .glassEffect(GlassStyle.control.interactive(), in: Circle())
    .accessibilityLabel(agent.name)
  }

  /// A pick: the bubble's glass flies into the drop while its mark flies over the glass (the overlay) and grows to
  /// the centre's size as the old mark shrinks away. As it nears the centre the agent changes under it and the hero's
  /// name and folder rise back; once landed the flown mark leaves in the frame the centre's own mark shows, so it stays
  private func pick(_ a: AgentInfo, from spot: CGSize) {
    guard picking == nil else { return }
    guard motion else {
      onSelect(a.id)
      open = false
      return
    }
    flightFrom = spot
    withAnimation(Motion.ringFlight) {
      picking = a.id
      open = false
    }
    Task { @MainActor in
      var swap = Transaction()
      swap.disablesAnimations = true
      try? await Task.sleep(for: .seconds(Motion.ringLand))
      // Under the flown mark: the centre's mark is still hidden while picking, the name still veiled
      withTransaction(swap) { onSelect(a.id) }
      veil = false
      try? await Task.sleep(for: .seconds(Motion.ringFlightTime - Motion.ringLand))
      withTransaction(swap) { picking = nil }
    }
  }

  private func bubble(_ a: AgentInfo, at i: Int, of n: Int) -> some View {
    let spot = spot(i, of: n)
    let missing = a.available == false
    let flying = picking == a.id
    // Out on the ring, or on its way into the centre after a pick
    let shown = open || flying
    let lag = Motion.ringStagger(n)
    let outDelay = Double(i) * lag
    let homeDelay = Double(n - 1 - i) * lag / 2
    return Button { pick(a, from: spot) } label: {
      VendorMark(brand: a.id, name: a.name, size: Metrics.ringMark)
        .foregroundStyle(Palette.fg1)
        .opacity(missing ? Self.missingInk : 1)
        // A picked bubble flies as glass alone: its mark leaves at once for the overlay, which starts on this spot
        .opacity(flying ? 0 : 1)
        .transaction { $0.animation = nil }
        .frame(width: Metrics.ringBubble, height: Metrics.ringBubble)
        // Glass hosts its content in a layer of its own, so the bubble's opacity and scale below do not reach the mark:
        // it carries both itself, fading on the bubble's own delays and shrinking with its glass on the same spring.
        // Faded at once on the way home, it left empty drops drifting into the centre
        .opacity(shown ? 1 : 0)
        .animation(motion ? (flying ? Motion.ringFlight
                             : open ? Motion.ringShow.delay(outDelay + Motion.ringShowDelay)
                             : Motion.ringHide.delay(homeDelay + Motion.ringHideDelay)) : nil,
                   value: open)
        .scaleEffect(shown ? 1 : Metrics.gooSeed)
        .animation(motion ? (flying ? Motion.ringFlight : Motion.ringSpring.delay(open ? outDelay : homeDelay)) : nil,
                   value: open)
    }
    .buttonStyle(.plain)
    .glassEffect(GlassStyle.control.interactive(), in: Circle())
    // Comes and goes inside the drop, where the system's own morph would only lag the spring
    .glassEffectTransition(.identity)
    .overlay(alignment: .bottom) {
      Text(web: a.name, paragraph: false)
        // The hero's name style, so a name reads the same on the ring and in the centre
        .textStyle(TypeScale.field, weight: .semibold)
        .foregroundStyle(missing ? Palette.fg3 : Palette.fgStrong)
        .fixedSize()
        .offset(y: Metrics.s6 - Metrics.s0_5)
        .opacity(open ? 1 : 0)
        .animation(motion ? (open ? Motion.ringCaption.delay(outDelay + Motion.ringCaptionDelay) : Motion.ringCaptionOut) : nil,
                   value: open)
    }
    .disabled(missing)
    .accessibilityLabel(a.name)
    // The picked bubble leaves the ring at once when its agent takes the centre, right where its mark has landed
    .transition(.identity)
    .opacity(shown ? 1 : 0)
    .animation(motion ? (flying ? Motion.ringFlight
                         : open ? Motion.ringShow.delay(outDelay + Motion.ringShowDelay)
                         : Motion.ringHide.delay(homeDelay + Motion.ringHideDelay)) : nil,
               value: open)
    .scaleEffect(shown ? 1 : Metrics.gooSeed)
    .offset(open && !flying ? spot : .zero)
    .animation(motion ? (flying ? Motion.ringFlight : Motion.ringSpring.delay(open ? outDelay : homeDelay)) : nil,
               value: open)
  }
}

/// The flown mark's way in: from its bubble's spot at the ring's size to the centre at full size
private struct RingFlight: ViewModifier {
  var offset: CGSize
  var scale: CGFloat
  func body(content: Content) -> some View {
    content.scaleEffect(scale).offset(offset)
  }
}
