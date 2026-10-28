import Foundation
import Security
import Synchronization

/// Where this device's identity key lives: 32 bytes whose public half is the EndpointId every paired computer pinned.
/// Losing it means pairing again; copying it to another device would make two devices one identity
public protocol IdentityStore: Sendable {
  func load() throws -> Data?
  func save(_ key: Data) throws
}

public struct KeychainError: Error, Equatable, Sendable {
  public let status: OSStatus
}

/// The Keychain: a generic password readable after the first unlock (a reconnect may run before the screen is unlocked
/// again), on this device only and never in iCloud Keychain, so a restored or second device cannot share the identity
public struct KeychainIdentityStore: IdentityStore {
  public let service: String
  public let account: String

  public init(service: String = "cc.hotic.acpira.remote", account: String = "identity") {
    self.service = service
    self.account = account
  }

  private var query: [String: Any] {
    [
      kSecClass as String: kSecClassGenericPassword,
      kSecAttrService as String: service,
      kSecAttrAccount as String: account,
      kSecAttrSynchronizable as String: false,
      // macOS: the same keychain as iOS, not the legacy file keychain
      kSecUseDataProtectionKeychain as String: true,
    ]
  }

  public func load() throws -> Data? {
    var q = query
    q[kSecReturnData as String] = true
    q[kSecMatchLimit as String] = kSecMatchLimitOne
    var item: CFTypeRef?
    let status = SecItemCopyMatching(q as CFDictionary, &item)
    switch status {
    case errSecSuccess: return item as? Data
    case errSecItemNotFound: return nil
    default: throw KeychainError(status: status)
    }
  }

  public func save(_ key: Data) throws {
    var q = query
    q[kSecValueData as String] = key
    q[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
    var status = SecItemAdd(q as CFDictionary, nil)
    if status == errSecDuplicateItem {
      let change: [String: Any] = [kSecValueData as String: key, kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly]
      status = SecItemUpdate(query as CFDictionary, change as CFDictionary)
    }
    guard status == errSecSuccess else { throw KeychainError(status: status) }
  }
}

/// A key that lives as long as the process: tests, and previews that must not touch the Keychain
public final class MemoryIdentityStore: IdentityStore {
  private let key: Mutex<Data?>

  public init(_ key: Data? = nil) {
    self.key = Mutex(key)
  }

  public func load() throws -> Data? {
    key.withLock { $0 }
  }

  public func save(_ key: Data) throws {
    self.key.withLock { $0 = key }
  }
}
