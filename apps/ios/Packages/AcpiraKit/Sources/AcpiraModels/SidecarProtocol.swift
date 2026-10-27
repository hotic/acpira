// The engine contract the app speaks. The source of truth is TypeScript (packages/shared/sidecar.ts, protocol.ts,
// transcript.ts), mirrored in Rust (rust/crates/acpira-shared); this module mirrors the subset the app renders.
// Decoding stays tolerant: the engine on the Mac updates faster than the App Store ships the app, so an unknown
// kind decodes into a placeholder instead of failing the whole view.

/// The ndjson envelope protocol between a shell and the Rust sidecar.
public enum SidecarProtocol {
  /// Must equal SIDECAR_PROTOCOL_VERSION in packages/shared/sidecar.ts; the engine rejects any other value at hello.
  public static let version = 1
}
