import CoreGraphics
import Testing
@testable import AcpiraUI

// The flowchart layout against mermaid 11.17 in Chrome (the LAB reference page, showcase fixture)
@MainActor struct MermaidTests {
  static let source = """
    graph LR
      A[CLI] --> B[parseArgs]
      B --> C{dryRun?}
      C -->|是| D[只打印]
      C -->|否| E[执行]
    """

  @Test func parsesTheShowcaseChart() throws {
    let chart = try #require(FlowChart.parse(Self.source))
    #expect(chart.direction == .lr)
    #expect(chart.nodes.map(\.id) == ["A", "B", "C", "D", "E"])
    #expect(chart.nodes[2].shape == .diamond)
    #expect(chart.edges.map(\.label) == ["", "", "是", "否"])
  }

  @Test func layoutMatchesMermaid() throws {
    let layout = MermaidLayout(try #require(FlowChart.parse(Self.source)))
    print("mermaid size", layout.size, layout.nodes.map { ($0.center, $0.size) })
    // viewBox 0 0 648.34375 174
    #expect(abs(layout.size.width - 648.34) < 0.1)
    #expect(abs(layout.size.height - 174) < 0.01)
    // Node centres from the SVG's transforms
    let expected: [(CGFloat, CGFloat)] = [(52.45, 87), (220.26, 87), (404.33, 87), (585.86, 35), (585.86, 139)]
    for (node, e) in zip(layout.nodes, expected) {
      #expect(abs(node.center.x - e.0) < 0.1)
      #expect(abs(node.center.y - e.1) < 0.01)
    }
    // C → D starts on the diamond's edge toward the label at (498.21, 35)
    let label = try #require(layout.edges[2].labelCenter)
    #expect(abs(label.x - 498.21) < 0.1 && abs(label.y - 35) < 0.01)
    #expect(abs(layout.edges[2].start.point.x - 443.40) < 0.1)
    #expect(abs(layout.edges[2].start.point.y - 65.36) < 0.1)
  }

  @Test func otherSyntaxParses() throws {
    let chart = try #require(FlowChart.parse("""
      flowchart TD
        a((start)) -- go --> b([run]) & c{{hex}}
        b -.-> d[(db)]
        c ==> d
        d --- e
      """))
    #expect(chart.nodes.map(\.shape) == [.circle, .stadium, .hexagon, .rect, .rect])
    #expect(chart.edges.count == 5)
    #expect(chart.edges[0].label == "go")
    #expect(chart.edges[2].stroke == .dotted)
    #expect(chart.edges[3].stroke == .thick)
    #expect(chart.edges[4].end == .none)
    _ = MermaidLayout(chart)
    #expect(FlowChart.parse("sequenceDiagram\n A->>B: hi") == nil)
  }
}
