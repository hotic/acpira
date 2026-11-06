import AcpiraDesign
import AcpiraModels
import SwiftUI

// The add-project picker: the computer's folders in a system sheet, as Files walks a drive. It opens on the user's
// home; a folder row pushes that folder, back returns, and the first page's up button goes above the home (projects
// often live elsewhere, a second volume included). The add button under the list takes the folder on screen; a typed
// path stays one tap away in the bar. An engine without `localDir` keeps the typed path alone.

/// The folder menus' Add: the computer's folders where the engine lists them, else the typed-path alert
struct AddProjectPicker: ViewModifier {
  @Environment(AppModel.self) private var model
  @Binding var isPresented: Bool
  var onAdd: (String) -> Void

  func body(content: Content) -> some View {
    if model.browsesFolders {
      content.sheet(isPresented: $isPresented) {
        FolderBrowserSheet(onAdd: onAdd).environment(model)
      }
    } else {
      content.modifier(AddProjectAlert(isPresented: $isPresented, onAdd: onAdd))
    }
  }
}

/// The picker's sheet: a navigation stack of folders, rooted at the home until the user goes up
struct FolderBrowserSheet: View {
  var onAdd: (String) -> Void
  @Environment(\.dismiss) private var dismiss
  /// The first page's folder: nil is the home, going up makes the parent the first page
  @State private var root: String?
  @State private var path: [String] = []
  @State private var typing = false

  var body: some View {
    NavigationStack(path: $path) {
      page(root, up: { root = $0 })
        // A new first page loads afresh rather than keeping the old one's listing
        .id(root ?? "")
        .navigationDestination(for: String.self) { page($0, up: nil) }
    }
    .modifier(AddProjectAlert(isPresented: $typing, onAdd: pick))
  }

  private func page(_ folder: String?, up: ((String) -> Void)?) -> some View {
    FolderPage(folder: folder, onUp: up, onPick: pick, onType: { typing = true }, onClose: { dismiss() })
  }

  private func pick(_ folder: String) {
    onAdd(folder)
    dismiss()
  }
}

/// One folder: its folders as rows, the add button under them
private struct FolderPage: View {
  @Environment(AppModel.self) private var model
  let folder: String?
  /// The first page's up button, to the folder above (nil on pushed pages: back does it)
  var onUp: ((String) -> Void)?
  var onPick: (String) -> Void
  var onType: () -> Void
  var onClose: () -> Void
  @State private var listing: LocalListing?

  var body: some View {
    Form {
      if case let .folder(path, _, entries)? = listing {
        Section {
          ForEach(entries, id: \.name) { entry in
            NavigationLink(value: childPath(path, entry.name)) {
              Label {
                HStack(spacing: Metrics.s2) {
                  Text(verbatim: entry.name).lineLimit(1).truncationMode(.middle)
                  // A work tree is the likely project: the tag says so without a second line on every row
                  if entry.git == true {
                    Text(verbatim: "git").textStyle(TypeScale.text3).foregroundStyle(Palette.fg3)
                  }
                }
              } icon: {
                Image(systemName: "folder")
              }
            }
          }
        }
      }
    }
    .formPage()
    .overlay {
      switch listing {
      case nil: ProgressView()
      case .failed(let error)?: note(error)
      case let .folder(_, _, entries)? where entries.isEmpty: note(t("mobile.projects.noFolders"))
      default: EmptyView()
      }
    }
    .safeAreaInset(edge: .bottom) {
      if case let .folder(path, _, _)? = listing {
        Button { onPick(path) } label: {
          Text(web: t("mobile.projects.addHere", ["name": folderName(path)]), paragraph: false).truncationMode(.middle)
        }
        .buttonStyle(PairPrimaryStyle())
        .padding(.horizontal, Metrics.page)
        .padding(.bottom, Metrics.s2)
      }
    }
    .navigationTitle(title)
    .navigationSubtitleIfAvailable(subtitle)
    .navigationBarTitleDisplayModeInline()
    .toolbar {
      if let onUp, case let .folder(_, parent?, _)? = listing {
        ToolbarItem(placement: .cancellationAction) {
          Button { onUp(parent) } label: { Image(systemName: "chevron.up") }
            .accessibilityLabel(Text(web: t("mobile.projects.parent")))
        }
      }
      ToolbarItem(placement: .primaryAction) { CloseButton(action: onClose) }
      ToolbarItem(placement: .secondaryAction) {
        Button(action: onType) { Label(t("mobile.projects.typePath"), systemImage: "character.cursor.ibeam") }
      }
    }
    .task { listing = await model.browse(folder) }
  }

  private var title: String {
    if case let .folder(path, _, _)? = listing { return folderName(path) }
    return folder.map(folderName) ?? t("mobile.projects.add")
  }

  /// The whole path under the name, so two folders of one name stay apart
  private var subtitle: String {
    if case let .folder(path, _, _)? = listing { return path }
    return folder ?? ""
  }

  private func note(_ text: String) -> some View {
    Text(web: text, paragraph: false)
      .textStyle(TypeScale.text2)
      .foregroundStyle(Palette.fg3)
      .multilineTextAlignment(.center)
      .padding(.horizontal, Metrics.page)
  }
}

/// `name` inside `folder` as the computer writes paths: its own separator, none doubled after a root (`/`, `C:\`)
func childPath(_ folder: String, _ name: String) -> String {
  let separator = folder.contains("\\") && !folder.contains("/") ? "\\" : "/"
  return folder.hasSuffix(separator) ? folder + name : folder + separator + name
}
