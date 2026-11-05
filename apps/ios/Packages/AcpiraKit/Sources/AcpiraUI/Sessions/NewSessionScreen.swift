import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

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
  /// Each agent keeps its own picks; later probe replies must not replace a draft that has been edited.
  @State private var picks: [AgentId: SessionControls] = [:]
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
          // Nothing here scrolls, so the thread's scroll dismissal has no counterpart: a tap or a downward drag on
          // the empty page ends typing instead, under the hero so its mark and capsule keep their own taps
          Color.clear
            .contentShape(Rectangle())
            .onTapGesture { Self.endEditing() }
            .gesture(DragGesture(minimumDistance: Metrics.s3).onChanged { if $0.translation.height > abs($0.translation.width) { Self.endEditing() } })
          if ring {
            Palette.gooDim
              .ignoresSafeArea()
              .contentShape(Rectangle())
              .onTapGesture { ring = false }
              .transition(.opacity)
          }
          NewSessionHero(agent: agent, cwd: draft.cwd, agents: model.agents,
                         recent: model.projects, chat: model.startsChat(draft.cwd), onAgent: pickAgent,
                         onProject: { draft.cwd = $0 }, onAdd: { model.addProject($0); draft.cwd = $0 },
                         // An empty folder is a chat (AppModel.startsChat); offered where the engine keeps chats
                         onChat: model.chatRoot == nil ? nil : { draft.cwd = "" }, ring: $ring)
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.page)).maxY } action: { heroBottom = $0 }
            .opacity(covered || picking ? 0 : 1)
            .allowsHitTesting(!covered)
            .animation(motion ? .easeOut(duration: Motion.durOpen) : nil, value: covered)
            // Gone as the beans leave the chip, back only as they get home (the dim's own timing)
            .animation(motion ? picking ? .easeOut(duration: Motion.durOpen) : Motion.gooDimOut : nil, value: picking)
            .position(x: page.size.width / 2, y: page.size.height * Metrics.heroRise)
        }
        // The dim lifts with an ease-in, held while the bubbles go home (the composer pickers' dim). After a pick it
        // lifts on the flight's own spring instead: the ease-in put most of the change into its last frames, a
        // second step right as the picked agent's name came back
        .transaction(value: ring) {
          $0.animation = ring ? .easeOut(duration: Motion.durOpen) : $0[RingPick.self] ? Motion.ringFlight : Motion.gooDimOut
        }
      }
      .overlay(alignment: .bottom) {
        // The composer rests where the list's system bottom bar does
        DockSeat(homeInset: homeInset) {
          Composer(session: draft, text: $text, staged: $staged, onSend: start,
                   onSetMode: { draft.controls.modeId = $0; picks[draft.agent] = draft.controls }, onSetConfig: setConfig,
                   placeholder: t("mobile.new.placeholder"))
            .opacity(handoff?.hidden == draft.id ? 0 : 1)
            #if os(iOS)
            .background { BarHandoffProbe(session: draft, placeholder: t("mobile.new.placeholder")) }
            #endif
            .padding(.horizontal, Metrics.pad)
            .readableWidth()
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(Self.page)).minY } action: { composerTop = $0 }
        }
      }
    }
    .coordinateSpace(.named(Self.page))
    // Animated, so the bar's items leave and come back with the hero rather than at once (`barAside`)
    .onPreferenceChange(ComposerPickerKey.self) { now in
      withAnimation(motion ? now ? .easeOut(duration: Motion.durOpen) : Motion.gooDimOut : nil) { picking = now }
    }
    .onAppear { if let initialDraft, text.isEmpty { text = initialDraft } }
    .ignoresSafeArea(.container, edges: .bottom)
    .modifier(HomeInset(inset: $homeInset))
    // The agent's models as the engine knows them (its probe or its newest session), so the model chip is there
    // before the first prompt; they fill the draft once they arrive unless something was picked meanwhile. Asked
    // again on every new wire: an answer the lost one never brought left the page without a model chip for good
    .task(id: ControlsAsk(agent: draft.agent, wire: ObjectIdentifier(model))) { model.loadControls(draft.agent) }
    // The ring's other agents too, so a switch finds their models in hand rather than dropping the chip meanwhile
    .task(id: RingAsk(agents: launchable(model.agents).map(\.id), wire: ObjectIdentifier(model))) {
      model.prefetchControls(launchable(model.agents).map(\.id))
    }
    // The agent's slash commands as its newest session advertised them, so `/` opens the menu before the first prompt
    // has started the CLI (the session's own list replaces them once it runs)
    .onChange(of: model.commands(for: draft.agent), initial: true) { _, commands in draft.commands = commands }
    .onChange(of: model.agentControls[draft.agent]) { _, _ in
      model.probeUnknownControls(draft.agent)
      guard picks[draft.agent] == nil else { return }
      let known = model.controls(for: draft.agent)
      if known != draft.controls { draft.controls = known }
    }
    // The composer's liquid pickers, drawn over the whole screen so every piece takes taps
    .gooLayer()
    // Under the keyboard too, behind its rounded corners (ChatScreen)
    .background(Palette.bg0.ignoresSafeArea())
    // The title and back button leave the bar while a picker is out (`barAside`)
    .modifier(SystemTitle(title: t("session.new"), aside: picking))
    .environment(\.agentAccent, Palette.accent(for: draft.agent))
    .environment(\.hiddenModels, model.settings?.hiddenOptions[draft.agent] ?? [:])
    .environment(\.sessionActions, SessionActions(toast: { model.show($0, icon: $1) }))
    // Staged photos open in a Lightbox, as in a chat (the draft has no blobs of its own: they are inline pixels)
    .modifier(MediaLayer(session: draft.id, loader: model.media))
  }

  /// Ends typing in the composer, whichever field holds the keyboard
  private static func endEditing() {
    #if canImport(UIKit)
    UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
    #endif
  }

  /// The page's coordinate space, for the hero and the composer to meet in
  nonisolated private static let page = "new.page"

  /// What the controls are asked for: the agent, on this wire's app model (every wire brings a new one)
  private struct ControlsAsk: Equatable {
    var agent: AgentId
    var wire: ObjectIdentifier
  }

  /// The agents the ring offers, on this wire's app model
  private struct RingAsk: Equatable {
    var agents: [AgentId]
    var wire: ObjectIdentifier
  }

  /// Another agent: its own controls (models and modes differ per CLI)
  private func pickAgent(_ id: AgentId) {
    draft.agent = id
    draft.controls = picks[id] ?? model.controls(for: id)
  }

  private func setConfig(_ control: String, _ value: String) {
    guard draft.controls.options.contains(where: { $0.id == control }) else { return }
    // No session answers set_config_option here, so a model pick brings that model's own parameters from the
    // remembered shapes (TurnEdit.swift): a model without Fast loses the previous model's Fast, as on the desktop
    draft.controls = editTurnConfig(draft.controls, live: draft.controls, id: control, value: value,
                                    shapes: model.shapes(for: draft.agent))
    picks[draft.agent] = draft.controls
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
  /// The session starts as a chat, in no project: the capsule says so instead of a folder
  var chat = false
  var onAgent: (AgentId) -> Void = { _ in }
  var onProject: (String) -> Void = { _ in }
  var onAdd: (String) -> Void = { _ in }
  /// Turns the draft into a chat; nil where the engine keeps no chats
  var onChat: (() -> Void)?
  @Binding var ring: Bool
  /// The name and the folder are hidden while the ring is out and until AgentRing lets them back
  @State private var veil = false
  /// The agent picked on the ring while its mark flies into the centre (AgentRing): the name already shows it, so
  /// it can rise during the flight rather than wait for the swap
  @State private var landing: AgentId?
  @State private var adding = false
  /// The folder popover is open (regular width)
  @State private var folders = false
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.motionEnabled) private var motion
  @Environment(\.horizontalSizeClass) private var widthClass

  var body: some View {
    VStack(spacing: Metrics.s2) {
      AgentRing(agent: agent, agents: agents, open: $ring, veil: $veil, onSelect: onAgent, picking: $landing)
        .padding(.bottom, Metrics.s3)
      VStack(spacing: Metrics.s3) {
        name
          // A new agent's name is a new text, swapped at once while hidden. A `.transaction` with no animation here
          // also stopped the group's offset from reaching it, so the name jumped up as the ring opened
          .id(landing ?? agent.id)
          .transition(.identity)
        folderButton
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

  /// The folder capsule: the system menu on the phone, a popover centred under it in regular width
  @ViewBuilder private var folderButton: some View {
    if widthClass == .regular {
      Button { folders = true } label: { capsule }
        .buttonStyle(.plain)
        .popover(isPresented: $folders) {
          ProjectPopover(cwd: cwd, recent: recent, chat: chat,
                         onSelect: { folders = false; onProject($0) },
                         onAdd: { folders = false; adding = true },
                         onChat: onChat.map { chosen in { folders = false; chosen() } })
        }
        .task {
          // `-popup folder`: the popover open on launch (screenshots)
          guard initialPopup == "folder" else { return }
          try? await Task.sleep(for: .seconds(1))
          folders = true
        }
    } else {
      Menu {
        NativeProjectMenu(cwd: cwd, recent: recent, chat: chat, onSelect: onProject, onAdd: { adding = true },
                          onChat: onChat)
      } label: { capsule }
      .menuStyle(.button)
      // Keep local before remote and Add last, even when the system opens the menu upward.
      .menuOrder(.fixed)
      .buttonStyle(.plain)
    }
  }

  private var capsule: some View {
    HStack(spacing: Metrics.s1_5) {
      Icon(chat ? .messagesSquare : .folder, size: Metrics.icon)
      Text(web: chat ? t("session.group.chats") : folderName(cwd), paragraph: false).textStyle(TypeScale.text2)
    }
    .foregroundStyle(Palette.fg2)
    .padding(.horizontal, Metrics.s3)
    .padding(.vertical, Metrics.s1_5)
    .glassSurface(GlassStyle.control.interactive(), in: Capsule())
  }

  private var name: some View {
    Text(web: (agents.first { $0.id == landing } ?? agent).name, paragraph: false)
      .textStyle(TypeScale.field, weight: .semibold)
      .foregroundStyle(Palette.fgStrong)
  }
}

/// The agent's mark in a round drop of Liquid Glass with the other launchable agents on a ring around it, all one body
/// of glass. Tapping the drop buds the bubbles off it one after another, clockwise from the top; a tap outside melts
/// them back. A pick sends the picked bubble into the drop while the others fold home: its mark grows to the centre's
/// size as the old mark shrinks away under it, and the picked agent takes the centre where its mark has landed. The
/// drop stays: clearing it after a pick, even with the name rising in the same move, read as two things at once. An
/// agent that is not installed stays on the ring, dimmed.
///
/// Every bubble runs on a progress of its own (`RingBubble`), and its glass, its mark, its caption and the flown mark
/// are all placed from that progress frame by frame. Animated as plain `offset` / `scaleEffect`, the glass moved on
/// the container's own clock, well ahead of the marks: the picked mark trailed outside its drop, and the others' marks
/// were left hanging on the centre's rim after their glass had melted in
struct AgentRing: View {
  let agent: AgentInfo
  let agents: [AgentInfo]
  @Binding var open: Bool
  /// Holds the hero's name and folder back until the picked mark has landed or the bubbles are home
  @Binding var veil: Bool
  var onSelect: (AgentId) -> Void
  /// The bubble on its way into the centre after a pick, until its agent takes the centre (held by the hero, whose
  /// name shows it meanwhile)
  @Binding var picking: AgentId?
  /// Where the picked bubble sat on the ring, from the centre: its mark flies in from there
  @State private var flightFrom = CGSize.zero
  @Environment(\.motionEnabled) private var motion
  @Environment(\.initialPopup) private var initialPopup

  var body: some View {
    let others = launchable(agents).filter { $0.id != agent.id }
    let n = others.count
    let out: CGFloat = open ? 1 : 0
    ZStack {
      GlassGroup(spacing: Metrics.s6) {
        ZStack {
          ForEach(Array(others.enumerated()), id: \.element.id) { i, a in
            let spot = spot(i, of: n)
            RingBubble(t: out, agent: a, spot: spot, flying: picking == a.id, live: open && picking == nil) {
              pick(a, from: spot)
            }
            .animation(motion ? move(i, of: n) : nil, value: open)
          }
          hub
        }
      }
      // The captions ride the same progress as their bubbles, outside the glass: inside it the merging drops
      // refracted a caption they passed over
      ForEach(Array(others.enumerated()), id: \.element.id) { i, a in
        RingCaption(t: out, agent: a, spot: spot(i, of: n))
          .animation(motion ? move(i, of: n) : nil, value: open)
          .opacity(open ? 1 : 0)
          .animation(motion ? (open ? Motion.ringCaption.delay(Double(i) * Motion.ringStagger(n) + Motion.ringCaptionDelay)
                               : Motion.ringCaptionOut) : nil,
                     value: open)
      }
      // The picked agent's mark, over all the glass: as the bubble's own content it went under the drop's glass the
      // moment the two met, and the centre stood empty until the swap. Looked up among all agents: it is the centre's
      // own agent for the last part of the flight. On the flight's spring, as the picked bubble is
      RingFlown(t: out, agent: picking.flatMap { id in agents.first { $0.id == id } }, from: flightFrom)
        .animation(motion ? Motion.ringFlight : nil, value: open)
    }
    .frame(width: Metrics.heroTile, height: Metrics.heroTile)
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

  /// How bubble `i` of `n` moves: out clockwise one lag after another, home the last one first at half the lag. After
  /// a pick the others fold home together with the flight, on its spring and without the lag: on the staggered
  /// `ringSpring` the last one was still creeping into the drop well after the picked mark had landed, which read as a
  /// stall before the name came back
  private func move(_ i: Int, of n: Int) -> Animation {
    if picking != nil && !open { return Motion.ringFlight }
    let lag = Motion.ringStagger(n)
    return Motion.ringSpring.delay(open ? Double(i) * lag : Double(n - 1 - i) * lag / 2)
  }

  /// The centre: the mark in its drop of glass. During a pick the old mark shrinks away under the arriving one
  private var hub: some View {
    Button { if picking == nil { open.toggle() } } label: {
      RingHubMark(t: picking == nil ? 1 : 0, agent: agent)
        .animation(motion ? Motion.ringFlight : nil, value: picking)
        .frame(width: Metrics.heroTile, height: Metrics.heroTile)
        .contentShape(Circle())
    }
    .buttonStyle(.plain)
    .glassSurface(GlassStyle.control.interactive(), in: Circle())
    .accessibilityLabel(agent.name)
  }

  /// A pick: the bubble's glass flies into the drop while its mark flies over the glass (`RingFlown`) and grows to
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
    // Marked as a pick, so the screen's dim lifts on the flight too (RingPick)
    var fly = Transaction(animation: Motion.ringFlight)
    fly[RingPick.self] = true
    withTransaction(fly) {
      picking = a.id
      open = false
    }
    Task { @MainActor in
      var swap = Transaction()
      swap.disablesAnimations = true
      // The name (already the picked agent's) rises while the mark is still landing, so the two read as one move
      try? await Task.sleep(for: .seconds(Motion.ringHeroLead))
      veil = false
      try? await Task.sleep(for: .seconds(Motion.ringLand - Motion.ringHeroLead))
      // Under the flown mark, once its glass is inside the drop: the centre's mark is still hidden while picking. The
      // picked bubble leaves the ring here, right where its mark has landed
      withTransaction(swap) { onSelect(a.id) }
      try? await Task.sleep(for: .seconds(Motion.ringFlightTime - Motion.ringLand))
      withTransaction(swap) { picking = nil }
    }
  }
}

/// What a ring bubble shows at one point of its way out: 0 home inside the drop, 1 on the ring (a spring's overshoot
/// passes through, so the ring wobbles as one)
private enum RingGeometry {
  /// Under this a bubble is home and nothing of it is drawn
  static let gone: CGFloat = 0.001
  /// The mark shows over the last part of the way out, so it has left before its glass melts into the drop
  static let glyphFrom: CGFloat = 0.5, glyphSpan: CGFloat = 0.4
  /// Taps land once a bubble has arrived
  static let arrived: CGFloat = 0.9
  /// A missing agent's mark, faded as a disabled row's
  static let missingInk: Double = 0.35

  static func unit(_ x: CGFloat) -> CGFloat { min(1, max(0, x)) }
  /// The bubble swells out of the drop from `gooSeed` of its size
  static func side(_ t: CGFloat) -> CGFloat { max(0, Metrics.ringBubble * (Metrics.gooSeed + (1 - Metrics.gooSeed) * t)) }
  static func glyph(_ t: CGFloat) -> Double { Double(unit((t - glyphFrom) / glyphSpan)) }
  static func at(_ spot: CGSize, _ t: CGFloat) -> CGSize { CGSize(width: spot.width * t, height: spot.height * t) }
}

/// One bubble of the ring, its glass and its mark placed from its own progress. The mark keeps its size: scaling
/// glass content shifts it off the glass's centre. The picked bubble flies home at full size, its mark gone at once
/// to the flown one over the glass
private struct RingBubble: View, Animatable {
  var t: CGFloat
  let agent: AgentInfo
  let spot: CGSize
  let flying: Bool
  let live: Bool
  let onTap: () -> Void

  var animatableData: CGFloat {
    get { t }
    set { t = newValue }
  }

  var body: some View {
    if t > RingGeometry.gone || flying {
      let missing = agent.available == false
      let side = flying ? Metrics.ringBubble : RingGeometry.side(t)
      Button(action: onTap) {
        VendorMark(brand: agent.id, name: agent.name, size: Metrics.ringMark)
          .foregroundStyle(Palette.fg1)
          .opacity(flying ? 0 : RingGeometry.glyph(t) * (missing ? RingGeometry.missingInk : 1))
          // The progress already moves it frame by frame: the pick's own transaction faded the picked mark out on
          // the flight's spring, a dim second copy under the flown one
          .transaction { $0.animation = nil }
          .frame(width: side, height: side)
      }
      .buttonStyle(.plain)
      .glassSurface(GlassStyle.control.interactive(), in: Circle())
      // Comes and goes inside the drop, where the system's own morph would only lag the spring
      .glassMorph(.identity)
      .disabled(missing)
      .allowsHitTesting(live && t > RingGeometry.arrived)
      .accessibilityLabel(agent.name)
      .offset(RingGeometry.at(spot, t))
      .transition(.identity)
    }
  }
}

/// A bubble's name under it, placed from the bubble's progress (its fade runs on its own timing in `AgentRing`)
private struct RingCaption: View, Animatable {
  var t: CGFloat
  let agent: AgentInfo
  let spot: CGSize

  var animatableData: CGFloat {
    get { t }
    set { t = newValue }
  }

  var body: some View {
    Color.clear
      .frame(width: Metrics.ringBubble, height: Metrics.ringBubble)
      .overlay(alignment: .bottom) {
        Text(web: agent.name, paragraph: false)
          // The hero's name style, so a name reads the same on the ring and in the centre
          .textStyle(TypeScale.field, weight: .semibold)
          .foregroundStyle(agent.available == false ? Palette.fg3 : Palette.fgStrong)
          .fixedSize()
          .offset(y: Metrics.s6 - Metrics.s0_5)
      }
      .offset(RingGeometry.at(spot, t))
      .allowsHitTesting(false)
      .accessibilityHidden(true)
  }
}

/// The centre's mark, 1 shown and 0 shrunk away under an arriving mark. Sized rather than scaled, as glass content
private struct RingHubMark: View, Animatable {
  var t: CGFloat
  let agent: AgentInfo

  var animatableData: CGFloat {
    get { t }
    set { t = newValue }
  }

  var body: some View {
    VendorMark(brand: agent.id, name: agent.name, size: Metrics.heroBare * (Metrics.gooSeed + (1 - Metrics.gooSeed) * t))
      .foregroundStyle(Palette.fgStrong)
      .opacity(Double(t))
  }
}

/// The picked agent's mark on its way in: from its bubble's spot at the ring's size (t 1) to the centre at the
/// centre's size (t 0), in step with the picked bubble's glass
private struct RingFlown: View, Animatable {
  var t: CGFloat
  let agent: AgentInfo?
  let from: CGSize

  var animatableData: CGFloat {
    get { t }
    set { t = newValue }
  }

  var body: some View {
    if let agent {
      VendorMark(brand: agent.id, name: agent.name, size: Metrics.heroBare + (Metrics.ringMark - Metrics.heroBare) * t)
        .foregroundStyle(Palette.fgStrong)
        .offset(RingGeometry.at(from, t))
        .allowsHitTesting(false)
        // Shown whole the moment the bubble's own mark goes: inserted in the pick's transaction it faded in on the
        // flight's spring
        .transition(.identity)
    }
  }
}

/// Set on the transaction that closes the ring after a pick, as opposed to a plain close
private struct RingPick: TransactionKey {
  static let defaultValue = false
}
