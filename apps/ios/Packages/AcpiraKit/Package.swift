// swift-tools-version: 6.2

// AcpiraKit holds nearly all of the iOS app; the Xcode project only wraps it in an app target. It is a local package so
// new files never touch the project file, and so the platform-neutral modules test with `swift test` on the Mac in seconds.
// Every module must also build for macOS (`swift test` builds all of them): UIKit-only code goes behind `#if canImport(UIKit)`.

import PackageDescription

// The app target's concurrency dialect (SWIFT_APPROACHABLE_CONCURRENCY, SWIFT_UPCOMING_FEATURE_MEMBER_IMPORT_VISIBILITY),
// so code reads the same on both sides of the package boundary
let dialect: [SwiftSetting] = [
  .enableUpcomingFeature("MemberImportVisibility"),
  .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
  .enableUpcomingFeature("InferIsolatedConformances"),
]

let modules = ["AcpiraModels", "AcpiraSync", "AcpiraDesign", "AcpiraMarkdown", "AcpiraFixtures", "AcpiraUI"]

let package = Package(
  name: "AcpiraKit",
  platforms: [.iOS(.v26), .macOS(.v26)],
  products: [
    // One product for the app to link; it imports the modules it needs one by one
    .library(name: "AcpiraKit", targets: modules),
  ],
  targets: [
    // Codable mirrors of the host ↔ webview contract in packages/shared
    .target(name: "AcpiraModels", swiftSettings: dialect),
    // Envelope client, session patches, transports
    .target(name: "AcpiraSync", dependencies: ["AcpiraModels"], swiftSettings: dialect),
    // Tokens and motion parameters from the webview CSS plus the phone's own sizes, the Lucide and vendor marks
    // Inter ships as a bundled resource (OFL), registered at launch
    .target(name: "AcpiraDesign", resources: [.copy("Fonts")], swiftSettings: dialect),
    // Markdown → block AST → native views, safe against half-streamed syntax
    .target(name: "AcpiraMarkdown", swiftSettings: dialect),
    // Synthetic session views shared with the web reference page (lab/ios-reference), for previews, screenshots and tests
    .target(name: "AcpiraFixtures", dependencies: ["AcpiraModels"], resources: [.copy("synthetic")], swiftSettings: dialect),
    // Screens and components; main-actor by default, like the app target. The i18n tables are exported from the
    // webview dictionaries by scripts/export/ios-i18n.ts, the syntax highlighter (shiki for JavaScriptCore, its grammars
    // and themes) by scripts/export/ios-shiki.ts
    .target(
      name: "AcpiraUI",
      dependencies: ["AcpiraModels", "AcpiraSync", "AcpiraDesign", "AcpiraMarkdown", "AcpiraFixtures"],
      resources: [.copy("Resources/i18n"), .copy("Resources/shiki")],
      swiftSettings: dialect + [.defaultIsolation(MainActor.self)]
    ),
    .testTarget(
      name: "AcpiraKitTests",
      dependencies: modules.map { .target(name: $0) },
      swiftSettings: dialect
    ),
  ]
)
