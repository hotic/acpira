import AcpiraUI
import SwiftUI
#if canImport(AcpiraRemote)
import AcpiraRemote
#endif

// The app target stays thin: scenes and system integration live here, everything else in Packages/AcpiraKit
@main
struct AcpiraApp: App {
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
