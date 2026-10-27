// Talks to the engine: the envelope client (hello / attachView / webviewMessage), session patches applied with stable
// identity (packages/shared/sessionPatch.ts and packages/shared/reuse.ts are the reference), and transports behind one protocol:
// a development WebSocket to `pnpm harness` first, the paired remote connection later. No SwiftUI or UIKit here.

import AcpiraModels

/// Entry point of the sync layer.
public enum AcpiraSync {
  /// The protocol version hello announces.
  public static let protocolVersion = SidecarProtocol.version
}
