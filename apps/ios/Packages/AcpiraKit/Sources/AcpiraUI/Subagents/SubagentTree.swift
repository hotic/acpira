import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/subagents/SubagentGraph.tsx at phone width, a page the chat's "…" menu pushes: the session's every child as the
// transcript's rows, one lead + gap deeper per level under the session's own label row. It is the only way to a
// grandchild, since a turn shows its top-level children alone. The web's elbow lines are left out: the indent carries
// the tree at this width.

struct SubagentTreePage: View {
  let session: String
  @Environment(AppModel.self) private var model

  var body: some View {
    let root = model.views[session]
    // Summoned helpers only: sidekick rounds live in the transcript alone, as on the web
    let nodes = (root?.subagents ?? []).filter { !isSidekick($0) }
    ScrollView(.vertical) {
      VStack(alignment: .leading, spacing: 0) {
        // The session labels the tree; it is not a node to open
        Row {
          LeadIcon(.gitBranch)
        } content: {
          Text(web: root?.title ?? t("subagents.root")).foregroundStyle(Palette.fg1).lineLimit(1).cssBaseline()
        } trailing: {
          Text(web: t("subagents.entry", ["n": nodes.count]))
        }
        ForEach(flattenTree(nodes), id: \.node.id) { item in
          SubagentRowButton(node: item.node, descendants: false)
            .padding(.leading, CGFloat(item.depth + 1) * WebMetrics.indent)
        }
      }
      .padding(.horizontal, Metrics.s1 + Metrics.pad)
      .padding(.vertical, Metrics.padY)
    }
    .scrollIndicators(.hidden)
    .topFade()
    .background(Palette.bg0)
    .navigationTitle(t("subagents.graph"))
    .navigationBarTitleDisplayModeInline()
    .backWithoutTitle()
    .environment(\.rowHeight, WebMetrics.chatRow)
    .environment(\.allSubagents, nodes)
    .environment(\.inspectSubagent, SubagentInspect(session: session) { model.path.append(.subagent(session: session, id: $0)) })
  }
}
