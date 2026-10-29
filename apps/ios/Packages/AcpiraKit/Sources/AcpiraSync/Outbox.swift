import AcpiraModels
import Foundation

// What the phone sends again after a lost wire. A paired link can die between a tap and the engine reading it (a network
// switch, a relay that went quiet), and the composer has already cleared the prompt. The actions that drive a session
// carry a clientId the engine running the session claims once (protocol.ts), so the next wire can resend whatever the
// last one may not have delivered without running anything twice.

extension WebviewMsg {
  /// The id the engine dedupes this action by; nil for actions that carry none
  public var clientId: String? {
    switch self {
    case .send(_, _, _, _, let id), .stop(_, let id), .permission(_, _, _, let id), .answer(_, _, _, _, let id),
         .steerQueued(_, _, let id):
      id
    default:
      nil
    }
  }

  /// The action with a fresh clientId where it can carry one and has none yet; a resend keeps the id it was given
  public func stamped() -> WebviewMsg {
    let id = UUID().uuidString
    return switch self {
    case let .send(session, text, attachments, steer, nil): .send(sessionId: session, text: text, attachments: attachments, steer: steer, clientId: id)
    case let .stop(session, nil): .stop(sessionId: session, clientId: id)
    case let .permission(session, block, option, nil): .permission(sessionId: session, blockId: block, optionId: option, clientId: id)
    case let .answer(session, block, answers, skip, nil): .answer(sessionId: session, blockId: block, answers: answers, skip: skip, clientId: id)
    case let .steerQueued(session, item, nil): .steerQueued(sessionId: session, id: item, clientId: id)
    default: self
    }
  }

  /// Worth sending again on the next wire: what the user did to a session. Reads, seq requests, terminals and view
  /// switches belong to the wire that asked (the screens ask again), and a new session or fork waits on its own wire's
  /// answer to open its chat
  public var replayable: Bool {
    switch self {
    case .send, .stop, .permission, .answer, .steerQueued, .sendQueued, .setMode, .setConfig: true
    default: false
    }
  }

  /// The session a replayable action acts on; the engine refuses one for a session its view does not show
  public var sessionTarget: String? {
    switch self {
    case .send(let id, _, _, _, _), .stop(let id, _), .setMode(let id, _), .setConfig(let id, _, _): id
    case .permission(let id, _, _, _), .answer(let id, _, _, _, _), .steerQueued(let id, _, _), .sendQueued(let id, _): id
    default: nil
    }
  }
}
