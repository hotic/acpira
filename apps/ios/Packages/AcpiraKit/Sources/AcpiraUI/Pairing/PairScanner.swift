import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import AVFoundation
import UIKit
import Vision
import VisionKit
#endif

// The camera card that leads the add-computer sheet: VisionKit's data scanner reading QR codes only, under a viewfinder.
// Where the phone has no camera (the simulator) the card is a quiet stand-in; where access was refused it says so and
// opens Settings. Any QR code that is not an `acpira://pair/…` link is ignored.

struct PairCameraCard: View {
  /// Scans only while the sheet's form shows and nothing is being paired
  var active: Bool
  /// A pairing link and the code's rows as read
  var onLink: (String, [String]?) -> Void
  @Environment(\.pairing) private var pairing
  @Environment(\.scenePhase) private var scenePhase
  @State private var source: Source?

  private enum Source { case live, unavailable, denied }

  var body: some View {
    ZStack {
      switch source {
      case .live?:
        #if canImport(UIKit)
        PairDataScanner(active: active, onLink: onLink) { source = .unavailable }
        PairFinder(ink: Palette.pairFinderInk, dim: true)
        #endif
      case .denied?:
        Palette.formRow
        VStack(spacing: Metrics.gap) {
          Image(systemName: "camera").font(.system(size: Metrics.pairGlyph, weight: .light)).foregroundStyle(Palette.fg3)
          Text(web: t("mobile.pair.camera.denied"), size: TypeScale.text2.size, paragraph: false)
            .textStyle(TypeScale.text2)
            .foregroundStyle(Palette.fg2)
          #if canImport(UIKit)
          Button {
            if let url = URL(string: UIApplication.openSettingsURLString) { UIApplication.shared.open(url) }
          } label: { Text(web: t("mobile.pair.camera.settings"), size: TypeScale.text2.size, paragraph: false) }
          .buttonStyle(PairGlassStyle())
          #endif
        }
        .padding(.horizontal, Metrics.page)
      case .unavailable?:
        // The viewfinder alone, faint, so the card keeps its shape and says where a camera would be
        Palette.formRow
        PairFinder(ink: Palette.line, dim: false)
        Text(web: t("mobile.camera.unavailable"), size: TypeScale.text3.size, paragraph: false)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg3)
      case nil:
        // Until access is known (the system asks on first use)
        Palette.formRow
      }
    }
    .frame(maxWidth: .infinity)
    .frame(height: Metrics.pairScanCard)
    .clipped()
    .task { source = await resolve() }
    // Access granted in Settings meanwhile: the card comes back to life on return
    .onChange(of: scenePhase) { _, phase in
      guard phase == .active, source == .denied else { return }
      Task { source = await resolve() }
    }
  }

  private func resolve() async -> Source {
    switch pairing?.camera ?? .auto {
    case .unavailable: return .unavailable
    case .denied: return .denied
    case .auto: break
    }
    #if canImport(UIKit)
    guard DataScannerViewController.isSupported else { return .unavailable }
    switch AVCaptureDevice.authorizationStatus(for: .video) {
    case .authorized: return .live
    case .notDetermined: return await AVCaptureDevice.requestAccess(for: .video) ? .live : .denied
    default: return .denied
    }
    #else
    return .unavailable
    #endif
  }
}

/// The viewfinder: four rounded corners on a square in the card's middle, the rest dimmed over a live camera
struct PairFinder: View {
  var ink: Color
  var dim: Bool

  var body: some View {
    GeometryReader { card in
      let side = Metrics.pairFinder
      let square = CGRect(x: (card.size.width - side) / 2, y: (card.size.height - side) / 2, width: side, height: side)
      ZStack {
        if dim {
          // Even-odd, so the square stays clear
          Path { p in
            p.addRect(CGRect(origin: .zero, size: card.size))
            p.addRoundedRect(in: square, cornerSize: CGSize(width: Metrics.pairFinderRadius, height: Metrics.pairFinderRadius))
          }
          .fill(Palette.pairFinderDim, style: FillStyle(eoFill: true))
        }
        PairFinderCorners()
          .stroke(ink, style: StrokeStyle(lineWidth: Metrics.pairFinderStroke, lineCap: .round, lineJoin: .round))
          .frame(width: side, height: side)
          .position(x: square.midX, y: square.midY)
      }
    }
    .allowsHitTesting(false)
    .accessibilityHidden(true)
  }
}

/// The viewfinder's four rounded corners
nonisolated struct PairFinderCorners: Shape {
  func path(in r: CGRect) -> Path {
    let length = Metrics.pairFinderCorner
    var p = Path()
    let corners: [(CGPoint, CGFloat, CGFloat)] = [
      (CGPoint(x: r.minX, y: r.minY), 1, 1), (CGPoint(x: r.maxX, y: r.minY), -1, 1),
      (CGPoint(x: r.maxX, y: r.maxY), -1, -1), (CGPoint(x: r.minX, y: r.maxY), 1, -1),
    ]
    for (corner, dx, dy) in corners {
      p.move(to: CGPoint(x: corner.x, y: corner.y + dy * length))
      p.addArc(tangent1End: corner, tangent2End: CGPoint(x: corner.x + dx * length, y: corner.y), radius: Metrics.pairFinderRadius)
      p.addLine(to: CGPoint(x: corner.x + dx * length, y: corner.y))
    }
    return p
  }
}

#if canImport(UIKit)
/// VisionKit's scanner set to QR codes, without its own highlights or guidance (the card draws the viewfinder). It hands
/// on the first pairing link it sees, with the modules it read (the sheet draws that very code), and stays quiet about
/// anything else
struct PairDataScanner: UIViewControllerRepresentable {
  var active: Bool
  var onLink: (String, [String]?) -> Void
  var onUnavailable: () -> Void

  func makeUIViewController(context: Context) -> DataScannerViewController {
    let scanner = DataScannerViewController(
      recognizedDataTypes: [.barcode(symbologies: [.qr])], qualityLevel: .balanced, recognizesMultipleItems: false,
      isHighFrameRateTrackingEnabled: false, isPinchToZoomEnabled: false, isGuidanceEnabled: false,
      isHighlightingEnabled: false)
    scanner.delegate = context.coordinator
    return scanner
  }

  func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {
    context.coordinator.parent = self
    if active && !scanner.isScanning {
      do { try scanner.startScanning() } catch { onUnavailable() }
    } else if !active && scanner.isScanning {
      scanner.stopScanning()
    }
  }

  static func dismantleUIViewController(_ scanner: DataScannerViewController, coordinator: Coordinator) {
    scanner.stopScanning()
  }

  func makeCoordinator() -> Coordinator { Coordinator(self) }

  final class Coordinator: NSObject, DataScannerViewControllerDelegate {
    var parent: PairDataScanner
    init(_ parent: PairDataScanner) { self.parent = parent }

    func dataScanner(_ scanner: DataScannerViewController, didAdd added: [RecognizedItem], allItems: [RecognizedItem]) {
      guard parent.active else { return }
      for item in added {
        guard case .barcode(let code) = item, let text = code.payloadStringValue, PairingModel.isPairingLink(text) else { continue }
        scanner.stopScanning()
        parent.onLink(text, PairCodeRows.make(link: text, scanned: code.observation.barcodeDescriptor))
        return
      }
    }

    func dataScanner(_ scanner: DataScannerViewController, becameUnavailableWithError error: DataScannerViewController.ScanningUnavailable) {
      parent.onUnavailable()
    }
  }
}
#endif
