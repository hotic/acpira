import AcpiraModels

/// Changes to the model sources in order (packages/ui/settings/providerWrites.ts). The engine answers every write and
/// every read with the whole list and no request id, so one goes out at a time: the next `providers` answer settles the
/// one in flight, unless a read went out before it. Row edits (a model's switch, an added model) show at once and
/// consecutive ones of one source collapse into its newest snapshot (they have no waiter); a form's write waits for its
/// own answer and is never merged.
struct ProviderWrites: Equatable {
  struct Write: Equatable {
    var id: Int
    var action: ProviderAction
    var optimistic: Bool
  }

  private(set) var active: Write?
  private(set) var queue: [Write] = []
  /// A read is out: the next answer is its, not a write's
  private(set) var reading = false
  private var serial = 0

  var pending: Bool { active != nil || !queue.isEmpty }

  /// Whether a read may go out now (none while writes are pending: their answers bring the list anyway)
  mutating func refresh() -> Bool {
    guard !reading, !pending else { return false }
    reading = true
    return true
  }

  /// Queues `action`; returns its write id and the action to send now, if nothing is in flight
  mutating func push(_ action: ProviderAction, optimistic: Bool) -> (id: Int, send: ProviderAction?) {
    serial += 1
    let write = Write(id: serial, action: action, optimistic: optimistic)
    if active == nil, !reading {
      active = write
      return (write.id, action)
    }
    // Consecutive row edits of one source need only its newest snapshot; a form's write is never merged
    if optimistic, let last = queue.last, last.optimistic, case .save(let next, _) = action, case .save(let previous, _) = last.action,
       next.id == previous.id {
      queue[queue.count - 1] = write
      return (write.id, nil)
    }
    queue.append(write)
    return (write.id, nil)
  }

  /// An answer came: the write it settles (none when it answered a read), and the next write to send
  mutating func receive() -> (settled: Write?, send: ProviderAction?) {
    let settled = reading ? nil : active
    reading = false
    active = queue.isEmpty ? nil : queue.removeFirst()
    return (settled, active?.action)
  }

  /// The sources as the pending row edits leave them, by id, laid over the engine's last answer
  var drafts: [String: ProviderEntry] {
    var drafts: [String: ProviderEntry] = [:]
    for write in (active.map { [$0] } ?? []) + queue where write.optimistic {
      if case .save(let provider, _) = write.action { drafts[provider.id] = provider }
    }
    return drafts
  }
}
