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
  /// The root, made once with the paired-remote service when this build carries the networking artifacts
  /// (apps/ios/scripts/build-net.sh), which owns the device's endpoint for the app's lifetime. The scene's body runs
  /// again on pushes, and a root made there each time built and threw away a whole app model and language table
  @State private var root: RootView

  init() {
    #if canImport(AcpiraRemote)
    _root = State(initialValue: RootView(pairingService: RemotePairingService()))
    #else
    _root = State(initialValue: RootView())
    #endif
  }

  var body: some Scene {
    WindowGroup { root }
  }
}

/// Notifications from paired computers (AcpiraUI `PushCenter`): its delegate is set before a launch from a notification
/// delivers it, and the APNs token goes to it
final class AppDelegate: NSObject, UIApplicationDelegate {
  func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
    PushCenter.shared.install()
    // Settings → Developer's frame-rate and hitch overlay follows its switches from here on
    PerfHUD.shared.install()
    #if DEBUG
    // `-frameprobe`: frame timing for scripted performance runs (AcpiraUI FrameProbe)
    FrameProbe.installIfAsked()
    #endif
    return true
  }

  func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
    PushCenter.shared.registered(deviceToken)
  }

  func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
    PushCenter.shared.failed(error)
  }
}
