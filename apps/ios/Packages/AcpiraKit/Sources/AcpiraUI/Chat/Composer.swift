import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import SwiftUI
#if canImport(PhotosUI)
import PhotosUI
#endif
#if canImport(UIKit)
import UIKit
#endif

// The phone composer. The webview's composer (a rounded-lg field over a dense toolbar row) reads as a desktop control
// on a phone, so iOS draws its own layout in the same materials (--cmp-bg, the conversation hairline, the btn-1 fill,
// the desktop's rounded-lg card and rounded-md controls), and starts from subtraction: the field, then one row with
// attach on the left and the model button and send on the right. The model button opens one sheet holding everything
// the desktop toolbar spreads over chips: the model, its effort and parameters, the mode and any other agent option.
// Picked attachments wait in a strip above the field.

struct Composer: View {
  let session: SessionView
  @Binding var text: String
  @Binding var staged: [Attachment]
  var onSend: (String, [Attachment]) -> Void = { _, _ in }
  var onStop: () -> Void = {}
  var onSetMode: (String) -> Void = { _ in }
  var onSetConfig: (String, String) -> Void = { _, _ in }
  /// The new-session screen's prompt: no run to stop, the first message starts the session
  var placeholder: String?
  @FocusState private var focused: Bool
  @Environment(\.snapshot) private var snapshot
  @Environment(\.shellStyle) private var style
  @Environment(\.motionEnabled) private var motion
  /// Design study `chrome=system`: the liquid chip that is open (attach, model), if any, the model bean whose panel
  /// is out (1…, 0 for none), and the picker `+` opened
  @State private var goo: String?
  @State private var gooStage = 0
  @State private var source: AttachSource?
  /// The width of the bar's group of `+` and the mark (the mark is gone when the agent offers nothing to pick)
  @State private var lead = Metrics.barCtl * 2
  /// The bar's height, so the beans rise over the card while typing
  @State private var tall = Metrics.barCtl
  @Environment(\.initialPopup) private var initialPopup

  private var hint: String { placeholder ?? (session.running ? t("composer.placeholder.queue") : t("composer.placeholder")) }

  private var filled: Bool { !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !staged.isEmpty }

  var body: some View {
    if style.chrome == .system {
      bar
    } else {
      switch style.composer {
      case .card: card
      case .stack: stack
      case .pill: pill
      }
    }
  }

  /// The field: a snapshot draws the placeholder (or the draft) where the field would be
  private func field(minLines: CGFloat) -> some View {
    Group {
      if snapshot {
        Text(web: text.isEmpty ? hint : text)
          .foregroundStyle(text.isEmpty ? Palette.fg3 : Palette.fgStrong)
          .frame(maxWidth: .infinity, alignment: .leading)
      } else {
        TextField(text: $text, axis: .vertical) {
          Text(web: hint).foregroundStyle(Palette.fg3)
        }
        .focused($focused)
      }
    }
    .textStyle(TypeScale.field)
    .foregroundStyle(Palette.fgStrong)
    .lineLimit(1...8)
    .frame(minHeight: TypeScale.field.lineHeight * minLines, alignment: .topLeading)
  }

  private var send: some View {
    SendButton(running: session.running, filled: filled) {
      if session.running && !filled {
        onStop()
      } else {
        onSend(text, staged)
        text = ""
        staged = []
      }
    }
  }

  /// Design study `chrome=system`. At rest one glass bar: `+` and the model's mark at its start, the field, and send at
  /// its end, where Messages keeps it (dim with nothing to send, stop while a turn runs). `+` and the mark are liquid
  /// chips. Typing (focused, or a draft waiting) the bar grows into one glass card as in Codex: the field across the
  /// whole width over a row of the chips and send, so the text is never squeezed between controls. Nothing comes or
  /// goes between the two: the chips keep the bottom-left corner and send the bottom-right one, the glass grows up
  /// around them, and the field stays the same view throughout, so focus holds. The chips and send are glass circles
  /// of their own, as tall as the bar, that its container melts into the bar: their glyphs are glass content, a press
  /// lights only the circle under the finger, and the beans melt out of a chip. The beans are glass in the same
  /// container (`gooGlass()`), over a dim the bar spreads under itself
  private var bar: some View {
    let expanded = focused || filled
    let corner = expanded ? Metrics.rComposer : Metrics.barCtl / 2
    let line = (Metrics.barCtl - TypeScale.field.lineHeight) / 2
    // In the card the beans rest over its top edge, not inside it
    let lift = expanded ? max(0, tall - Metrics.s1 - Metrics.barCtl) : 0
    return VStack(alignment: .leading, spacing: Metrics.s1_5) {
      if !staged.isEmpty { StagedAttachments(items: $staged) }
      GlassEffectContainer(spacing: Metrics.gooMerge) {
        ZStack(alignment: .bottomLeading) {
          field(minLines: 1)
            // Lowered by the ink's offset in both forms, so the text is centred in the bar and does not move as it grows
            .padding(.top, line + Metrics.fieldInkDrop)
            .padding(.bottom, expanded ? Metrics.s1 + Metrics.barCtl + Metrics.s1 : line - Metrics.fieldInkDrop)
            .padding(.leading, expanded ? Metrics.s3 + Metrics.s1 : lead + Metrics.s1)
            .padding(.trailing, expanded ? Metrics.s3 + Metrics.s1 : Metrics.barCtl + Metrics.s1)
            .glassEffect(GlassStyle.control, in: RoundedRectangle(cornerRadius: corner, style: .continuous))
            .contentShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
            .onTapGesture { focused = true }
          // The chips' row at the start; the room after them takes no taps, so they reach the field under it
          HStack(spacing: 0) {
            Group {
              if style.chips == .menu {
                // Design study `chips=menu`: the same circles opening the system's menus
                HStack(spacing: 0) {
                  MenuAttach { source = $0 }
                  MenuControls(controls: session.controls, onSetMode: onSetMode, onSetConfig: onSetConfig)
                }
              } else {
                HStack(spacing: 0) {
                  LiquidAttach(open: $goo, lift: lift) { source = $0 }
                  LiquidControls(controls: session.controls, open: $goo, stage: $gooStage, lift: lift, onSetMode: onSetMode,
                                 onSetConfig: onSetConfig)
                }
                .gooGlass()
              }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { lead = $0 }
            Spacer(minLength: 0)
          }
          .padding([.horizontal, .bottom], expanded ? Metrics.s1 : 0)
        }
      }
      // Send sits over the container, not in it: the container draws its glass over every plain view inside, so a
      // solid button there came out refracted into a soft blob
      .overlay(alignment: .bottomTrailing) {
        send.padding([.trailing, .bottom], expanded ? Metrics.s1 : 0)
      }
      .animation(motion ? Motion.gooSpring : nil, value: expanded)
    }
    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { tall = $0 }
    // The dim under an open picker, as Messages dims its page under `+`: under the bar's glass, over the transcript.
    // Taps on it go to the screen's `gooLayer()`
    .background {
      Rectangle()
        .fill(Palette.gooDim)
        .frame(width: Metrics.gooScrim, height: Metrics.gooScrim)
        .opacity(goo == nil ? 0 : 1)
        .allowsHitTesting(false)
        // It lifts only as the pieces get home, so the last drop never crosses the bare transcript
        .animation(motion ? goo == nil ? Motion.gooDimOut : Motion.gooSpring : nil, value: goo)
    }
    .onAppear(perform: startPopup)
    .modifier(AttachPickers(staged: $staged, source: $source))
    .environment(\.composerInk, true)
  }

  /// `-popup attach`, `-popup model` or `-popup model.2` (a bean's panel out) for screenshots; a `focus.` prefix
  /// (`-popup focus`, `-popup focus.model`) raises the keyboard first. With motion on both wait a moment after launch,
  /// so a recording shows them change. `-popup demo` plays every liquid transition in turn, for a recording read
  /// frame by frame
  private func startPopup() {
    guard var popup = initialPopup else { return }
    if popup == "demo" {
      Task { await demo() }
      return
    }
    let focus = popup.hasPrefix("focus")
    if focus { popup = String(popup.dropFirst("focus".count).drop { $0 == "." }) }
    let chip = popup == "attach" ? popup : popup.hasPrefix("model") ? "model" : nil
    let stage = popup.hasPrefix("model.") ? Int(popup.dropFirst("model.".count)) ?? 0 : 0
    Task {
      if motion { try? await Task.sleep(for: .seconds(1)) }
      if focus { focused = true }
      if motion, focus, chip != nil { try? await Task.sleep(for: .seconds(1)) }
      withAnimation(motion ? Motion.gooSpring : nil) {
        goo = chip
        gooStage = stage
      }
    }
  }

  /// The demo's steps, each after a pause: the model's beans, two panels in and out, one panel back and another bean
  /// tapped as soon as the beans take taps (the first panel's spring still settling), closing, `+`, then the same from
  /// the card, closing from an open panel
  private func demo() async {
    let steps: [(Double, () -> Void)] = [
      (1.0, { goo = "model" }), (1.2, { gooStage = 2 }), (1.4, { gooStage = 0 }), (1.2, { gooStage = 1 }),
      (1.4, { gooStage = 0 }), (1.2, { gooStage = 3 }), (1.4, { gooStage = 0 }), (0.5, { gooStage = 2 }),
      (1.4, { goo = nil; gooStage = 0 }), (1.4, { goo = "attach" }), (1.2, { goo = nil }), (1.2, { focused = true }),
      (1.4, { goo = "model" }), (1.2, { gooStage = 3 }), (1.4, { goo = nil; gooStage = 0 }), (1.4, { focused = false }),
    ]
    for (pause, step) in steps {
      try? await Task.sleep(for: .seconds(pause))
      withAnimation(motion ? Motion.gooSpring : nil) { step() }
    }
  }

  /// Design study `stack`: the field (one line at rest) over a row of attach, a short model chip and send, on glass
  private var stack: some View {
    let shape = RoundedRectangle(cornerRadius: style.shape == .round ? Metrics.rComposer : Metrics.rLg)
    return VStack(alignment: .leading, spacing: Metrics.s1) {
      if !staged.isEmpty { StagedAttachments(items: $staged) }
      field(minLines: 1)
        .padding(.horizontal, Metrics.s2)
        .padding(.top, Metrics.s2)
      HStack(spacing: Metrics.s1_5) {
        AttachButton(staged: $staged)
        ModelButton(controls: session.controls, look: .name, onSetMode: onSetMode, onSetConfig: onSetConfig)
        Spacer(minLength: 0)
        send
      }
    }
    .padding(Metrics.s2)
    .environment(\.composerInk, true)
    .glassEffect(GlassStyle.control, in: shape)
    .contentShape(shape)
    .onTapGesture { focused = true }
  }

  /// Design study `pill`: one line of attach, the field, the model's mark and send; it grows as the text wraps
  private var pill: some View {
    let shape = RoundedRectangle(cornerRadius: style.shape == .round ? Metrics.rComposer : Metrics.rLg)
    return VStack(alignment: .leading, spacing: Metrics.s1) {
      if !staged.isEmpty { StagedAttachments(items: $staged) }
      HStack(alignment: .bottom, spacing: Metrics.s1) {
        AttachButton(staged: $staged)
        field(minLines: 1)
          .padding(.vertical, (Metrics.mCtl - TypeScale.field.lineHeight) / 2)
          .padding(.horizontal, Metrics.s1)
        ModelButton(controls: session.controls, look: .mark, onSetMode: onSetMode, onSetConfig: onSetConfig)
        send
      }
    }
    .padding(Metrics.s1_5)
    .environment(\.composerInk, true)
    .glassEffect(GlassStyle.control, in: shape)
    .contentShape(shape)
    .onTapGesture { focused = true }
  }

  private var card: some View {
    let running = session.running
    let filled = self.filled
    return VStack(alignment: .leading, spacing: Metrics.s2) {
      if !staged.isEmpty {
        StagedAttachments(items: $staged)
      }
      Group {
        if snapshot {
          // A snapshot draws the placeholder (or the draft) where the field would be
          Text(web: text.isEmpty ? hint : text)
            .foregroundStyle(text.isEmpty ? Palette.fg3 : Palette.fgStrong)
            .frame(maxWidth: .infinity, alignment: .leading)
        } else {
          TextField(text: $text, axis: .vertical) {
            Text(web: hint).foregroundStyle(Palette.fg3)
          }
          .focused($focused)
        }
      }
      .textStyle(TypeScale.field)
      .foregroundStyle(Palette.fgStrong)
      .lineLimit(1...8)
      .frame(minHeight: TypeScale.field.lineHeight * Metrics.mFieldLines, alignment: .topLeading)
      .padding(.horizontal, Metrics.s1)
      .padding(.top, Metrics.s1)

      HStack(spacing: Metrics.s1) {
        AttachButton(staged: $staged)
        Spacer(minLength: 0)
        ModelButton(controls: session.controls, onSetMode: onSetMode, onSetConfig: onSetConfig)
        SendButton(running: running, filled: filled) {
          if running && !filled {
            onStop()
          } else {
            onSend(text, staged)
            text = ""
            staged = []
          }
        }
      }
    }
    .padding(Metrics.s3)
    .environment(\.composerInk, true)
    .background(RoundedRectangle(cornerRadius: Metrics.rLg).fill(Palette.cmpBg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg)
      .strokeBorder(focused ? Palette.conversationLineFocus : Palette.conversationLine))
    .contentShape(RoundedRectangle(cornerRadius: Metrics.rLg))
    .onTapGesture { focused = true }
  }
}

/// The square plain control on the card (attach): no fill at rest, the chip fill while pressed or open
struct ComposerButtonStyle: ButtonStyle {
  var open = false
  @Environment(\.shellStyle) private var style
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .foregroundStyle(configuration.isPressed || open ? Palette.fg1 : Palette.cmpFg2)
      .frame(width: Metrics.mCtl, height: Metrics.mCtl)
      .background(style.controlShape().fill(configuration.isPressed || open ? Palette.chipHover : .clear))
      .contentShape(style.controlShape())
  }
}

/// The model button: quiet text as tall as the controls, the chip fill while pressed or open
struct ModelButtonStyle: ButtonStyle {
  var open = false
  /// Design study: the `name` chip sits on the chip fill at rest, so it reads as a control beside the bare `+`
  var filled = false
  @Environment(\.shellStyle) private var style
  func makeBody(configuration: Configuration) -> some View {
    let lit = configuration.isPressed || open
    HStack(spacing: Metrics.s1_5) { configuration.label }
      .textStyle(TypeScale.text2, weight: .medium)
      .foregroundStyle(Palette.fg1)
      .lineLimit(1)
      .padding(.horizontal, filled ? Metrics.s3 : Metrics.s2)
      .frame(minWidth: Metrics.mCtl, minHeight: Metrics.mCtl, maxHeight: Metrics.mCtl)
      .background(style.controlShape().fill(lit ? Palette.chipHover : filled ? Palette.chip : .clear))
      .contentShape(style.controlShape())
  }
}

// MARK: - Attach

/// `+`: where an attachment comes from, as the app's popup menu (the liquid `+` of system chrome is `LiquidAttach`)
struct AttachButton: View {
  @Binding var staged: [Attachment]
  @State private var open = false
  @State private var source: AttachSource?
  @Environment(\.initialPopup) private var initialPopup

  var body: some View {
    Button { open.toggle() } label: {
      Icon(.plus, size: Metrics.iconCtl + Metrics.s1, strokeWidth: 1.75)
    }
    .buttonStyle(ComposerButtonStyle(open: open))
    .popup(isPresented: $open, edge: .leading, width: Metrics.popSm) {
      AttachMenu(onPhotos: { source = .photos }, onCamera: { source = .camera }, onFiles: { source = .files })
    }
    .onAppear { if initialPopup == "attach" { open = true } }
    .accessibilityLabel(Text(web: t("mobile.attach")))
    .modifier(AttachPickers(staged: $staged, source: $source))
  }
}

/// The system pickers behind `+`, shown for `source`: photos and files go through the system pickers and wait in the
/// strip above the field; the camera opens where the device has one, else a toast says there is none. Hosted by a
/// view that stays put (the composer, not its `+`, which moves between the bar and the card)
struct AttachPickers: ViewModifier {
  @Binding var staged: [Attachment]
  @Binding var source: AttachSource?
  #if canImport(PhotosUI)
  @State private var picked: [PhotosPickerItem] = []
  #endif
  @Environment(\.sessionActions) private var actions

  private static var cameraAvailable: Bool {
    #if canImport(UIKit)
    UIImagePickerController.isSourceTypeAvailable(.camera)
    #else
    false
    #endif
  }

  private func shown(_ s: AttachSource) -> Binding<Bool> {
    Binding(get: { source == s && (s != .camera || Self.cameraAvailable) }, set: { if !$0, source == s { source = nil } })
  }

  func body(content: Content) -> some View {
    content
      #if canImport(PhotosUI)
      .photosPicker(isPresented: shown(.photos), selection: $picked, maxSelectionCount: 4, matching: .images)
      .onChange(of: picked) { _, items in
        guard !items.isEmpty else { return }
        picked = []
        Task {
          for item in items {
            if let data = try? await item.loadTransferable(type: Data.self), let image = AttachmentImage.thumbnail(data) {
              staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil))
            }
          }
        }
      }
      #endif
      .fileImporter(isPresented: shown(.files), allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
        guard case .success(let urls) = result else { return }
        staged += urls.map { .file(uri: $0.absoluteString, name: $0.lastPathComponent) }
      }
      #if canImport(UIKit)
      .fullScreenCover(isPresented: shown(.camera)) {
        CameraPicker { data in
          if let image = AttachmentImage.thumbnail(data) { staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil)) }
        }
        .ignoresSafeArea()
      }
      #endif
      .onChange(of: source) { _, now in
        guard now == .camera, !Self.cameraAvailable else { return }
        source = nil
        actions.toast(t("mobile.camera.unavailable"), .camera)
      }
  }
}

struct AttachMenu: View {
  var onPhotos: () -> Void = {}
  var onCamera: () -> Void = {}
  var onFiles: () -> Void = {}
  @Environment(\.dismissPopup) private var dismiss
  var body: some View {
    PopupList {
      MenuOption(t("mobile.attach.photos"), checked: false, checkSlot: false) { dismiss(); onPhotos() } mark: { Icon(.image) }
      MenuOption(t("mobile.attach.camera"), checked: false, checkSlot: false) { dismiss(); onCamera() } mark: { Icon(.camera) }
      MenuOption(t("mobile.attach.files"), checked: false, checkSlot: false) { dismiss(); onFiles() } mark: { Icon(.paperclip) }
    }
  }
}

/// Where an attachment comes from
enum AttachSource: Equatable { case photos, camera, files }

/// The attachments picked for the next prompt: image tiles and file pills, each with its ✕
struct StagedAttachments: View {
  @Binding var items: [Attachment]
  var body: some View {
    ScrollView(.horizontal) {
      HStack(spacing: Metrics.s2) {
        ForEach(Array(items.enumerated()), id: \.offset) { i, item in
          Group {
            switch item {
            case .image(let blob, _, _):
              AttachmentThumb(blob: blob)
            case .file(_, let name), .text(_, let name):
              AttachmentPill(name: name, icon: .fileText)
            default:
              EmptyView()
            }
          }
          .overlay(alignment: .topTrailing) {
            Button { items.remove(at: i) } label: {
              Icon(.x, size: Metrics.chevron, strokeWidth: 2)
                .foregroundStyle(Palette.btn1Fg)
                .frame(width: Metrics.lead, height: Metrics.lead)
                .background(Circle().fill(Palette.btn1Bg))
            }
            .buttonStyle(.plain)
            .offset(x: Metrics.s1_5, y: -Metrics.s1_5)
            .accessibilityLabel(Text(web: t("common.remove")))
          }
        }
      }
      .padding(.top, Metrics.s1_5)
      .padding(.trailing, Metrics.s1_5)
      .padding(.horizontal, Metrics.s1)
    }
    .scrollIndicators(.hidden)
  }
}

/// An image attachment's tile: its pixels when the blob is inline data, the image glyph otherwise
struct AttachmentThumb: View {
  var blob: String?
  var body: some View {
    RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.chip)
      .frame(width: Metrics.thumb, height: Metrics.thumb)
      .overlay {
        #if canImport(UIKit)
        if let image = blob.flatMap(AttachmentImage.decode) {
          Image(uiImage: image).resizable().scaledToFill()
        } else {
          Icon(.image, size: Metrics.icon).foregroundStyle(Palette.fg3)
        }
        #else
        Icon(.image, size: Metrics.icon).foregroundStyle(Palette.fg3)
        #endif
      }
      .clipShape(RoundedRectangle(cornerRadius: Metrics.rSm))
  }
}

/// Picked photos as inline JPEG, scaled down: the demo keeps them in memory, the engine will take blob names
enum AttachmentImage {
  /// The longest side a staged photo keeps
  static let side: CGFloat = 1024

  static func thumbnail(_ data: Data) -> String? {
    #if canImport(UIKit)
    guard let image = UIImage(data: data) else { return nil }
    let scale = min(1, side / max(image.size.width, image.size.height))
    let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
    let scaled = UIGraphicsImageRenderer(size: size).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
    return scaled.jpegData(compressionQuality: 0.8)?.base64EncodedString()
    #else
    return nil
    #endif
  }

  #if canImport(UIKit)
  static func decode(_ blob: String) -> UIImage? { Data(base64Encoded: blob).flatMap(UIImage.init(data:)) }
  #endif
}

#if canImport(UIKit)
/// The system camera, handing back the photo's JPEG
struct CameraPicker: UIViewControllerRepresentable {
  var onPick: (Data) -> Void
  @Environment(\.dismiss) private var dismiss

  func makeUIViewController(context: Context) -> UIImagePickerController {
    let picker = UIImagePickerController()
    picker.sourceType = .camera
    picker.delegate = context.coordinator
    return picker
  }
  func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}
  func makeCoordinator() -> Coordinator { Coordinator(self) }

  final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
    let parent: CameraPicker
    init(_ parent: CameraPicker) { self.parent = parent }
    func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
      if let image = info[.originalImage] as? UIImage, let data = image.jpegData(compressionQuality: 0.9) { parent.onPick(data) }
      parent.dismiss()
    }
    func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { parent.dismiss() }
  }
}
#endif

// MARK: - Project

/// The folder a new session works in, by its last path component; the sheet lists the recent projects and adds one
struct ProjectChip: View {
  let cwd: String
  var recent: [String]
  var onSelect: (String) -> Void
  var onAdd: (String) -> Void
  @State private var open = false
  @Environment(\.initialPopup) private var initialPopup

  @State private var adding = false
  @Environment(\.shellStyle) private var style

  var body: some View {
    if style.chrome == .system {
      Menu {
        NativeProjectMenu(cwd: cwd, recent: recent, onSelect: onSelect, onAdd: { adding = true })
      } label: {
        ChipLabel(folderName(cwd)) { Icon(.folder, size: Metrics.icon) }
      }
      .menuStyle(.button)
      .buttonStyle(ChipStyle())
      .modifier(AddProjectAlert(isPresented: $adding, onAdd: onAdd))
    } else {
      Button { open.toggle() } label: {
        ChipLabel(folderName(cwd)) {
          Icon(.folder, size: Metrics.icon)
        }
      }
      .buttonStyle(ChipStyle(open: open))
      .pickerSheet(isPresented: $open, title: t("mobile.projects"), current: cwd) {
        ProjectList(cwd: cwd, recent: recent, onSelect: onSelect, onAdd: onAdd)
      }
      .onAppear { if initialPopup == "project" { open = true } }
    }
  }
}

/// The recent projects, current first and checked, then "Add folder" (a path on the engine's computer)
struct ProjectList: View {
  let cwd: String
  let recent: [String]
  var onSelect: (String) -> Void = { _ in }
  var onAdd: (String) -> Void = { _ in }
  @State private var adding = false
  @State private var path = ""
  @Environment(\.dismissPopup) private var dismiss

  var body: some View {
    var seen = Set<String>()
    let folders = ([cwd] + recent).filter { !$0.isEmpty && seen.insert($0).inserted }
    VStack(alignment: .leading, spacing: 0) {
      Text(web: t("mobile.projects.recent"), paragraph: false)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.popFg3)
        .padding(.horizontal, Metrics.s3)
        .padding(.vertical, Metrics.s1)
      PopupList(current: cwd) {
        ForEach(folders, id: \.self) { folder in
          MenuOption(folderName(folder), description: folder, checked: folder == cwd, checkSlot: true) {
            onSelect(folder)
            dismiss()
          } mark: {
            Icon(folder == cwd ? .folderOpen : .folder)
          }
          .id(folder)
        }
        MenuOption(t("mobile.projects.add"), checked: false, checkSlot: false) { adding = true } mark: { Icon(.plus) }
      }
    }
    .alert(t("mobile.projects.add"), isPresented: $adding) {
      TextField(text: $path) { Text(web: t("mobile.projects.path")) }
        .autocorrectionDisabled()
        #if os(iOS)
        .textInputAutocapitalization(.never)
        #endif
      Button(t("mobile.cancel"), role: .cancel) { path = "" }
      Button(t("mobile.add")) {
        let folder = path.trimmingCharacters(in: .whitespacesAndNewlines)
        path = ""
        guard !folder.isEmpty else { return }
        onAdd(folder)
        dismiss()
      }
    } message: {
      Text(web: t("mobile.projects.pathHint"))
    }
  }
}

/// The last component of a path, `~` for a bare home directory
func folderName(_ path: String) -> String {
  let parts = path.split(separator: "/")
  return parts.last.map(String.init) ?? path
}

// MARK: - Mode

/// modeIcons.ts: the glyph is inferred from keywords in id + name; first match wins
func modeIcon(_ mode: SessionOption) -> LucideIcon {
  let key = "\(mode.id) \(mode.name)".lowercased()
  let rules: [(String, LucideIcon)] = [
    (#"bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe"#, .shieldOff),
    (#"debug"#, .bug),
    (#"plan"#, .listTodo),
    (#"\bask\b|chat|question"#, .messageCircleQuestionMark),
    (#"smart|\bauto\b"#, .sparkles),
    (#"code|edit|build"#, .code),
    (#"agent|default|normal"#, .infinity),
  ]
  return rules.first { key.range(of: $0.0, options: .regularExpression) != nil }?.1 ?? .circleDashed
}

/// modeIcons.ts `modeLabel`: the short name for a mode the built-in integrations are known to send, matched on the
/// exact id (never the display name, so a user's own agent keeps its name), in the app's language and one length in
/// CJK; anything unknown shows the agent's own name
func modeLabel(_ mode: SessionOption) -> String {
  let short: [String: String] = [
    "default": "composer.modeShort.default", "agent": "composer.modeShort.default", "build": "composer.modeShort.build",
    "plan": "composer.modeShort.plan", "raw": "composer.modeShort.raw", "acceptEdits": "composer.modeShort.autoEdit",
    "auto": "composer.modeShort.auto", "smart": "composer.modeShort.smart", "bypassPermissions": "composer.modeShort.full",
    "bypass": "composer.modeShort.full", "yolo": "composer.modeShort.full", "agent-full-access": "composer.modeShort.full",
    "read-only": "composer.modeShort.readOnly", "workspace-write": "composer.modeShort.workspace",
    "ask": "composer.modeShort.ask", "code": "composer.modeShort.code",
  ]
  guard let key = short[mode.id] ?? (mode.kind == "full_access" ? "composer.modeShort.full" : nil) else { return mode.name }
  return t(key)
}

/// The mode menu's rows: glyph and name, the description left to the accessibility hint
struct ModeMenu: View {
  let modes: [SessionOption]
  var value: String?
  var onSelect: (String) -> Void
  @Environment(\.dismissPopup) private var dismiss

  var body: some View {
    let checkSlot = modes.contains { $0.id == value }
    PopupList(current: value) {
      ForEach(modes) { m in
        MenuOption(modeLabel(m), checked: m.id == value, checkSlot: checkSlot) {
          onSelect(m.id)
          dismiss()
        } mark: {
          Icon(modeIcon(m), size: Metrics.icon, strokeWidth: 1.75)
            .foregroundStyle(m.kind == "full_access" ? Palette.warn : Palette.fg1)
        }
        .accessibilityHint(Text(web: m.description ?? ""))
        .id(m.id)
      }
    }
  }
}

// MARK: - Model

/// The model button and its sheet. The button names the model family (its mark, name and badges) with the mode's
/// glyph after it once the mode is not the agent's first; the sheet holds every control the desktop toolbar shows as
/// chips: the model list with its effort form (ModelPicker.tsx ModelPanel), then the modes, then other options
/// How much of the model the composer's button shows: everything (`full`), the family name only (`name`), or the
/// vendor's mark only (`mark`); the sheet behind it is the same
enum ModelLook { case full, name, mark }

struct ModelButton: View {
  let controls: SessionControls
  var look = ModelLook.full
  var onSetMode: (String) -> Void
  var onSetConfig: (String, String) -> Void
  @State private var open = false
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.shellStyle) private var style

  var body: some View {
    let split = ComposerControls.split(controls.options)
    let model = split.models.first
    let families = model.map { ModelCatalog.group($0.options) } ?? []
    let cur = families.first { f in f.variants.contains { $0.id == model?.value } }
    let curVar = cur?.variants.first { $0.id == model?.value }
    let tags = ComposerControls.chipTags(cur, curVar, reasoning: split.reasoning, modelConfig: split.modelConfig, standard: t("composer.standard"))
    let mode = controls.modes.first { $0.id == controls.modeId }
    let name = cur?.name ?? model.flatMap { m in m.options.first { $0.id == m.value }?.name } ?? model?.name ?? mode.map(modeLabel)
    if let name {
      let label = Group {
        if let model { VendorMark(brand: cur?.brand, name: name, size: Metrics.iconCtl).accessibilityHidden(true) }
        if look == .mark && model != nil {
          EmptyView()
        } else if look == .name {
          Text(web: name).fixedSize()
          // Design study: under the system menus the name reads as plain text with a caret, as in Claude and Cursor
          if style.chrome == .system { Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.fg3) }
        } else {
          // The family name is never cut (ModelPicker: the chip keeps its name whole); the row gives way elsewhere
          Text(web: name).fixedSize()
          if !tags.isEmpty {
            HStack(spacing: Metrics.s0_5) {
              ForEach(tags, id: \.label) { ChipTag($0.label, ultra: $0.ultra, fast: $0.fast) }
            }
            .fixedSize()
          }
          // A mode the agent does not start in shows beside the model (plan, full access …)
          if let mode, model != nil, mode.id != controls.modes.first?.id {
            Icon(modeIcon(mode), size: Metrics.icon, strokeWidth: 1.75)
              .foregroundStyle(mode.kind == "full_access" ? Palette.warn : Palette.fg2)
          }
          Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
        }
      }
      let a11y = Text(web: [name, mode.map(modeLabel) ?? ""].filter { !$0.isEmpty }.joined(separator: t("common.metaSep")))
      if style.chrome == .system {
        // Design study: the model panel in a system popover over the button. Effort, Fast and the mode change in
        // place and keep it open; picking a model closes it
        Button { open.toggle() } label: { label }
          .buttonStyle(ModelButtonStyle(open: open))
          .accessibilityLabel(a11y)
          .popover(isPresented: $open, arrowEdge: .bottom) {
            ControlsSheet(controls: controls, split: split, families: families, cur: cur, curVar: curVar,
                          onSetMode: onSetMode, onSetConfig: onSetConfig)
              .textStyle(TypeScale.text2)
              .foregroundStyle(Palette.fg1)
              .padding(Metrics.s2)
              .frame(width: Metrics.popLg)
              .fixedSize(horizontal: false, vertical: true)
              .environment(\.dismissPopup) { open = false }
              .presentationCompactAdaptation(.popover)
          }
          .onAppear { if initialPopup == "model" { open = true } }
      } else {
        Button { open.toggle() } label: { label }
          .buttonStyle(ModelButtonStyle(open: open, filled: look == .name))
          .accessibilityLabel(a11y)
          // A family list with its effort form is a long read on a phone: a bottom sheet, not a popup over the card
          .pickerSheet(isPresented: $open, title: t("stats.model"), current: cur?.key) {
            ControlsSheet(controls: controls, split: split, families: families, cur: cur, curVar: curVar,
                          onSetMode: onSetMode, onSetConfig: onSetConfig)
          }
          .onAppear { if initialPopup == "model" { open = true } }
      }
    }
  }
}

/// The model sheet's body: the model panel, then the modes and the remaining options under their own captions
struct ControlsSheet: View {
  let controls: SessionControls
  let split: ComposerControls.Split
  let families: [ModelFamily]
  var cur: ModelFamily?
  var curVar: ModelVariant?
  var onSetMode: (String) -> Void
  var onSetConfig: (String, String) -> Void

  var body: some View {
    let others = split.collaboration + split.other + (split.models.isEmpty ? split.modelConfig : [])
    VStack(alignment: .leading, spacing: Metrics.s3) {
      if let model = split.models.first {
        ModelPanel(families: families, cur: cur, curVar: curVar, reasoning: split.reasoning, modelConfig: split.modelConfig,
                   onSelect: { onSetConfig(model.id, $0) }, onSetConfig: onSetConfig)
      } else {
        ForEach(split.reasoning) { c in ReasoningParams(control: c) { onSetConfig(c.id, $0) } }
      }
      if !controls.modes.isEmpty {
        VStack(alignment: .leading, spacing: 0) {
          SheetCaption(t("composer.mode"))
          // A mode change keeps the panel open, like effort and Fast: only a model pick closes it
          ModeMenu(modes: controls.modes, value: controls.modeId, onSelect: onSetMode)
            .environment(\.dismissPopup) {}
        }
      }
      if !others.isEmpty {
        VStack(alignment: .leading, spacing: 0) {
          ForEach(others) { c in
            SelectRow(label: c.name, options: c.options.map { .init(value: $0.id, label: $0.name) }, value: c.value ?? "") {
              onSetConfig(c.id, $0)
            }
          }
        }
      }
    }
  }
}

/// A group's caption inside a sheet: text-3 in the popup's faint ink
struct SheetCaption: View {
  let text: String
  init(_ text: String) { self.text = text }
  var body: some View {
    Text(web: text, paragraph: false)
      .textStyle(TypeScale.text3)
      .foregroundStyle(Palette.popFg3)
      .padding(.horizontal, Metrics.s3)
      .padding(.vertical, Metrics.s1)
  }
}

// MARK: - Context ring

/// The context window ring: a 14 pt arc on a faint track, inside an icon-button slot
struct ContextRing: View {
  let usage: Usage
  @State private var open = false

  var body: some View {
    let pct = usage.size > 0 ? min(1, Double(usage.used) / Double(usage.size)) : 0
    let stroke = Metrics.icon * 2 / 16
    Button { open.toggle() } label: {
      ZStack {
        Circle().stroke(lineWidth: stroke).opacity(0.25)
        Circle().trim(from: 0, to: pct).stroke(style: StrokeStyle(lineWidth: stroke, lineCap: .round))
          .rotationEffect(.degrees(-90))
      }
      .padding(Metrics.icon / 16 * 1)
      .frame(width: Metrics.icon, height: Metrics.icon)
    }
    .buttonStyle(IconButtonStyle(side: Metrics.ctlSm, open: open))
    .accessibilityLabel(Text(web: t("usage.usedPct", ["pct": Int((pct * 100).rounded())])))
    .popover(isPresented: $open) {
      VStack(alignment: .leading, spacing: Metrics.s1) {
        Text(web: "\(usage.used.formatted()) / \(usage.size.formatted())").textStyle(TypeScale.text3)
        Text(web: t("usage.usedPct", ["pct": Int((pct * 100).rounded())])).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3)
      }
      .padding(Metrics.s3)
      .presentationCompactAdaptation(.popover)
    }
  }
}

// MARK: - Send

/// Send / stop in one: a square control, dim when empty, inverted (btn-1) when there is text or a run in progress, and
/// then ringed in metal-fx's silver (`MetalRing`, the webview's default `metal` send look); the ring holds still when
/// motion is off, as the webview's `paused`
struct SendButton: View {
  @Environment(\.motionEnabled) private var motion
  @Environment(\.shellStyle) private var style
  var running: Bool
  var filled: Bool
  var action: () -> Void

  var body: some View {
    if style.chrome == .system { glass } else { metal }
  }

  /// Design study `chrome=system`: a solid circle set into the bar's end with a ring of the bar around it, as the
  /// system keyboard's and Messages' send. Lit (the btn-1 fill, crisp) once there is something to send or a run to
  /// stop, a quiet chip fill otherwise. Not glass: a white tint on Liquid Glass blooms into a soft glowing blob
  /// (tried 2026-10-27 and rejected), and the bar's glass already frames it. The 48 pt slot keeps the bar's layout
  private var glass: some View {
    let on = running || filled
    let side = Metrics.barCtl - Metrics.s1 * 2
    return Button(action: action) {
      Group {
        if running && !filled {
          RoundedRectangle(cornerRadius: Metrics.s0_5).frame(width: Metrics.barIcon / 2, height: Metrics.barIcon / 2)
        } else {
          Icon(.arrowUp, size: Metrics.iconCtl, strokeWidth: 2)
        }
      }
      .foregroundStyle(on ? Palette.btn1Fg : Palette.sendInk)
      .frame(width: side, height: side)
      .background(Circle().fill(on ? Palette.btn1Bg : Palette.chip))
      .animation(motion ? Motion.ease : nil, value: on)
      .frame(width: Metrics.barCtl, height: Metrics.barCtl)
      .contentShape(Circle())
    }
    .buttonStyle(PressButtonStyle())
    .disabled(!on)
    .accessibilityLabel(Text(web: running ? t("composer.stop") : t("composer.send")))
  }

  private var metal: some View {
    let on = running || filled
    return Button(action: action) {
      Group {
        if running && !filled {
          RoundedRectangle(cornerRadius: 1.5).frame(width: Metrics.chevron * 0.75, height: Metrics.chevron * 0.75)
        } else {
          Icon(.arrowUp, size: Metrics.iconCtl, strokeWidth: 2)
        }
      }
      .foregroundStyle(on ? Palette.btn1Fg : Palette.sendInk)
      .frame(width: Metrics.mCtl, height: Metrics.mCtl)
      .background(style.controlShape().fill(on ? Palette.btn1Bg : Palette.chip))
      .overlay {
        // Under the round shape language the ring's corners reach half the side: a circle
        if on { MetalRing(radius: style.shape == .round ? Metrics.mCtl / 2 : Metrics.rMd, paused: !motion).transition(.opacity) }
      }
      // `send-btn`: background and ink ease over --dur
      .animation(motion ? Motion.ease : nil, value: on)
      .contentShape(style.controlShape())
    }
    // The resting look is --send-ink, not the system's dimmed disabled label
    .buttonStyle(PressButtonStyle())
    .disabled(!on)
    .accessibilityLabel(Text(web: running ? t("composer.stop") : t("composer.send")))
  }
}

/// A button that draws its label as is, enabled or not
struct BareButtonStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View { configuration.label }
}

/// A bare button that gives under the finger: the webview's hover `brightness(1.08)` as a press, with a slight sink
struct PressButtonStyle: ButtonStyle {
  @Environment(\.motionEnabled) private var motion
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .brightness(configuration.isPressed ? Motion.pressBrightness : 0)
      .scaleEffect(configuration.isPressed && motion ? Motion.pressScale : 1)
      .animation(motion ? Motion.press : nil, value: configuration.isPressed)
  }
}
