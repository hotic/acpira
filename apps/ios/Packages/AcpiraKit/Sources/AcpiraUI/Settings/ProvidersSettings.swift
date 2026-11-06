import AcpiraDesign
import AcpiraModels
import SwiftUI

// The built-in agent's model sources on the phone (ProvidersSection.tsx), as system form pages: the sources (and local
// model servers found on the computer, added in one tap), a source's models with a switch each, its add-model list and
// its form. Add model asks the endpoint first and opens its list only with an answer; a refused key or a dead address
// opens the source's form with the error instead. Saving a form checks the endpoint the same way, so a source that
// cannot list its models never gets in. Keys go to the computer once and never come back: the page knows only whether
// one is stored.

/// The sources: local servers not added yet, then one row per source, then Add source
struct ProvidersPage: View {
  @Environment(AppModel.self) private var model
  @State private var local: [LocalSource] = []
  @State private var opened: String?
  @State private var form: SourceForm?
  @State private var busy = false

  var body: some View {
    let providers = model.providerList
    Form {
      if let error = model.providersView?.error ?? model.providersError {
        Section { Text(verbatim: error).textStyle(TypeScale.text2).foregroundStyle(Palette.danger) }
      }
      let found = local.filter { s in !providers.contains { sameUrl($0.baseUrl, s.baseUrl) } }
      if !found.isEmpty {
        Section {
          ForEach(found, id: \.baseUrl) { s in
            LabeledContent {
              Button(t("settings.providers.local.add")) { add(s) }.disabled(busy)
            } label: {
              SettingLabel(title: t("settings.providers.local", ["name": s.name]), hint: s.baseUrl)
            }
          }
        }
      }
      Section {
        if model.providersView == nil {
          Text(web: t("settings.loading")).foregroundStyle(Palette.fg3)
        } else if providers.isEmpty {
          Text(web: t("settings.providers.none")).foregroundStyle(Palette.fg3)
        }
        ForEach(providers) { p in
          Button { opened = p.id } label: {
            LabeledContent {
              Text(verbatim: "\(p.models.count)")
            } label: {
              SettingLabel(title: p.name.isEmpty ? p.id : p.name, hint: sourceStatus(p))
            }
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .foregroundStyle(Palette.fg1)
        }
      } footer: {
        Text(web: t("mobile.providers.desc"))
      }
      Section {
        Button { form = SourceForm(id: nil) } label: { Label(t("settings.providers.add"), systemImage: "plus") }
      }
    }
    .formPage()
    .navigationTitle(t("settings.providers"))
    .navigationBarTitleDisplayModeInline()
    .navigationDestination(item: $opened) { id in SourcePage(id: id, onGone: { opened = nil }) }
    .navigationDestination(item: $form) { _ in SourceFormPage(provider: nil, onDone: { form = nil }) }
    .onAppear { model.loadProviders() }
    .task {
      // Ollama / LM Studio answering on the computer: offered once, as the desktop page does
      if case .local(let servers)? = await model.probe(.local) { local = servers }
    }
  }

  private func add(_ s: LocalSource) {
    busy = true
    Task {
      _ = await model.changeProviders(.save(provider: s.provider))
      busy = false
    }
  }
}

/// The form a navigation item names: a new source (nil) or a saved one
struct SourceForm: Hashable {
  var id: String?
  var error: String?
}

/// One source: its models with a switch each (a swipe removes one), Add model, and its settings
struct SourcePage: View {
  @Environment(AppModel.self) private var model
  let id: String
  /// The source is gone (removed from its form): back to the list
  var onGone: () -> Void
  @State private var opening = false
  @State private var adding: [ProviderModelEntry]?
  @State private var form: SourceForm?

  var body: some View {
    if let p = model.providerList.first(where: { $0.id == id }) {
      Form {
        Section {
          ForEach(p.models) { m in
            Toggle(isOn: Binding(get: { m.enabled }, set: { on in
              var next = p
              next.models = p.models.map { x in
                guard x.id == m.id else { return x }
                var changed = x
                changed.enabled = on
                return changed
              }
              // Turning a model on also turns its source on
              if on { next.enabled = true }
              model.saveProviderRow(next)
            })) {
              SettingLabel(title: m.name ?? m.id, hint: m.name == nil ? nil : m.id)
            }
            .tint(.green)
            .swipeActions {
              Button(role: .destructive) {
                var next = p
                next.models = p.models.filter { $0.id != m.id }
                model.saveProviderRow(next)
              } label: { Label(t("common.remove"), systemImage: "trash") }
            }
          }
          Button(action: { openAdd(p) }) {
            HStack {
              Label(t("settings.providers.addModel"), systemImage: "plus")
              Spacer(minLength: 0)
              if opening { ProgressView() }
            }
          }
          .disabled(opening)
        } header: {
          Text(web: t("settings.providers.models"))
        }
        Section {
          Button { form = SourceForm(id: p.id) } label: {
            LabeledContent(t("settings.providers.editShort")) { Text(verbatim: p.baseUrl).lineLimit(1).truncationMode(.middle) }
          }
          .foregroundStyle(Palette.fg1)
        } footer: {
          Text(web: sourceStatus(p))
        }
      }
      .formPage()
      .navigationTitle(p.name.isEmpty ? p.id : p.name)
      .navigationBarTitleDisplayModeInline()
      .navigationDestination(item: $adding) { listed in AddModelPage(id: id, listed: listed, onDone: { adding = nil }) }
      .navigationDestination(item: $form) { f in
        SourceFormPage(provider: p, error: f.error, onDone: { form = nil }, onRemoved: onGone)
      }
    } else {
      // Removed meanwhile (here or on the computer)
      Form { Section { Text(web: t("settings.providers.none")).foregroundStyle(Palette.fg3) } }
        .formPage()
    }
  }

  /// Asks the endpoint for its models first: the list opens with them, a failure opens the form with the error
  private func openAdd(_ p: ProviderEntry) {
    opening = true
    Task {
      let outcome = await model.probe(.models(provider: p))
      opening = false
      switch outcome {
      case .models(let listed)?: adding = listed
      case .failed(let error)?: form = SourceForm(id: p.id, error: error)
      default: form = SourceForm(id: p.id, error: t("mobile.engineNeeded"))
      }
    }
  }
}

/// A source's add list (ProvidersSection.tsx AddModelDialog): the models its endpoint listed, a tap adds one and a
/// second takes it out again, each change saved at once; whatever the search holds that the list does not can be
/// added as an id
struct AddModelPage: View {
  @Environment(AppModel.self) private var model
  let id: String
  let listed: [ProviderModelEntry]
  var onDone: () -> Void
  @State private var query = ""

  var body: some View {
    let p = model.providerList.first { $0.id == id }
    let taken = Set(p?.models.map(\.id) ?? [])
    let typed = query.trimmingCharacters(in: .whitespacesAndNewlines)
    let q = typed.lowercased()
    let shown = q.isEmpty ? listed : listed.filter { $0.id.lowercased().contains(q) || ($0.name ?? "").lowercased().contains(q) }
    Form {
      Section {
        if !typed.isEmpty, !listed.contains(where: { $0.id == typed }), !taken.contains(typed) {
          Button { toggle(ProviderModelEntry(typed: typed)); query = "" } label: {
            Label(t("settings.providers.addModel.useId", ["id": typed]), systemImage: "plus")
          }
        }
        ForEach(shown) { m in
          Button { toggle(m) } label: {
            HStack {
              SettingLabel(title: m.name ?? m.id, hint: m.name == nil ? nil : m.id)
              Spacer(minLength: 0)
              if taken.contains(m.id) { Image(systemName: "checkmark").fontWeight(.semibold) }
            }
            .contentShape(Rectangle())
          }
          .buttonStyle(.plain)
          .foregroundStyle(Palette.fg1)
        }
        if shown.isEmpty, typed.isEmpty {
          Text(web: t("settings.providers.addModel.typeId")).foregroundStyle(Palette.fg3)
        }
      } footer: {
        Text(verbatim: t("settings.providers.addModel.count", ["count": p?.models.count ?? 0]))
      }
    }
    .formPage()
    .searchable(text: $query, prompt: Text(web: t("settings.providers.addModel.search")))
    .autocorrectionDisabled()
    #if os(iOS)
    .textInputAutocapitalization(.never)
    #endif
    .navigationTitle(t("settings.providers.addModel"))
    .navigationSubtitleIfAvailable(p.map { $0.name.isEmpty ? $0.id : $0.name } ?? "")
    .navigationBarTitleDisplayModeInline()
    .navigationBarBackButtonHidden()
    .toolbar {
      ToolbarItem(placement: .confirmationAction) { Button(t("settings.providers.addModel.done"), action: onDone) }
    }
  }

  private func toggle(_ m: ProviderModelEntry) {
    guard var p = model.providerList.first(where: { $0.id == id }) else { return }
    if p.models.contains(where: { $0.id == m.id }) {
      p.models = p.models.filter { $0.id != m.id }
    } else {
      var added = m
      added.enabled = true
      p.models = p.models + [added]
    }
    model.saveProviderRow(p)
  }
}

/// A source's settings (ProvidersSection.tsx ProviderForm): name, API format, URL with the request URL it produces, key.
/// Save checks the endpoint first when anything that reaches it changed; a new source then opens its add-model list
struct SourceFormPage: View {
  @Environment(AppModel.self) private var model
  let provider: ProviderEntry?
  var error: String?
  /// Closes the form (and anything it opened)
  var onDone: () -> Void
  /// The source was removed: its own page goes too
  var onRemoved: (() -> Void)?
  @State private var name = ""
  @State private var format = "openai-chat"
  @State private var baseUrl = ""
  @State private var fullOverride: Bool?
  @State private var key = ""
  @State private var failure: String?
  @State private var busy = false
  @State private var removing = false
  /// A new source saved: its id and what its endpoint listed, for the add-model list
  @State private var added: AddedSource?

  struct AddedSource: Hashable {
    var id: String
    var listed: [ProviderModelEntry]
  }

  init(provider: ProviderEntry?, error: String? = nil, onDone: @escaping () -> Void, onRemoved: (() -> Void)? = nil) {
    self.provider = provider
    self.error = error
    self.onDone = onDone
    self.onRemoved = onRemoved
    // A saved source's fields, and the failure that opened the form (a refused Add model), from the first frame: set
    // later, the field change would clear the failure at once
    _name = State(initialValue: provider?.name ?? "")
    _format = State(initialValue: provider?.format ?? "openai-chat")
    _baseUrl = State(initialValue: provider?.baseUrl ?? "")
    _fullOverride = State(initialValue: provider?.fullUrl)
    _failure = State(initialValue: error)
  }

  var body: some View {
    let fullUrl = fullOverride ?? looksFull(baseUrl)
    let valid = baseUrl.trimmingCharacters(in: .whitespaces).range(of: #"^https?://\S+$"#, options: .regularExpression) != nil
    Form {
      Section {
        TextField(text: $name) { Text(web: t("settings.providers.name.placeholder")) }
        Picker(t("settings.providers.format"), selection: $format) {
          Text(web: t("settings.providers.format.openai")).tag("openai-chat")
          Text(web: t("settings.providers.format.responses")).tag("openai-responses")
          Text(web: t("settings.providers.format.anthropic")).tag("anthropic")
        }
      } header: {
        Text(web: t("settings.providers.name"))
      }
      Section {
        TextField(text: $baseUrl) {
          Text(web: t(fullUrl ? "settings.providers.fullUrl.placeholder" : "settings.providers.baseUrl.placeholder"))
        }
        .textStyle(TypeScale.mono)
        #if os(iOS)
        .keyboardType(.URL)
        #endif
        if valid {
          // The URL as typed or with the format's path appended: the desktop's text button, as a switch
          Toggle(isOn: Binding(get: { fullUrl }, set: { fullOverride = $0 })) {
            Text(web: t("settings.providers.asTyped"), paragraph: false)
          }
          .tint(.green)
        }
      } header: {
        Text(web: t("settings.providers.baseUrl"))
      } footer: {
        if valid { Text(verbatim: t("settings.providers.endpoint", ["url": endpointOf(baseUrl, format: format, fullUrl: fullUrl)])) }
      }
      if provider?.isLocal != true {
        Section {
          SecureField(text: $key) {
            Text(web: t(provider?.hasKey == true ? "settings.providers.keyKept" : "settings.providers.key.placeholder"))
          }
        } header: {
          Text(web: t("settings.providers.key"))
        } footer: {
          if let failure { Text(verbatim: failure).foregroundStyle(Palette.danger) }
        }
      } else if let failure {
        Section { Text(verbatim: failure).textStyle(TypeScale.text2).foregroundStyle(Palette.danger) }
      }
      if provider != nil {
        Section {
          Button(t("common.remove"), role: .destructive) { removing = true }.disabled(busy)
        }
      }
    }
    .autocorrectionDisabled()
    #if os(iOS)
    .textInputAutocapitalization(.never)
    #endif
    .formPage()
    .navigationTitle(provider.map { t("settings.providers.edit", ["name": $0.name.isEmpty ? $0.id : $0.name]) } ?? t("settings.providers.add"))
    .navigationBarTitleDisplayModeInline()
    .toolbar {
      ToolbarItem(placement: .confirmationAction) {
        if busy {
          ProgressView()
        } else {
          Button(t("settings.providers.save")) { save(fullUrl: fullUrl) }.disabled(!valid)
        }
      }
    }
    .confirmationDialog(t("mobile.providers.removeConfirm", ["name": provider?.name ?? ""]), isPresented: $removing, titleVisibility: .visible) {
      Button(t("common.remove"), role: .destructive) { remove() }
      Button(t("mobile.cancel"), role: .cancel) {}
    }
    .navigationDestination(item: $added) { a in AddModelPage(id: a.id, listed: a.listed, onDone: onDone) }
    // A field changed: the last failure no longer applies
    .onChange(of: [name, format, baseUrl, key]) { _, _ in failure = nil }
  }

  private func draft(fullUrl: Bool) -> ProviderEntry {
    var p = provider?.stripped ?? ProviderEntry(name: "", format: format, baseUrl: "", fullUrl: false)
    p.name = name.trimmingCharacters(in: .whitespaces)
    p.format = format
    p.baseUrl = baseUrl.trimmingCharacters(in: .whitespaces)
    p.fullUrl = fullUrl
    return p
  }

  private func save(fullUrl: Bool) {
    let next = draft(fullUrl: fullUrl)
    let typedKey = key.trimmingCharacters(in: .whitespaces)
    // Only what reaches the endpoint needs the check; a rename saves as it is
    let reaches = provider == nil || !typedKey.isEmpty || next.format != provider?.format || next.baseUrl != provider?.baseUrl
      || next.fullUrl != provider?.fullUrl
    let before = Set(model.providerList.map(\.id))
    busy = true
    failure = nil
    Task {
      var listed: [ProviderModelEntry] = []
      if reaches {
        switch await model.probe(.models(provider: next, key: typedKey.isEmpty ? nil : typedKey)) {
        case .models(let models)?: listed = models
        case .failed(let error)?:
          failure = error
          busy = false
          return
        default:
          failure = t("mobile.engineNeeded")
          busy = false
          return
        }
      }
      if let error = await model.changeProviders(.save(provider: next, key: typedKey.isEmpty ? nil : typedKey)) {
        failure = error
        busy = false
        return
      }
      busy = false
      // A new source goes on to its add-model list with what its endpoint listed
      if provider == nil, let id = model.providerList.map(\.id).first(where: { !before.contains($0) }) {
        added = AddedSource(id: id, listed: listed)
      } else {
        onDone()
      }
    }
  }

  private func remove() {
    guard let provider else { return }
    busy = true
    Task {
      let error = await model.changeProviders(.delete(id: provider.id))
      busy = false
      if let error { failure = error } else { (onRemoved ?? onDone)() }
    }
  }
}

/// A source's state, its row's second line: off, a local server, a key stored or not
func sourceStatus(_ p: ProviderEntry) -> String {
  if !p.enabled { return t("settings.providers.off") }
  if p.isLocal { return t("settings.providers.keyNotNeeded") }
  return t(p.hasKey ? "settings.providers.keySaved" : "settings.providers.noKey")
}

/// Two URLs naming the same server: case and trailing slashes aside
func sameUrl(_ a: String, _ b: String) -> Bool {
  func plain(_ s: String) -> String {
    var t = s.trimmingCharacters(in: .whitespaces).lowercased()
    while t.hasSuffix("/") { t.removeLast() }
    return t
  }
  return plain(a) == plain(b)
}

/// A URL already naming the endpoint, used as typed (ProvidersSection.tsx looksFull)
func looksFull(_ url: String) -> Bool {
  url.trimmingCharacters(in: .whitespaces).range(of: #"/(chat/completions|responses|messages)/?$"#, options: [.regularExpression, .caseInsensitive]) != nil
}

/// The request URL a source's settings produce, as the agent builds it (ProvidersSection.tsx endpointOf)
func endpointOf(_ baseUrl: String, format: String, fullUrl: Bool) -> String {
  let url = baseUrl.trimmingCharacters(in: .whitespaces)
  if fullUrl { return url }
  let suffix = format == "anthropic" ? "messages" : format == "openai-responses" ? "responses" : "chat/completions"
  var root = url
  while root.hasSuffix("/") { root.removeLast() }
  return "\(root)/\(suffix)"
}
