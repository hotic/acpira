import Foundation

// SSH remote workspaces (packages/shared/remote.ts, acpira_shared::remote). A folder on an SSH host is written
// `ssh://<alias>/<absolute path>` in every field that holds a folder (a session's cwd, the project list); the alias is
// the host's name in ~/.ssh/config (or a typed `user@host:port`). Every side runs test/fixtures/remote-folders.json.

/// A folder on an SSH host: the host's alias and the absolute path there
public struct RemoteFolder: Hashable, Sendable {
  public var alias: String
  public var path: String

  public static let scheme = "ssh://"

  /// `ssh://<alias>/<path>` split and normalized; nil for anything else, a local path included
  public static func parse(_ folder: String) -> RemoteFolder? {
    guard folder.hasPrefix(scheme) else { return nil }
    let rest = folder.dropFirst(scheme.count)
    guard let slash = rest.firstIndex(of: "/") else { return nil }
    let alias = String(rest[..<slash])
    guard isAlias(alias), let path = normalize(String(rest[slash...])) else { return nil }
    return RemoteFolder(alias: alias, path: path)
  }

  /// Rust's `char::is_control`: C0, DEL and C1
  private static func isControl(_ scalar: Unicode.Scalar) -> Bool {
    scalar.value < 0x20 || (0x7f...0x9f).contains(scalar.value)
  }

  /// A host alias as ssh takes it: no whitespace or control characters, nothing that ends the authority, no leading `-`
  static func isAlias(_ alias: String) -> Bool {
    !alias.isEmpty && !alias.hasPrefix("-") && !alias.unicodeScalars.contains {
      isControl($0) || $0.properties.isWhitespace || "/?#\\".unicodeScalars.contains($0)
    }
  }

  /// An absolute POSIX path normalized: empty and `.` segments dropped, no trailing slash; `..` and control characters
  /// refused
  static func normalize(_ path: String) -> String? {
    guard path.hasPrefix("/"), !path.unicodeScalars.contains(where: isControl) else { return nil }
    var parts: [Substring] = []
    for segment in path.split(separator: "/", omittingEmptySubsequences: true) where segment != "." {
      if segment == ".." { return nil }
      parts.append(segment)
    }
    return "/" + parts.joined(separator: "/")
  }
}

/// How a host's sessions run: `local` keeps the agent loop here and runs only the tools on the host (the built-in agent
/// alone works there), `remote` drives the host's own engine
public enum RunMode: String, TolerantEnum, CaseIterable {
  case local, remote, unknown
  public static var unknownCase: Self { .unknown }
}

public enum ConnectionState: String, TolerantEnum, CaseIterable {
  case off, connecting, connected, error, unknown
  public static var unknownCase: Self { .unknown }
}

/// One SSH host as the pages see it (HostMsg `connections`), the part the list and the new session read
public struct ConnectionView: Codable, Hashable, Sendable {
  public var alias: String
  public var mode: RunMode
  public var disabled: Bool?
  public var state: ConnectionState
  public var error: Failure?

  /// Why the last attempt failed; the message is the engine's, already worded for people
  public struct Failure: Codable, Hashable, Sendable {
    public var message: String
  }

  public init(alias: String, mode: RunMode, disabled: Bool? = nil, state: ConnectionState, error: Failure? = nil) {
    self.alias = alias; self.mode = mode; self.disabled = disabled; self.state = state; self.error = error
  }
}
