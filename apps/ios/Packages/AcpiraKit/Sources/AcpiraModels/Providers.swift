import Foundation

// The built-in agent's model sources (packages/shared/providers.ts) as far as the phone edits them: sources and their
// models, the probes that list a source's models or find local servers, and the save / delete actions. providers.json
// keeps fields this build does not know, and an entry sent back must carry them along, so a source and a model are
// their raw JSON objects with typed accessors over the fields the phone reads or writes.

/// One entry of a folder listing (remote.ts RemoteDirEntry): `localDir` answers carry folders only
public struct RemoteDirEntry: Codable, Hashable, Sendable {
  public var name: String
  public var dir: Bool
  /// A folder holding `.git`
  public var git: Bool?

  public init(name: String, dir: Bool, git: Bool? = nil) {
    self.name = name
    self.dir = dir
    self.git = git
  }
}

/// A raw JSON object with typed field access, the storage of ProviderEntry and ProviderModelEntry
public protocol RawJSONObject: Codable, Hashable, Sendable {
  var raw: [String: JSONValue] { get set }
  init(raw: [String: JSONValue])
}

public extension RawJSONObject {
  init(from decoder: Decoder) throws { self.init(raw: try [String: JSONValue](from: decoder)) }
  func encode(to encoder: Encoder) throws { try raw.encode(to: encoder) }

  func string(_ key: String) -> String? { if case .string(let s) = raw[key] { s } else { nil } }
  func bool(_ key: String) -> Bool? { if case .bool(let b) = raw[key] { b } else { nil } }
  func number(_ key: String) -> Double? { if case .number(let n) = raw[key] { n } else { nil } }
}

/// A model of a source (providers.ts ProviderModel)
public struct ProviderModelEntry: RawJSONObject, Identifiable {
  public var raw: [String: JSONValue]
  public init(raw: [String: JSONValue]) { self.raw = raw }

  /// A hand-typed id with the file's defaults (providers.ts newModel)
  public init(typed id: String) {
    raw = ["id": .string(id), "enabled": true, "input": ["text"], "thinking": "auto", "efforts": [], "sampling": [:], "estimated": []]
  }

  public var id: String { string("id") ?? "" }
  public var name: String? { string("name").flatMap { $0.isEmpty ? nil : $0 } }
  public var enabled: Bool {
    get { bool("enabled") ?? false }
    set { raw["enabled"] = .bool(newValue) }
  }
  /// Context window in tokens, when known
  public var context: Int? { number("context").map { Int($0) } }
  /// The model takes images
  public var image: Bool {
    if case .array(let input) = raw["input"] { input.contains(.string("image")) } else { false }
  }
}

/// A model source (providers.ts Provider / ProviderView)
public struct ProviderEntry: RawJSONObject, Identifiable {
  public var raw: [String: JSONValue]
  public init(raw: [String: JSONValue]) { self.raw = raw }

  /// A new source as the add form builds it: the engine makes its id from the name
  public init(name: String, format: String, baseUrl: String, fullUrl: Bool, preset: String = "custom",
              models: [ProviderModelEntry] = []) {
    raw = ["id": "", "name": .string(name), "preset": .string(preset), "format": .string(format), "baseUrl": .string(baseUrl),
           "fullUrl": .bool(fullUrl), "enabled": true, "models": .array(models.map { .object($0.raw) })]
  }

  public var id: String { string("id") ?? "" }
  public var name: String {
    get { string("name") ?? "" }
    set { raw["name"] = .string(newValue) }
  }
  /// `custom`, or `ollama` / `lmstudio` for a local server
  public var preset: String { string("preset") ?? "custom" }
  public var format: String {
    get { string("format") ?? "openai-chat" }
    set { raw["format"] = .string(newValue) }
  }
  public var baseUrl: String {
    get { string("baseUrl") ?? "" }
    set { raw["baseUrl"] = .string(newValue) }
  }
  public var fullUrl: Bool {
    get { bool("fullUrl") ?? false }
    set { raw["fullUrl"] = .bool(newValue) }
  }
  public var enabled: Bool {
    get { bool("enabled") ?? false }
    set { raw["enabled"] = .bool(newValue) }
  }
  /// The view says a key is stored (never the key itself); not part of the file
  public var hasKey: Bool { bool("hasKey") ?? false }
  public var models: [ProviderModelEntry] {
    get {
      guard case .array(let list) = raw["models"] else { return [] }
      return list.compactMap { if case .object(let o) = $0 { ProviderModelEntry(raw: o) } else { nil } }
    }
    set { raw["models"] = .array(newValue.map { .object($0.raw) }) }
  }
  /// A model server on the computer, which takes no key
  public var isLocal: Bool { preset == "ollama" || preset == "lmstudio" }
  /// What goes back to the engine: the entry as the view showed it, without the view-only `hasKey`
  public var stripped: ProviderEntry {
    var copy = self
    copy.raw["hasKey"] = nil
    return copy
  }
}

/// The sources (providers.ts ProvidersView); the agent-wide fields stay on the computer and are not read here
public struct ProvidersView: Codable, Hashable, Sendable {
  public var providers: [ProviderEntry]
  /// providers.json exists but cannot be read
  public var error: String?

  public init(providers: [ProviderEntry], error: String? = nil) {
    self.providers = providers
    self.error = error
  }

  private enum Keys: String, CodingKey { case providers, error }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    providers = try c.decode([ProviderEntry].self, forKey: .providers)
    error = try c.decodeIfPresent(String.self, forKey: .error)
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(providers, forKey: .providers)
    try c.encodeIfPresent(error, forKey: .error)
  }
}

/// A model server answering on the computer, ready to save as a source
public struct LocalSource: Codable, Hashable, Sendable {
  public var preset: String
  public var name: String
  public var baseUrl: String
  public var models: [ProviderModelEntry]

  public init(preset: String, name: String, baseUrl: String, models: [ProviderModelEntry]) {
    self.preset = preset
    self.name = name
    self.baseUrl = baseUrl
    self.models = models
  }

  /// The source it becomes: Ollama and LM Studio serve the OpenAI Chat format at the root found
  public var provider: ProviderEntry {
    ProviderEntry(name: name, format: "openai-chat", baseUrl: baseUrl, fullUrl: false, preset: preset, models: models)
  }
}

/// A network question about a source (providers.ts ProviderProbe); none changes providers.json. key: typed into the
/// form and not saved yet; absent uses the stored key of the source with this id
public enum ProviderProbe: Codable, Hashable, Sendable {
  case models(provider: ProviderEntry, key: String? = nil)
  case local

  private enum Keys: String, CodingKey { case kind, provider, key }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "models": self = .models(provider: try c.decode(ProviderEntry.self, forKey: .provider), key: try c.decodeIfPresent(String.self, forKey: .key))
    case "local": self = .local
    case let kind: throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "probe \(kind)")
    }
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case let .models(provider, key):
      try c.encode("models", forKey: .kind)
      try c.encode(provider.stripped, forKey: .provider)
      try c.encodeIfPresent(key, forKey: .key)
    case .local:
      try c.encode("local", forKey: .kind)
    }
  }
}

/// A probe's answer (providers.ts ProbeOutcome); `check` and `test` answers, which the phone never asks for, read as other
public enum ProbeOutcome: Codable, Hashable, Sendable {
  case models([ProviderModelEntry])
  case local([LocalSource])
  case failed(String)
  case other(String)

  private enum Keys: String, CodingKey { case kind, models, servers, error }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "models": self = .models(try c.decode([ProviderModelEntry].self, forKey: .models))
    case "local": self = .local(try c.decode([LocalSource].self, forKey: .servers))
    case "failed": self = .failed(try c.decode(String.self, forKey: .error))
    case let kind: self = .other(kind)
    }
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case .models(let models):
      try c.encode("models", forKey: .kind)
      try c.encode(models, forKey: .models)
    case .local(let servers):
      try c.encode("local", forKey: .kind)
      try c.encode(servers, forKey: .servers)
    case .failed(let error):
      try c.encode("failed", forKey: .kind)
      try c.encode(error, forKey: .error)
    case .other(let kind):
      try c.encode(kind, forKey: .kind)
    }
  }
}

/// A change to the sources a paired phone may make (providers.ts ProviderAction): save one (key absent keeps the
/// stored key, empty removes it) or remove one with its key
public enum ProviderAction: Codable, Hashable, Sendable {
  case save(provider: ProviderEntry, key: String? = nil)
  case delete(id: String)

  private enum Keys: String, CodingKey { case kind, provider, key, id }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "save": self = .save(provider: try c.decode(ProviderEntry.self, forKey: .provider), key: try c.decodeIfPresent(String.self, forKey: .key))
    case "delete": self = .delete(id: try c.decode(String.self, forKey: .id))
    case let kind: throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "action \(kind)")
    }
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case let .save(provider, key):
      try c.encode("save", forKey: .kind)
      try c.encode(provider.stripped, forKey: .provider)
      try c.encodeIfPresent(key, forKey: .key)
    case .delete(let id):
      try c.encode("delete", forKey: .kind)
      try c.encode(id, forKey: .id)
    }
  }
}
