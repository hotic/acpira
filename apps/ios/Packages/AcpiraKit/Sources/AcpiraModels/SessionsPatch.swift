/// The session list as runs over the list the phone holds (`SessionsPatch` in packages/shared/protocol.ts,
/// rust/crates/acpira-shared/src/sessions_patch.rs). The engine sends it instead of the whole list once `ready` said
/// `listPatches`: the whole list is hundreds of KB and changes whenever any session starts, waits or ends
public struct SessionsPatch: Codable, Hashable, Sendable {
  /// One run of the new list: entries copied from the held list, or a new or changed entry
  public enum Run: Codable, Hashable, Sendable {
    case keep(from: Int, count: Int)
    case entry(SessionSummary)

    public init(from decoder: Decoder) throws {
      // `[from, count]` on the wire; anything else is an entry
      if var pair = try? decoder.unkeyedContainer() {
        self = .keep(from: try pair.decode(Int.self), count: try pair.decode(Int.self))
        guard pair.isAtEnd else {
          throw DecodingError.dataCorruptedError(in: pair, debugDescription: "a kept run is [from, count]")
        }
      } else {
        self = .entry(try SessionSummary(from: decoder))
      }
    }

    public func encode(to encoder: Encoder) throws {
      switch self {
      case let .keep(from, count):
        var pair = encoder.unkeyedContainer()
        try pair.encode(from)
        try pair.encode(count)
      case .entry(let summary):
        try summary.encode(to: encoder)
      }
    }
  }

  /// The list revision the patch applies to and the one it makes: a whole list is 0, each patch one more
  public var base: Int
  public var rev: Int
  /// Length of the list the patch applies to
  public var size: Int
  public var runs: [Run]

  public init(base: Int, rev: Int, size: Int, runs: [Run]) {
    self.base = base
    self.rev = rev
    self.size = size
    self.runs = runs
  }

  /// `held` (at revision `heldRev`) with the patch applied; nil when it does not fit (the phone then asks for the
  /// whole list)
  public func apply(to held: [SessionSummary], at heldRev: Int) -> [SessionSummary]? {
    guard heldRev == base, held.count == size else { return nil }
    var out: [SessionSummary] = []
    for run in runs {
      switch run {
      case let .keep(from, count):
        guard from >= 0, count >= 0, from <= held.count - count else { return nil }
        out.append(contentsOf: held[from..<from + count])
      case .entry(let summary):
        out.append(summary)
      }
    }
    return out
  }
}
