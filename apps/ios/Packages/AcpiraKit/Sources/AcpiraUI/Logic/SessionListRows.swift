import AcpiraModels

/// The desktop's `SessionList.tsx` cutRows: keep attention and the open chat outside paging, plus a small live tail.
struct SessionListRows {
  let rows: [SessionSummary]
  let rest: [SessionSummary]
  let canFold: Bool

  init(_ sessions: [SessionSummary], cut: Int, extra: Int, activeId: String?) {
    let beyond = sessions.dropFirst(cut + extra)
    let attention: (SessionSummary) -> Bool = { isUnread($0) || $0.state == .waiting || $0.state == .error }
    let live = beyond.filter { ($0.state == .working || $0.state == .background) && !attention($0) }
    var kept = Set(beyond.filter(attention).map(\.id))
    // Match LIVE_KEEP: many pure live sessions stay paged, but attention is never capped.
    if live.count <= 5 { kept.formUnion(live.map(\.id)) }
    if let activeId, beyond.contains(where: { $0.id == activeId }) { kept.insert(activeId) }
    let pageable = sessions.dropFirst(cut).filter { !kept.contains($0.id) }
    rows = Array(sessions.prefix(cut)) + Array(pageable.prefix(extra)) + beyond.filter { kept.contains($0.id) }
    rest = Array(pageable.dropFirst(extra))
    canFold = !pageable.isEmpty
  }
}
