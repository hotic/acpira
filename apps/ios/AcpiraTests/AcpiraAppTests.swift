import Foundation
import Testing

// Unit tests run hosted inside the app, so Bundle.main is Acpira.app: this pins the test host wiring and the bundle id
struct AcpiraAppTests {
  @Test func testsRunInsideTheApp() {
    #expect(Bundle.main.bundleIdentifier == "cc.hotic.acpira")
  }
}
