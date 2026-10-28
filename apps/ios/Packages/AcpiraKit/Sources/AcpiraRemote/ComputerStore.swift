import AcpiraSync
import Foundation

/// The paired computers as JSON in Application Support. Left out of backups: the identity key never leaves the device
/// (`KeychainIdentityStore`), so a list restored onto another device would name computers that do not know it
struct ComputerStore {
  let file: URL

  /// `<Application Support>/Acpira/Remote/computers.json`
  static func defaultDirectory() -> URL {
    URL.applicationSupportDirectory.appending(path: "Acpira/Remote", directoryHint: .isDirectory)
  }

  init(directory: URL) {
    file = directory.appending(path: "computers.json")
  }

  func load() -> [PairedComputer] {
    guard let data = try? Data(contentsOf: file) else { return [] }
    return (try? Self.decoder.decode([PairedComputer].self, from: data)) ?? []
  }

  func save(_ computers: [PairedComputer]) throws {
    try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
    var options: Data.WritingOptions = .atomic
    #if os(iOS)
      options.insert(.completeFileProtectionUntilFirstUserAuthentication)
    #endif
    try Self.encoder.encode(computers).write(to: file, options: options)
    var url = file
    var values = URLResourceValues()
    values.isExcludedFromBackup = true
    try? url.setResourceValues(values)
  }

  private static let encoder: JSONEncoder = {
    let e = JSONEncoder()
    e.dateEncodingStrategy = .iso8601
    e.outputFormatting = [.prettyPrinted, .sortedKeys]
    return e
  }()

  private static let decoder: JSONDecoder = {
    let d = JSONDecoder()
    d.dateDecodingStrategy = .iso8601
    return d
  }()
}
