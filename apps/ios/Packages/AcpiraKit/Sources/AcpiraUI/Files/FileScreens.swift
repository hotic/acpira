import AcpiraDesign
import AcpiraModels
import SwiftUI

// The review on the phone (panel/FileTree.tsx's changes, panel/FileViewer.tsx), read-only: the files the session
// changed with their counts, and one file as it is now with the removed lines put back and the added ones tinted, a
// change navigator in the bar. A file link in a reply opens the same page at its line.

/// The files the session's edits left changed
struct ChangesScreen: View {
  let sessionId: String
  @Environment(AppModel.self) private var model
  @State private var files: [ProjectChange]?

  var body: some View {
    List {
      if let files {
        if files.isEmpty {
          Text(web: t("mobile.changes.none"), paragraph: false).foregroundStyle(Palette.fg3)
        } else {
          Section {
            ForEach(files, id: \.path) { change in
              NavigationLink(value: Route.file(session: sessionId, path: change.path, line: nil)) {
                ChangeRow(change: change)
              }
            }
          } header: {
            let add = files.reduce(0) { $0 + $1.add }, del = files.reduce(0) { $0 + $1.del }
            HStack(spacing: Metrics.s1_5) {
              Text(verbatim: t("panel.fileCount", ["n": files.count]))
              DiffCounts(add: add, del: del)
            }
          }
        }
      } else {
        ProgressView().frame(maxWidth: .infinity)
      }
    }
    .formPage()
    .navigationTitle(t("panel.changes"))
    .navigationBarTitleDisplayModeInline()
    .task { files = await model.changes(sessionId) }
    .refreshable { files = await model.changes(sessionId) }
  }
}

/// One changed file: its name, the folder under it, the counts at the end; a deleted file is struck through
struct ChangeRow: View {
  let change: ProjectChange
  var body: some View {
    HStack(spacing: Metrics.gap) {
      VStack(alignment: .leading, spacing: Metrics.s0_5) {
        Text(verbatim: fileName(change.path)).lineLimit(1).truncationMode(.middle)
          .strikethrough(change.gone == true)
        let dir = folderOf(change.path)
        if !dir.isEmpty {
          Text(verbatim: dir).lineLimit(1).truncationMode(.head).textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
        }
      }
      Spacer(minLength: 0)
      DiffCounts(add: change.add, del: change.del)
    }
  }
}

/// `+12 −3` in the diff colours, as the tool rows' DiffStat
struct DiffCounts: View {
  let add: Int
  let del: Int
  var body: some View {
    HStack(spacing: Metrics.s1) {
      if add > 0 { Text(verbatim: "+\(add)").foregroundStyle(Palette.ok) }
      if del > 0 { Text(verbatim: "−\(del)").foregroundStyle(Palette.danger) }
    }
    .textStyle(TypeScale.text2)
    .monospacedDigit()
  }
}

/// One project file, read-only, its changes marked; opens at `line` or else at the first change
struct FileScreen: View {
  let sessionId: String
  let path: String
  var line: Int?
  @Environment(AppModel.self) private var model
  @Environment(\.colorScheme) private var scheme
  @State private var content: FileContent?
  @State private var tokens: [[CodeToken]?]?
  /// The change the navigator last went to
  @State private var at: Int?

  var body: some View {
    Group {
      switch content {
      case .lines(let lines)?:
        rows(lines)
      case .failed(let error)?:
        Text(web: message(error), paragraph: false).foregroundStyle(Palette.fg3)
          .frame(maxWidth: .infinity, maxHeight: .infinity)
      case nil:
        ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    }
    .background(Palette.bg0)
    .navigationTitle(fileName(path))
    .navigationBarTitleDisplayModeInline()
    .task { content = await model.file(sessionId, path: path) }
    // Coloured again when the scheme changes: the first pass can run before a pinned theme reaches the page
    .task(id: Highlight(content: content, theme: CodeTheme(scheme))) {
      guard case .lines(let lines)? = content, codeLanguage(path) != nil else { return }
      let theme = CodeTheme(scheme), path = path
      tokens = await Task.detached(priority: .utility) { SyntaxHighlighter.shared.diff(lines, path: path, theme: theme) }.value
    }
  }

  private struct Highlight: Equatable {
    var content: FileContent?
    var theme: CodeTheme
  }

  private func rows(_ lines: [DiffLine]) -> some View {
    let starts = changeStarts(lines)
    let mono = WebType.mono
    let ch = mono.size * 0.602
    let digits = CGFloat(String(lines.compactMap { $0.newLine ?? $0.oldLine }.max() ?? 1).count)
    return ScrollViewReader { proxy in
      ScrollView(.vertical) {
        LazyVStack(alignment: .leading, spacing: 0) {
          ForEach(lines.indices, id: \.self) { i in
            let row = lines[i]
            HStack(alignment: .firstTextBaseline, spacing: 0) {
              Text(web: (row.kind == .del ? nil : row.newLine).map(String.init) ?? "", size: mono.size)
                .foregroundStyle(Palette.fg3)
                .monospacedDigit()
                .frame(width: (digits + 1) * ch, alignment: .trailing)
              Text(web: row.kind == .add ? "+" : row.kind == .del ? "−" : " ", size: mono.size)
                .foregroundStyle(row.kind == .add ? Palette.ok : row.kind == .del ? Palette.danger : Palette.fg3)
                .frame(width: 2 * ch)
              // Soft wrap: a phone column cannot scroll sideways inside a page that scrolls down
              Group {
                if let runs = tokens?[safe: i] ?? nil, !row.code.isEmpty { Text(codeRuns(runs, size: mono.size)) }
                else { Text(web: row.code.isEmpty ? "\u{200b}" : row.code, size: mono.size) }
              }
              .foregroundStyle(Palette.fg1)
              .frame(maxWidth: .infinity, alignment: .leading)
              .padding(.trailing, Metrics.pad)
            }
            .textStyle(WebType.codeOutput)
            .frame(minHeight: WebMetrics.codeOutputLine)
            .background(Rectangle().fill(row.kind == .add ? Palette.diffAdd : row.kind == .del ? Palette.diffDel : .clear))
            .id(i)
          }
        }
        .padding(.vertical, Metrics.gapHalf)
        .textSelection(.enabled)
      }
      .onAppear {
        // A linked line, else the first change, comes into view
        let target = line.flatMap { n in lines.firstIndex { $0.newLine == n } } ?? starts.first
        if let target { proxy.scrollTo(target, anchor: .center) }
      }
      .toolbar {
        if starts.count > 0 {
          ToolbarItemGroup(placement: .primaryAction) {
            Button { go(-1, starts, proxy) } label: { Image(systemName: "chevron.up") }
              .accessibilityLabel(Text(web: t("panel.prevChange")))
            Button { go(1, starts, proxy) } label: { Image(systemName: "chevron.down") }
              .accessibilityLabel(Text(web: t("panel.nextChange")))
          }
        }
      }
    }
  }

  /// The previous or next change, wrapping around
  private func go(_ step: Int, _ starts: [Int], _ proxy: ScrollViewProxy) {
    let current = at.flatMap { starts.firstIndex(of: $0) } ?? (step > 0 ? -1 : 0)
    let next = starts[(current + step + starts.count) % starts.count]
    at = next
    withAnimation(.easeOut(duration: Motion.durOpen)) { proxy.scrollTo(next, anchor: .center) }
  }

  private func message(_ error: ProjectError) -> String {
    switch error {
    case .missing: t("panel.error.missing")
    case .binary: t("panel.error.binary")
    case .tooLarge: t("panel.error.tooLarge")
    case .outside: t("panel.error.outside")
    case .noProject: t("panel.error.other", ["error": "no project"])
    case .message(let text): t("panel.error.other", ["error": text])
    }
  }
}

/// Where each run of changed lines starts
nonisolated func changeStarts(_ lines: [DiffLine]) -> [Int] {
  lines.indices.filter { i in lines[i].kind != .ctx && (i == 0 || lines[i - 1].kind == .ctx) }
}

nonisolated func fileName(_ path: String) -> String { path.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? path }

nonisolated func folderOf(_ path: String) -> String {
  guard let cut = path.lastIndex(where: { $0 == "/" || $0 == "\\" }) else { return "" }
  return String(path[..<cut])
}
