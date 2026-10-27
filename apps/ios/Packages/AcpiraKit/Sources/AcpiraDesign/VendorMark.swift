import Foundation
import SwiftUI

// chat/marks.tsx MarkSvg + shared/models.ts modelBrand: a vendor logo filled in currentColor, or the name's initial in a
// rounded square when the brand is unknown, so rows with and without a known brand stay aligned.

public enum VendorBrand {
  /// The brand key of a model family ("Claude Fable 5.1" → claude), by the webview's BRAND rules
  public static func of(_ family: String) -> String? {
    let lower = family.lowercased()
    for rule in VendorMarks.brands {
      if lower.range(of: rule.pattern, options: .regularExpression) != nil { return rule.brand }
    }
    return nil
  }
}

private enum MarkCache {
  nonisolated(unsafe) static var paths: [String: Path] = [:]
  static let lock = NSLock()
  static func path(_ key: String) -> Path? {
    lock.lock(); defer { lock.unlock() }
    if let hit = paths[key] { return hit }
    guard let ds = VendorMarks.paths[VendorMarks.alias[key] ?? key] else { return nil }
    var path = Path()
    for d in ds { path.addPath(SVGPath.parse(d)) }
    paths[key] = path
    return path
  }
}

public struct VendorMark: View {
  let brand: String?
  let name: String
  var size: CGFloat
  /// Fit the logo's own ink to the frame, centred, instead of drawing its 24-unit box: logos sit off-centre and at
  /// different sizes inside their boxes, which shows when several stand in one column (the phone's session list)
  var fit: Bool

  public init(brand: String?, name: String, size: CGFloat = Metrics.icon, fit: Bool = false) {
    self.brand = brand; self.name = name; self.size = size; self.fit = fit
  }

  public var body: some View {
    if let brand, let path = MarkCache.path(brand) {
      Group {
        if fit {
          FittedMarkShape(path: path).fill(style: FillStyle(eoFill: true))
        } else {
          MarkShape(path: path).fill(style: FillStyle(eoFill: true))
        }
      }
      .frame(width: size, height: size)
    } else {
      let letter = name.trimmingCharacters(in: .whitespaces).first.map { String($0).uppercased() } ?? "?"
      ZStack {
        RoundedRectangle(cornerRadius: size * 4 / 24).strokeBorder(lineWidth: size * 1.5 / 24)
          .frame(width: size * 17 / 24, height: size * 17 / 24)
        Text(verbatim: letter).font(.system(size: size / 2, weight: .semibold))
      }
      .frame(width: size, height: size)
    }
  }
}

/// A path's bounding box scaled to fit its frame (aspect kept) and centred in it
struct FittedMarkShape: Shape {
  let path: Path
  func path(in rect: CGRect) -> Path {
    let box = path.boundingRect
    guard box.width > 0, box.height > 0 else { return path }
    let scale = min(rect.width / box.width, rect.height / box.height)
    let dx = rect.midX - box.midX * scale
    let dy = rect.midY - box.midY * scale
    return path.applying(CGAffineTransform(a: scale, b: 0, c: 0, d: scale, tx: dx, ty: dy))
  }
}

/// A 24-unit path scaled into its frame
struct MarkShape: Shape {
  let path: Path
  func path(in rect: CGRect) -> Path {
    path.applying(CGAffineTransform(scaleX: rect.width / 24, y: rect.height / 24).translatedBy(x: rect.minX, y: rect.minY))
  }
}
