import AcpiraSync
import Foundation
import Observation
import UserNotifications
import os
#if canImport(UIKit)
import UIKit
#endif

// Notifications from paired computers. In the background the app has no wire (iOS suspends it), so the computer's
// gateway posts through APNs (acpira-gateway `push.rs`); this side asks for permission, registers with APNs, tells each
// computer where to send (`PairedConnection.registerPush`), and routes an opened notification to its session.

/// A notification the user opened: the session and the computer (gateway id) it came from
struct PushOpen: Equatable {
  let computer: String
  let session: String
}

@Observable
public final class PushCenter: NSObject {
  public static let shared = PushCenter()

  enum Permission {
    /// Not read from the system yet
    case unknown
    case notAsked, on, off
  }

  private(set) var permission: Permission = .unknown
  /// What the computers are told: the registration while notifications are allowed and APNs gave a token
  private(set) var target: PushTarget?
  /// An opened notification waiting for its computer's wire (RootView)
  var opening: PushOpen?
  /// Whether the screens show this session of this computer now; a banner for it would only repeat the screen
  @ObservationIgnored var showing: (_ computer: String, _ session: String) -> Bool = { _, _ in false }
  @ObservationIgnored private var token: String?
  @ObservationIgnored private let log = Logger(subsystem: "cc.hotic.acpira", category: "push")

  /// Whether the computers can be told something definite: a registration, or that there is none. While the system's
  /// answer or APNs' token is still on its way, saying "none" would only stop the notifications for a moment
  var settled: Bool {
    switch permission {
    case .unknown: false
    case .on: target != nil
    case .notAsked, .off: true
    }
  }

  /// At launch (the app delegate): the delegate for presenting and opening notifications, and the system's word
  public func install() {
    UNUserNotificationCenter.current().delegate = self
    Task { await refresh() }
  }

  /// A computer's wire is up: the first time, ask; then register with APNs while allowed
  func activate() async {
    let center = UNUserNotificationCenter.current()
    if await center.notificationSettings().authorizationStatus == .notDetermined {
      do {
        _ = try await center.requestAuthorization(options: [.alert, .sound])
      } catch {
        log.error("authorization failed: \(error.localizedDescription, privacy: .public)")
      }
    }
    await refresh()
  }

  /// Reads the system's word again (the user may have changed it in Settings) and registers with APNs while allowed
  func refresh() async {
    let status = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    permission = switch status {
    case .notDetermined: .notAsked
    case .denied: .off
    default: .on
    }
    #if canImport(UIKit)
    if permission == .on { UIApplication.shared.registerForRemoteNotifications() }
    #endif
    updateTarget()
  }

  /// APNs' device token (the app delegate)
  public func registered(_ deviceToken: Data) {
    token = deviceToken.map { String(format: "%02x", $0) }.joined()
    log.info("registered for notifications (\(Self.sandbox ? "sandbox" : "production", privacy: .public)): \(self.token ?? "", privacy: .private)")
    updateTarget()
  }

  public func failed(_ error: Error) {
    log.error("APNs registration failed: \(error.localizedDescription, privacy: .public)")
  }

  private func updateTarget() {
    let info = Bundle.main.infoDictionary
    guard permission == .on, let token, let team = info?["AcpiraTeamID"] as? String, !team.isEmpty,
          let topic = Bundle.main.bundleIdentifier else {
      if target != nil { target = nil }
      return
    }
    let next = PushTarget(token: token, topic: topic, team: team, sandbox: Self.sandbox)
    if next != target { target = next }
  }

  /// Whether this build's token is a sandbox one: a development-signed build carries a provisioning profile with
  /// `aps-environment` development; App Store and TestFlight builds carry none and use production
  static let sandbox: Bool = {
    guard let url = Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision"),
          let data = try? Data(contentsOf: url) else {
      #if targetEnvironment(simulator)
      return true
      #else
      return false
      #endif
    }
    // The profile is a signed plist; its XML sits in the clear between the signature's bytes
    let text = String(decoding: data, as: UTF8.self)
    guard let key = text.range(of: "<key>aps-environment</key>") else { return true }
    return text[key.upperBound...].prefix(80).contains("development")
  }()

  /// `acpira` in a notification's payload: the computer and the session
  nonisolated static func origin(_ info: [AnyHashable: Any]) -> PushOpen? {
    guard let acpira = info["acpira"] as? [String: Any], let session = acpira["session"] as? String,
          let computer = acpira["computer"] as? String else { return nil }
    return PushOpen(computer: computer, session: session)
  }
}

extension PushCenter: UNUserNotificationCenterDelegate {
  /// In the foreground: shown, unless the screen already shows that session
  public nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async -> UNNotificationPresentationOptions {
    let origin = Self.origin(notification.request.content.userInfo)
    let shown = await MainActor.run { origin.map { showing($0.computer, $0.session) } ?? false }
    return shown ? [] : [.banner, .list, .sound]
  }

  /// Opened: RootView follows that computer and opens the session
  public nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
    guard response.actionIdentifier == UNNotificationDefaultActionIdentifier,
          let origin = Self.origin(response.notification.request.content.userInfo) else { return }
    await MainActor.run { opening = origin }
  }
}
