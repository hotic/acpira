import AcpiraUI
import SwiftUI

// The app target stays thin: scenes and system integration live here, everything else in Packages/AcpiraKit
@main
struct AcpiraApp: App {
  var body: some Scene {
    WindowGroup {
      RootView()
    }
  }
}
