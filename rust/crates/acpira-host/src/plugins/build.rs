//! Plugins written in TypeScript, TSX or modern JavaScript (docs/dev/plugins.md, Building). A frozen copy is compiled
//! after the freeze, during the static check, so nothing of the plugin runs before the user confirms: oxc parses each
//! module, strips its types and lowers JSX, and the output goes to `.build/<id>/<hash>-<BUILD_VERSION>/`, never into the
//! read-only copy. The output is derived from the copy alone, so it is rebuilt whenever it is missing.
//!
//! - `runtime.main`: every module of the plugin becomes an ES module file Node runs as it is; packages under
//!   `node_modules` are left to Node, addressed by the absolute path the resolver found in the frozen copy, so CommonJS,
//!   ES modules and native addons load as Node loads them. `@acpira/plugin` is the SDK the engine ships.
//! - `ui[].entry`: the whole module graph is inlined into one document (a frame has no network and no module loader):
//!   ES modules through oxc's module runner transform, CommonJS in a function wrapper, JSON, CSS and small assets.
//!   React and `@acpira/plugin-ui` are the runtime the page sends into the frame (`UI_EXTERNALS`), never bundled.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration, ImportExpression};
use oxc_ast_visit::{Visit, walk};
use oxc_codegen::Codegen;
use oxc_parser::Parser;
use oxc_resolver::{ResolveError, ResolveOptions, Resolver};
use oxc_semantic::SemanticBuilder;
use oxc_span::{SourceType, Span};
use oxc_transformer::{JsxRuntime, TransformOptions, Transformer};
use oxc_transformer_plugins::ModuleRunnerTransform;

/// The folder under the plugins root holding compiled output
pub const BUILD: &str = ".build";
/// Part of an output folder's name: another compiler (another Acpira) builds again instead of trusting old output
pub const BUILD_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "-1");
/// The SDK a resident process imports (`definePlugin`), served from the engine (`sdk.rs`)
pub const SDK_SPECIFIER: &str = "@acpira/plugin";
/// The interface kit and React: the page sends them into the frame (`packages/plugin-ui`), a piece never bundles them
pub const UI_EXTERNALS: &[&str] = &["react", "react/jsx-runtime", "react/jsx-dev-runtime", "react-dom", "react-dom/client", "@acpira/plugin-ui"];
/// Code another plugin shares (`acpira plugin vendor`): compiled with the plugin, like its own modules
pub const VENDORED_SCOPE: &str = "@acpira-plugin";
/// A compiled interface document, dependencies included
pub const DOC_MAX_BYTES: usize = 4 << 20;
/// Modules in one graph: a runaway dependency tree fails the check instead of the engine
const MODULES_MAX: usize = 4000;
/// Assets an interface module imports, inlined as data URLs
const ASSET_MAX_BYTES: usize = 512 * 1024;

/// Source files compiled rather than handed to Node as they are
const SCRIPT_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];
const TYPED_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "jsx"];
const ASSET_TYPES: &[(&str, &str)] = &[
  ("png", "image/png"),
  ("jpg", "image/jpeg"),
  ("jpeg", "image/jpeg"),
  ("gif", "image/gif"),
  ("webp", "image/webp"),
  ("svg", "image/svg+xml"),
  ("woff", "font/woff"),
  ("woff2", "font/woff2"),
];

/// Where the compiled output of snapshot `hash` of `id` lives under the plugins root
pub fn output_dir(root: &Path, id: &str, hash: &str) -> PathBuf {
  root.join(BUILD).join(id).join(format!("{hash}-{BUILD_VERSION}"))
}

/// Whether a manifest path names a source the engine compiles (`.ts`, `.tsx`, `.jsx`, …): such a `runtime.main` or
/// `ui[].entry` needs no build step of the author's
pub fn compiled_source(rel: &str) -> bool {
  ext_of(Path::new(rel)).is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e.as_str()))
}

/// Compile `main` of the frozen copy `dir` for a resident process into `out` (written whole, then renamed into place;
/// an existing output is reused). Returns the bootstrap module Node runs
pub fn runtime(dir: &Path, main: &str, out: &Path) -> Result<PathBuf, String> {
  let boot = out.join("runtime").join("__main.mjs");
  if boot.is_file() {
    return Ok(boot);
  }
  let tmp = sibling_tmp(out)?;
  let result = write_runtime(dir, main, &tmp, out);
  match result {
    Ok(()) => {
      publish(&tmp, out)?;
      Ok(boot)
    }
    Err(e) => {
      let _ = std::fs::remove_dir_all(&tmp);
      Err(e)
    }
  }
}

/// The interface document of piece `piece` (`entry` of the frozen copy `dir`), compiled once into `out`
pub fn ui(dir: &Path, entry: &str, piece: &str, out: &Path) -> Result<String, String> {
  let file = out.join("ui").join(format!("{}.html", safe_name(piece)));
  if let Ok(doc) = std::fs::read_to_string(&file) {
    return Ok(doc);
  }
  let doc = ui_document(dir, entry)?;
  let tmp = file.with_extension(format!("tmp-{}", crate::util::random_hex(6)));
  if let Some(parent) = file.parent() {
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
  }
  std::fs::write(&tmp, &doc).map_err(|e| format!("{}: {e}", tmp.display()))?;
  std::fs::rename(&tmp, &file).map_err(|e| format!("{}: {e}", file.display()))?;
  Ok(doc)
}

/// Check that every compiled source of the manifest compiles, writing the output the engine will load; one message
/// per failure, each naming the file and line
pub fn check(dir: &Path, manifest: &crate::plugins::manifest::Manifest, out: &Path) -> Vec<String> {
  // A source that is missing or of another kind is the manifest check's error already
  let compiles = |rel: &str| compiled_source(rel) && crate::plugins::manifest::inside(dir, rel).is_some_and(|p| p.is_file());
  let mut errors = vec![];
  if let Some(main) = manifest.contributes.runtime.as_ref().and_then(|r| r.main.as_deref()).filter(|m| compiles(m))
    && let Err(e) = runtime(dir, main, out)
  {
    errors.push(format!("runtime.main: {e}"));
  }
  for u in &manifest.contributes.ui {
    if let Some(entry) = u.entry.as_deref().filter(|e| compiles(e))
      && let Err(e) = ui(dir, entry, &u.id, out)
    {
      errors.push(format!("ui `{}`: {e}", u.id));
    }
  }
  if let Some(e) = manifest.exports.as_deref().filter(|e| compiles(e))
    && let Err(e) = exports_compile(dir, e)
  {
    errors.push(format!("exports: {e}"));
  }
  errors
}

/// The shared module and every module of the plugin it reaches parse, compile and resolve, as a dependant's process
/// would load them once vendored (the copy carries this plugin's `node_modules`). The page's externals and the SDK are
/// left to whoever imports it
fn exports_compile(dir: &Path, rel: &str) -> Result<(), String> {
  let resolver = resolver(Target::Node);
  let root = canonical(dir)?;
  let entry = crate::plugins::manifest::inside(dir, rel).ok_or_else(|| format!("`{rel}` is not a file of the plugin"))?;
  let mut queue = vec![canonical(&entry)?];
  let mut seen: HashSet<PathBuf> = HashSet::new();
  while let Some(file) = queue.pop() {
    if !seen.insert(file.clone()) {
      continue;
    }
    if seen.len() > MODULES_MAX {
      return Err(format!("more than {MODULES_MAX} modules"));
    }
    let rel = rel_of(&root, &file)?;
    let code = compile_module(&file, &rel)?;
    for (_, spec) in specifiers(&code, &rel)? {
      if spec == SDK_SPECIFIER || UI_EXTERNALS.contains(&spec.as_str()) {
        continue;
      }
      match resolve(&resolver, &file, &spec) {
        Resolved::Builtin => {}
        Resolved::Missing(e) => return Err(format!("{rel}: cannot find `{spec}` ({e})")),
        Resolved::File(target) if !target.starts_with(&root) => return Err(format!("{rel}: `{spec}` lies outside the plugin folder")),
        Resolved::File(target) => {
          if compiles_for_node(&root, &target) {
            queue.push(target);
          }
        }
      }
    }
  }
  Ok(())
}

// Resident process

fn write_runtime(dir: &Path, main: &str, tmp: &Path, out: &Path) -> Result<(), String> {
  let resolver = resolver(Target::Node);
  let root = canonical(dir)?;
  let entry = crate::plugins::manifest::inside(dir, main).ok_or_else(|| format!("`{main}` is not a file of the plugin"))?;
  let entry = canonical(&entry)?;
  let runtime_out = out.join("runtime");
  let sdk = runtime_out.join("__acpira").join("plugin.mjs");
  let mut queue = vec![entry.clone()];
  let mut seen: HashSet<PathBuf> = HashSet::new();
  while let Some(file) = queue.pop() {
    if !seen.insert(file.clone()) {
      continue;
    }
    if seen.len() > MODULES_MAX {
      return Err(format!("more than {MODULES_MAX} modules"));
    }
    let rel = rel_of(&root, &file)?;
    let code = compile_module(&file, &rel)?;
    let mut edits = vec![];
    for (span, spec) in specifiers(&code, &rel)? {
      let to = if spec == SDK_SPECIFIER {
        file_url(&sdk)
      } else {
        match resolve(&resolver, &file, &spec) {
          Resolved::Builtin => continue,
          Resolved::Missing(e) => return Err(format!("{rel}: cannot find `{spec}` ({e})")),
          Resolved::File(target) => {
            if !target.starts_with(&root) {
              return Err(format!("{rel}: `{spec}` lies outside the plugin folder"));
            }
            if compiles_for_node(&root, &target) {
              queue.push(target.clone());
              file_url(&runtime_out.join(format!("{}.mjs", rel_of(&root, &target)?)))
            } else {
              file_url(&target)
            }
          }
        }
      };
      edits.push((span, to));
    }
    let code = apply_edits(&code, edits);
    write_file(&tmp.join("runtime").join(format!("{rel}.mjs")), code.as_bytes())?;
  }
  write_file(&tmp.join("runtime").join("__acpira").join("plugin.mjs"), crate::plugins::sdk::SDK.as_bytes())?;
  let main_url = file_url(&runtime_out.join(format!("{}.mjs", rel_of(&root, &entry)?)));
  let boot = format!(
    "// Written by Acpira: loads the plugin's definition and hands it to the SDK (docs/dev/plugins.md, Runtime)\n\
     import {{ start }} from {sdk};\n\
     let def;\n\
     try {{\n  def = (await import({main})).default;\n}} catch (e) {{\n  process.stderr.write(`the plugin failed to load: ${{e?.stack ?? e}}\\n`);\n  process.exit(1);\n}}\n\
     start(def ?? {{}});\n",
    sdk = js_string(&file_url(&sdk)),
    main = js_string(&main_url),
  );
  write_file(&tmp.join("runtime").join("__main.mjs"), boot.as_bytes())
}

/// Whether Node gets the compiled module rather than the file: the plugin's own sources and shared plugin code, never
/// a package's published files (Node runs those as they are)
fn compiles_for_node(root: &Path, target: &Path) -> bool {
  let Some(ext) = ext_of(target) else { return false };
  if !SCRIPT_EXTENSIONS.contains(&ext.as_str()) || ext == "cjs" {
    return false;
  }
  let rel = target.strip_prefix(root).unwrap_or(target);
  let comps: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
  match comps.iter().position(|c| c == "node_modules") {
    None => true,
    Some(i) => comps.get(i + 1).map(String::as_str) == Some(VENDORED_SCOPE) || TYPED_EXTENSIONS.contains(&ext.as_str()),
  }
}

/// One module as JavaScript: types stripped, JSX lowered to `react/jsx-runtime` calls
fn compile_module(file: &Path, rel: &str) -> Result<String, String> {
  let src = std::fs::read_to_string(file).map_err(|e| format!("{rel}: {e}"))?;
  let alloc = Allocator::default();
  let source_type = source_type(file);
  let parsed = Parser::new(&alloc, &src, source_type).parse();
  if let Some(d) = parsed.diagnostics.first() {
    return Err(diagnostic(rel, &src, d));
  }
  let mut program = parsed.program;
  let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
  let options = transform_options();
  let done = Transformer::new(&alloc, file, &options).build_with_scoping(scoping, &mut program);
  if let Some(d) = done.diagnostics.first() {
    return Err(diagnostic(rel, &src, d));
  }
  Ok(Codegen::new().build(&program).code)
}

// Interface document

#[derive(Debug)]
enum Kind {
  Esm,
  Cjs,
  Json,
  Css,
  Asset,
}

struct Module {
  id: String,
  kind: Kind,
  /// Specifier to module id (`lib:<spec>` for the page's runtime, empty for a CommonJS require that resolved nowhere)
  deps: BTreeMap<String, String>,
  dynamic: BTreeMap<String, String>,
  /// JavaScript of a function body (ES / CommonJS) or a JSON value
  body: String,
}

fn ui_document(dir: &Path, entry: &str) -> Result<String, String> {
  let resolver = resolver(Target::Browser);
  let root = canonical(dir)?;
  let entry = crate::plugins::manifest::inside(dir, entry).ok_or_else(|| format!("`{entry}` is not a file of the plugin"))?;
  let entry = canonical(&entry)?;
  let entry_id = rel_of(&root, &entry)?;
  let mut modules: Vec<Module> = vec![];
  let mut queue = vec![entry];
  let mut seen: HashSet<PathBuf> = HashSet::new();
  let mut bytes = 0usize;
  while let Some(file) = queue.pop() {
    if !seen.insert(file.clone()) {
      continue;
    }
    if seen.len() > MODULES_MAX {
      return Err(format!("more than {MODULES_MAX} modules"));
    }
    let id = rel_of(&root, &file)?;
    let mut module = ui_module(&file, &id)?;
    let mut link = |spec: &str, required: bool| -> Result<Option<String>, String> {
      if UI_EXTERNALS.contains(&spec) {
        return Ok(Some(format!("lib:{spec}")));
      }
      if spec == SDK_SPECIFIER {
        return Err(format!("{id}: `{SDK_SPECIFIER}` belongs to the plugin's process; an interface piece imports `@acpira/plugin-ui`"));
      }
      match resolve(&resolver, &file, spec) {
        Resolved::File(target) if target.starts_with(&root) => {
          let target_id = rel_of(&root, &target)?;
          queue.push(target);
          Ok(Some(target_id))
        }
        Resolved::File(_) => Err(format!("{id}: `{spec}` lies outside the plugin folder")),
        Resolved::Builtin if !required => Ok(None),
        Resolved::Builtin => Err(format!("{id}: `{spec}` is a Node module; an interface piece runs in a page")),
        Resolved::Missing(_) if !required => Ok(None),
        Resolved::Missing(e) => Err(format!("{id}: cannot find `{spec}` ({e})")),
      }
    };
    let required = matches!(module.kind, Kind::Esm);
    for (spec, to) in module.deps.iter_mut() {
      *to = link(spec, required)?.unwrap_or_default();
    }
    for (spec, to) in module.dynamic.iter_mut() {
      *to = link(spec, true)?.unwrap_or_default();
    }
    bytes += module.body.len();
    if bytes > DOC_MAX_BYTES {
      return Err(format!("the piece and what it imports exceed {} MB", DOC_MAX_BYTES >> 20));
    }
    modules.push(module);
  }
  modules.sort_by(|a, b| a.id.cmp(&b.id));
  Ok(render_document(&entry_id, &modules))
}

fn ui_module(file: &Path, id: &str) -> Result<Module, String> {
  let ext = ext_of(file).unwrap_or_default();
  let read = || std::fs::read(file).map_err(|e| format!("{id}: {e}"));
  let mut module = Module { id: id.to_owned(), kind: Kind::Json, deps: BTreeMap::new(), dynamic: BTreeMap::new(), body: String::new() };
  if ext == "json" {
    let bytes = read()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| format!("{id}: {e}"))?;
    module.body = value.to_string();
    return Ok(module);
  }
  if ext == "css" {
    module.kind = Kind::Css;
    module.body = js_string(&String::from_utf8_lossy(&read()?));
    return Ok(module);
  }
  if let Some((_, mime)) = ASSET_TYPES.iter().find(|(e, _)| *e == ext) {
    let bytes = read()?;
    if bytes.len() > ASSET_MAX_BYTES {
      return Err(format!("{id}: an imported asset may hold at most {} KB", ASSET_MAX_BYTES >> 10));
    }
    use base64::Engine as _;
    module.kind = Kind::Asset;
    module.body = js_string(&format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)));
    return Ok(module);
  }
  if !SCRIPT_EXTENSIONS.contains(&ext.as_str()) {
    return Err(format!("{id}: an interface piece cannot import a `.{ext}` file"));
  }
  let src = String::from_utf8(read()?).map_err(|_| format!("{id}: not UTF-8 text"))?;
  let alloc = Allocator::default();
  let source_type = source_type(file);
  let parsed = Parser::new(&alloc, &src, source_type).parse();
  if let Some(d) = parsed.diagnostics.first() {
    return Err(diagnostic(id, &src, d));
  }
  let esm = parsed.module_record.has_module_syntax || TYPED_EXTENSIONS.contains(&ext.as_str()) || ext == "mjs";
  let mut program = parsed.program;
  if !esm {
    // CommonJS: the source as it is, its literal `require` calls resolved up front
    let mut found = Requires::default();
    found.visit_program(&program);
    module.kind = Kind::Cjs;
    module.deps = found.0.into_iter().map(|s| (s, String::new())).collect();
    module.body = src;
    return Ok(module);
  }
  let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
  let options = transform_options();
  let done = Transformer::new(&alloc, file, &options).build_with_scoping(scoping, &mut program);
  if let Some(d) = done.diagnostics.first() {
    return Err(diagnostic(id, &src, d));
  }
  let scoping = SemanticBuilder::new().build(&program).semantic.into_scoping();
  let (deps, dynamic) = ModuleRunnerTransform::new().transform(&alloc, &mut program, scoping);
  module.kind = Kind::Esm;
  module.deps = deps.iter().map(|s| (s.display().to_string(), String::new())).collect();
  module.dynamic = dynamic.iter().map(|s| (s.display().to_string(), String::new())).collect();
  module.body = Codegen::new().build(&program).code;
  Ok(module)
}

/// The document: the modules as a table of functions and the loader that runs them in import order (docs/dev/plugins.md,
/// Building). The page writes it into the frame after its runtime (`globalThis.__acpiraLib`)
fn render_document(entry: &str, modules: &[Module]) -> String {
  let mut table = String::from("{\n");
  for m in modules {
    let deps = |map: &BTreeMap<String, String>| {
      let pairs: Vec<String> = map.iter().map(|(k, v)| format!("{}:{}", js_string(k), js_string(v))).collect();
      format!("{{{}}}", pairs.join(","))
    };
    let (kind, value) = match m.kind {
      Kind::Esm => (
        "esm",
        format!(
          "async function (__vite_ssr_exports__, __vite_ssr_import_meta__, __vite_ssr_import__, __vite_ssr_dynamic_import__, __vite_ssr_exportAll__) {{\n\"use strict\";\n{}\n}}",
          m.body
        ),
      ),
      Kind::Cjs => ("cjs", format!("function (module, exports, require, process, global) {{\n{}\n}}", m.body)),
      Kind::Json => ("json", m.body.clone()),
      Kind::Css => ("css", m.body.clone()),
      Kind::Asset => ("asset", m.body.clone()),
    };
    let _ = writeln!(table, "{}: [{}, {}, {}, {}],", js_string(&m.id), js_string(kind), deps(&m.deps), deps(&m.dynamic), value);
  }
  table.push('}');
  let script = LOADER.replace("__ENTRY__", &js_string(entry)).replace("__MODULES__", &table);
  format!(
    "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"acpira-runtime\" content=\"react\">\n</head>\n<body>\n<div id=\"root\"></div>\n<script>\n{}\n</script>\n</body>\n</html>\n",
    script_safe(&script)
  )
}

/// The frame's module loader: runs a module after the modules it imports (post order, a cycle sees what is defined so
/// far, as ES modules do), CommonJS on first `require`, and mounts the entry's default export through the kit
const LOADER: &str = r#"(function () {
  "use strict";
  var M = __MODULES__;
  var ENTRY = __ENTRY__;
  var done = Object.create(null), state = Object.create(null);
  var proc = { env: { NODE_ENV: "production" } };
  if (!globalThis.process) globalThis.process = proc;
  function lib(spec) {
    var l = globalThis.__acpiraLib;
    if (!l || !l[spec]) throw new Error(spec + " is not available here: this window did not load Acpira's plugin runtime");
    return l[spec];
  }
  function target(table, id, spec) {
    var t = table[spec];
    if (t === undefined || t === "") throw new Error("cannot find module '" + spec + "' from " + id);
    return t;
  }
  function namespaceOf(exp) {
    if (exp && exp.__esModule) return exp;
    var ns = Object.create(null);
    if (exp && (typeof exp === "object" || typeof exp === "function")) for (var k in exp) ns[k] = exp[k];
    ns["default"] = exp;
    return ns;
  }
  function style(id, css) {
    var el = document.createElement("style");
    el.setAttribute("data-module", id);
    el.textContent = css;
    document.head.appendChild(el);
  }
  // Synchronous: CommonJS, data, and ES modules already run (a CommonJS module cannot wait for one)
  function need(id) {
    if (id.slice(0, 4) === "lib:") return lib(id.slice(4));
    var m = M[id];
    if (!m) throw new Error("no module " + id);
    if (m[0] === "cjs") {
      if (done[id]) return done[id].exports;
      var module = { exports: {} };
      done[id] = module;
      m[3].call(module.exports, module, module.exports, function (spec) { return need(target(m[1], id, spec)); }, globalThis.process || proc, globalThis);
      return module.exports;
    }
    if (m[0] === "json" || m[0] === "asset") return m[3];
    if (m[0] === "css") { if (!done[id]) { done[id] = true; style(id, m[3]); } return m[3]; }
    if (done[id]) return done[id];
    throw new Error("the CommonJS module requiring " + id + " cannot load an ES module");
  }
  function namespace(id) {
    if (id.slice(0, 4) === "lib:") return lib(id.slice(4));
    var m = M[id];
    if (m[0] === "esm") return done[id];
    if (m[0] === "cjs") return namespaceOf(need(id));
    return { "default": need(id) };
  }
  async function run(id) {
    if (id.slice(0, 4) === "lib:" || state[id]) return;
    var m = M[id];
    if (!m) throw new Error("no module " + id);
    if (m[0] !== "esm") { state[id] = 2; need(id); return; }
    state[id] = 1;
    var ex = Object.create(null);
    done[id] = ex;
    // Imports run as the compiled body reaches them, one after another: source order (styles cascade by it)
    await m[3](
      ex,
      { url: "plugin:///" + id },
      async function (spec) { var t = target(m[1], id, spec); await run(t); return namespace(t); },
      async function (spec) { var t = target(m[2], id, spec); await run(t); return namespace(t); },
      function (from) {
        for (var k in from) if (k !== "default" && !(k in ex)) (function (k) {
          Object.defineProperty(ex, k, { enumerable: true, configurable: true, get: function () { return from[k]; } });
        })(k);
      }
    );
    state[id] = 2;
  }
  function fail(e) {
    console.error(e);
    var box = document.getElementById("root") || document.body;
    box.textContent = String(e && e.message || e);
    box.setAttribute("data-error", "");
  }
  run(ENTRY).then(function () {
    var ns = done[ENTRY];
    if (ns && ns["default"] !== undefined) lib("@acpira/plugin-ui").mount(ns["default"]);
  }).catch(fail);
})();"#;

/// A frame's script element ends at the first `</script`; `<!--` changes how the rest is read
fn script_safe(js: &str) -> String {
  // Both markers are ASCII, so a byte match never lands inside a character; the characters between are copied whole
  let bytes = js.as_bytes();
  let at = |i: usize, marker: &[u8]| bytes.get(i..i + marker.len()).is_some_and(|b| b.eq_ignore_ascii_case(marker));
  let mut out = String::with_capacity(js.len());
  let mut copied = 0;
  let mut i = 0;
  while i < bytes.len() {
    let (marker, replacement): (&[u8], &str) = if at(i, b"</script") {
      (b"</", "<\\/")
    } else if at(i, b"<!--") {
      (b"<!--", "<\\!--")
    } else {
      i += 1;
      continue;
    };
    out.push_str(js.get(copied..i).unwrap_or_default());
    out.push_str(replacement);
    i += marker.len();
    copied = i;
  }
  out.push_str(js.get(copied..).unwrap_or_default());
  out
}

// Shared

#[derive(Clone, Copy)]
enum Target {
  Node,
  Browser,
}

fn resolver(target: Target) -> Resolver {
  let conditions: &[&str] = match target {
    // What Node itself matches: no bundler `module` condition or field, which may name code Node never loads
    Target::Node => &["node", "import", "default"],
    Target::Browser => &["browser", "import", "module", "default"],
  };
  let main_fields: &[&str] = match target {
    Target::Node => &["main"],
    Target::Browser => &["browser", "module", "main"],
  };
  let strings = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
  Resolver::new(ResolveOptions {
    condition_names: strings(conditions),
    main_fields: strings(main_fields),
    alias_fields: match target {
      Target::Browser => vec![vec!["browser".into()]],
      Target::Node => vec![],
    },
    extensions: strings(&[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"]),
    // `./util.js` in TypeScript names `./util.ts`, as tsc reads it
    extension_alias: vec![(".js".into(), strings(&[".ts", ".tsx", ".js", ".jsx"])), (".mjs".into(), strings(&[".mts", ".mjs"]))],
    // Never NODE_PATH; what resolves outside the frozen copy (a parent folder's node_modules) is refused by the
    // callers, which compare the result with the copy's root (the resolver's own `restrictions` admit only the root)
    node_path: false,
    builtin_modules: true,
    ..ResolveOptions::default()
  })
}

enum Resolved {
  File(PathBuf),
  Builtin,
  Missing(String),
}

fn resolve(resolver: &Resolver, from: &Path, spec: &str) -> Resolved {
  let Some(dir) = from.parent() else { return Resolved::Missing("no folder".into()) };
  match resolver.resolve(dir, spec) {
    Ok(r) => Resolved::File(r.path().to_path_buf()),
    Err(ResolveError::Builtin { .. }) => Resolved::Builtin,
    Err(e) => Resolved::Missing(e.to_string()),
  }
}

fn transform_options() -> TransformOptions {
  let mut options = TransformOptions::default();
  options.jsx.runtime = JsxRuntime::Automatic;
  options
}

fn source_type(file: &Path) -> SourceType {
  match ext_of(file).as_deref() {
    Some("ts" | "mts" | "cts") => SourceType::ts(),
    Some("tsx") => SourceType::tsx(),
    // Plain `.js` may hold JSX as React projects write it
    Some("cjs") => SourceType::cjs(),
    _ => SourceType::jsx(),
  }
}

/// The literal module specifiers of compiled JavaScript with where each string sits (quotes included)
fn specifiers(code: &str, rel: &str) -> Result<Vec<(Span, String)>, String> {
  let alloc = Allocator::default();
  let parsed = Parser::new(&alloc, code, SourceType::mjs()).parse();
  if let Some(d) = parsed.diagnostics.first() {
    return Err(format!("{rel}: compiled output does not parse: {}", d.message));
  }
  let mut found = Specifiers::default();
  found.visit_program(&parsed.program);
  Ok(found.0)
}

#[derive(Default)]
struct Specifiers(Vec<(Span, String)>);

impl<'a> Visit<'a> for Specifiers {
  fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
    self.0.push((it.source.span, it.source.value.display().to_string()));
  }

  fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
    self.0.push((it.source.span, it.source.value.display().to_string()));
  }

  fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
    self.0.push((it.source.span, it.source.value.display().to_string()));
  }

  fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
    if let Expression::StringLiteral(s) = &it.source {
      self.0.push((s.span, s.value.display().to_string()));
    }
    walk::walk_import_expression(self, it);
  }
}

/// `require('x')` with a literal argument, anywhere in a CommonJS module
#[derive(Default)]
struct Requires(std::collections::BTreeSet<String>);

impl<'a> Visit<'a> for Requires {
  fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
    if let Expression::Identifier(callee) = &it.callee
      && callee.name == "require"
      && let Some(Argument::StringLiteral(s)) = it.arguments.first()
    {
      self.0.insert(s.value.display().to_string());
    }
    walk::walk_call_expression(self, it);
  }
}

fn apply_edits(code: &str, mut edits: Vec<(Span, String)>) -> String {
  edits.sort_by_key(|(s, _)| std::cmp::Reverse(s.start));
  let mut out = code.to_owned();
  for (span, to) in edits {
    let (a, b) = (span.start as usize, span.end as usize);
    if a <= b && b <= out.len() && out.is_char_boundary(a) && out.is_char_boundary(b) {
      out.replace_range(a..b, &js_string(&to));
    }
  }
  out
}

fn diagnostic(rel: &str, src: &str, d: &oxc_diagnostics::OxcDiagnostic) -> String {
  match d.labels.first() {
    Some(label) => {
      let (line, col) = line_col(src, label.offset() as usize);
      format!("{rel}:{line}:{col}: {}", d.message)
    }
    None => format!("{rel}: {}", d.message),
  }
}

fn line_col(src: &str, offset: usize) -> (usize, usize) {
  let upto = src.get(..offset.min(src.len())).unwrap_or(src);
  let line = upto.matches('\n').count() + 1;
  let col = upto.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) + 1;
  (line, col)
}

fn ext_of(path: &Path) -> Option<String> {
  path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn canonical(path: &Path) -> Result<PathBuf, String> {
  dunce_canonical(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// `canonicalize` without Windows' `\\?\` prefix, which file URLs and the resolver do not expect
fn dunce_canonical(path: &Path) -> std::io::Result<PathBuf> {
  let p = std::fs::canonicalize(path)?;
  #[cfg(windows)]
  {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\")
      && !rest.starts_with("UNC\\")
    {
      return Ok(PathBuf::from(rest));
    }
  }
  Ok(p)
}

/// A module's id: its path inside the frozen copy with `/`
fn rel_of(root: &Path, file: &Path) -> Result<String, String> {
  let rel = file.strip_prefix(root).map_err(|_| format!("{} lies outside the plugin folder", file.display()))?;
  let parts: Vec<String> = rel
    .components()
    .filter_map(|c| match c {
      Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
      _ => None,
    })
    .collect();
  Ok(parts.join("/"))
}

/// `file:` URL of an absolute path, percent-encoding what a URL path may not hold
pub fn file_url(path: &Path) -> String {
  let s = path.to_string_lossy().replace('\\', "/");
  let mut out = String::from("file://");
  if !s.starts_with('/') {
    out.push('/');
  }
  for b in s.bytes() {
    match b {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' | b'@' | b'+' | b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b',' | b';' | b'=' => out.push(b as char),
      _ => {
        let _ = write!(out, "%{b:02X}");
      }
    }
  }
  out
}

fn js_string(s: &str) -> String {
  serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()).replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029")
}

fn safe_name(s: &str) -> String {
  s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
  if let Some(parent) = path.parent() {
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
  }
  std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn sibling_tmp(out: &Path) -> Result<PathBuf, String> {
  let parent = out.parent().ok_or("no output folder")?;
  std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
  Ok(parent.join(format!(".tmp-{}", crate::util::random_hex(8))))
}

/// Put a finished output in place; another engine that finished the same build first wins
fn publish(tmp: &Path, out: &Path) -> Result<(), String> {
  if out.join("runtime").is_dir() {
    let _ = std::fs::remove_dir_all(tmp);
    return Ok(());
  }
  if out.is_dir() {
    // An output holding only interface documents so far: move the runtime in
    let r = std::fs::rename(tmp.join("runtime"), out.join("runtime"));
    let _ = std::fs::remove_dir_all(tmp);
    return if r.is_ok() || out.join("runtime").is_dir() { Ok(()) } else { r.map_err(|e| format!("{}: {e}", out.display())) };
  }
  match std::fs::rename(tmp, out) {
    Ok(()) => Ok(()),
    Err(_) if out.join("runtime").is_dir() => {
      let _ = std::fs::remove_dir_all(tmp);
      Ok(())
    }
    Err(e) => {
      let _ = std::fs::remove_dir_all(tmp);
      Err(format!("{}: {e}", out.display()))
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
  }

  fn node() -> bool {
    std::process::Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success())
  }

  fn run_node(script: &str) -> String {
    let out = std::process::Command::new("node").args(["--input-type=module", "-e", script]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
  }

  #[test]
  fn runtime_modules_run_under_node() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("snap");
    write(&dir, "plugin.ts", "import pad from 'left-pad';\nimport { twice, type Num } from './util';\nimport path from 'node:path';\nconst n: Num = twice(21);\nexport default { v: pad(String(n), 4) + path.sep };\n");
    write(&dir, "util.ts", "export type Num = number;\nexport function twice(n: Num): Num { return n * 2; }\n");
    write(&dir, "node_modules/left-pad/package.json", r#"{ "name": "left-pad", "main": "index.js" }"#);
    write(&dir, "node_modules/left-pad/index.js", "module.exports = (s, n) => ' '.repeat(Math.max(0, n - s.length)) + s;\n");
    let out = tmp.path().join("out");
    let boot = runtime(&dir, "plugin.ts", &out).unwrap();
    assert!(boot.is_file());
    let compiled = out.join("runtime").join("plugin.ts.mjs");
    let text = std::fs::read_to_string(&compiled).unwrap();
    assert!(!text.contains(": Num"), "types stripped: {text}");
    assert!(text.contains("util.ts.mjs") && text.contains("left-pad/index.js") && text.contains("\"node:path\""), "{text}");
    // The SDK the bootstrap imports is the engine's
    assert!(out.join("runtime/__acpira/plugin.mjs").is_file());
    if node() {
      let got = run_node(&format!("const m = await import({}); console.log(JSON.stringify(m.default));", js_string(&file_url(&compiled))));
      // JSON.stringify escapes the Windows separator, so build the expectation the same way
      let want = serde_json::json!({ "v": format!("  42{}", std::path::MAIN_SEPARATOR) }).to_string();
      assert_eq!(got, want);
    }
    // A second call reuses the output
    assert_eq!(runtime(&dir, "plugin.ts", &out).unwrap(), boot);
  }

  #[test]
  fn interface_documents_inline_the_graph_and_mount_the_default_export() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("snap");
    write(
      &dir,
      "Panel.tsx",
      r#"import { useState } from 'react';
import { Button } from '@acpira/plugin-ui';
import label from './label';
import { shout } from 'tiny-esm';
import cjs from 'tiny-cjs';
import data from './data.json';
import './panel.css';
import './after.css';
import icon from './dot.svg';
type P = { n: number };
export default function Panel({ n }: P) {
  const [c] = useState<number>(n);
  return <Button title={icon.slice(0, 14)}>{label(c)} {shout(cjs.word)} {data.k}</Button>;
}
"#,
    );
    write(&dir, "label.ts", "export default (n: number) => `n=${n}`;\n");
    write(&dir, "data.json", r#"{ "k": "json" }"#);
    write(&dir, "panel.css", ".x { color: red }\n</script><script>alert(1)</script>\n");
    write(&dir, "after.css", ".after {}\n");
    write(&dir, "dot.svg", "<svg xmlns='http://www.w3.org/2000/svg'/>");
    write(&dir, "node_modules/tiny-esm/package.json", r#"{ "name": "tiny-esm", "type": "module", "exports": { ".": { "browser": "./b.js", "default": "./n.js" } } }"#);
    write(&dir, "node_modules/tiny-esm/b.js", "export const shout = s => s.toUpperCase() + '!';\n");
    write(&dir, "node_modules/tiny-esm/n.js", "throw new Error('the node build');\n");
    write(&dir, "node_modules/tiny-cjs/package.json", r#"{ "name": "tiny-cjs", "main": "index.js" }"#);
    write(&dir, "node_modules/tiny-cjs/index.js", "var w = require('./word');\ntry { require('optional-missing'); } catch (e) {}\nexports.word = w;\n");
    write(&dir, "node_modules/tiny-cjs/word.js", "module.exports = 'hi';\n");
    let out = tmp.path().join("out");
    let doc = ui(&dir, "Panel.tsx", "main", &out).unwrap();
    assert!(doc.contains("acpira-runtime"));
    // A stylesheet's `</script>` cannot end the frame's script
    assert_eq!(doc.matches("</script>").count(), 1, "{doc}");
    assert!(!doc.contains("the node build"), "the browser condition picks the browser build");
    assert!(out.join("ui/main.html").is_file());
    if node() {
      let script = doc.split("<script>\n").nth(1).unwrap().split("\n</script>").next().unwrap();
      let harness = format!(
        r#"const styles = [];
globalThis.document = {{ head: {{ appendChild: el => styles.push(el.textContent) }}, createElement: () => ({{ setAttribute() {{}} }}), getElementById: () => null, body: {{}} }};
const jsx = (t, p) => ({{ t: typeof t === 'string' ? t : t.name, p }});
let mounted;
globalThis.__acpiraLib = {{
  react: {{ useState: v => [v, () => {{}}], default: {{}} }},
  'react/jsx-runtime': {{ jsx, jsxs: jsx, Fragment: 'Fragment' }},
  '@acpira/plugin-ui': {{ Button: function Button() {{}}, mount: C => {{ mounted = C; }} }},
}};
{script}
await new Promise(r => setTimeout(r, 50));
const tree = mounted({{ n: 3 }});
console.log(JSON.stringify({{ t: tree.t, title: tree.p.title, children: tree.p.children, css: styles.map(s => s.slice(0, 2)) }}));"#
      );
      let got = run_node(&harness);
      assert_eq!(got, r#"{"t":"Button","title":"data:image/svg","children":["n=3"," ","HI!"," ","json"],"css":[".x",".a"]}"#);
    }
  }

  #[test]
  fn errors_name_the_file_and_line() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("snap");
    write(&dir, "bad.tsx", "export default function A() {\n  return <div>;\n}\n");
    let e = ui(&dir, "bad.tsx", "p", &tmp.path().join("o1")).unwrap_err();
    assert!(e.starts_with("bad.tsx:") && e.split(':').nth(1).is_some_and(|l| l.parse::<u32>().is_ok()), "{e}");
    write(&dir, "missing.ts", "import x from './nowhere';\nexport default x;\n");
    let e = runtime(&dir, "missing.ts", &tmp.path().join("o2")).unwrap_err();
    assert!(e.contains("cannot find `./nowhere`"), "{e}");
    write(&dir, "sdk.tsx", "import { definePlugin } from '@acpira/plugin';\nexport default definePlugin;\n");
    let e = ui(&dir, "sdk.tsx", "p", &tmp.path().join("o3")).unwrap_err();
    assert!(e.contains("@acpira/plugin-ui"), "{e}");
    write(tmp.path(), "outside.ts", "export default 1;\n");
    write(&dir, "escape.ts", "import x from '../outside';\nexport default x;\n");
    assert!(runtime(&dir, "escape.ts", &tmp.path().join("o4")).is_err());
    write(&dir, "fs.tsx", "import fs from 'node:fs';\nexport default fs;\n");
    let e = ui(&dir, "fs.tsx", "p", &tmp.path().join("o5")).unwrap_err();
    assert!(e.contains("Node module"), "{e}");
  }

  #[test]
  fn shared_code_is_checked_through_what_it_imports() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write(dir, "shared.ts", "import { two } from './lib/two';\nimport React from 'react';\nexport const one = (): number => two() - 1;\n");
    write(dir, "lib/two.ts", "export const two = (): number => 2;\n");
    assert_eq!(exports_compile(dir, "shared.ts"), Ok(()));
    write(dir, "lib/two.ts", "export const two = (: number => 2;\n");
    assert!(exports_compile(dir, "shared.ts").unwrap_err().contains("lib/two.ts"));
    write(dir, "lib/two.ts", "export { gone } from './gone';\n");
    assert!(exports_compile(dir, "shared.ts").unwrap_err().contains("cannot find `./gone`"));
  }

  #[test]
  fn script_safe_escapes_what_ends_a_script() {
    assert_eq!(script_safe("a</SCRIPT>b<!--c"), "a<\\/SCRIPT>b<\\!--c");
    assert_eq!(script_safe("中</script"), "中<\\/script");
  }

  #[test]
  fn file_urls_encode_what_a_path_may_hold() {
    assert_eq!(file_url(Path::new("/a b/c#d.mjs")), "file:///a%20b/c%23d.mjs");
  }
}
