import Foundation

/// How the phone reads a file from a session's blob store (images in a transcript): `(sessionId, blobName) -> bytes`.
/// The development engine serves them over HTTP (`HTTPBlobs`); a paired connection brings its own
public typealias BlobFetch = @Sendable (_ session: String, _ name: String) async throws -> Data

/// What the phone keeps of fetched media, by key: values held under a cost budget and a count cap, the least recently
/// used dropped first. Concurrent requests for one key share one load. A failed load is not kept, so the next request
/// tries again, and a value costing more than the whole budget is handed out but never kept (Attachments.tsx's text
/// cache does the same). Blob names are content hashes, so a kept value never goes stale
@MainActor
public final class MediaCache<Key: Hashable & Sendable, Value: Sendable> {
  private struct Entry {
    var value: Value
    var cost: Int
    var used: UInt64
  }

  public let budget: Int
  public let limit: Int
  private let cost: (Value) -> Int
  private var entries: [Key: Entry] = [:]
  private var loading: [Key: Task<Value, any Error>] = [:]
  private var clock: UInt64 = 0
  /// The summed cost of the kept values
  public private(set) var total = 0

  /// budget: the summed cost the kept values may reach; limit: how many may be kept
  public init(budget: Int, limit: Int, cost: @escaping (Value) -> Int) {
    self.budget = budget
    self.limit = limit
    self.cost = cost
  }

  public var count: Int { entries.count }

  /// The kept value, marked as just used
  public func cached(_ key: Key) -> Value? {
    guard var entry = entries[key] else { return nil }
    clock += 1
    entry.used = clock
    entries[key] = entry
    return entry.value
  }

  /// The kept value, else the load already under way for `key`, else a new one from `make`. The load runs on even when
  /// the caller is cancelled, so a row scrolled away still fills the cache for the next one
  public func load(_ key: Key, _ make: @escaping @Sendable () async throws -> Value) async throws -> Value {
    if let hit = cached(key) { return hit }
    if let running = loading[key] { return try await running.value }
    let task = Task {
      do {
        let value = try await make()
        loading[key] = nil
        keep(key, value)
        return value
      } catch {
        loading[key] = nil
        throw error
      }
    }
    loading[key] = task
    return try await task.value
  }

  /// Stores a value under `key` and drops the least recently used ones until the bounds hold again
  public func keep(_ key: Key, _ value: Value) {
    let size = cost(value)
    if let old = entries.removeValue(forKey: key) { total -= old.cost }
    guard size <= budget else { return }
    clock += 1
    entries[key] = Entry(value: value, cost: size, used: clock)
    total += size
    while total > budget || entries.count > limit,
          let oldest = entries.filter({ $0.key != key }).min(by: { $0.value.used < $1.value.used })?.key {
      total -= entries.removeValue(forKey: oldest)?.cost ?? 0
    }
  }
}

/// The development engine's blob route (`acpira --ws`, sidecar/ws.rs): `GET {blobBase}/{sessionId}/{blob}`, no
/// authentication, loopback only
public enum HTTPBlobs {
  public enum Failure: Error, Equatable {
    case badBase
    case status(Int)
  }

  /// `{base}/{session}/{name}`, each part one percent-encoded path segment
  public static func url(base: String, session: String, name: String) -> URL? {
    guard var url = URL(string: base), url.scheme != nil else { return nil }
    url.append(path: session, directoryHint: .notDirectory)
    url.append(path: name, directoryHint: .notDirectory)
    return url
  }

  /// A fetch against `base` (InitState.blobBase); nil reads as no blob route at all
  public static func fetch(base: @escaping @Sendable () async -> String?, session urlSession: URLSession = .shared) -> BlobFetch {
    { session, name in
      guard let root = await base(), let url = url(base: root, session: session, name: name) else { throw Failure.badBase }
      let (data, response) = try await urlSession.data(from: url)
      if let http = response as? HTTPURLResponse, http.statusCode != 200 { throw Failure.status(http.statusCode) }
      return data
    }
  }
}
