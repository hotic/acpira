import AcpiraUI
import SwiftUI
import UIKit
#if canImport(AcpiraRemote)
import AcpiraRemote
#endif

// The app target stays thin: scenes and system integration live here, everything else in Packages/AcpiraKit
@main
struct AcpiraApp: App {
  /// APNs hands the device token to an app delegate only
  @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
  /// The paired-remote service, when this build carries the networking artifacts (apps/ios/scripts/build-net.sh); one
  /// for the app's lifetime, since it owns the device's endpoint
  #if canImport(AcpiraRemote)
  @State private var pairing = RemotePairingService()
  #endif

  var body: some Scene {
    WindowGroup {
      #if canImport(AcpiraRemote)
      RootView(pairingService: pairing)
      #else
      RootView()
      #endif
    }
  }
}

/// Notifications from paired computers (AcpiraUI `PushCenter`): its delegate is set before a launch from a notification
/// delivers it, and the APNs token goes to it
final class AppDelegate: NSObject, UIApplicationDelegate {
  func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
    PushCenter.shared.install()
    return true
  }

  func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
    PushCenter.shared.registered(deviceToken)
  }

  func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
    PushCenter.shared.failed(error)
  }
}
