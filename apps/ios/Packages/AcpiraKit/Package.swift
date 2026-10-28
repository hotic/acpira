// swift-tools-version: 6.2

// AcpiraKit holds nearly all of the iOS app; the Xcode project only wraps it in an app target. It is a local package so
// new files never touch the project file, and so the platform-neutral modules test with `swift test` on the Mac in seconds.
// Every module must also build for macOS (`swift test` builds all of them): UIKit-only code goes behind `#if canImport(UIKit)`.

import Foundation
import PackageDescription

// The app target's concurrency dialect (SWIFT_APPROACHABLE_CONCURRENCY, SWIFT_UPCOMING_FEATURE_MEMBER_IMPORT_VISIBILITY),
// so code reads the same on both sides of the package boundary
let dialect: [SwiftSetting] = [
  .enableUpcomingFeature("MemberImportVisibility"),
  .enableUpcomingFeature("NonisolatedNonsendingByDefault"),
  .enableUpcomingFeature("InferIsolatedConformances"),
]

let modules = ["AcpiraModels", "AcpiraSync", "AcpiraDesign", "AcpiraMarkdown", "AcpiraFixtures", "AcpiraUI"]

// Paired remote access: acpira-mobile's XCFramework and its UniFFI bindings (apps/ios/scripts/build-net.sh), git-ignored
// build output under Artifacts/. AcpiraNet wraps them and AcpiraRemote implements AcpiraSync's PairingService on top;
// both exist only once the artifacts were built, so a checkout that never built them still resolves and tests.
// Trap: SwiftPM and Xcode cache this manifest's evaluation keyed by its text and the environment, not by the files it
// looks at. check.sh passes ACPIRA_NET_STAMP (the build's stamp) so a fresh build is seen; elsewhere a run with a cached
// evaluation keeps the old answer until the environment or this file changes (`swift package --manifest-cache none`,
// or Xcode's Reset Package Caches)
// ACPIRA_NET=0 leaves them out even when built (check.sh's run without them)
let artifacts = Context.packageDirectory + "/Artifacts"
let net = Context.environment["ACPIRA_NET"] != "0"
  && FileManager.default.fileExists(atPath: artifacts + "/AcpiraNetFFI.xcframework/Info.plist")
  && FileManager.default.fileExists(atPath: artifacts + "/AcpiraNet/acpira_mobile.swift")
let netTargets: [Target] = !net ? [] : [
  .binaryTarget(name: "AcpiraNetFFI", path: "Artifacts/AcpiraNetFFI.xcframework"),
  // The generated bindings, without the dialect: code written for plain Swift 6
  .target(
    name: "AcpiraNet",
    dependencies: ["AcpiraNetFFI"],
    path: "Artifacts/AcpiraNet",
    // What the Rust side calls into: interface and route lookups (netdev, netwatch)
    linkerSettings: ["CoreFoundation", "Network", "SystemConfiguration"].map { .linkedFramework($0) }
  ),
  .target(name: "AcpiraRemote", dependencies: ["AcpiraModels", "AcpiraSync", "AcpiraNet"], swiftSettings: dialect),
  .testTarget(name: "AcpiraRemoteTests", dependencies: ["AcpiraModels", "AcpiraSync", "AcpiraNet", "AcpiraRemote"], swiftSettings: dialect),
]

let package = Package(
  name: "AcpiraKit",
  platforms: [.iOS(.v26), .macOS(.v26)],
  products: [
    // One product for the app to link; it imports the modules it needs one by one
    // With the networking artifacts built it carries the real pairing service too, so the app target links nothing
    // else and picks it up with `canImport(AcpiraRemote)`
    .library(name: "AcpiraKit", targets: modules + (net ? ["AcpiraNet", "AcpiraRemote"] : [])),
  ] + (net ? [.library(name: "AcpiraRemote", targets: ["AcpiraRemote"])] : []),
  dependencies: [
    // The terminal page's emulator and view (MIT), pinned: its 2.0 API is still moving between releases. No traits:
    // the default one only adds PNG decoders for platforms without ImageIO
    .package(url: "https://github.com/migueldeicaza/SwiftTerm", exact: "1.99.1", traits: []),
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
    // and themes) by scripts/export/ios-shiki.ts. SwiftTerm draws the terminal page on iOS only: the Mac build (swift
    // test) keeps the terminal's logic and skips compiling the emulator's AppKit front end
    .target(
      name: "AcpiraUI",
      dependencies: [
        "AcpiraModels", "AcpiraSync", "AcpiraDesign", "AcpiraMarkdown", "AcpiraFixtures",
        .product(name: "SwiftTerm", package: "SwiftTerm", condition: .when(platforms: [.iOS])),
      ],
      resources: [.copy("Resources/i18n"), .copy("Resources/shiki"), .copy("Resources/web")],
      swiftSettings: dialect + [.defaultIsolation(MainActor.self)]
    ),
    .testTarget(
      name: "AcpiraKitTests",
      dependencies: modules.map { .target(name: $0) },
      swiftSettings: dialect
    ),
  ] + netTargets
)
