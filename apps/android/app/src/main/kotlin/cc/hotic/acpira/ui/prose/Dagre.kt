package cc.hotic.acpira.ui.prose

import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.floor

/*
 * A port of the dagre layout mermaid 11 runs for flowcharts (dagre-d3-es 7.0.14, src/dagre), so a diagram's nodes,
 * edge bends and label spots land where the webview's SVG puts them. Clusters (subgraphs) are not ported; everything
 * else follows the JS step by step, including graphlib's iteration orders, which decide ties: insertion order, except
 * that integer-like keys of a JS object come first in ascending order.
 *
 * Ported from the iOS app's `AcpiraUI/Prose/Mermaid/Dagre.swift`, function by function.
 */

/** A point in layout units (CGPoint in the Swift) */
data class DPoint(val x: Double, val y: Double)

/** A JS object's key order: canonical array-index keys ascending, then the rest in insertion order */
class JSKeys {
  val inserted = mutableListOf<String>()
  private val present = HashSet<String>()

  val ordered: List<String>
    get() {
      val numeric = inserted.filter(::isIndex).sortedBy { it.toLong() }
      return if (numeric.isEmpty()) inserted.toList() else numeric + inserted.filter { !isIndex(it) }
    }

  fun contains(k: String) = k in present

  fun insert(k: String) {
    if (!present.add(k)) return
    inserted.add(k)
  }

  fun remove(k: String) {
    if (!present.remove(k)) return
    inserted.remove(k)
  }

  companion object {
    fun isIndex(k: String): Boolean {
      val n = k.toLongOrNull() ?: return false
      if (n < 0 || n >= 0xFFFF_FFFFL) return false
      return n.toString() == k
    }
  }
}

/** JS string comparison (UTF-16 code units, which is what Kotlin's String.compareTo compares) */
fun jsLess(a: String, b: String): Boolean = a < b

class DNode(var width: Double = 0.0, var height: Double = 0.0) {
  var rank: Int? = null
  var order: Int? = null
  var x = 0.0
  var y = 0.0
  /** "edge", "edge-label", "edge-proxy", "root" */
  var dummy: String? = null
  var edgeLabel: DEdgeLabel? = null
  var edgeObj: DEdgeKey? = null
  /** Network simplex tree labels */
  var low = 0
  var lim = 0
  var parent: String? = null
}

class DEdgeLabel(var minlen: Int = 1, var weight: Double = 1.0, var width: Double = 0.0, var height: Double = 0.0) {
  var labelRank: Double? = null
  var points: MutableList<DPoint> = mutableListOf()
  var x: Double? = null
  var y: Double? = null
  var reversed = false
  var forwardName: String? = null
  var cutvalue = 0.0
  /** The caller's edge index */
  var tag = -1
}

data class DEdgeKey(val v: String, val w: String, val name: String? = null)

class DGraph(val directed: Boolean = true, val multigraph: Boolean = false) {
  private val nodeKeys = JSKeys()
  private val nodeLabels = HashMap<String, DNode>()
  private val edgeKeys = JSKeys()
  private val edgeObjs = HashMap<String, DEdgeKey>()
  private val edgeLabels = HashMap<String, DEdgeLabel>()
  private val inMap = HashMap<String, JSKeys>()
  private val outMap = HashMap<String, JSKeys>()
  private val preds = HashMap<String, HashMap<String, Int>>()
  private val predKeys = HashMap<String, JSKeys>()
  private val sucs = HashMap<String, HashMap<String, Int>>()
  private val sucKeys = HashMap<String, JSKeys>()

  // Nodes

  val nodes: List<String> get() = nodeKeys.ordered
  val nodeCount: Int get() = nodeLabels.size
  fun hasNode(v: String) = nodeLabels.containsKey(v)
  fun node(v: String): DNode = nodeLabels.getValue(v)

  fun setNode(v: String, label: DNode? = null): DNode {
    val existing = nodeLabels[v]
    if (existing != null) {
      if (label != null) {
        nodeLabels[v] = label
        return label
      }
      return existing
    }
    val l = label ?: DNode()
    nodeKeys.insert(v)
    nodeLabels[v] = l
    inMap[v] = JSKeys(); outMap[v] = JSKeys()
    preds[v] = HashMap(); predKeys[v] = JSKeys()
    sucs[v] = HashMap(); sucKeys[v] = JSKeys()
    return l
  }

  fun removeNode(v: String) {
    if (!nodeLabels.containsKey(v)) return
    for (id in inMap.getValue(v).ordered) removeEdgeId(id)
    for (id in outMap.getValue(v).ordered) removeEdgeId(id)
    nodeKeys.remove(v)
    nodeLabels.remove(v)
    inMap.remove(v); outMap.remove(v)
    preds.remove(v); predKeys.remove(v)
    sucs.remove(v); sucKeys.remove(v)
  }

  val sources: List<String> get() = nodes.filter { inMap.getValue(it).inserted.isEmpty() }

  fun predecessors(v: String): List<String> = predKeys[v]?.ordered ?: emptyList()
  fun successors(v: String): List<String> = sucKeys[v]?.ordered ?: emptyList()
  fun neighbors(v: String): List<String> {
    val p = predecessors(v)
    val seen = p.toHashSet()
    return p + successors(v).filter { it !in seen }
  }

  // Edges

  private fun edgeId(v: String, w: String, name: String?): String {
    var a = v
    var b = w
    if (!directed && jsLess(b, a)) { a = w; b = v }
    return a + "\u0001" + b + "\u0001" + (name ?: "\u0000")
  }

  val edges: List<DEdgeKey> get() = edgeKeys.ordered.map { edgeObjs.getValue(it) }

  fun setEdge(v: String, w: String, label: DEdgeLabel, name: String? = null) {
    val n = if (multigraph) name else null
    val id = edgeId(v, w, n)
    if (edgeLabels.containsKey(id)) {
      edgeLabels[id] = label
      return
    }
    setNode(v)
    setNode(w)
    var a = v
    var b = w
    if (!directed && jsLess(b, a)) { a = w; b = v }
    val key = DEdgeKey(a, b, n)
    edgeKeys.insert(id)
    edgeObjs[id] = key
    edgeLabels[id] = label
    preds.getValue(b).merge(a, 1, Int::plus)
    predKeys.getValue(b).insert(a)
    sucs.getValue(a).merge(b, 1, Int::plus)
    sucKeys.getValue(a).insert(b)
    inMap.getValue(b).insert(id)
    outMap.getValue(a).insert(id)
  }

  fun setEdge(e: DEdgeKey, label: DEdgeLabel) = setEdge(e.v, e.w, label, e.name)

  fun edge(v: String, w: String, name: String? = null): DEdgeLabel? = edgeLabels[edgeId(v, w, name)]
  fun edge(e: DEdgeKey): DEdgeLabel = edgeLabels.getValue(edgeId(e.v, e.w, e.name))
  fun hasEdge(v: String, w: String, name: String? = null) = edgeLabels.containsKey(edgeId(v, w, name))

  fun removeEdge(e: DEdgeKey) = removeEdgeId(edgeId(e.v, e.w, e.name))
  fun removeEdge(v: String, w: String) = removeEdgeId(edgeId(v, w, null))

  private fun removeEdgeId(id: String) {
    val e = edgeObjs[id] ?: return
    edgeKeys.remove(id)
    edgeObjs.remove(id)
    edgeLabels.remove(id)
    val p = preds.getValue(e.w)
    p[e.v] = p.getValue(e.v) - 1
    if (p.getValue(e.v) == 0) { p.remove(e.v); predKeys.getValue(e.w).remove(e.v) }
    val s = sucs.getValue(e.v)
    s[e.w] = s.getValue(e.w) - 1
    if (s.getValue(e.w) == 0) { s.remove(e.w); sucKeys.getValue(e.v).remove(e.w) }
    inMap.getValue(e.w).remove(id)
    outMap.getValue(e.v).remove(id)
  }

  fun inEdges(v: String): List<DEdgeKey> = inMap[v]?.ordered?.map { edgeObjs.getValue(it) } ?: emptyList()
  fun outEdges(v: String): List<DEdgeKey> = outMap[v]?.ordered?.map { edgeObjs.getValue(it) } ?: emptyList()
  fun nodeEdges(v: String): List<DEdgeKey> = inEdges(v) + outEdges(v)
}

// Layout

enum class RankDir { TB, BT, LR, RL }

data class DagreConfig(
  var rankdir: RankDir = RankDir.TB,
  var nodesep: Double = 50.0,
  var edgesep: Double = 20.0,
  var ranksep: Double = 50.0,
  var marginx: Double = 8.0,
  var marginy: Double = 8.0,
)

/** A laid-out graph's width and height */
data class DSize(val width: Double, val height: Double)

/**
 * Runs dagre's layout on `g` in place: nodes get x / y, edges points (and x / y for labelled ones); returns the
 * graph's width and height
 */
fun dagreLayout(g: DGraph, config: DagreConfig): DSize {
  var counter = 0
  val cfg = config.copy()
  fun uniqueId(prefix: String): String {
    counter += 1
    return prefix + counter
  }
  fun addDummyNode(type: String, label: DNode, prefix: String): String {
    var v: String
    do { v = uniqueId(prefix) } while (g.hasNode(v))
    label.dummy = type
    g.setNode(v, label)
    return v
  }

  // makeSpaceForEdgeLabels (mermaid flowchart edges are labelpos "c": no label offset)
  cfg.ranksep /= 2
  for (e in g.edges) g.edge(e).minlen *= 2

  // removeSelfEdges: mermaid splits self loops into extra nodes before dagre, so none arrive here
  for (e in g.edges) if (e.v == e.w) g.removeEdge(e)

  // acyclic.run (dfsFAS)
  val fas = mutableListOf<DEdgeKey>()
  run {
    val stack = HashSet<String>()
    val visited = HashSet<String>()
    fun dfs(v: String) {
      if (!visited.add(v)) return
      stack.add(v)
      for (e in g.outEdges(v)) {
        if (e.w in stack) fas.add(e) else dfs(e.w)
      }
      stack.remove(v)
    }
    for (v in g.nodes) dfs(v)
  }
  for (e in fas) {
    val label = g.edge(e)
    g.removeEdge(e)
    label.forwardName = e.name
    label.reversed = true
    g.setEdge(e.w, e.v, label, uniqueId("rev"))
  }

  // nestingGraph.run without clusters: a root above every node, weight 0, nodeSep 1
  val root = addDummyNode("root", DNode(), "_root")
  for (v in g.nodes) if (v != root) g.setEdge(root, v, DEdgeLabel(minlen = 1, weight = 0.0))

  // rank (network simplex)
  networkSimplex(g)

  // injectEdgeLabelProxies
  val proxies = mutableListOf<Triple<String, DEdgeKey, Double>>()
  for (e in g.edges) {
    val edge = g.edge(e)
    if (edge.width != 0.0 && edge.height != 0.0) {
      val v = g.node(e.v)
      val w = g.node(e.w)
      val rank = (w.rank!! - v.rank!!).toDouble() / 2 + v.rank!!.toDouble()
      val id = addDummyNode("edge-proxy", DNode(), "_ep")
      proxies.add(Triple(id, e, rank))
    }
  }
  // removeEmptyRanks with nodeRankFactor 1 never removes a rank

  // nestingGraph.cleanup
  g.removeNode(root)

  // normalizeRanks (proxies carry fractional ranks, kept aside)
  val realMin = g.nodes.mapNotNull { g.node(it).rank }.minOrNull() ?: 0
  for (v in g.nodes) g.node(v).rank?.let { g.node(v).rank = it - realMin }

  // removeEdgeLabelProxies
  for ((id, e, rank) in proxies) {
    g.edge(e).labelRank = rank - realMin
    g.removeNode(id)
  }

  // normalize.run
  val dummyChains = mutableListOf<String>()
  for (e in g.edges) {
    var v = e.v
    var vRank = g.node(v).rank!!
    val w = e.w
    val wRank = g.node(w).rank!!
    val edgeLabel = g.edge(e)
    if (wRank == vRank + 1) continue
    g.removeEdge(e)
    var i = 0
    vRank += 1
    while (vRank < wRank) {
      edgeLabel.points = mutableListOf()
      val attrs = DNode()
      attrs.edgeLabel = edgeLabel
      attrs.edgeObj = e
      attrs.rank = vRank
      val dummy = addDummyNode("edge", attrs, "_d")
      val lr = edgeLabel.labelRank
      if (lr != null && vRank.toDouble() == lr) {
        attrs.width = edgeLabel.width
        attrs.height = edgeLabel.height
        attrs.dummy = "edge-label"
      }
      g.setEdge(v, dummy, DEdgeLabel(weight = edgeLabel.weight), e.name)
      if (i == 0) dummyChains.add(dummy)
      v = dummy
      i += 1
      vRank += 1
    }
    g.setEdge(v, w, DEdgeLabel(weight = edgeLabel.weight), e.name)
  }

  order(g)

  // adjustCoordinateSystem
  if (cfg.rankdir == RankDir.LR || cfg.rankdir == RankDir.RL) swapWidthHeight(g)

  position(g, cfg)

  // normalize.undo
  for (start in dummyChains) {
    var v = start
    var node = g.node(v)
    val origLabel = node.edgeLabel ?: continue
    val edgeObj = node.edgeObj ?: continue
    g.setEdge(edgeObj, origLabel)
    while (node.dummy != null) {
      val w = g.successors(v)[0]
      g.removeNode(v)
      origLabel.points.add(DPoint(node.x, node.y))
      if (node.dummy == "edge-label") {
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
  if (cfg.rankdir == RankDir.BT || cfg.rankdir == RankDir.RL) {
    for (v in g.nodes) g.node(v).y = -g.node(v).y
    for (e in g.edges) {
      val edge = g.edge(e)
      edge.points = edge.points.map { DPoint(it.x, -it.y) }.toMutableList()
      edge.y?.let { edge.y = -it }
    }
  }
  if (cfg.rankdir == RankDir.LR || cfg.rankdir == RankDir.RL) {
    for (v in g.nodes) {
      val n = g.node(v)
      val x = n.x
      n.x = n.y
      n.y = x
    }
    for (e in g.edges) {
      val edge = g.edge(e)
      edge.points = edge.points.map { DPoint(it.y, it.x) }.toMutableList()
      val x = edge.x
      val y = edge.y
      if (x != null && y != null) { edge.x = y; edge.y = x }
    }
    swapWidthHeight(g)
  }

  // translateGraph
  var minX = Double.POSITIVE_INFINITY
  var maxX = 0.0
  var minY = Double.POSITIVE_INFINITY
  var maxY = 0.0
  fun extremes(x: Double, y: Double, w: Double, h: Double) {
    minX = minOf(minX, x - w / 2); maxX = maxOf(maxX, x + w / 2)
    minY = minOf(minY, y - h / 2); maxY = maxOf(maxY, y + h / 2)
  }
  for (v in g.nodes) { val n = g.node(v); extremes(n.x, n.y, n.width, n.height) }
  for (e in g.edges) {
    val edge = g.edge(e)
    val x = edge.x
    val y = edge.y
    if (x != null && y != null) extremes(x, y, edge.width, edge.height)
  }
  minX -= cfg.marginx
  minY -= cfg.marginy
  for (v in g.nodes) { val n = g.node(v); n.x -= minX; n.y -= minY }
  for (e in g.edges) {
    val edge = g.edge(e)
    edge.points = edge.points.map { DPoint(it.x - minX, it.y - minY) }.toMutableList()
    edge.x?.let { edge.x = it - minX }
    edge.y?.let { edge.y = it - minY }
  }
  val size = DSize(maxX - minX + cfg.marginx, maxY - minY + cfg.marginy)

  // assignNodeIntersects
  for (e in g.edges) {
    val edge = g.edge(e)
    val nodeV = g.node(e.v)
    val nodeW = g.node(e.w)
    val p1: DPoint
    val p2: DPoint
    if (edge.points.isEmpty()) {
      p1 = DPoint(nodeW.x, nodeW.y)
      p2 = DPoint(nodeV.x, nodeV.y)
    } else {
      p1 = edge.points[0]
      p2 = edge.points[edge.points.size - 1]
    }
    edge.points.add(0, intersectRect(nodeV, p1))
    edge.points.add(intersectRect(nodeW, p2))
  }

  // reversePoints + acyclic.undo
  for (e in g.edges) {
    val label = g.edge(e)
    if (!label.reversed) continue
    label.points.reverse()
    g.removeEdge(e)
    label.reversed = false
    val name = label.forwardName
    label.forwardName = null
    g.setEdge(e.w, e.v, label, name)
  }
  return size
}

private fun swapWidthHeight(g: DGraph) {
  for (v in g.nodes) { val n = g.node(v); val w = n.width; n.width = n.height; n.height = w }
  for (e in g.edges) { val l = g.edge(e); val w = l.width; l.width = l.height; l.height = w }
}

/** dagre util.intersectRect */
fun intersectRect(n: DNode, point: DPoint): DPoint {
  val dx = point.x - n.x
  val dy = point.y - n.y
  var w = n.width / 2
  var h = n.height / 2
  if (dx == 0.0 && dy == 0.0) return DPoint(n.x, n.y)
  if (abs(dy) * w > abs(dx) * h) {
    if (dy < 0) h = -h
    return DPoint(n.x + h * dx / dy, n.y + h)
  }
  if (dx < 0) w = -w
  return DPoint(n.x + w, n.y + w * dy / dx)
}

// Rank (network simplex)

private fun slack(g: DGraph, e: DEdgeKey): Int = g.node(e.w).rank!! - g.node(e.v).rank!! - g.edge(e).minlen

private fun networkSimplex(input: DGraph) {
  // simplify: one edge per node pair, weights summed, the longest minlen
  val g = DGraph()
  for (v in input.nodes) g.setNode(v, input.node(v))
  for (e in input.edges) {
    val prev = g.edge(e.v, e.w)
    val label = input.edge(e)
    g.setEdge(e.v, e.w, DEdgeLabel(minlen = maxOf(prev?.minlen ?: 1, label.minlen), weight = (prev?.weight ?: 0.0) + label.weight))
  }

  longestPath(g)
  val t = feasibleTree(g)
  initLowLimValues(t)
  initCutValues(t, g)
  while (true) {
    val e = t.edges.firstOrNull { t.edge(it).cutvalue < 0 } ?: break
    val f = enterEdge(t, g, e)
    // exchangeEdges
    t.removeEdge(e.v, e.w)
    t.setEdge(f.v, f.w, DEdgeLabel())
    initLowLimValues(t)
    initCutValues(t, g)
    updateRanks(t, g)
  }
}

private fun longestPath(g: DGraph) {
  val visited = HashSet<String>()
  fun dfs(v: String): Int {
    val label = g.node(v)
    if (v in visited) return label.rank!!
    visited.add(v)
    val ranks = g.outEdges(v).map { dfs(it.w) - g.edge(it).minlen }
    val rank = ranks.minOrNull() ?: 0
    label.rank = rank
    return rank
  }
  for (v in g.sources) dfs(v)
}

private fun feasibleTree(g: DGraph): DGraph {
  val t = DGraph(directed = false)
  val start = g.nodes[0]
  val size = g.nodeCount
  t.setNode(start, DNode())

  fun tightTree(): Int {
    fun dfs(v: String) {
      for (e in g.nodeEdges(v)) {
        val w = if (v == e.v) e.w else e.v
        if (!t.hasNode(w) && slack(g, e) == 0) {
          t.setNode(w, DNode())
          t.setEdge(v, w, DEdgeLabel())
          dfs(w)
        }
      }
    }
    for (v in t.nodes) dfs(v)
    return t.nodeCount
  }

  while (tightTree() < size) {
    // findMinSlackEdge: the first edge with one end in the tree and the least slack
    var best: DEdgeKey? = null
    var bestSlack = Int.MAX_VALUE
    for (e in g.edges) {
      if (t.hasNode(e.v) == t.hasNode(e.w)) continue
      val s = slack(g, e)
      if (s < bestSlack) { best = e; bestSlack = s }
    }
    val edge = best ?: break
    val delta = if (t.hasNode(edge.v)) slack(g, edge) else -slack(g, edge)
    for (v in t.nodes) g.node(v).rank = g.node(v).rank!! + delta
  }
  return t
}

private fun initLowLimValues(tree: DGraph) {
  val visited = HashSet<String>()
  fun dfs(nextLim: Int, v: String, parent: String?): Int {
    var next = nextLim
    val low = next
    val label = tree.node(v)
    visited.add(v)
    for (w in tree.neighbors(v)) {
      if (w !in visited) next = dfs(next, w, v)
    }
    label.low = low
    label.lim = next
    next += 1
    label.parent = parent
    return next
  }
  dfs(1, tree.nodes[0], null)
}

/** graphlib alg.postorder / preorder over the undirected tree */
private fun treeOrder(t: DGraph, roots: List<String>, post: Boolean): MutableList<String> {
  val acc = mutableListOf<String>()
  val visited = HashSet<String>()
  fun walk(v: String) {
    if (!visited.add(v)) return
    if (!post) acc.add(v)
    for (w in t.neighbors(v)) walk(w)
    if (post) acc.add(v)
  }
  for (v in roots) walk(v)
  return acc
}

private fun initCutValues(t: DGraph, g: DGraph) {
  val vs = treeOrder(t, t.nodes, post = true)
  vs.removeAt(vs.size - 1)
  for (child in vs) {
    val parent = t.node(child).parent!!
    t.edge(child, parent)!!.cutvalue = calcCutValue(t, g, child)
  }
}

private fun calcCutValue(t: DGraph, g: DGraph, child: String): Double {
  val parent = t.node(child).parent!!
  var childIsTail = true
  var graphEdge = g.edge(child, parent)
  if (graphEdge == null) {
    childIsTail = false
    graphEdge = g.edge(parent, child)
  }
  var cutValue = graphEdge!!.weight
  for (e in g.nodeEdges(child)) {
    val isOutEdge = e.v == child
    val other = if (isOutEdge) e.w else e.v
    if (other == parent) continue
    val pointsToHead = isOutEdge == childIsTail
    val otherWeight = g.edge(e).weight
    cutValue += if (pointsToHead) otherWeight else -otherWeight
    if (t.hasEdge(child, other)) {
      val otherCut = t.edge(child, other)!!.cutvalue
      cutValue += if (pointsToHead) -otherCut else otherCut
    }
  }
  return cutValue
}

private fun enterEdge(t: DGraph, g: DGraph, edge: DEdgeKey): DEdgeKey {
  var v = edge.v
  var w = edge.w
  if (!g.hasEdge(v, w)) { v = edge.w; w = edge.v }
  val vLabel = t.node(v)
  val wLabel = t.node(w)
  var tail = vLabel
  var flip = false
  if (vLabel.lim > wLabel.lim) { tail = wLabel; flip = true }
  fun isDescendant(n: DNode) = tail.low <= n.lim && n.lim <= tail.lim
  var best: DEdgeKey? = null
  var bestSlack = Int.MAX_VALUE
  for (e in g.edges) {
    if (!(flip == isDescendant(t.node(e.v)) && flip != isDescendant(t.node(e.w)))) continue
    val s = slack(g, e)
    if (s < bestSlack) { best = e; bestSlack = s }
  }
  return best!!
}

private fun updateRanks(t: DGraph, g: DGraph) {
  // Node labels carry no compound parent, so the root is the tree's first node
  val root = t.nodes[0]
  for (v in treeOrder(t, listOf(root), post = false).drop(1)) {
    val parent = t.node(v).parent!!
    var flipped = false
    var edge = g.edge(v, parent)
    if (edge == null) { edge = g.edge(parent, v); flipped = true }
    g.node(v).rank = g.node(parent).rank!! + if (flipped) edge!!.minlen else -edge!!.minlen
  }
}

// Order

private fun buildLayerMatrix(g: DGraph): List<List<String>> {
  val maxRank = g.nodes.mapNotNull { g.node(it).rank }.maxOrNull() ?: -1
  val layers = List(maxRank + 1) { mutableListOf<String?>() }
  for (v in g.nodes) {
    val n = g.node(v)
    val r = n.rank ?: continue
    val o = n.order ?: continue
    while (layers[r].size <= o) layers[r].add(null)
    layers[r][o] = v
  }
  return layers.map { it.filterNotNull() }
}

/** A rank's nodes with the weights toward their neighbours in the fixed layer, in graphlib's order */
private class LayerGraph(val nodes: List<String>, val inEdges: Map<String, List<Pair<String, Double>>>)

private class SortEntry(val vs: List<String>, val i: Int, val barycenter: Double?, val weight: Double)

private fun order(g: DGraph) {
  val maxRank = g.nodes.mapNotNull { g.node(it).rank }.maxOrNull() ?: 0

  // initOrder: DFS along successors from the nodes sorted by rank
  val layers = List(maxRank + 1) { mutableListOf<String>() }
  val visited = HashSet<String>()
  fun dfs(v: String) {
    if (!visited.add(v)) return
    layers[g.node(v).rank!!].add(v)
    for (w in g.successors(v)) dfs(w)
  }
  val ordered = g.nodes.withIndex().sortedWith { a, b ->
    val ra = g.node(a.value).rank!!
    val rb = g.node(b.value).rank!!
    if (ra != rb) ra.compareTo(rb) else a.index.compareTo(b.index)
  }.map { it.value }
  for (v in ordered) dfs(v)
  assignOrder(g, layers)

  fun layerGraph(rank: Int, incoming: Boolean): LayerGraph {
    val nodes = mutableListOf<String>()
    val inEdges = HashMap<String, List<Pair<String, Double>>>()
    for (v in g.nodes) {
      if (g.node(v).rank != rank) continue
      nodes.add(v)
      val list = mutableListOf<Pair<String, Double>>()
      for (e in if (incoming) g.inEdges(v) else g.outEdges(v)) {
        val u = if (e.v == v) e.w else e.v
        val i = list.indexOfFirst { it.first == u }
        if (i >= 0) list[i] = u to (list[i].second + g.edge(e).weight) else list.add(u to g.edge(e).weight)
      }
      inEdges[v] = list
    }
    return LayerGraph(nodes, inEdges)
  }
  val down = if (maxRank >= 1) (1..maxRank).map { layerGraph(it, true) } else emptyList()
  val up = if (maxRank >= 1) (0..maxRank - 1).reversed().map { layerGraph(it, false) } else emptyList()

  fun sweep(graphs: List<LayerGraph>, biasRight: Boolean) {
    for (lg in graphs) {
      val entries = mutableListOf<SortEntry>()
      for ((i, v) in lg.nodes.withIndex()) {
        val ins = lg.inEdges[v] ?: emptyList()
        if (ins.isEmpty()) {
          entries.add(SortEntry(listOf(v), i, null, 0.0))
        } else {
          var sum = 0.0
          var weight = 0.0
          for ((u, w) in ins) { sum += w * g.node(u).order!!.toDouble(); weight += w }
          entries.add(SortEntry(listOf(v), i, sum / weight, weight))
        }
      }
      // sort.js
      val sortable = entries.filter { it.barycenter != null }.sortedWith { a, b ->
        if (a.barycenter!! != b.barycenter!!) a.barycenter.compareTo(b.barycenter)
        else if (biasRight) b.i.compareTo(a.i) else a.i.compareTo(b.i)
      }
      val unsortable = entries.filter { it.barycenter == null }.sortedByDescending { it.i }.toMutableList()
      val vs = mutableListOf<String>()
      var index = 0
      fun consume() {
        while (true) {
          val last = unsortable.lastOrNull() ?: break
          if (last.i > index) break
          unsortable.removeAt(unsortable.size - 1)
          vs += last.vs
          index += 1
        }
      }
      consume()
      for (entry in sortable) {
        index += entry.vs.size
        vs += entry.vs
        consume()
      }
      for ((i, v) in vs.withIndex()) g.node(v).order = i
    }
  }

  var bestCC = Double.POSITIVE_INFINITY
  var best = buildLayerMatrix(g)
  var i = 0
  var lastBest = 0
  while (lastBest < 4) {
    sweep(if (i % 2 == 1) down else up, i % 4 >= 2)
    val layering = buildLayerMatrix(g)
    val cc = crossCount(g, layering)
    if (cc < bestCC) {
      lastBest = 0
      best = layering
      bestCC = cc
    }
    i += 1
    lastBest += 1
  }
  assignOrder(g, best)
}

private fun assignOrder(g: DGraph, layering: List<List<String>>) {
  for (layer in layering) {
    for ((i, v) in layer.withIndex()) g.node(v).order = i
  }
}

private fun crossCount(g: DGraph, layering: List<List<String>>): Double {
  var cc = 0.0
  if (layering.size <= 1) return 0.0
  for (i in 1 until layering.size) {
    val south = layering[i]
    val southPos = HashMap<String, Int>()
    for ((j, v) in south.withIndex()) southPos[v] = j
    val entries = mutableListOf<Pair<Int, Double>>()
    for (v in layering[i - 1]) {
      entries += g.outEdges(v).mapNotNull { e -> southPos[e.w]?.let { it to g.edge(e).weight } }
        .withIndex()
        .sortedWith { a, b -> if (a.value.first != b.value.first) a.value.first.compareTo(b.value.first) else a.index.compareTo(b.index) }
        .map { it.value }
    }
    var firstIndex = 1
    while (firstIndex < south.size) firstIndex = firstIndex shl 1
    val treeSize = 2 * firstIndex - 1
    firstIndex -= 1
    val tree = DoubleArray(treeSize)
    for ((pos, weight) in entries) {
      var index = pos + firstIndex
      tree[index] += weight
      var weightSum = 0.0
      while (index > 0) {
        if (index % 2 == 1) weightSum += tree[index + 1]
        index = (index - 1) shr 1
        tree[index] += weight
      }
      cc += weight * weightSum
    }
  }
  return cc
}

// Position

private fun position(g: DGraph, cfg: DagreConfig) {
  val layering = buildLayerMatrix(g)
  // positionY
  var prevY = 0.0
  for (layer in layering) {
    val maxHeight = layer.maxOfOrNull { g.node(it).height } ?: 0.0
    for (v in layer) g.node(v).y = prevY + maxHeight / 2
    prevY += maxHeight + cfg.ranksep
  }
  for ((v, x) in positionX(g, layering, cfg)) g.node(v).x = x
}

/** One alignment's coordinates, keyed in the JS object's insertion order */
private class Coords {
  val keys = mutableListOf<String>()
  val values = HashMap<String, Double>()
  operator fun get(v: String): Double? = values[v]
  operator fun set(v: String, value: Double?) {
    if (value == null) return
    if (!values.containsKey(v)) keys.add(v)
    values[v] = value
  }
  val all: List<Double> get() = keys.map { values.getValue(it) }
  fun map(f: (Double) -> Double): Coords {
    val out = Coords()
    for (k in keys) out[k] = f(values.getValue(k))
    return out
  }
}

/** Brandes–Köpf, as bk.js */
private fun positionX(g: DGraph, layering: List<List<String>>, cfg: DagreConfig): List<Pair<String, Double>> {
  // Type 1 conflicts: a non-inner segment crossing an inner one (between dummies)
  val conflicts = HashMap<String, HashSet<String>>()
  fun addConflict(a: String, b: String) {
    val (v, w) = if (jsLess(b, a)) b to a else a to b
    conflicts.getOrPut(v) { HashSet() }.add(w)
  }
  fun hasConflict(a: String, b: String): Boolean {
    val (v, w) = if (jsLess(b, a)) b to a else a to b
    return conflicts[v]?.contains(w) ?: false
  }
  if (layering.size > 1) {
    for (li in 1 until layering.size) {
      val prevLayer = layering[li - 1]
      val layer = layering[li]
      var k0 = 0
      var scanPos = 0
      val lastNode = layer.lastOrNull()
      for ((i, v) in layer.withIndex()) {
        val w = if (g.node(v).dummy != null) g.predecessors(v).firstOrNull { g.node(it).dummy != null } else null
        val k1 = w?.let { g.node(it).order!! } ?: prevLayer.size
        if (w != null || v == lastNode) {
          for (scanNode in layer.subList(scanPos, i + 1)) {
            for (u in g.predecessors(scanNode)) {
              val uLabel = g.node(u)
              val uPos = uLabel.order!!
              if ((uPos < k0 || k1 < uPos) && !(uLabel.dummy != null && g.node(scanNode).dummy != null)) {
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

  fun sep(v: String, w: String): Double {
    val vl = g.node(v)
    val wl = g.node(w)
    var sum = vl.width / 2
    sum += (if (vl.dummy != null) cfg.edgesep else cfg.nodesep) / 2
    sum += (if (wl.dummy != null) cfg.edgesep else cfg.nodesep) / 2
    sum += wl.width / 2
    return sum
  }

  val xss = mutableListOf<Pair<String, Coords>>()
  for (vert in listOf("u", "d")) {
    var adjusted: List<List<String>> = if (vert == "u") layering else layering.reversed()
    for (horiz in listOf("l", "r")) {
      if (horiz == "r") adjusted = adjusted.map { it.reversed() }
      val neighbor: (String) -> List<String> = if (vert == "u") g::predecessors else g::successors

      // verticalAlignment
      val root = HashMap<String, String>()
      val align = HashMap<String, String>()
      val pos = HashMap<String, Int>()
      val alignKeys = mutableListOf<String>()
      for (layer in adjusted) {
        for ((o, v) in layer.withIndex()) { root[v] = v; align[v] = v; pos[v] = o; alignKeys.add(v) }
      }
      for (layer in adjusted) {
        var prevIdx = -1
        for (v in layer) {
          var ws = neighbor(v)
          if (ws.isEmpty()) continue
          ws = ws.withIndex().sortedWith { a, b ->
            val pa = pos.getValue(a.value)
            val pb = pos.getValue(b.value)
            if (pa != pb) pa.compareTo(pb) else a.index.compareTo(b.index)
          }.map { it.value }
          val mp = (ws.size - 1).toDouble() / 2
          var i = floor(mp).toInt()
          while (i <= ceil(mp).toInt()) {
            val w = ws[i]
            if (align[v] == v && prevIdx < pos.getValue(w) && !hasConflict(v, w)) {
              align[w] = v
              root[v] = root.getValue(w)
              align[v] = root.getValue(w)
              prevIdx = pos.getValue(w)
            }
            i += 1
          }
        }
      }

      // horizontalCompaction over the block graph
      val blockNodes = JSKeys()
      val blockEdges = HashMap<String, HashMap<String, Double>>() // u -> v -> sep
      val blockPreds = HashMap<String, JSKeys>()
      val blockSucs = HashMap<String, JSKeys>()
      for (layer in adjusted) {
        var u: String? = null
        for (v in layer) {
          val vRoot = root.getValue(v)
          blockNodes.insert(vRoot)
          if (u != null) {
            val uRoot = root.getValue(u)
            val edgesOut = blockEdges.getOrPut(uRoot) { HashMap() }
            val prev = edgesOut[vRoot]
            edgesOut[vRoot] = maxOf(sep(v, u), prev ?: 0.0)
            blockSucs.getOrPut(uRoot) { JSKeys() }.insert(vRoot)
            blockPreds.getOrPut(vRoot) { JSKeys() }.insert(uRoot)
          }
          u = v
        }
      }
      var xs = Coords()
      fun iterate(setX: (String) -> Unit, next: (String) -> List<String>) {
        val stack = blockNodes.ordered.toMutableList()
        val visited = HashSet<String>()
        while (stack.isNotEmpty()) {
          val elem = stack.removeAt(stack.size - 1)
          if (elem in visited) {
            setX(elem)
          } else {
            visited.add(elem)
            stack.add(elem)
            stack += next(elem)
          }
        }
      }
      iterate({ elem ->
        var acc = 0.0
        for (p in blockPreds[elem]?.ordered ?: emptyList()) acc = maxOf(acc, xs[p]!! + blockEdges.getValue(p).getValue(elem))
        xs[elem] = acc
      }, { blockPreds[it]?.ordered ?: emptyList() })
      iterate({ elem ->
        var m = Double.POSITIVE_INFINITY
        for (s in blockSucs[elem]?.ordered ?: emptyList()) m = minOf(m, xs[s]!! - blockEdges.getValue(elem).getValue(s))
        if (m != Double.POSITIVE_INFINITY) xs[elem] = maxOf(xs[elem]!!, m)
      }, { blockSucs[it]?.ordered ?: emptyList() })
      for (k in alignKeys) {
        val v = align.getValue(k)
        xs[v] = xs[root.getValue(v)]
      }
      if (horiz == "r") xs = xs.map { -it }
      xss.add(vert + horiz to xs)
    }
  }

  // findSmallestWidthAlignment
  var smallest = 0
  var smallestWidth = Double.POSITIVE_INFINITY
  for ((i, pair) in xss.withIndex()) {
    val xs = pair.second
    var hi = Double.NEGATIVE_INFINITY
    var lo = Double.POSITIVE_INFINITY
    for (k in xs.keys) {
      val half = g.node(k).width / 2
      hi = maxOf(hi, xs[k]!! + half)
      lo = minOf(lo, xs[k]!! - half)
    }
    if (hi - lo < smallestWidth) { smallestWidth = hi - lo; smallest = i }
  }
  // alignCoordinates
  val alignTo = xss[smallest].second
  val toMin = alignTo.all.minOrNull() ?: 0.0
  val toMax = alignTo.all.maxOrNull() ?: 0.0
  for (i in xss.indices) {
    if (i == smallest) continue
    val xs = xss[i].second
    val delta = if (xss[i].first.endsWith("l")) toMin - (xs.all.minOrNull() ?: 0.0) else toMax - (xs.all.maxOrNull() ?: 0.0)
    if (delta != 0.0) xss[i] = xss[i].first to xs.map { it + delta }
  }
  // balance: the mean of the two middle candidates
  val ul = xss[0].second
  return ul.keys.map { v ->
    val sorted = xss.map { it.second[v]!! }.sorted()
    v to (sorted[1] + sorted[2]) / 2
  }
}
