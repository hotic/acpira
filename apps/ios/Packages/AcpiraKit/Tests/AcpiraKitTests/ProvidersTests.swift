import AcpiraModels
import Foundation
import Testing
@testable import AcpiraUI

// Model sources edited from the phone: providers.json fields this build does not know survive a rewrite, and writes go
// out one at a time in order (providerWrites.ts). Plus the folder picker's child paths on either kind of computer.

private func decode<T: Decodable>(_ json: String) -> T { try! JSONDecoder().decode(T.self, from: Data(json.utf8)) }

private func object(_ value: some Encodable) -> [String: Any] {
  try! JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as! [String: Any]
}

@MainActor @Suite struct ProvidersTests {
  private let source: ProviderEntry = decode(#"""
  {"id":"gw","name":"Gateway","preset":"custom","format":"openai-chat","baseUrl":"https://gw/v1","fullUrl":false,"enabled":true,
   "hasKey":true,"headers":{"X-Team":"a"},"models":[{"id":"m1","enabled":true,"input":["text"],"thinking":"auto","efforts":[],
   "sampling":{"temperature":0.2},"estimated":[],"maxSteps":40}]}
  """#)

  @Test func anEditKeepsFieldsThisBuildDoesNotKnowAndDropsHasKey() throws {
    var next = source
    next.models = next.models.map { var m = $0; m.enabled = false; return m } + [ProviderModelEntry(typed: "m2")]
    let sent = object(ProviderAction.save(provider: next))
    let provider = try #require(sent["provider"] as? [String: Any])
    #expect(provider["hasKey"] == nil)
    #expect((provider["headers"] as? [String: String]) == ["X-Team": "a"])
    let models = try #require(provider["models"] as? [[String: Any]])
    #expect(models[0]["enabled"] as? Bool == false)
    #expect(models[0]["maxSteps"] as? Int == 40)
    #expect((models[0]["sampling"] as? [String: Double]) == ["temperature": 0.2])
    // A typed id carries the file's defaults
    #expect(models[1]["thinking"] as? String == "auto" && models[1]["enabled"] as? Bool == true)
  }

  @Test func writesGoOutOneAtATimeAndRowEditsOfOneSourceCollapse() {
    var writes = ProviderWrites()
    var a = source
    a.name = "A"
    let first = writes.push(.save(provider: a), optimistic: true)
    #expect(first.send != nil)
    var b = source
    b.name = "B"
    var c = source
    c.name = "C"
    #expect(writes.push(.save(provider: b), optimistic: true).send == nil)
    // The queued snapshot of the same source is replaced, not queued behind
    #expect(writes.push(.save(provider: c), optimistic: true).send == nil)
    #expect(writes.queue.count == 1)
    #expect(writes.drafts["gw"]?.name == "C")
    // A form's write is never merged
    let form = writes.push(.delete(id: "gw"), optimistic: false)
    #expect(writes.queue.count == 2)
    let one = writes.receive()
    #expect(one.settled?.id == first.id)
    #expect(one.send == .save(provider: c))
    _ = writes.receive()
    let three = writes.receive()
    #expect(three.settled?.id == form.id && three.send == nil && !writes.pending)
  }

  @Test func aReadAnswerSettlesNoWrite() {
    var writes = ProviderWrites()
    let first = writes.refresh(), second = writes.refresh()
    #expect(first && !second)
    let queued = writes.push(.delete(id: "gw"), optimistic: false)
    #expect(queued.send == nil)
    let read = writes.receive()
    #expect(read.settled == nil && read.send == .delete(id: "gw"))
    #expect(writes.receive().settled?.id == queued.id)
  }

  @Test func childPathsFollowTheComputersSeparator() {
    #expect(childPath("/Users/me", "code") == "/Users/me/code")
    #expect(childPath("/", "Volumes") == "/Volumes")
    #expect(childPath(#"C:\Users\me"#, "code") == #"C:\Users\me\code"#)
    #expect(childPath(#"C:\"#, "Users") == #"C:\Users"#)
  }
}
