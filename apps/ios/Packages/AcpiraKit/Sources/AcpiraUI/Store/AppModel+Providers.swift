import AcpiraModels
import Foundation

// What a paired phone may set up on the computer beyond sessions (devices.rs `device_allowed`): a project folder picked
// from the computer's own folders, and the built-in agent's model sources with their models (ProvidersSection.tsx).
// Both need an engine whose helloOk.features names them; an older one leaves the typed path and no sources page.

/// One folder of the computer as the add-project picker shows it
enum LocalListing: Equatable {
  case folder(path: String, parent: String?, entries: [RemoteDirEntry])
  case failed(String)
}

extension AppModel {
  /// The engine browses the computer's folders for the add-project picker
  var browsesFolders: Bool { engineFeatures.contains("localDir") }
  /// The engine lets this device edit the built-in agent's model sources
  var editsProviders: Bool { engineFeatures.contains("providers") }

  /// A folder of the computer (nil: the user's home), its folders by name
  func browse(_ path: String?) async -> LocalListing {
    switch await ask({ .localDir(path: path, seq: $0) }) {
    case let .localDir(_, path, parent, entries, nil)?: .folder(path: path, parent: parent, entries: entries)
    case let .localDir(_, _, _, _, error?)?: .failed(error)
    default: .failed(t("mobile.engineNeeded"))
    }
  }

  // MARK: - Model sources

  /// The sources with the pending row edits laid over them, in the engine's order
  var providerList: [ProviderEntry] {
    let drafts = providerWrites.drafts
    return (providersView?.providers ?? []).map { p in drafts[p.id].map { draft in
      // The draft is what was sent back (no hasKey); the view's own flag stays
      var merged = draft
      merged.raw["hasKey"] = p.raw["hasKey"]
      return merged
    } ?? p }
  }

  /// Asks for the sources once nothing is pending (the page opened, or came back)
  func loadProviders() {
    guard live, providerWrites.refresh() else { return }
    post(.providers)
  }

  /// A row edit: shown at once, sent in order
  func saveProviderRow(_ provider: ProviderEntry) {
    _ = write(.save(provider: provider.stripped), optimistic: true)
  }

  /// A form's save or a removal: the engine's error, nil once it took it
  func changeProviders(_ action: ProviderAction) async -> String? {
    guard live else { return t("mobile.engineNeeded") }
    let id = write(action, optimistic: false)
    return await withCheckedContinuation { done in writeWaits[id] = done }
  }

  /// Asks a source's endpoint for its models, or the computer for local model servers; nil when no answer came
  func probe(_ probe: ProviderProbe) async -> ProbeOutcome? {
    guard live else { return nil }
    probeSeq += 1
    let id = "probe-\(probeSeq)"
    return await withCheckedContinuation { done in
      probing[id] = done
      post(.providerProbe(id: id, probe: probe))
      // A listing goes over the network from the computer; one that never answers ends the wait
      Task { @MainActor [weak self] in
        try? await Task.sleep(for: .seconds(60))
        self?.probing.removeValue(forKey: id)?.resume(returning: nil)
      }
    }
  }

  private func write(_ action: ProviderAction, optimistic: Bool) -> Int {
    let (id, send) = providerWrites.push(action, optimistic: optimistic)
    if let send { post(.providerAction(action: send)) }
    return id
  }

  /// The engine's answer to a read or a write: the list, and the settled write's outcome
  func receiveProviders(_ view: ProvidersView, error: String?) {
    providersView = view
    let (settled, send) = providerWrites.receive()
    if let settled {
      providersError = settled.optimistic ? error : nil
      writeWaits.removeValue(forKey: settled.id)?.resume(returning: error)
      // New or removed models reach the model menus through fresh controls, once the last write landed
      if error == nil, send == nil { loadControls(AppModel.builtinAgent, fresh: true) }
    }
    if let send { post(.providerAction(action: send)) }
  }
}
