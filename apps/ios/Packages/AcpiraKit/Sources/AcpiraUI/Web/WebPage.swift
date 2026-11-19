import Foundation
#if canImport(UIKit)
import SafariServices
import UIKit
#endif

extension URL {
  /// A page a browser shows. `SFSafariViewController` takes http and https only and raises on any other scheme
  var isWebPage: Bool { ["http", "https"].contains(scheme?.lowercased() ?? "") }
}

/// A web link from a reply, a preview or the terminal: read in the app's own browser, so the chat stays one
/// swipe away, or handed to the system's
enum WebPage {
  /// True when the in-app browser took the link
  @discardableResult static func open(_ url: URL) -> Bool {
    #if canImport(UIKit)
    guard url.isWebPage,
          let window = UIApplication.shared.connectedScenes.compactMap({ ($0 as? UIWindowScene)?.keyWindow }).first,
          var top = window.rootViewController else { return false }
    // From the topmost controller: a link inside a sheet cannot be presented by the root under it
    while let next = top.presentedViewController, !next.isBeingDismissed { top = next }
    let page = SFSafariViewController(url: url)
    // The app's theme, which may differ from the system's: the browser's bars would follow the system otherwise
    page.overrideUserInterfaceStyle = window.traitCollection.userInterfaceStyle
    top.present(page, animated: true)
    return true
    #else
    return false
    #endif
  }

  /// The system browser, or the app that claims the link
  static func openOutside(_ url: URL) {
    #if canImport(UIKit)
    UIApplication.shared.open(url)
    #endif
  }
}
