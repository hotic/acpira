import AcpiraDesign
import SwiftUI

// Design study: the style switcher floating over every screen, so a pick shows on the screen being judged instead of
// one navigation step away in Settings. A glass knob on the right edge opens the panel; holding the knob (or the
// panel's eye) shows the look before the study for as long as the finger stays down. The panel docks at the bottom
// or the top, so both the header and the composer can be watched while switching. Hidden under `-lab` (screenshots).

struct LabOverlay: View {
  /// True while a finger holds a compare control: RootView shows the look before the study meanwhile
  @Binding var comparing: Bool
  @AppStorage(Prefs.shellStyle) private var labCode = ""
  @AppStorage(Prefs.labDock) private var dockTop = false
  @State private var open = false
  @Environment(\.motionEnabled) private var motion
  @Environment(\.initialPopup) private var initialPopup

  var body: some View {
    ZStack(alignment: dockTop ? .top : .bottom) {
      Color.clear.allowsHitTesting(false)
      if open {
        panel
          .transition(.move(edge: dockTop ? .top : .bottom).combined(with: .opacity))
      }
    }
    .overlay(alignment: .trailing) {
      if !open {
        knob.transition(.opacity)
      }
    }
    .animation(motion ? Motion.ease : nil, value: open)
    .animation(motion ? Motion.ease : nil, value: dockTop)
    // `-popup lab`: open on launch (screenshots of the panel itself)
    .onAppear { if initialPopup == "lab" { open = true } }
  }

  /// Tap: open the panel; hold: compare with the look before the study
  private var knob: some View {
    Icon(.slidersHorizontal, size: Metrics.glassIcon)
      .foregroundStyle(Palette.fg1)
      .frame(width: Metrics.glassCtl, height: Metrics.glassCtl)
      .glassEffect(.regular.interactive(), in: Circle())
      .contentShape(Circle())
      .onTapGesture { open = true }
      .onLongPressGesture(minimumDuration: Motion.labHold, perform: {}, onPressingChanged: { held in withAnimation(motion ? Motion.ease : nil) { comparing = held } })
      .padding(.trailing, Metrics.s2)
      .accessibilityLabel(Text(verbatim: "设计选型"))
  }

  private var panel: some View {
    let style = ShellStyle(code: labCode)
    let preset = Binding<String>(get: { style.preset ?? "" }, set: { pick in
      withAnimation(motion ? Motion.ease : nil) { labCode = ShellStyle(code: pick).code }
    })
    return VStack(alignment: .leading, spacing: Metrics.s2) {
      HStack(spacing: Metrics.s1) {
        Segmented(options: ShellStyle.presets.map { ($0.id, String($0.label.prefix(2)).trimmingCharacters(in: .whitespaces)) },
                  value: preset)
        Spacer(minLength: 0)
        // Hold to see the look before the study
        Icon(.eye, size: Metrics.iconCtl)
          .foregroundStyle(comparing ? Palette.fgStrong : Palette.fg2)
          .frame(width: Metrics.ctl, height: Metrics.ctl)
          .contentShape(Rectangle())
          .onLongPressGesture(minimumDuration: Motion.labHold, perform: {}, onPressingChanged: { held in withAnimation(motion ? Motion.ease : nil) { comparing = held } })
          .accessibilityLabel(Text(verbatim: "按住看现状"))
        IconButton(.chevronsUpDown, label: "换到另一边") { dockTop.toggle() }
        IconButton(.x, label: "收起") { open = false }
      }
      ScrollView(.vertical) {
        VStack(spacing: 0) {
          ForEach(ShellStyle.axes, id: \.key) { axis in
            let value = Binding<String>(get: { style.value(axis.key) }, set: { pick in
              var next = style
              next.set(axis.key, pick)
              withAnimation(motion ? Motion.ease : nil) { labCode = next.code }
            })
            HStack(spacing: Metrics.s2) {
              Text(verbatim: axis.title)
                .textStyle(TypeScale.text3)
                .foregroundStyle(Palette.fg2)
                .lineLimit(1)
              Spacer(minLength: 0)
              Segmented(options: axis.options, value: value)
            }
            .frame(minHeight: Metrics.ctl + Metrics.s1)
          }
        }
      }
      .scrollIndicators(.hidden)
      .frame(maxHeight: Metrics.labRowsMax)
      .fixedSize(horizontal: false, vertical: true)
    }
    .padding(Metrics.s3)
    .glassEffect(.regular, in: RoundedRectangle(cornerRadius: Metrics.rComposer))
    .padding(.horizontal, Metrics.s2)
  }
}

extension Prefs {
  /// Whether the design study's panel docks at the top (else the bottom)
  static let labDock = "acpira.lab.dockTop"
}
