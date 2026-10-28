import AcpiraDesign
import CoreImage
import CoreImage.CIFilterBuiltins
import SwiftUI

// The code the phone pairs with, drawn on the sheet as the computer's pairing card draws it (packages/ui/remote/
// pairGeometry.ts, devices.css): round modules in rings from the centre, rounded finder eyes, Acpira's mark in a
// knock-out, the dim track on the box edge. The card learns of the pairing from the gateway the moment it answers the
// phone, so both screens reach it together, and both dissolve the rings outward on the same timeline (Motion.pair…):
// a phone held up to the card plays one moment with it.

/// A code's modules as rows of `1` (dark) and `0`, no quiet zone, as the gateway sends them to its card
enum PairCodeRows {
  private static let context = CIContext()

  /// The symbol a scan read, drawn again from its descriptor: the card's very modules (version, mask and codewords
  /// all come with it). Without one (a pasted link, a link opened from outside) a code made from the link's text: the
  /// same content and size, but the encoder may pick another mask than the gateway's
  static func make(link: String, scanned: CIBarcodeDescriptor? = nil) -> [String]? {
    let image: CIImage?
    if let scanned {
      let generator = CIFilter.barcodeGenerator()
      generator.barcodeDescriptor = scanned
      image = generator.outputImage
    } else {
      let generator = CIFilter.qrCodeGenerator()
      generator.message = Data(link.utf8)
      // The gateway's `qrcode` default
      generator.correctionLevel = "M"
      image = generator.outputImage
    }
    // One pixel a module, the generator's own quiet zone around it
    guard let image, image.extent.width == image.extent.height, image.extent.width <= 256 else { return nil }
    let side = Int(image.extent.width)
    guard side > 0 else { return nil }
    var pixels = [UInt8](repeating: 255, count: side * side * 4)
    context.render(image, toBitmap: &pixels, rowBytes: side * 4, bounds: image.extent, format: .RGBA8,
                   colorSpace: CGColorSpaceCreateDeviceRGB())
    let dark = (0..<side).map { y in (0..<side).map { x in pixels[(y * side + x) * 4] < 128 } }
    // The finders' top edge is the first dark row; the quiet zone is as wide on every side
    guard let quiet = dark.firstIndex(where: { $0.contains(true) }), side - 2 * quiet > 0 else { return nil }
    let n = side - 2 * quiet
    return dark[quiet..<(quiet + n)].map { row in String(row[quiet..<(quiet + n)].map { $0 ? "1" : "0" }) }
  }
}

/// The code as paths in module units, one per ring from the centre out (pairGeometry.ts `qrShape`)
nonisolated struct PairCodeShape {
  /// The quiet zone, in modules, and the rings the paired moment dissolves outward
  static let quiet = 4
  static let bandCount = 11
  /// A dot's radius and the finder eye's three corner radii (outer ring, its hole, the inner square), in modules
  private static let dot: CGFloat = 0.42
  private static let eye: (CGFloat, CGFloat, CGFloat) = (2.3, 1.5, 1.05)

  let n: Int
  /// The centre knock-out for the mark, in modules
  let hole: Int
  /// Dots and eyes of each ring, filled even-odd (an eye's hole is a subpath)
  let bands: [Path]

  /// The knock-out: 11 on a 57-module code, smaller on smaller codes, odd so it is centred on whole modules
  static func holeOf(_ n: Int) -> Int {
    let h = max(7, min(11, Int((Double(n) * 11 / 57).rounded())))
    return h % 2 == n % 2 ? h : h - 1
  }

  init(rows: [String]) {
    let grid = rows.map { Array($0) }
    let n = grid.count
    let hole = Self.holeOf(n)
    let c = CGFloat(n - 1) / 2, far = max(hypot(c, c), 1)
    let band = { (x: Int, y: Int) in
      min(Self.bandCount - 1, Int(hypot(CGFloat(x) - c, CGFloat(y) - c) / far * CGFloat(Self.bandCount)))
    }
    let finder = { (x: Int, y: Int) in (x < 7 && y < 7) || (x >= n - 7 && y < 7) || (x < 7 && y >= n - 7) }
    let h0 = (n - hole) / 2
    let open = { (x: Int, y: Int) in x >= h0 && x < h0 + hole && y >= h0 && y < h0 + hole }
    var bands = Array(repeating: Path(), count: Self.bandCount)
    for y in 0..<n {
      for x in 0..<grid[y].count where grid[y][x] == "1" && !finder(x, y) && !open(x, y) {
        let r = Self.dot
        bands[band(x, y)].addEllipse(in: CGRect(x: CGFloat(x) + 0.5 - r, y: CGFloat(y) + 0.5 - r, width: 2 * r, height: 2 * r))
      }
    }
    if n >= 7 {
      for (fx, fy) in [(0, 0), (n - 7, 0), (0, n - 7)] {
        var eye = Path()
        for (inset, radius) in [(0, Self.eye.0), (1, Self.eye.1), (2, Self.eye.2)] {
          let side = CGFloat(7 - 2 * inset)
          eye.addRoundedRect(in: CGRect(x: CGFloat(fx + inset), y: CGFloat(fy + inset), width: side, height: side),
                             cornerSize: CGSize(width: radius, height: radius), style: .circular)
        }
        bands[band(fx + 3, fy + 3)].addPath(eye)
      }
    }
    self.n = n
    self.hole = hole
    self.bands = bands
  }
}

/// One ring's modules scaled from module units into the code's box, quiet zone included
nonisolated struct PairCodeBand: Shape {
  let path: Path
  let modules: CGFloat

  func path(in rect: CGRect) -> Path {
    let s = rect.width / modules
    let q = CGFloat(PairCodeShape.quiet)
    return path.applying(CGAffineTransform(a: s, b: 0, c: 0, d: s, tx: rect.minX + q * s, ty: rect.minY + q * s))
  }
}

/// The scanned code in its box. `paired` dissolves the rings outward from the mark, fades the track, and puts the
/// computer with its check in the mark's place; `dim` steps the code back behind a failure. Every state draws in the
/// same box, so nothing under it moves
struct PairCodeFrame: View {
  let rows: [String]
  var symbol: String
  var paired: Bool
  var dim = false
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    let shape = PairCodeFrame.shape(rows)
    let side = Metrics.pairCode
    let modules = CGFloat(shape.n + 2 * PairCodeShape.quiet)
    let hole = CGFloat(shape.hole) / modules * side
    ZStack {
      RoundedRectangle(cornerRadius: Metrics.pairCodeRadius, style: .circular)
        .strokeBorder(Palette.pairTrack, lineWidth: Metrics.pairCodeTrack)
        .opacity(paired ? 0 : 1)
        .animation(motion ? Motion.pairTrackOut : nil, value: paired)
      ForEach(shape.bands.indices, id: \.self) { i in
        let lag = Double(i) * Motion.pairBandStagger
        PairCodeBand(path: shape.bands[i], modules: modules)
          .fill(Palette.fg1, style: FillStyle(eoFill: true))
          .animation(motion ? Motion.pairBandFade.delay(lag) : nil) {
            $0.opacity(paired ? 0 : 1).blur(radius: paired ? Motion.pairBandBlur : 0)
          }
          .animation(motion ? Motion.pairBandGrow.delay(lag) : nil) { $0.scaleEffect(paired ? Motion.pairBandScale : 1) }
      }
      ZStack {
        VendorMark(brand: "acpira", name: "Acpira", size: hole * Metrics.pairCodeMark)
          .foregroundStyle(Palette.fg1)
          .animation(motion ? Motion.pairMarkFade : nil) {
            $0.opacity(paired ? 0 : 1).blur(radius: paired ? Motion.pairMarkBlur : 0)
          }
          .animation(motion ? Motion.pairMarkGrow : nil) { $0.scaleEffect(paired ? Motion.pairMarkScale : 1) }
        // The computer overflows the knock-out, as the card's device does: the rings around it are gone by then
        PairGlyph(symbol: symbol, ink: Palette.fg1, checked: paired)
          .fixedSize()
          .animation(motion ? Motion.pairGlyphIn : nil) {
            $0.opacity(paired ? 1 : 0)
              .scaleEffect(paired ? 1 : Motion.pairGlyphFrom)
              .blur(radius: paired ? 0 : Motion.pairGlyphBlur)
          }
      }
      .frame(width: hole, height: hole)
    }
    .frame(width: side, height: side)
    .opacity(dim ? Palette.pairCodeDim : 1)
    .animation(motion ? Motion.ease : nil, value: dim)
    .accessibilityHidden(true)
  }

  /// The last codes drawn: a phase change redraws the frame, the rows stay the same
  private static var shapes: [String: PairCodeShape] = [:]
  private static func shape(_ rows: [String]) -> PairCodeShape {
    let key = rows.joined()
    if let hit = shapes[key] { return hit }
    let shape = PairCodeShape(rows: rows)
    if shapes.count > 4 { shapes.removeAll() }
    shapes[key] = shape
    return shape
  }
}

/// A computer's glyph, the paired check landing on its corner (devices.css `.dv-qr-device`, `.dv-qr-check`). A glyph
/// that shows up already paired (the code page's stage) lands its check too, on the same beat as the card's
struct PairGlyph: View {
  var symbol: String
  var ink: Color
  var checked: Bool
  @Environment(\.motionEnabled) private var motion
  @State private var landed = false

  var body: some View {
    let on = checked && landed
    Image(systemName: symbol)
      .font(.system(size: Metrics.pairGlyph))
      .foregroundStyle(ink)
      .overlay(alignment: .bottomTrailing) {
        ZStack {
          Circle().fill(Palette.ok)
          Icon(.check, size: Metrics.pairCheckIcon, strokeWidth: Metrics.pairCheckStroke).foregroundStyle(Palette.pairCheckInk)
        }
        .frame(width: Metrics.pairCheck, height: Metrics.pairCheck)
        // A ring of the sheet's page cut around the disc, so it reads apart from the glyph under it
        .background(Circle().fill(Palette.formPage).padding(-Metrics.pairCheckRing))
        .offset(x: Metrics.pairCheckX, y: Metrics.pairCheckY)
        .animation(motion ? Motion.pairCheckIn : nil) {
          $0.opacity(on ? 1 : 0).scaleEffect(on ? 1 : Motion.pairCheckFrom)
        }
      }
      .onAppear { landed = true }
  }
}
