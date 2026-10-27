// Talks to the engine: transports behind one protocol (EngineTransport: the development WebSocket to `pnpm harness`
// first, the paired remote connection later), the envelope client (EngineClient), and session patches applied against
// the views the page keeps (SessionPatches, SessionViews; packages/shared/sessionPatch.ts and packages/ui/sessionViews.ts
// are the reference). No SwiftUI or UIKit here.

import AcpiraModels

/// Entry point of the sync layer.
public enum AcpiraSync {
  /// The protocol version hello announces.
  public static let protocolVersion = SidecarProtocol.version
}
