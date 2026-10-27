import CoreGraphics

// A port of the dagre layout mermaid 11 runs for flowcharts (dagre-d3-es 7.0.14, src/dagre), so a diagram's nodes,
// edge bends and label spots land where the webview's SVG puts them. Clusters (subgraphs) are not ported; everything
// else follows the JS step by step, including graphlib's iteration orders, which decide ties: insertion order, except
// that integer-like keys of a JS object come first in ascending order.

// MARK: - graphlib

/// A JS object's key order: canonical array-index keys ascending, then the rest in insertion order
struct JSKeys {
  private(set) var inserted: [String] = []
  private var present: Set<String> = []

  var ordered: [String] {
    let numeric = inserted.filter(Self.isIndex).sorted { UInt32($0)! < UInt32($1)! }
    return numeric.isEmpty ? inserted : numeric + inserted.filter { !Self.isIndex($0) }
  }

  func contains(_ k: String) -> Bool { present.contains(k) }

  mutating func insert(_ k: String) {
    guard !present.contains(k) else { return }
    present.insert(k)
    inserted.append(k)
  }

  mutating func remove(_ k: String) {
    guard present.remove(k) != nil else { return }
    inserted.removeAll { $0 == k }
  }

  static func isIndex(_ k: String) -> Bool {
    guard let n = UInt32(k), n < UInt32.max else { return false }
    return String(n) == k
  }
}

/// JS string comparison (UTF-16 code units)
func jsLess(_ a: String, _ b: String) -> Bool { a.utf16.lexicographicallyPrecedes(b.utf16) }

final class DNode {
  var width: Double = 0
  var height: Double = 0
  var rank: Int?
  var order: Int?
  var x: Double = 0
  var y: Double = 0
  /// "edge", "edge-label", "edge-proxy", "root"
  var dummy: String?
  var edgeLabel: DEdgeLabel?
  var edgeObj: DEdgeKey?
  /// Network simplex tree labels
  var low = 0, lim = 0
  var parent: String?

  init(width: Double = 0, height: Double = 0) {
    self.width = width
    self.height = height
  }
}

final class DEdgeLabel {
  var minlen: Int = 1
  var weight: Double = 1
  var width: Double = 0
  var height: Double = 0
  var labelRank: Double?
  var points: [CGPoint] = []
  var x: Double?
  var y: Double?
  var reversed = false
  var forwardName: String?
  var cutvalue: Double = 0
  /// The caller's edge index
  var tag = -1

  init(minlen: Int = 1, weight: Double = 1, width: Double = 0, height: Double = 0) {
    self.minlen = minlen
    self.weight = weight
    self.width = width
    self.height = height
  }
}

struct DEdgeKey: Hashable {
  var v: String
  var w: String
  var name: String?
}

final class DGraph {
  let directed: Bool
  let multigraph: Bool
  private var nodeKeys = JSKeys()
  private var nodeLabels: [String: DNode] = [:]
  private var edgeKeys = JSKeys()
  private var edgeObjs: [String: DEdgeKey] = [:]
  private var edgeLabels: [String: DEdgeLabel] = [:]
  private var inMap: [String: JSKeys] = [:]
  private var outMap: [String: JSKeys] = [:]
  private var preds: [String: [String: Int]] = [:]
  private var predKeys: [String: JSKeys] = [:]
  private var sucs: [String: [String: Int]] = [:]
  private var sucKeys: [String: JSKeys] = [:]

  init(directed: Bool = true, multigraph: Bool = false) {
    self.directed = directed
    self.multigraph = multigraph
  }

  // Nodes

  var nodes: [String] { nodeKeys.ordered }
  var nodeCount: Int { nodeLabels.count }
  func hasNode(_ v: String) -> Bool { nodeLabels[v] != nil }
  func node(_ v: String) -> DNode { nodeLabels[v]! }

  @discardableResult
  func setNode(_ v: String, _ label: DNode? = nil) -> DNode {
    if let existing = nodeLabels[v] {
      if let label { nodeLabels[v] = label; return label }
      return existing
    }
    let l = label ?? DNode()
    nodeKeys.insert(v)
    nodeLabels[v] = l
    inMap[v] = JSKeys(); outMap[v] = JSKeys()
    preds[v] = [:]; predKeys[v] = JSKeys()
    sucs[v] = [:]; sucKeys[v] = JSKeys()
    return l
  }

  func removeNode(_ v: String) {
    guard nodeLabels[v] != nil else { return }
    for id in inMap[v]!.ordered { removeEdge(id: id) }
    for id in outMap[v]!.ordered { removeEdge(id: id) }
    nodeKeys.remove(v)
    nodeLabels[v] = nil
    inMap[v] = nil; outMap[v] = nil
    preds[v] = nil; predKeys[v] = nil
    sucs[v] = nil; sucKeys[v] = nil
  }

  var sources: [String] { nodes.filter { inMap[$0]!.inserted.isEmpty } }

  func predecessors(_ v: String) -> [String] { predKeys[v]?.ordered ?? [] }
  func successors(_ v: String) -> [String] { sucKeys[v]?.ordered ?? [] }
  func neighbors(_ v: String) -> [String] {
    let p = predecessors(v)
    let seen = Set(p)
    return p + successors(v).filter { !seen.contains($0) }
  }

  // Edges

  private func edgeID(_ v: String, _ w: String, _ name: String?) -> String {
    var (a, b) = (v, w)
    if !directed && jsLess(b, a) { swap(&a, &b) }
    return a + "\u{1}" + b + "\u{1}" + (name ?? "\u{0}")
  }

  var edges: [DEdgeKey] { edgeKeys.ordered.map { edgeObjs[$0]! } }

  func setEdge(_ v: String, _ w: String, _ label: DEdgeLabel, name: String? = nil) {
    let name = multigraph ? name : nil
    let id = edgeID(v, w, name)
    if edgeLabels[id] != nil {
      edgeLabels[id] = label
      return
    }
    setNode(v)
    setNode(w)
    var (a, b) = (v, w)
    if !directed && jsLess(b, a) { swap(&a, &b) }
    let key = DEdgeKey(v: a, w: b, name: name)
    edgeKeys.insert(id)
    edgeObjs[id] = key
    edgeLabels[id] = label
    preds[b]![a, default: 0] += 1
    predKeys[b]!.insert(a)
    sucs[a]![b, default: 0] += 1
    sucKeys[a]!.insert(b)
    inMap[b]!.insert(id)
    outMap[a]!.insert(id)
  }

  func setEdge(_ e: DEdgeKey, _ label: DEdgeLabel) { setEdge(e.v, e.w, label, name: e.name) }

  func edge(_ v: String, _ w: String, name: String? = nil) -> DEdgeLabel? { edgeLabels[edgeID(v, w, name)] }
  func edge(_ e: DEdgeKey) -> DEdgeLabel { edgeLabels[edgeID(e.v, e.w, e.name)]! }
  func hasEdge(_ v: String, _ w: String, name: String? = nil) -> Bool { edgeLabels[edgeID(v, w, name)] != nil }

  func removeEdge(_ e: DEdgeKey) { removeEdge(id: edgeID(e.v, e.w, e.name)) }
  func removeEdge(_ v: String, _ w: String) { removeEdge(id: edgeID(v, w, nil)) }

  private func removeEdge(id: String) {
    guard let e = edgeObjs[id] else { return }
    edgeKeys.remove(id)
    edgeObjs[id] = nil
    edgeLabels[id] = nil
    preds[e.w]![e.v]! -= 1
    if preds[e.w]![e.v]! == 0 { preds[e.w]![e.v] = nil; predKeys[e.w]!.remove(e.v) }
    sucs[e.v]![e.w]! -= 1
    if sucs[e.v]![e.w]! == 0 { sucs[e.v]![e.w] = nil; sucKeys[e.v]!.remove(e.w) }
    inMap[e.w]!.remove(id)
    outMap[e.v]!.remove(id)
  }

  func inEdges(_ v: String) -> [DEdgeKey] { inMap[v]?.ordered.map { edgeObjs[$0]! } ?? [] }
  func outEdges(_ v: String) -> [DEdgeKey] { outMap[v]?.ordered.map { edgeObjs[$0]! } ?? [] }
  func nodeEdges(_ v: String) -> [DEdgeKey] { inEdges(v) + outEdges(v) }
}

// MARK: - Layout

enum RankDir: String { case tb, bt, lr, rl }

struct DagreConfig {
  var rankdir: RankDir = .tb
  var nodesep: Double = 50
  var edgesep: Double = 20
  var ranksep: Double = 50
  var marginx: Double = 8
  var marginy: Double = 8
}

/// Runs dagre's layout on `g` in place: nodes get x / y, edges points (and x / y for labelled ones); returns the
/// graph's width and height
@discardableResult
func dagreLayout(_ g: DGraph, _ config: DagreConfig) -> CGSize {
  var counter = 0
  var cfg = config
  func uniqueID(_ prefix: String) -> String {
    counter += 1
    return prefix + String(counter)
  }
  func addDummyNode(_ type: String, _ label: DNode, _ prefix: String) -> String {
    var v: String
    repeat { v = uniqueID(prefix) } while g.hasNode(v)
    label.dummy = type
    g.setNode(v, label)
    return v
  }

  // makeSpaceForEdgeLabels (mermaid flowchart edges are labelpos "c": no label offset)
  cfg.ranksep /= 2
  for e in g.edges { g.edge(e).minlen *= 2 }

  // removeSelfEdges: mermaid splits self loops into extra nodes before dagre, so none arrive here
  for e in g.edges where e.v == e.w { g.removeEdge(e) }

  // acyclic.run (dfsFAS)
  var fas: [DEdgeKey] = []
  do {
    var stack: Set<String> = []
    var visited: Set<String> = []
    func dfs(_ v: String) {
      guard !visited.contains(v) else { return }
      visited.insert(v)
      stack.insert(v)
      for e in g.outEdges(v) {
        if stack.contains(e.w) { fas.append(e) } else { dfs(e.w) }
      }
      stack.remove(v)
    }
    for v in g.nodes { dfs(v) }
  }
  for e in fas {
    let label = g.edge(e)
    g.removeEdge(e)
    label.forwardName = e.name
    label.reversed = true
    g.setEdge(e.w, e.v, label, name: uniqueID("rev"))
  }

  // nestingGraph.run without clusters: a root above every node, weight 0, nodeSep 1
  let root = addDummyNode("root", DNode(), "_root")
  for v in g.nodes where v != root {
    g.setEdge(root, v, DEdgeLabel(minlen: 1, weight: 0))
  }

  // rank (network simplex)
  networkSimplex(g)

  // injectEdgeLabelProxies
  var proxies: [(String, DEdgeKey, Double)] = []
  for e in g.edges {
    let edge = g.edge(e)
    if edge.width != 0 && edge.height != 0 {
      let v = g.node(e.v), w = g.node(e.w)
      let rank = Double(w.rank! - v.rank!) / 2 + Double(v.rank!)
      let id = addDummyNode("edge-proxy", DNode(), "_ep")
      proxies.append((id, e, rank))
    }
  }
  // removeEmptyRanks with nodeRankFactor 1 never removes a rank

  // nestingGraph.cleanup
  g.removeNode(root)

  // normalizeRanks (proxies carry fractional ranks, kept aside)
  let minRank = (g.nodes.compactMap { g.node($0).rank } + proxies.map { Int($0.2.rounded(.down)) }).min() ?? 0
  let realMin = g.nodes.compactMap { g.node($0).rank }.min() ?? 0
  _ = minRank
  for v in g.nodes { if let r = g.node(v).rank { g.node(v).rank = r - realMin } }

  // removeEdgeLabelProxies
  for (id, e, rank) in proxies {
    g.edge(e).labelRank = rank - Double(realMin)
    g.removeNode(id)
  }

  // normalize.run
  var dummyChains: [String] = []
  for e in g.edges {
    var v = e.v
    var vRank = g.node(v).rank!
    let w = e.w
    let wRank = g.node(w).rank!
    let edgeLabel = g.edge(e)
    if wRank == vRank + 1 { continue }
    g.removeEdge(e)
    var i = 0
    vRank += 1
    while vRank < wRank {
      edgeLabel.points = []
      let attrs = DNode()
      attrs.edgeLabel = edgeLabel
      attrs.edgeObj = e
      attrs.rank = vRank
      let dummy = addDummyNode("edge", attrs, "_d")
      if let lr = edgeLabel.labelRank, Double(vRank) == lr {
        attrs.width = edgeLabel.width
        attrs.height = edgeLabel.height
        attrs.dummy = "edge-label"
      }
      let link = DEdgeLabel(weight: edgeLabel.weight)
      g.setEdge(v, dummy, link, name: e.name)
      if i == 0 { dummyChains.append(dummy) }
      v = dummy
      i += 1
      vRank += 1
    }
    g.setEdge(v, w, DEdgeLabel(weight: edgeLabel.weight), name: e.name)
  }

  order(g)

  // adjustCoordinateSystem
  if cfg.rankdir == .lr || cfg.rankdir == .rl { swapWidthHeight(g) }

  position(g, cfg)

  // normalize.undo
  for start in dummyChains {
    var v = start
    var node = g.node(v)
    guard let origLabel = node.edgeLabel, let edgeObj = node.edgeObj else { continue }
    g.setEdge(edgeObj, origLabel)
    while node.dummy != nil {
      let w = g.successors(v)[0]
      g.removeNode(v)
      origLabel.points.append(CGPoint(x: node.x, y: node.y))
      if node.dummy == "edge-label" {
        origLabel.x = node.x
        origLabel.y = node.y
        origLabel.width = node.width
        origLabel.height = node.height
      }
      v = w
      node = g.node(v)
    }
  }

  // undoCoordinateSystem
  if cfg.rankdir == .bt || cfg.rankdir == .rl {
    for v in g.nodes { g.node(v).y = -g.node(v).y }
    for e in g.edges {
      let edge = g.edge(e)
      edge.points = edge.points.map { CGPoint(x: $0.x, y: -$0.y) }
      if let y = edge.y { edge.y = -y }
    }
  }
  if cfg.rankdir == .lr || cfg.rankdir == .rl {
    for v in g.nodes { let n = g.node(v); (n.x, n.y) = (n.y, n.x) }
    for e in g.edges {
      let edge = g.edge(e)
      edge.points = edge.points.map { CGPoint(x: $0.y, y: $0.x) }
      if let x = edge.x, let y = edge.y { edge.x = y; edge.y = x }
    }
    swapWidthHeight(g)
  }

  // translateGraph
  var minX = Double.infinity, maxX = 0.0, minY = Double.infinity, maxY = 0.0
  func extremes(_ x: Double, _ y: Double, _ w: Double, _ h: Double) {
    minX = min(minX, x - w / 2); maxX = max(maxX, x + w / 2)
    minY = min(minY, y - h / 2); maxY = max(maxY, y + h / 2)
  }
  for v in g.nodes { let n = g.node(v); extremes(n.x, n.y, n.width, n.height) }
  for e in g.edges {
    let edge = g.edge(e)
    if let x = edge.x, let y = edge.y { extremes(x, y, edge.width, edge.height) }
  }
  minX -= cfg.marginx
  minY -= cfg.marginy
  for v in g.nodes { let n = g.node(v); n.x -= minX; n.y -= minY }
  for e in g.edges {
    let edge = g.edge(e)
    edge.points = edge.points.map { CGPoint(x: $0.x - minX, y: $0.y - minY) }
    if let x = edge.x { edge.x = x - minX }
    if let y = edge.y { edge.y = y - minY }
  }
  let size = CGSize(width: maxX - minX + cfg.marginx, height: maxY - minY + cfg.marginy)

  // assignNodeIntersects
  for e in g.edges {
    let edge = g.edge(e)
    let nodeV = g.node(e.v), nodeW = g.node(e.w)
    let p1: CGPoint, p2: CGPoint
    if edge.points.isEmpty {
      p1 = CGPoint(x: nodeW.x, y: nodeW.y)
      p2 = CGPoint(x: nodeV.x, y: nodeV.y)
    } else {
      p1 = edge.points[0]
      p2 = edge.points[edge.points.count - 1]
    }
    edge.points.insert(intersectRect(nodeV, p1), at: 0)
    edge.points.append(intersectRect(nodeW, p2))
  }

  // reversePoints + acyclic.undo
  for e in g.edges {
    let label = g.edge(e)
    guard label.reversed else { continue }
    label.points.reverse()
    g.removeEdge(e)
    label.reversed = false
    let name = label.forwardName
    label.forwardName = nil
    g.setEdge(e.w, e.v, label, name: name)
  }
  return size
}

private func swapWidthHeight(_ g: DGraph) {
  for v in g.nodes { let n = g.node(v); (n.width, n.height) = (n.height, n.width) }
  for e in g.edges { let l = g.edge(e); (l.width, l.height) = (l.height, l.width) }
}

/// dagre util.intersectRect
func intersectRect(_ n: DNode, _ point: CGPoint) -> CGPoint {
  let dx = point.x - n.x, dy = point.y - n.y
  var w = n.width / 2, h = n.height / 2
  guard dx != 0 || dy != 0 else { return CGPoint(x: n.x, y: n.y) }
  if abs(dy) * w > abs(dx) * h {
    if dy < 0 { h = -h }
    return CGPoint(x: n.x + h * dx / dy, y: n.y + h)
  }
  if dx < 0 { w = -w }
  return CGPoint(x: n.x + w, y: n.y + w * dy / dx)
}

// MARK: - Rank (network simplex)

private func slack(_ g: DGraph, _ e: DEdgeKey) -> Int {
  g.node(e.w).rank! - g.node(e.v).rank! - g.edge(e).minlen
}

private func networkSimplex(_ input: DGraph) {
  // simplify: one edge per node pair, weights summed, the longest minlen
  let g = DGraph()
  for v in input.nodes { g.setNode(v, input.node(v)) }
  for e in input.edges {
    let prev = g.edge(e.v, e.w)
    let label = input.edge(e)
    g.setEdge(e.v, e.w, DEdgeLabel(minlen: max(prev?.minlen ?? 1, label.minlen), weight: (prev?.weight ?? 0) + label.weight))
  }

  longestPath(g)
  let t = feasibleTree(g)
  initLowLimValues(t)
  initCutValues(t, g)
  while let e = t.edges.first(where: { t.edge($0).cutvalue < 0 }) {
    let f = enterEdge(t, g, e)
    // exchangeEdges
    t.removeEdge(e.v, e.w)
    t.setEdge(f.v, f.w, DEdgeLabel())
    initLowLimValues(t)
    initCutValues(t, g)
    updateRanks(t, g)
  }
}

private func longestPath(_ g: DGraph) {
  var visited: Set<String> = []
  @discardableResult
  func dfs(_ v: String) -> Int {
    let label = g.node(v)
    if visited.contains(v) { return label.rank! }
    visited.insert(v)
    let ranks = g.outEdges(v).map { dfs($0.w) - g.edge($0).minlen }
    let rank = ranks.min() ?? 0
    label.rank = rank
    return rank
  }
  for v in g.sources { dfs(v) }
}

private func feasibleTree(_ g: DGraph) -> DGraph {
  let t = DGraph(directed: false)
  let start = g.nodes[0]
  let size = g.nodeCount
  t.setNode(start, DNode())

  func tightTree() -> Int {
    func dfs(_ v: String) {
      for e in g.nodeEdges(v) {
        let w = v == e.v ? e.w : e.v
        if !t.hasNode(w) && slack(g, e) == 0 {
          t.setNode(w, DNode())
          t.setEdge(v, w, DEdgeLabel())
          dfs(w)
        }
      }
    }
    for v in t.nodes { dfs(v) }
    return t.nodeCount
  }

  while tightTree() < size {
    // findMinSlackEdge: the first edge with one end in the tree and the least slack
    var best: DEdgeKey?
    var bestSlack = Int.max
    for e in g.edges where t.hasNode(e.v) != t.hasNode(e.w) {
      let s = slack(g, e)
      if s < bestSlack { best = e; bestSlack = s }
    }
    guard let edge = best else { break }
    let delta = t.hasNode(edge.v) ? slack(g, edge) : -slack(g, edge)
    for v in t.nodes { g.node(v).rank! += delta }
  }
  return t
}

private func initLowLimValues(_ tree: DGraph) {
  var visited: Set<String> = []
  func dfs(_ nextLim: Int, _ v: String, _ parent: String?) -> Int {
    var next = nextLim
    let low = next
    let label = tree.node(v)
    visited.insert(v)
    for w in tree.neighbors(v) where !visited.contains(w) {
      next = dfs(next, w, v)
    }
    label.low = low
    label.lim = next
    next += 1
    label.parent = parent
    return next
  }
  _ = dfs(1, tree.nodes[0], nil)
}

/// graphlib alg.postorder / preorder over the undirected tree
private func treeOrder(_ t: DGraph, _ roots: [String], post: Bool) -> [String] {
  var acc: [String] = []
  var visited: Set<String> = []
  func walk(_ v: String) {
    guard !visited.contains(v) else { return }
    visited.insert(v)
    if !post { acc.append(v) }
    for w in t.neighbors(v) { walk(w) }
    if post { acc.append(v) }
  }
  for v in roots { walk(v) }
  return acc
}

private func initCutValues(_ t: DGraph, _ g: DGraph) {
  var vs = treeOrder(t, t.nodes, post: true)
  vs.removeLast()
  for child in vs {
    let parent = t.node(child).parent!
    t.edge(child, parent)!.cutvalue = calcCutValue(t, g, child)
  }
}

private func calcCutValue(_ t: DGraph, _ g: DGraph, _ child: String) -> Double {
  let parent = t.node(child).parent!
  var childIsTail = true
  var graphEdge = g.edge(child, parent)
  if graphEdge == nil {
    childIsTail = false
    graphEdge = g.edge(parent, child)
  }
  var cutValue = graphEdge!.weight
  for e in g.nodeEdges(child) {
    let isOutEdge = e.v == child
    let other = isOutEdge ? e.w : e.v
    guard other != parent else { continue }
    let pointsToHead = isOutEdge == childIsTail
    let otherWeight = g.edge(e).weight
    cutValue += pointsToHead ? otherWeight : -otherWeight
    if t.hasEdge(child, other) {
      let otherCut = t.edge(child, other)!.cutvalue
      cutValue += pointsToHead ? -otherCut : otherCut
    }
  }
  return cutValue
}

private func enterEdge(_ t: DGraph, _ g: DGraph, _ edge: DEdgeKey) -> DEdgeKey {
  var v = edge.v, w = edge.w
  if !g.hasEdge(v, w) { (v, w) = (edge.w, edge.v) }
  let vLabel = t.node(v), wLabel = t.node(w)
  var tail = vLabel
  var flip = false
  if vLabel.lim > wLabel.lim { tail = wLabel; flip = true }
  func isDescendant(_ n: DNode) -> Bool { tail.low <= n.lim && n.lim <= tail.lim }
  var best: DEdgeKey?
  var bestSlack = Int.max
  for e in g.edges where flip == isDescendant(t.node(e.v)) && flip != isDescendant(t.node(e.w)) {
    let s = slack(g, e)
    if s < bestSlack { best = e; bestSlack = s }
  }
  return best!
}

private func updateRanks(_ t: DGraph, _ g: DGraph) {
  // Node labels carry no compound parent, so the root is the tree's first node
  let root = t.nodes[0]
  for v in treeOrder(t, [root], post: false).dropFirst() {
    let parent = t.node(v).parent!
    var flipped = false
    var edge = g.edge(v, parent)
    if edge == nil { edge = g.edge(parent, v); flipped = true }
    g.node(v).rank = g.node(parent).rank! + (flipped ? edge!.minlen : -edge!.minlen)
  }
}

// MARK: - Order

private func buildLayerMatrix(_ g: DGraph) -> [[String]] {
  let maxRank = g.nodes.compactMap { g.node($0).rank }.max() ?? -1
  var layers = [[String?]](repeating: [], count: maxRank + 1)
  for v in g.nodes {
    let n = g.node(v)
    guard let r = n.rank, let o = n.order else { continue }
    if layers[r].count <= o { layers[r] += [String?](repeating: nil, count: o + 1 - layers[r].count) }
    layers[r][o] = v
  }
  return layers.map { $0.compactMap { $0 } }
}

private func order(_ g: DGraph) {
  let maxRank = g.nodes.compactMap { g.node($0).rank }.max() ?? 0

  // initOrder: DFS along successors from the nodes sorted by rank
  var layers = [[String]](repeating: [], count: maxRank + 1)
  var visited: Set<String> = []
  func dfs(_ v: String) {
    guard !visited.contains(v) else { return }
    visited.insert(v)
    layers[g.node(v).rank!].append(v)
    for w in g.successors(v) { dfs(w) }
  }
  let ordered = g.nodes.enumerated().sorted { a, b in
    let ra = g.node(a.element).rank!, rb = g.node(b.element).rank!
    return ra != rb ? ra < rb : a.offset < b.offset
  }.map(\.element)
  for v in ordered { dfs(v) }
  assignOrder(g, layers)

  /// A rank's nodes with the weights toward their neighbours in the fixed layer, in graphlib's order
  struct LayerGraph {
    var nodes: [String]
    var inEdges: [String: [(u: String, weight: Double)]]
  }
  func layerGraph(_ rank: Int, _ incoming: Bool) -> LayerGraph {
    var nodes: [String] = []
    var inEdges: [String: [(u: String, weight: Double)]] = [:]
    for v in g.nodes where g.node(v).rank == rank {
      nodes.append(v)
      var list: [(u: String, weight: Double)] = []
      for e in incoming ? g.inEdges(v) : g.outEdges(v) {
        let u = e.v == v ? e.w : e.v
        if let i = list.firstIndex(where: { $0.u == u }) {
          list[i].weight += g.edge(e).weight
        } else {
          list.append((u, g.edge(e).weight))
        }
      }
      inEdges[v] = list
    }
    return LayerGraph(nodes: nodes, inEdges: inEdges)
  }
  let down = maxRank >= 1 ? (1...maxRank).map { layerGraph($0, true) } : []
  let up = maxRank >= 1 ? (0...(maxRank - 1)).reversed().map { layerGraph($0, false) } : []

  func sweep(_ graphs: [LayerGraph], _ biasRight: Bool) {
    for lg in graphs {
      struct Entry { var vs: [String]; var i: Int; var barycenter: Double?; var weight: Double }
      var entries: [Entry] = []
      for (i, v) in lg.nodes.enumerated() {
        let ins = lg.inEdges[v] ?? []
        if ins.isEmpty {
          entries.append(Entry(vs: [v], i: i, barycenter: nil, weight: 0))
        } else {
          var sum = 0.0, weight = 0.0
          for (u, w) in ins { sum += w * Double(g.node(u).order!); weight += w }
          entries.append(Entry(vs: [v], i: i, barycenter: sum / weight, weight: weight))
        }
      }
      // sort.js
      var sortable = entries.filter { $0.barycenter != nil }
      var unsortable = entries.filter { $0.barycenter == nil }.sorted { $0.i > $1.i }
      sortable.sort { a, b in
        if a.barycenter! != b.barycenter! { return a.barycenter! < b.barycenter! }
        return biasRight ? a.i > b.i : a.i < b.i
      }
      var vs: [String] = []
      var index = 0
      func consume() {
        while let last = unsortable.last, last.i <= index {
          unsortable.removeLast()
          vs += last.vs
          index += 1
        }
      }
      consume()
      for entry in sortable {
        index += entry.vs.count
        vs += entry.vs
        consume()
      }
      for (i, v) in vs.enumerated() { g.node(v).order = i }
    }
  }

  var bestCC = Double.infinity
  var best = buildLayerMatrix(g)
  var i = 0, lastBest = 0
  while lastBest < 4 {
    sweep(i % 2 == 1 ? down : up, i % 4 >= 2)
    let layering = buildLayerMatrix(g)
    let cc = crossCount(g, layering)
    if cc < bestCC {
      lastBest = 0
      best = layering
      bestCC = cc
    }
    i += 1
    lastBest += 1
  }
  assignOrder(g, best)
}

private func assignOrder(_ g: DGraph, _ layering: [[String]]) {
  for layer in layering {
    for (i, v) in layer.enumerated() { g.node(v).order = i }
  }
}

private func crossCount(_ g: DGraph, _ layering: [[String]]) -> Double {
  var cc = 0.0
  guard layering.count > 1 else { return 0 }
  for i in 1..<layering.count {
    let south = layering[i]
    var southPos: [String: Int] = [:]
    for (j, v) in south.enumerated() { southPos[v] = j }
    var entries: [(pos: Int, weight: Double)] = []
    for v in layering[i - 1] {
      entries += g.outEdges(v).compactMap { e in southPos[e.w].map { ($0, g.edge(e).weight) } }
        .enumerated().sorted { $0.element.0 != $1.element.0 ? $0.element.0 < $1.element.0 : $0.offset < $1.offset }
        .map(\.element)
    }
    var firstIndex = 1
    while firstIndex < south.count { firstIndex <<= 1 }
    let treeSize = 2 * firstIndex - 1
    firstIndex -= 1
    var tree = [Double](repeating: 0, count: treeSize)
    for entry in entries {
      var index = entry.pos + firstIndex
      tree[index] += entry.weight
      var weightSum = 0.0
      while index > 0 {
        if index % 2 == 1 { weightSum += tree[index + 1] }
        index = (index - 1) >> 1
        tree[index] += entry.weight
      }
      cc += entry.weight * weightSum
    }
  }
  return cc
}

// MARK: - Position

private func position(_ g: DGraph, _ cfg: DagreConfig) {
  let layering = buildLayerMatrix(g)
  // positionY
  var prevY = 0.0
  for layer in layering {
    let maxHeight = layer.map { g.node($0).height }.max() ?? 0
    for v in layer { g.node(v).y = prevY + maxHeight / 2 }
    prevY += maxHeight + cfg.ranksep
  }
  for (v, x) in positionX(g, layering, cfg) { g.node(v).x = x }
}

/// Brandes–Köpf, as bk.js
private func positionX(_ g: DGraph, _ layering: [[String]], _ cfg: DagreConfig) -> [(String, Double)] {
  // Type 1 conflicts: a non-inner segment crossing an inner one (between dummies)
  var conflicts: [String: Set<String>] = [:]
  func addConflict(_ a: String, _ b: String) {
    let (v, w) = jsLess(b, a) ? (b, a) : (a, b)
    conflicts[v, default: []].insert(w)
  }
  func hasConflict(_ a: String, _ b: String) -> Bool {
    let (v, w) = jsLess(b, a) ? (b, a) : (a, b)
    return conflicts[v]?.contains(w) ?? false
  }
  if layering.count > 1 {
    for li in 1..<layering.count {
      let prevLayer = layering[li - 1], layer = layering[li]
      var k0 = 0, scanPos = 0
      let lastNode = layer.last
      for (i, v) in layer.enumerated() {
        let w = g.node(v).dummy != nil ? g.predecessors(v).first { g.node($0).dummy != nil } : nil
        let k1 = w.map { g.node($0).order! } ?? prevLayer.count
        if w != nil || v == lastNode {
          for scanNode in layer[scanPos...i] {
            for u in g.predecessors(scanNode) {
              let uLabel = g.node(u)
              let uPos = uLabel.order!
              if (uPos < k0 || k1 < uPos) && !(uLabel.dummy != nil && g.node(scanNode).dummy != nil) {
                addConflict(u, scanNode)
              }
            }
          }
          scanPos = i + 1
          k0 = k1
        }
      }
    }
  }

  /// One alignment's coordinates, keyed in the JS object's insertion order
  struct Coords {
    var keys: [String] = []
    var values: [String: Double] = [:]
    subscript(_ v: String) -> Double? {
      get { values[v] }
      set {
        if values[v] == nil { keys.append(v) }
        values[v] = newValue
      }
    }
    var all: [Double] { keys.map { values[$0]! } }
    func map(_ f: (Double) -> Double) -> Coords {
      var out = Coords()
      for k in keys { out[k] = f(values[k]!) }
      return out
    }
  }

  func sep(_ v: String, _ w: String, _ reverse: Bool) -> Double {
    let vl = g.node(v), wl = g.node(w)
    var sum = vl.width / 2
    sum += (vl.dummy != nil ? cfg.edgesep : cfg.nodesep) / 2
    sum += (wl.dummy != nil ? cfg.edgesep : cfg.nodesep) / 2
    sum += wl.width / 2
    return sum
  }

  var xss: [(String, Coords)] = []
  for vert in ["u", "d"] {
    var adjusted = vert == "u" ? layering : layering.reversed()
    for horiz in ["l", "r"] {
      if horiz == "r" { adjusted = adjusted.map { $0.reversed() } }
      let neighbor: (String) -> [String] = vert == "u" ? g.predecessors : g.successors

      // verticalAlignment
      var root: [String: String] = [:], align: [String: String] = [:], pos: [String: Int] = [:]
      var alignKeys: [String] = []
      for layer in adjusted {
        for (o, v) in layer.enumerated() { root[v] = v; align[v] = v; pos[v] = o; alignKeys.append(v) }
      }
      for layer in adjusted {
        var prevIdx = -1
        for v in layer {
          var ws = neighbor(v)
          guard !ws.isEmpty else { continue }
          ws = ws.enumerated().sorted { pos[$0.element]! != pos[$1.element]! ? pos[$0.element]! < pos[$1.element]! : $0.offset < $1.offset }.map(\.element)
          let mp = Double(ws.count - 1) / 2
          var i = Int(mp.rounded(.down))
          while i <= Int(mp.rounded(.up)) {
            let w = ws[i]
            if align[v] == v && prevIdx < pos[w]! && !hasConflict(v, w) {
              align[w] = v
              root[v] = root[w]
              align[v] = root[w]
              prevIdx = pos[w]!
            }
            i += 1
          }
        }
      }

      // horizontalCompaction over the block graph
      let reverseSep = horiz == "r"
      var blockNodes = JSKeys()
      var blockEdges: [String: [String: Double]] = [:]  // u -> v -> sep
      var blockPreds: [String: JSKeys] = [:], blockSucs: [String: JSKeys] = [:]
      for layer in adjusted {
        var u: String?
        for v in layer {
          let vRoot = root[v]!
          blockNodes.insert(vRoot)
          if let u {
            let uRoot = root[u]!
            let prev = blockEdges[uRoot]?[vRoot]
            blockEdges[uRoot, default: [:]][vRoot] = max(sep(v, u, reverseSep), prev ?? 0)
            blockSucs[uRoot, default: JSKeys()].insert(vRoot)
            blockPreds[vRoot, default: JSKeys()].insert(uRoot)
          }
          u = v
        }
      }
      var xs = Coords()
      func iterate(_ setX: (String) -> Void, _ next: (String) -> [String]) {
        var stack = blockNodes.ordered
        var visited: Set<String> = []
        while let elem = stack.popLast() {
          if visited.contains(elem) {
            setX(elem)
          } else {
            visited.insert(elem)
            stack.append(elem)
            stack += next(elem)
          }
        }
      }
      iterate({ elem in
        var acc = 0.0
        for p in blockPreds[elem]?.ordered ?? [] { acc = max(acc, xs[p]! + blockEdges[p]![elem]!) }
        xs[elem] = acc
      }, { blockPreds[$0]?.ordered ?? [] })
      iterate({ elem in
        var m = Double.infinity
        for s in blockSucs[elem]?.ordered ?? [] { m = min(m, xs[s]! - blockEdges[elem]![s]!) }
        if m != .infinity { xs[elem] = max(xs[elem]!, m) }
      }, { blockSucs[$0]?.ordered ?? [] })
      for k in alignKeys {
        let v = align[k]!
        xs[v] = xs[root[v]!]
      }
      if horiz == "r" { xs = xs.map { -$0 } }
      xss.append((vert + horiz, xs))
    }
  }

  // findSmallestWidthAlignment
  var smallest = 0
  var smallestWidth = Double.infinity
  for (i, (_, xs)) in xss.enumerated() {
    var hi = -Double.infinity, lo = Double.infinity
    for k in xs.keys {
      let half = g.node(k).width / 2
      hi = max(hi, xs[k]! + half)
      lo = min(lo, xs[k]! - half)
    }
    if hi - lo < smallestWidth { smallestWidth = hi - lo; smallest = i }
  }
  // alignCoordinates
  let alignTo = xss[smallest].1
  let toMin = alignTo.all.min() ?? 0, toMax = alignTo.all.max() ?? 0
  for i in xss.indices where i != smallest {
    let xs = xss[i].1
    let delta = xss[i].0.hasSuffix("l") ? toMin - (xs.all.min() ?? 0) : toMax - (xs.all.max() ?? 0)
    if delta != 0 { xss[i].1 = xs.map { $0 + delta } }
  }
  // balance: the mean of the two middle candidates
  let ul = xss[0].1
  return ul.keys.map { v in
    let sorted = xss.map { $0.1[v]! }.sorted()
    return (v, (sorted[1] + sorted[2]) / 2)
  }
}
