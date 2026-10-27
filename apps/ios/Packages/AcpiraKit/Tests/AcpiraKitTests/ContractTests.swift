import AcpiraModels
import Foundation
import Testing

// Pins the Swift mirrors to the TypeScript contract they copy. Runs wherever the repository is readable (`swift test`
// on the Mac); a test process that cannot see the source tree skips it instead of failing.
struct ContractTests {
  /// packages/shared/sidecar.ts; the repository root is seven levels above this file
  static let sidecarSource = (0..<7)
    .reduce(URL(filePath: #filePath)) { url, _ in url.deletingLastPathComponent() }
    .appending(path: "packages/shared/sidecar.ts")

  @Test(.enabled(if: FileManager.default.fileExists(atPath: sidecarSource.path)))
  func protocolVersionMatchesTypeScript() throws {
    let source = try String(contentsOf: Self.sidecarSource, encoding: .utf8)
    let match = try #require(source.firstMatch(of: /SIDECAR_PROTOCOL_VERSION = (\d+)/))
    #expect(Int(match.1) == SidecarProtocol.version)
  }
}
