import Foundation

// The engine contract the app speaks. The source of truth is TypeScript (packages/shared/sidecar.ts, protocol.ts,
// transcript.ts), mirrored in Rust (rust/crates/acpira-shared); this module mirrors the subset the app renders.
// Decoding stays tolerant: the engine on the Mac updates faster than the App Store ships the app, so an unknown
// kind decodes into a placeholder instead of failing the whole view.
//
// This file holds the envelopes (sidecar.ts): one JSON object per line on stdio, one per text frame over the harness
// WebSocket. A client says hello, waits for helloOk, attaches a view, then wraps every WebviewMsg in webviewMessage and
// receives every HostMsg as hostMessage. Field names are the TypeScript ones; `ContractTests` pins them.

/// The ndjson envelope protocol between a shell and the Rust sidecar.
public enum SidecarProtocol {
  /// Must equal SIDECAR_PROTOCOL_VERSION in packages/shared/sidecar.ts; the engine rejects any other value at hello.
  public static let version = 1
}

/// Which kind of webview a view is (protocol.ts WebviewHost); the engine only distinguishes the session list's
/// most-recent default, so the phone attaches as `sidebar`
public enum WebviewHost: String, TolerantEnum {
  case sidebar, editor
  public static var unknownCase: Self { .sidebar }
}

/// Facts about the client, sent with hello (ShellEnv)
public struct ShellEnv: Codable, Hashable, Sendable {
  /// Folder new sessions open in; the engine's home directory when absent
  public var cwd: String?
  public var hostLanguage: String
  /// Where views load attachment blobs from: `${blobBase}/${sessionId}/${blob}` (the harness serves `/blobs`)
  public var blobBase: String?
  /// The client organizes sessions by project folder: a view follows its session's folder, and no session starts on
  /// its own without a project (helloOk.features has `projects`)
  public var projects: Bool?

  public init(cwd: String? = nil, hostLanguage: String, blobBase: String? = nil, projects: Bool? = nil) {
    self.cwd = cwd; self.hostLanguage = hostLanguage; self.blobBase = blobBase; self.projects = projects
  }
}

/// The client in hello
public struct ClientInfo: Codable, Hashable, Sendable {
  public var name: String
  public var version: String
  /// Platform methods this client carries out (PlatformRequest methods); the engine sends no request outside this list,
  /// and falls back to its own handling where it has one (file search, settings snapshot)
  public var capabilities: [String]
  /// The shell process's SSH_CONNECTION, for a window of a remote workspace
  public var sshConnection: String?
  /// `peer`: another engine, never a window (docs/dev/remote.md). A plain wire leaves it unset
  public var role: String?

  public init(name: String, version: String, capabilities: [String] = [], sshConnection: String? = nil, role: String? = nil) {
    self.name = name; self.version = version; self.capabilities = capabilities
    self.sshConnection = sshConnection; self.role = role
  }
}

/// The first envelope on a wire
public struct Hello: Codable, Hashable, Sendable {
  public var protocolVersion: Int
  public var requestId: String
  public var client: ClientInfo
  public var env: ShellEnv
  /// Flat acpira.* keys without the prefix (`defaultAgent`, `appearance.motion`): the shell's settings snapshot
  public var settings: [String: JSONValue]

  public init(protocolVersion: Int = SidecarProtocol.version, requestId: String, client: ClientInfo, env: ShellEnv,
              settings: [String: JSONValue] = [:]) {
    self.protocolVersion = protocolVersion; self.requestId = requestId; self.client = client; self.env = env
    self.settings = settings
  }
}

/// What a view opens on (attachView `initial`)
public enum InitialView: Codable, Hashable, Sendable {
  /// That session
  case session(String)
  /// The newest listed session; with none, a new session starts in the workspace folder
  case mostRecent
  /// Nothing until a session is selected on the view: it never opens or starts one by itself
  case empty

  private enum Keys: String, CodingKey { case mostRecent, empty }
  public init(from decoder: Decoder) throws {
    if let id = try? decoder.singleValueContainer().decode(String.self) {
      self = .session(id)
      return
    }
    let c = try decoder.container(keyedBy: Keys.self)
    if try c.decodeIfPresent(Bool.self, forKey: .mostRecent) == true {
      self = .mostRecent
    } else if try c.decodeIfPresent(Bool.self, forKey: .empty) == true {
      self = .empty
    } else {
      throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "not an initial view"))
    }
  }
  public func encode(to encoder: Encoder) throws {
    switch self {
    case .session(let id):
      var c = encoder.singleValueContainer()
      try c.encode(id)
    case .mostRecent:
      var c = encoder.container(keyedBy: Keys.self)
      try c.encode(true, forKey: .mostRecent)
    case .empty:
      var c = encoder.container(keyedBy: Keys.self)
      try c.encode(true, forKey: .empty)
    }
  }
}

/// Client → engine (ShellMsg). Not mirrored: platformEvent (a window's focus, settings and environment; the engine
/// ignores a peer's) and previewOpen / previewClose (the desktop browser panel)
public enum ShellMsg: Codable, Hashable, Sendable {
  case hello(Hello)
  /// blobBase: this view's own blob base; the env's when absent
  case attachView(viewId: String, host: WebviewHost, initial: InitialView? = nil, blobBase: String? = nil)
  case detachView(viewId: String)
  case webviewMessage(viewId: String, message: WebviewMsg)
  /// The answer to a platformRequest that carried a requestId
  case platformResponse(requestId: String, result: JSONValue? = nil, error: String? = nil)
  case shutdown
  /// An envelope type this build does not know (decoding only)
  case unknown(String)

  /// The `type` literals this build mirrors
  public static let types = ["hello", "attachView", "detachView", "webviewMessage", "platformResponse", "shutdown"]

  private enum Keys: String, CodingKey { case type, viewId, host, initial, blobBase, message, requestId, result, error }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .type) {
    case "hello": self = .hello(try Hello(from: decoder))
    case "attachView":
      self = .attachView(viewId: try c.decode(String.self, forKey: .viewId), host: try c.decode(WebviewHost.self, forKey: .host),
                         initial: try c.decodeIfPresent(InitialView.self, forKey: .initial),
                         blobBase: try c.decodeIfPresent(String.self, forKey: .blobBase))
    case "detachView": self = .detachView(viewId: try c.decode(String.self, forKey: .viewId))
    case "webviewMessage":
      self = .webviewMessage(viewId: try c.decode(String.self, forKey: .viewId), message: try c.decode(WebviewMsg.self, forKey: .message))
    case "platformResponse":
      self = .platformResponse(requestId: try c.decode(String.self, forKey: .requestId),
                               result: try c.decodeIfPresent(JSONValue.self, forKey: .result),
                               error: try c.decodeIfPresent(String.self, forKey: .error))
    case "shutdown": self = .shutdown
    case let type: self = .unknown(type)
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case .hello(let h):
      try c.encode("hello", forKey: .type)
      try h.encode(to: encoder)
    case let .attachView(viewId, host, initial, blobBase):
      try c.encode("attachView", forKey: .type)
      try c.encode(viewId, forKey: .viewId)
      try c.encode(host, forKey: .host)
      try c.encodeIfPresent(initial, forKey: .initial)
      try c.encodeIfPresent(blobBase, forKey: .blobBase)
    case .detachView(let viewId):
      try c.encode("detachView", forKey: .type)
      try c.encode(viewId, forKey: .viewId)
    case let .webviewMessage(viewId, message):
      try c.encode("webviewMessage", forKey: .type)
      try c.encode(viewId, forKey: .viewId)
      try c.encode(message, forKey: .message)
    case let .platformResponse(requestId, result, error):
      try c.encode("platformResponse", forKey: .type)
      try c.encode(requestId, forKey: .requestId)
      try c.encodeIfPresent(result, forKey: .result)
      try c.encodeIfPresent(error, forKey: .error)
    case .shutdown:
      try c.encode("shutdown", forKey: .type)
    case .unknown(let type):
      try c.encode(type, forKey: .type)
    }
  }
}

/// The engine's answer to hello
public struct HelloOk: Codable, Hashable, Sendable {
  public struct Sidecar: Codable, Hashable, Sendable {
    public var version: String
    public var pid: Int
  }
  public var requestId: String
  public var protocolVersion: Int
  public var sidecar: Sidecar
  /// The engine's sessions directory on its own machine; blobs live at `<sessionsDir>/<sessionId>/<blob>`
  public var sessionsDir: String
  /// Additions usable without a protocol bump (`preview`, `projects`, `remote`, `plugins`); absent on older engines
  public var features: [String]?
  /// Every user plugin is unloaded (docs/dev/plugins.md)
  public var safeMode: Bool?
}

/// An IDE action the engine asks the client for (PlatformRequest). It only sends methods hello's capabilities name;
/// one with a requestId waits up to 30 s for a platformResponse. Mirrored: the two a phone can carry out itself
public enum PlatformRequest: Codable, Hashable, Sendable {
  /// `open` adds a button for that file (label already localized)
  case toast(level: String, text: String, open: ToastOpen? = nil)
  /// A session needs the user or has settled: a chime (`sound`) and / or a system notification (`system`), each only
  /// when its setting is on; `open` is the label of the button that brings the session up
  case notify(sessionId: String, title: String, body: String, open: String, sound: Bool, system: Bool)
  /// Any other method; answer it with an error when it carries a requestId
  case unknown(method: String)

  public struct ToastOpen: Codable, Hashable, Sendable {
    public var label: String
    public var path: String

    public init(label: String, path: String) { self.label = label; self.path = path }
  }

  /// The `method` literals this build mirrors
  public static let methods = ["toast", "notify"]

  public var method: String {
    switch self {
    case .toast: "toast"
    case .notify: "notify"
    case .unknown(let m): m
    }
  }

  private enum Keys: String, CodingKey { case method, level, text, open, sessionId, title, body, sound, system }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .method) {
    case "toast":
      self = .toast(level: try c.decode(String.self, forKey: .level), text: try c.decode(String.self, forKey: .text),
                    open: try c.decodeIfPresent(ToastOpen.self, forKey: .open))
    case "notify":
      self = .notify(sessionId: try c.decode(String.self, forKey: .sessionId), title: try c.decode(String.self, forKey: .title),
                     body: try c.decode(String.self, forKey: .body), open: try c.decode(String.self, forKey: .open),
                     sound: try c.decode(Bool.self, forKey: .sound), system: try c.decode(Bool.self, forKey: .system))
    case let method: self = .unknown(method: method)
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(method, forKey: .method)
    switch self {
    case let .toast(level, text, open):
      try c.encode(level, forKey: .level)
      try c.encode(text, forKey: .text)
      try c.encodeIfPresent(open, forKey: .open)
    case let .notify(sessionId, title, body, open, sound, system):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(title, forKey: .title)
      try c.encode(body, forKey: .body)
      try c.encode(open, forKey: .open)
      try c.encode(sound, forKey: .sound)
      try c.encode(system, forKey: .system)
    case .unknown: break
    }
  }
}

/// Engine → client (SidecarMsg). Not mirrored: previewOpened (answers previewOpen, which the phone never sends)
public enum SidecarMsg: Codable, Hashable, Sendable {
  case helloOk(HelloOk)
  /// The engine refused the wire (another protocol version, the engine shutting down, its runtime failing to start) and
  /// closes it
  case helloReject(requestId: String, protocolVersion: Int, reason: String)
  case hostMessage(viewId: String, message: HostMsg)
  /// requestId: present when the engine waits for a platformResponse
  case platformRequest(requestId: String? = nil, request: PlatformRequest)
  case shutdownOk
  /// An envelope type this build does not know
  case unknown(String)

  /// The `type` literals this build mirrors
  public static let types = ["helloOk", "helloReject", "hostMessage", "platformRequest", "shutdownOk"]

  private enum Keys: String, CodingKey { case type, requestId, protocolVersion, reason, viewId, message, request }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .type) {
    case "helloOk": self = .helloOk(try HelloOk(from: decoder))
    case "helloReject":
      self = .helloReject(requestId: try c.decode(String.self, forKey: .requestId),
                          protocolVersion: try c.decode(Int.self, forKey: .protocolVersion), reason: try c.decode(String.self, forKey: .reason))
    case "hostMessage":
      self = .hostMessage(viewId: try c.decode(String.self, forKey: .viewId), message: try c.decode(HostMsg.self, forKey: .message))
    case "platformRequest":
      self = .platformRequest(requestId: try c.decodeIfPresent(String.self, forKey: .requestId),
                              request: try c.decode(PlatformRequest.self, forKey: .request))
    case "shutdownOk": self = .shutdownOk
    case let type: self = .unknown(type)
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case .helloOk(let ok):
      try c.encode("helloOk", forKey: .type)
      try ok.encode(to: encoder)
    case let .helloReject(requestId, protocolVersion, reason):
      try c.encode("helloReject", forKey: .type)
      try c.encode(requestId, forKey: .requestId)
      try c.encode(protocolVersion, forKey: .protocolVersion)
      try c.encode(reason, forKey: .reason)
    case let .hostMessage(viewId, message):
      try c.encode("hostMessage", forKey: .type)
      try c.encode(viewId, forKey: .viewId)
      try c.encode(message, forKey: .message)
    case let .platformRequest(requestId, request):
      try c.encode("platformRequest", forKey: .type)
      try c.encodeIfPresent(requestId, forKey: .requestId)
      try c.encode(request, forKey: .request)
    case .shutdownOk:
      try c.encode("shutdownOk", forKey: .type)
    case .unknown(let type):
      try c.encode(type, forKey: .type)
    }
  }
}
