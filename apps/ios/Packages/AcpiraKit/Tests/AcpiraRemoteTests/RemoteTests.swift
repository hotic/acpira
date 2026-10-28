import AcpiraModels
import AcpiraNet
@testable import AcpiraRemote
import AcpiraSync
import Foundation
import Testing

/// The library and the Swift mirror speak the same sidecar protocol, or every link would end as incompatible
@Test func theLibrarySpeaksTheMirrorsProtocol() {
  #expect(protocolVersion() == Int64(SidecarProtocol.version))
}

@Test func aBonjourRecordNamesAComputer() throws {
  let id = String(repeating: "ab", count: 32)
  let c = try #require(NearbyBrowser.computer(name: "Studio", txt: ["v": "1", "id": id, "a": "192.168.71.85:64522, 192.168.71.147:64522", "m": "Mac16,11"]))
  #expect(c == NearbyComputer(id: id, name: "Studio", model: "Mac16,11", addrs: ["192.168.71.85:64522", "192.168.71.147:64522"]))
  #expect(NearbyBrowser.computer(name: "Old", txt: ["v": "2", "id": id]) == nil, "another version's record")
  #expect(NearbyBrowser.computer(name: "Bare", txt: ["v": "1"]) == nil)
  // Seen on Wi-Fi and Ethernet: one computer with both sets of addresses
  let merged = NearbyBrowser.merged([
    NearbyComputer(id: id, name: "Studio", addrs: ["192.168.1.2:5"]),
    NearbyComputer(id: id, name: "Studio", model: "Mac16,11", addrs: ["10.0.0.2:5", "192.168.1.2:5"]),
    NearbyComputer(id: "other", name: "Air"),
  ])
  #expect(merged.map(\.name) == ["Air", "Studio"])
  #expect(merged.last?.addrs == ["192.168.1.2:5", "10.0.0.2:5"])
  #expect(merged.last?.model == "Mac16,11")
}

@Test func failuresReadAsTheContractsErrors() {
  #expect(ConnectionError(LinkFailure.Offline) == .offline)
  #expect(ConnectionError(LinkFailure.Revoked) == .revoked)
  #expect(ConnectionError(LinkFailure.Incompatible(message: "update this app")) == .incompatible("update this app"))
  #expect(PairingError(PairFailure.Impostor) == .impostor)
  #expect(PairingError(PairFailure.Wrong) == .wrong)
  #expect(PairingError(PairFailure.InvalidLink) == .invalidLink)
}

@Test func pairedComputersOutliveTheService() async throws {
  let dir = FileManager.default.temporaryDirectory.appending(path: "acpira-remote-\(UUID().uuidString)")
  defer { try? FileManager.default.removeItem(at: dir) }
  let store = ComputerStore(directory: dir)
  let a = PairedComputer(id: "a", name: "Studio", relay: "https://relay.example./", addrs: ["192.168.1.2:5"], pairedAt: Date(timeIntervalSince1970: 1_000))
  let b = PairedComputer(id: "b", name: "Air", pairedAt: Date(timeIntervalSince1970: 2_000))
  try store.save([a, b])
  let service = RemotePairingService(directory: dir, keys: MemoryIdentityStore(), relays: .off, observesLifecycle: false)
  #expect(await service.computers().map(\.id) == ["b", "a"], "most recently paired first")
  #expect(throws: PairingError.invalidLink) { try service.computerName(link: "acpira://pair/nope") }
  await service.remove("b")
  #expect(store.load() == [a])
}

@Test func aComputerNobodyAnswersForIsOffline() async throws {
  let dir = FileManager.default.temporaryDirectory.appending(path: "acpira-remote-\(UUID().uuidString)")
  defer { try? FileManager.default.removeItem(at: dir) }
  let keys = MemoryIdentityStore()
  let service = RemotePairingService(directory: dir, keys: keys, relays: .off, observesLifecycle: false)
  let stranger = try await startNode(secret: generateKey(), relays: .off)
  // No address and no relay: refused at once, where a silent address would take the whole probe deadline
  let nobody = PairedComputer(id: stranger.id(), name: "Gone")
  await stranger.close()
  await #expect(throws: ConnectionError.offline) { _ = try await service.connect(nobody) }
  #expect(await service.presence(of: nobody) == .offline)
  // The node it started made a key and kept it
  #expect(try keys.load()?.count == 32)
}

/// Against a running gateway: `apps/ios/scripts/gateway.sh` starts one on a temporary home (relays off, the fake agent
/// as the only agent) and keeps a fresh pairing link in the file it prints; `ACPIRA_TEST_GATEWAY=<that file> swift test`
@Test(.enabled(if: ProcessInfo.processInfo.environment["ACPIRA_TEST_GATEWAY"] != nil))
func pairsWithALocalGatewayAndReachesItsEngine() async throws {
  let file = try #require(ProcessInfo.processInfo.environment["ACPIRA_TEST_GATEWAY"])
  let link = try String(contentsOfFile: file, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
  let dir = FileManager.default.temporaryDirectory.appending(path: "acpira-remote-\(UUID().uuidString)")
  defer { try? FileManager.default.removeItem(at: dir) }
  let service = RemotePairingService(directory: dir, keys: MemoryIdentityStore(), relays: .off,
                                     device: .init(name: "swift test", version: "0"), observesLifecycle: false)

  let named = try service.computerName(link: link)
  let computer = try await service.pair(link: link)
  #expect(computer.name == named)
  #expect(await service.computers() == [computer])
  #expect(!computer.addrs.isEmpty)
  await #expect(throws: PairingError.closed) { _ = try await service.pair(link: link) }
  #expect(await service.presence(of: computer) == .online(.lan), "probed without an engine wire")

  let connection = try await service.connect(computer)
  let hello = Hello(requestId: "swift-test", client: ClientInfo(name: "acpira-ios", version: "0", capabilities: EngineClient.capabilities),
                    env: ShellEnv(hostLanguage: "en", projects: true))
  let client = EngineClient(transport: connection, hello: hello)
  await client.start()
  // The page's first message: the engine answers it with the view's init
  client.post(.ready(patches: true))
  // Ends the wait below if the engine never answers
  let watchdog = Task {
    try await Task.sleep(for: .seconds(30))
    client.stop()
  }
  var connected: HelloOk?
  var listed = false
  for await event in client.events {
    switch event {
    case .connected(let ok): connected = ok
    case .host(.initState): listed = true
    case .closed(let why): Issue.record("closed: \(why ?? "from this side")")
    default: break
    }
    if listed { break }
  }
  watchdog.cancel()
  #expect(connected?.protocolVersion == SidecarProtocol.version)
  #expect(listed, "the view's init arrived through the gateway")

  var paths = connection.paths().makeAsyncIterator()
  #expect(await paths.next() == .lan, "a gateway on this Mac is on the LAN")
  let rtt = try await #require(connection as? RemoteConnection).probe(withinMs: 3_000)
  #expect(rtt < .seconds(3))
  client.stop()
}
