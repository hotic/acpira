import AcpiraDesign
import SwiftUI

// The phone's pickers for long lists (models, projects): a bottom sheet instead of a popup over the composer. The
// sheet is iOS's own (drag to dismiss, medium / large detents) dressed in the menu surface: --menu-bg, a title bar,
// and the popup's option rows at sheet size (`sheetRows`). Everything inside that closes a popup (`dismissPopup`)
// closes the sheet, so a panel built for a popup works unchanged in one.

extension EnvironmentValues {
  /// Option rows and lists inside a bottom sheet: taller rows, body type, no popup height cap
  @Entry var sheetRows = false
}

/// The sheet's body: the title bar fixed on top, everything under it scrolling as one, the current row (`current`,
/// a row's `.id`) brought to the middle when it opens
struct PickerSheet<Content: View>: View {
  let title: String
  var current: String?
  let content: Content

  init(_ title: String, current: String? = nil, @ViewBuilder content: () -> Content) {
    self.title = title; self.current = current; self.content = content()
  }

  var body: some View {
    VStack(spacing: 0) {
      Text(web: title, paragraph: false)
        .textStyle(TypeScale.field, weight: .semibold)
        .foregroundStyle(Palette.fgStrong)
        .frame(maxWidth: .infinity)
        .frame(height: Metrics.sheetRow)
        .padding(.top, Metrics.s2)
      ScrollViewReader { reader in
        ScrollView(.vertical) {
          content
            .textStyle(TypeScale.text2)
            .foregroundStyle(Palette.fg1)
            .padding(.horizontal, Metrics.s3)
            .padding(.bottom, Metrics.pad)
        }
        .scrollIndicators(.hidden)
        .onAppear { if let current { reader.scrollTo(current, anchor: .center) } }
      }
    }
    .environment(\.sheetRows, true)
  }
}

extension View {
  /// Presents `content` in a bottom picker sheet titled `title`
  func pickerSheet<Content: View>(isPresented: Binding<Bool>, title: String, current: String? = nil,
                                  @ViewBuilder content: @escaping () -> Content) -> some View {
    sheet(isPresented: isPresented) {
      PickerSheet(title, current: current, content: content)
        .environment(\.dismissPopup) { isPresented.wrappedValue = false }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationCornerRadius(Metrics.rSheet)
        .presentationBackground(Palette.menuBg)
    }
  }
}
