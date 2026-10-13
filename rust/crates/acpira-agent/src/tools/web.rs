//! `webfetch` and `websearch`, after OpenCode's tools of the same names (`tool/webfetch.ts`, `tool/websearch.ts`).
//!
//! `webfetch` GETs a URL on this machine: HTML becomes markdown (`html.rs`), text and JSON come back as they are, any
//! other file is saved under the session's `outputs/` and its path returned. Public addresses run without a card (as
//! OpenCode allows `webfetch` by default); the local machine and private networks ask first (permission `web-local`), and
//! a public URL that redirects or resolves to one is refused rather than followed, so a page cannot steer the agent at
//! the user's router or a dev server. Redirects are followed by hand for that reason, at most `MAX_HOPS`.
//!
//! `websearch` asks Jina Search first when the settings page picked it and holds its key (`SearchSetup`; Google's results,
//! after keke's `search/jina.rs`), then the free hosted services over MCP's `tools/call` (plain JSON-RPC over HTTP, no
//! session): Parallel's endpoint, then Exa's (its keyless endpoint is rate-limited per IP, which made it fail on
//! 2026-10-14 where Parallel's answered). `EXA_API_KEY` puts Exa first, `ACPIRA_WEBSEARCH_PROVIDER` (`parallel` / `exa`)
//! picks which free one leads, `PARALLEL_API_KEY` is sent when set. Queries leave the machine, results come back as
//! markdown

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::html;
use super::{Action, Ctx, Output, budgeted, num_arg, str_arg, text_content};
use crate::budget::Keep;
use crate::llm::ToolSpec;

/// The largest body read (OpenCode's limit)
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const DEFAULT_TIMEOUT_S: u64 = 30;
const MAX_TIMEOUT_S: u64 = 120;
const MAX_HOPS: usize = 5;
/// A page whose readable text is shorter than this is probably rendered by scripts
const THIN_PAGE: usize = 200;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(25);
const DEFAULT_RESULTS: usize = 8;
const MAX_RESULTS: usize = 20;
/// Characters of excerpt kept per search result
const EXCERPT_CHARS: usize = 2500;
/// A browser's user agent: many sites answer a bare client with a block page
const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36";

const PARALLEL_URL: &str = "https://search.parallel.ai/mcp";
const JINA_URL: &str = "https://s.jina.ai/";
/// Jina answers in 1.5 to 2.5 s; a stuck call gives way to the free services rather than taking the whole budget
const JINA_TIMEOUT: Duration = Duration::from_secs(10);
const EXA_URL: &str = "https://mcp.exa.ai/mcp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
  Markdown,
  Text,
  Html,
}

pub fn fetch_spec() -> ToolSpec {
  ToolSpec {
    name: super::WEBFETCH.into(),
    description: "Fetch a URL (http or https) and return its content. HTML pages come back as markdown (or plain text or the raw HTML \
                  on request), text and JSON as they are; other files (images, PDFs, archives) are saved and their path returned. \
                  Use it for any link the user gives and to read pages found with websearch. Addresses on this machine or a \
                  private network ask the user first."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "url": { "type": "string", "description": "The full URL, starting with http:// or https://" },
        "format": { "type": "string", "enum": ["markdown", "text", "html"], "description": "How an HTML page comes back (default markdown)" },
        "timeout": { "type": "number", "description": "Seconds to wait (default 30, at most 120)" },
      },
      "required": ["url"],
    }),
  }
}

/// The description names the current year: models otherwise search for news "of" their training year
pub fn search_spec() -> ToolSpec {
  ToolSpec {
    name: super::WEBSEARCH.into(),
    description: format!(
      "Search the web. Returns the top results with their URLs and excerpts, often enough to answer; read a result in full \
       with webfetch. Use it for current events, recent releases, documentation, error messages and anything that may have \
       changed since your training. The current year is {}.",
      current_year()
    ),
    parameters: json!({
      "type": "object",
      "properties": {
        "query": { "type": "string", "description": "What to search for: keywords, or a short question" },
        "count": { "type": "number", "description": "How many results (default 8, at most 20)" },
      },
      "required": ["query"],
    }),
  }
}

pub fn prepare_fetch(args: &Value) -> Result<Action, String> {
  let raw = str_arg(args, "url")?.trim();
  let lower = raw.to_ascii_lowercase();
  if !(lower.starts_with("http://") || lower.starts_with("https://")) {
    return Err("The URL must start with http:// or https://".into());
  }
  let url = normal_url(raw).ok_or("The URL has no host")?;
  let format = match args.get("format").and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
    None | Some("" | "markdown" | "md") => Format::Markdown,
    Some("text" | "txt" | "plain") => Format::Text,
    Some("html") => Format::Html,
    Some(other) => return Err(format!("Unknown format \"{other}\"; use markdown, text or html")),
  };
  let timeout_s = num_arg(args, "timeout").unwrap_or(DEFAULT_TIMEOUT_S).clamp(1, MAX_TIMEOUT_S);
  let private = private_name(&url);
  Ok(Action::Fetch { url, format, timeout_s, private })
}

pub fn prepare_search(args: &Value) -> Result<Action, String> {
  let query = str_arg(args, "query")?.trim();
  if query.is_empty() {
    return Err("The query is empty".into());
  }
  let count = num_arg(args, "count").map_or(DEFAULT_RESULTS, |n| n as usize).clamp(1, MAX_RESULTS);
  Ok(Action::Search { query: query.to_owned(), count })
}

// URLs

/// `scheme://authority` and the rest (path, query, fragment); None without `://` or a host
fn split(url: &str) -> Option<(&str, &str, &str)> {
  let (scheme, after) = url.split_once("://")?;
  if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) {
    return None;
  }
  let end = after.find(['/', '?', '#']).unwrap_or(after.len());
  let authority = &after[..end];
  (!host_of(authority).is_empty()).then_some((scheme, authority, &after[end..]))
}

/// The host of an authority, lowercase, without user info, port or IPv6 brackets
fn host_of(authority: &str) -> String {
  let a = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
  let host = if let Some(v6) = a.strip_prefix('[') {
    v6.split(']').next().unwrap_or("")
  } else {
    match a.rsplit_once(':') {
      Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
      _ => a,
    }
  };
  host.trim_end_matches('.').to_ascii_lowercase()
}

/// The port an authority names, else the scheme's
fn port_of(scheme: &str, authority: &str) -> u16 {
  let a = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
  let after = a.rsplit_once(']').map_or(a, |(_, p)| p);
  after.rsplit_once(':').and_then(|(_, p)| p.parse().ok()).unwrap_or(if scheme.eq_ignore_ascii_case("https") { 443 } else { 80 })
}

/// The URL with a lowercase scheme and at least `/` as its path, so origin patterns (`http://host:3000/*`) match it
fn normal_url(url: &str) -> Option<String> {
  let (scheme, authority, rest) = split(url)?;
  let rest = if rest.starts_with('/') { rest.to_owned() } else { format!("/{rest}") };
  Some(format!("{}://{authority}{rest}", scheme.to_ascii_lowercase()))
}

/// `scheme://authority` of a URL
pub fn origin(url: &str) -> Option<String> {
  split(url).map(|(s, a, _)| format!("{s}://{a}"))
}

/// `href` made absolute against `base` (RFC 3986 reference resolution, without percent-encoding work)
pub fn join(base: &str, href: &str) -> String {
  let href = href.trim();
  let has_scheme = href.find(':').is_some_and(|i| {
    let s = &href[..i];
    !s.is_empty() && s.starts_with(|c: char| c.is_ascii_alphabetic()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
  });
  let Some((scheme, authority, rest)) = split(base).filter(|_| !has_scheme) else {
    return href.to_owned();
  };
  let path_end = rest.find(['?', '#']).unwrap_or(rest.len());
  let path = &rest[..path_end];
  if let Some(net) = href.strip_prefix("//") {
    return format!("{scheme}://{net}");
  }
  if href.starts_with('/') {
    return format!("{scheme}://{authority}{}", remove_dots(href));
  }
  if href.is_empty() || href.starts_with('#') {
    let no_fragment = rest.split('#').next().unwrap_or("");
    return format!("{scheme}://{authority}{no_fragment}{href}");
  }
  if href.starts_with('?') {
    return format!("{scheme}://{authority}{path}{href}");
  }
  let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
  format!("{scheme}://{authority}{}", remove_dots(&format!("{dir}{href}")))
}

/// `.` and `..` segments resolved in the path part (the query and fragment stay as written)
fn remove_dots(reference: &str) -> String {
  let end = reference.find(['?', '#']).unwrap_or(reference.len());
  let (path, tail) = reference.split_at(end);
  let mut out: Vec<&str> = vec![];
  let segments: Vec<&str> = path.split('/').skip(1).collect();
  for (i, seg) in segments.iter().enumerate() {
    let last = i + 1 == segments.len();
    match *seg {
      "." => {
        if last {
          out.push("");
        }
      }
      ".." => {
        out.pop();
        if last {
          out.push("");
        }
      }
      s => out.push(s),
    }
  }
  format!("/{}{tail}", out.join("/"))
}

// Private addresses

/// Whether the URL names this machine or a private network by itself: `localhost`, a single-label or `.local` /
/// `.internal` / `.lan` name, or an address in a private, loopback, link-local or shared range. A name that only
/// resolves to one is caught when the call runs (`resolves_private`)
pub fn private_name(url: &str) -> bool {
  let Some((_, authority, _)) = split(url) else { return false };
  let host = host_of(authority);
  if let Ok(ip) = host.parse::<IpAddr>() {
    return private_ip(ip);
  }
  // Numeric forms getaddrinfo also takes (`2130706433`, `0x7f.1`): never treated as public
  if host.split('.').next_back().is_some_and(|l| l.chars().all(|c| c.is_ascii_digit()) || l.starts_with("0x")) {
    return true;
  }
  !host.contains('.')
    || ["localhost", "local", "internal", "lan", "home", "corp", "intranet", "home.arpa"].iter().any(|s| host.ends_with(&format!(".{s}")))
}

fn private_ip(ip: IpAddr) -> bool {
  match ip {
    IpAddr::V4(v4) => private_v4(v4),
    IpAddr::V6(v6) => {
      if let Some(v4) = v6.to_ipv4_mapped() {
        return private_v4(v4);
      }
      let first = v6.segments()[0];
      v6.is_loopback()
        || v6.is_unspecified()
        // Unique local fc00::/7 and link-local fe80::/10
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || v6 == Ipv6Addr::LOCALHOST
    }
  }
}

fn private_v4(v4: Ipv4Addr) -> bool {
  let [a, b, ..] = v4.octets();
  v4.is_loopback()
    || v4.is_private()
    || v4.is_link_local()
    || v4.is_unspecified()
    || v4.is_broadcast()
    // Shared address space 100.64.0.0/10 (carrier NAT, Tailscale)
    || (a == 100 && (64..128).contains(&b))
}

/// The first private address the URL's host resolves to (no lookup for a literal address)
fn resolves_private(url: &str) -> Option<IpAddr> {
  let (scheme, authority, _) = split(url)?;
  let host = host_of(authority);
  if host.parse::<IpAddr>().is_ok() {
    return None;
  }
  // A lookup that fails says nothing (a proxy may resolve what this machine cannot)
  (host.as_str(), port_of(scheme, authority)).to_socket_addrs().ok()?.map(|a| a.ip()).find(|ip| private_ip(*ip))
}

// Fetch

fn client(timeout: Duration, direct: bool) -> ureq::Agent {
  let mut config = ureq::Agent::config_builder()
    .http_status_as_error(false)
    // Each hop is checked before it is requested
    .max_redirects(0)
    .timeout_global(Some(timeout))
    .timeout_connect(Some(timeout.min(Duration::from_secs(15))));
  if direct {
    // The local machine and private networks are never reached through the user's proxy
    config = config.proxy(None);
  }
  config.build().into()
}

fn accept(format: Format) -> &'static str {
  match format {
    Format::Markdown => "text/markdown;q=1.0, text/x-markdown;q=0.9, text/plain;q=0.8, text/html;q=0.7, */*;q=0.1",
    Format::Text => "text/plain;q=1.0, text/markdown;q=0.9, text/html;q=0.8, */*;q=0.1",
    Format::Html => "text/html;q=1.0, application/xhtml+xml;q=0.9, text/plain;q=0.8, text/markdown;q=0.7, */*;q=0.1",
  }
}

/// Fetch on a blocking thread. `approved` is whether the call was allowed to reach private addresses (its URL named
/// one, so the rules for `web-local` decided)
pub fn fetch(url: &str, format: Format, timeout_s: u64, approved: bool, ctx: &Ctx) -> Output {
  let deadline = Instant::now() + Duration::from_secs(timeout_s);
  let mut current = url.to_owned();
  for _ in 0..=MAX_HOPS {
    let private = private_name(&current) || resolves_private(&current).is_some();
    if private && !approved {
      let why = match resolves_private(&current) {
        Some(ip) if !private_name(&current) => format!("{current} resolves to {ip}, a private network address"),
        _ => format!("{current} is on this machine or a private network"),
      };
      let note = if current == url { "" } else { " (reached by a redirect)" };
      return Output::error(format!(
        "Not fetched: {why}{note}. Such addresses are fetched only when the URL names them directly, after the user approves."
      ));
    }
    let Some(left) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) else {
      return Output::error(format!("Timed out after {timeout_s} s fetching {url}"));
    };
    let http = client(left, private);
    let get = |ua: &str| http.get(&current).header("User-Agent", ua).header("Accept", accept(format)).header("Accept-Language", "en-US,en;q=0.9").call();
    let mut res = match get(BROWSER_UA) {
      Ok(r) => r,
      Err(e) => return Output::error(format!("Could not fetch {current}: {e}")),
    };
    // Cloudflare's bot check rejects a browser agent whose TLS fingerprint is not a browser's; an honest one may pass
    if res.status() == 403 && res.headers().get("cf-mitigated").and_then(|v| v.to_str().ok()) == Some("challenge") {
      res = match get(&format!("acpira/{}", env!("CARGO_PKG_VERSION"))) {
        Ok(r) => r,
        Err(e) => return Output::error(format!("Could not fetch {current}: {e}")),
      };
    }
    let status = res.status();
    if status.is_redirection() {
      match res.headers().get("location").and_then(|v| v.to_str().ok()) {
        Some(location) => {
          current = normal_url(&join(&current, location)).unwrap_or_else(|| join(&current, location));
          if !current.to_ascii_lowercase().starts_with("http") {
            return Output::error(format!("{url} redirects to {current}, which is not an http(s) URL"));
          }
          continue;
        }
        None => return Output::error(format!("{current} answered {status} without a Location")),
      }
    }
    let content_type = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
    if res.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| n > MAX_BYTES) {
      return Output::error(format!("{current} is larger than {} MB; not fetched", MAX_BYTES / 1024 / 1024));
    }
    let bytes = match res.body_mut().with_config().limit(MAX_BYTES).read_to_vec() {
      Ok(b) => b,
      Err(ureq::Error::BodyExceedsLimit(_)) => return Output::error(format!("{current} is larger than {} MB; not fetched", MAX_BYTES / 1024 / 1024)),
      Err(e) => return Output::error(format!("Could not read {current}: {e}")),
    };
    if !status.is_success() {
      let body = String::from_utf8_lossy(&bytes);
      let detail = if is_html(&content_type, &bytes) { html::convert(&body, &current, true).body } else { body.into_owned() };
      return Output::error(format!("{current} answered HTTP {status}.\n\n{}", crate::budget::cut(detail.trim(), 2000)));
    }
    return render(&current, &content_type, &bytes, format, ctx);
  }
  Output::error(format!("{url} redirects more than {MAX_HOPS} times; stopped at {current}"))
}

fn is_html(content_type: &str, bytes: &[u8]) -> bool {
  let mime = mime_of(content_type);
  if mime == "text/html" || mime == "application/xhtml+xml" {
    return true;
  }
  // No type given: sniff the start
  mime.is_empty() && {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).trim_start().to_ascii_lowercase();
    head.starts_with("<!doctype html") || head.starts_with("<html")
  }
}

fn mime_of(content_type: &str) -> String {
  content_type.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

fn is_text(mime: &str) -> bool {
  mime.starts_with("text/")
    || mime.ends_with("+json")
    || mime.ends_with("+xml")
    || ["json", "xml", "javascript", "ecmascript", "yaml", "toml", "x-sh", "csv", "graphql", "x-ndjson"].iter().any(|t| mime.ends_with(&format!("/{t}")) || mime.ends_with(&format!("/x-{t}")))
}

fn render(url: &str, content_type: &str, bytes: &[u8], format: Format, ctx: &Ctx) -> Output {
  let mime = mime_of(content_type);
  let text = if is_html(content_type, bytes) && format != Format::Html {
    let page = html::convert(&String::from_utf8_lossy(bytes), url, format == Format::Text);
    let mut head = format!("URL: {url}\n");
    if let Some(t) = &page.title {
      head.push_str(&format!("Title: {t}\n"));
    }
    if let Some(d) = &page.description {
      head.push_str(&format!("Description: {d}\n"));
    }
    let thin = page.body.chars().filter(|c| !c.is_whitespace()).count() < THIN_PAGE;
    let note = if thin {
      "\n[Little text is rendered on the server: the page is probably built by scripts. The title and description above \
       may be all there is; try the site's API, a feed, or websearch.]"
    } else {
      ""
    };
    format!("{head}\n{}{note}", page.body)
  } else if is_html(content_type, bytes) || is_text(&mime) || (mime.is_empty() && std::str::from_utf8(bytes).is_ok()) {
    String::from_utf8_lossy(bytes).into_owned()
  } else {
    // A binary file: keep it for other tools rather than pouring bytes into the context
    let ext = extension(&mime, url);
    let file = ctx.outputs.join(format!("{}.{ext}", ctx.call_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect::<String>()));
    let saved = std::fs::create_dir_all(&ctx.outputs).and_then(|_| std::fs::write(&file, bytes));
    return match saved {
      Ok(()) => {
        let message = format!("{url} is not text ({}, {} bytes). Saved to {}; inspect it with a command (file, pdftotext, unzip -l …).", if mime.is_empty() { "no content type" } else { &mime }, bytes.len(), file.display());
        Output { content: vec![text_content(&message)], model: message, is_error: false, raw_output: Some(json!({ "url": url, "contentType": mime, "saved": file })) }
      }
      Err(e) => Output::error(format!("{url} is not text ({mime}) and could not be saved: {e}")),
    };
  };
  let model = budgeted(&text, Keep::Head, ctx);
  Output { content: vec![text_content(&text)], model, is_error: false, raw_output: Some(json!({ "url": url, "contentType": mime, "bytes": bytes.len() })) }
}

/// A file extension for a saved download: from the type, else the URL's last segment
fn extension(mime: &str, url: &str) -> String {
  let known = match mime {
    "application/pdf" => Some("pdf"),
    "image/png" => Some("png"),
    "image/jpeg" => Some("jpg"),
    "image/gif" => Some("gif"),
    "image/webp" => Some("webp"),
    "image/svg+xml" => Some("svg"),
    "application/zip" => Some("zip"),
    "application/gzip" | "application/x-gzip" => Some("gz"),
    _ => None,
  };
  if let Some(e) = known {
    return e.to_owned();
  }
  let path = url.split(['?', '#']).next().unwrap_or("");
  path.rsplit('/').next().and_then(|name| name.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).filter(|e| !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or_else(|| "bin".to_owned())
}

// Search

/// What the settings page set up for `websearch` (`Config::web_search`)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchSetup {
  /// Present when Jina is the picked backend and has a key
  pub jina_key: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
  Jina,
  Parallel,
  Exa,
}

impl Provider {
  fn label(self) -> &'static str {
    match self {
      Provider::Jina => "Jina",
      Provider::Parallel => "Parallel",
      Provider::Exa => "Exa",
    }
  }
}

/// The free providers in the order they are tried
fn providers() -> [Provider; 2] {
  let exa_key = std::env::var("EXA_API_KEY").ok().filter(|k| !k.trim().is_empty());
  let first = match std::env::var("ACPIRA_WEBSEARCH_PROVIDER").ok().as_deref().map(str::trim) {
    Some("exa") => Provider::Exa,
    Some("parallel") => Provider::Parallel,
    _ if exa_key.is_some() => Provider::Exa,
    _ => Provider::Parallel,
  };
  if first == Provider::Exa { [Provider::Exa, Provider::Parallel] } else { [Provider::Parallel, Provider::Exa] }
}

pub fn search(query: &str, count: usize, setup: &SearchSetup, ctx: &Ctx) -> Output {
  let http: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(SEARCH_TIMEOUT)).build().into();
  // Parallel limits its free tier per conversation id: the session's folder name is stable across turns
  let session = ctx.outputs.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
  let mut failures = vec![];
  let order = setup.jina_key.iter().map(|_| Provider::Jina).chain(providers());
  for provider in order {
    let found = match (provider, &setup.jina_key) {
      (Provider::Jina, Some(key)) => search_jina(key, query, count),
      _ => search_with(&http, provider, query, count, &session),
    };
    match found {
      Ok(text) => {
        // A failed pick is named, so a bad or exhausted key shows instead of quietly costing quality
        let fallback = if failures.is_empty() { String::new() } else { format!(" ({} failed)", failures.join("; ")) };
        let text = format!("Results from {}{fallback} for \"{query}\":\n\n{text}", provider.label());
        let model = budgeted(&text, Keep::Head, ctx);
        return Output { content: vec![text_content(&text)], model, is_error: false, raw_output: Some(json!({ "provider": provider.label() })) };
      }
      Err(e) => failures.push(format!("{}: {e}", provider.label())),
    }
  }
  Output::error(format!("Web search failed. {}", failures.join("; ")))
}

fn search_with(http: &ureq::Agent, provider: Provider, query: &str, count: usize, session: &str) -> Result<String, String> {
  let (url, tool, args) = match provider {
    Provider::Parallel => {
      let mut args = json!({ "objective": query, "search_queries": [query] });
      if !session.is_empty() {
        args["session_id"] = session.chars().take(100).collect::<String>().into();
      }
      (PARALLEL_URL.to_owned(), "web_search", args)
    }
    Provider::Jina => return Err("no key".into()),
    Provider::Exa => {
      let url = match std::env::var("EXA_API_KEY").ok().filter(|k| !k.trim().is_empty()) {
        Some(key) => format!("{EXA_URL}?exaApiKey={}", percent_encode(key.trim())),
        None => EXA_URL.to_owned(),
      };
      (url, "web_search_exa", json!({ "query": query, "type": "auto", "numResults": count, "livecrawl": "fallback" }))
    }
  };
  let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": tool, "arguments": args } });
  let mut req = http.post(&url).header("Accept", "application/json, text/event-stream").header("Content-Type", "application/json");
  if provider == Provider::Parallel
    && let Some(key) = std::env::var("PARALLEL_API_KEY").ok().filter(|k| !k.trim().is_empty())
  {
    req = req.header("Authorization", &format!("Bearer {}", key.trim()));
  }
  let mut res = req.send(body.to_string()).map_err(|e| e.to_string())?;
  let status = res.status();
  let text = res.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| e.to_string())?;
  if !status.is_success() {
    return Err(format!("HTTP {status}: {}", crate::budget::cut(text.trim(), 300)));
  }
  let reply = mcp_reply(&text).ok_or_else(|| format!("unexpected answer: {}", crate::budget::cut(text.trim(), 300)))?;
  let result = match reply.get("result") {
    Some(r) => r,
    None => {
      let message = reply.pointer("/error/message").and_then(Value::as_str).unwrap_or("no result");
      return Err(message.to_owned());
    }
  };
  let content: String =
    result.get("content").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n");
  // Exa's keyless endpoint answers a rate limit as a successful result
  let limited = result.pointer("/_meta/ai.exa~1rateLimited").and_then(Value::as_bool) == Some(true);
  if limited || result.get("isError").and_then(Value::as_bool) == Some(true) {
    return Err(crate::budget::cut(content.trim(), 300).to_owned());
  }
  if content.trim().is_empty() {
    return Err("no results".into());
  }
  Ok(match provider {
    Provider::Parallel => parallel_results(&content, count).unwrap_or(content),
    _ => content,
  })
}

/// Jina Search with `X-Respond-With: no-content`: titles, links, snippets and dates only. Without it every result's whole
/// page comes back (keke measured about a million characters and 20 s for one query); with it a call takes 1.5 to 2.5 s
/// and is billed a flat 10k tokens
fn search_jina(key: &str, query: &str, count: usize) -> Result<String, String> {
  let http: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(JINA_TIMEOUT)).build().into();
  let url = format!("{JINA_URL}?q={}&num={count}", percent_encode(query));
  let mut res = http
    .get(&url)
    .header("Authorization", &format!("Bearer {key}"))
    .header("Accept", "application/json")
    .header("X-Respond-With", "no-content")
    .call()
    .map_err(|e| e.to_string())?;
  let status = res.status();
  let text = res.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| e.to_string())?;
  if status == 401 || status == 403 {
    return Err(format!("the key was refused (HTTP {status})"));
  }
  if !status.is_success() {
    let message = serde_json::from_str::<Value>(&text).ok().and_then(|v| v.get("readableMessage").or_else(|| v.get("message")).and_then(Value::as_str).map(str::to_owned));
    return Err(format!("HTTP {status}: {}", message.unwrap_or_else(|| crate::budget::cut(text.trim(), 300).to_owned())));
  }
  jina_results(&text, count)
}

/// Jina's `data: [{ title, url, description, date }]` as markdown; entries without a link are dropped
fn jina_results(body: &str, count: usize) -> Result<String, String> {
  let data: Value = serde_json::from_str(body).map_err(|e| format!("unexpected answer ({e})"))?;
  let items: Vec<&Value> = data.get("data").and_then(Value::as_array).into_iter().flatten().filter(|i| i.get("url").and_then(Value::as_str).is_some_and(|u| !u.is_empty())).take(count).collect();
  if items.is_empty() {
    return Ok("No results.".into());
  }
  let mut out = String::new();
  for (i, item) in items.iter().enumerate() {
    let s = |k: &str| item.get(k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty());
    let url = s("url").unwrap_or("");
    out.push_str(&format!("## {}. [{}]({url})\n", i + 1, s("title").unwrap_or(url)));
    if let Some(date) = s("date") {
      out.push_str(&format!("Published {date}\n"));
    }
    if let Some(d) = s("description") {
      out.push_str(&format!("\n{d}\n"));
    }
    out.push('\n');
  }
  Ok(out.trim_end().to_owned())
}

/// The JSON-RPC answer in a plain JSON body or in the `data:` lines of an event stream
fn mcp_reply(body: &str) -> Option<Value> {
  let trimmed = body.trim();
  if trimmed.starts_with('{') {
    return serde_json::from_str(trimmed).ok();
  }
  body.lines().filter_map(|l| l.strip_prefix("data:")).map(str::trim).filter_map(|d| serde_json::from_str::<Value>(d).ok()).find(|v| v.get("result").is_some() || v.get("error").is_some())
}

/// Parallel's JSON (`results: [{ url, title, publish_date, excerpts }]`) as markdown, excerpts capped per result
fn parallel_results(content: &str, count: usize) -> Option<String> {
  let data: Value = serde_json::from_str(content).ok()?;
  let results = data.get("results")?.as_array()?;
  if results.is_empty() {
    return Some("No results.".into());
  }
  let mut out = String::new();
  for (i, r) in results.iter().take(count).enumerate() {
    let url = r.get("url").and_then(Value::as_str).unwrap_or("");
    let title = r.get("title").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or(url);
    out.push_str(&format!("## {}. [{}]({url})\n", i + 1, title.trim()));
    if let Some(date) = r.get("publish_date").and_then(Value::as_str).filter(|d| !d.is_empty()) {
      out.push_str(&format!("Published {date}\n"));
    }
    let excerpts: Vec<&str> = r.get("excerpts").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
    let joined = excerpts.join("\n…\n");
    let cut: String = joined.chars().take(EXCERPT_CHARS).collect();
    if !cut.trim().is_empty() {
      out.push('\n');
      out.push_str(cut.trim());
      if cut.len() < joined.len() {
        out.push_str(" …");
      }
      out.push('\n');
    }
    out.push('\n');
  }
  Some(out.trim_end().to_owned())
}

fn percent_encode(s: &str) -> String {
  s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The current year in UTC (civil-from-days, Howard Hinnant's algorithm)
fn current_year() -> i64 {
  let days = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64 + 719_468;
  let era = days.div_euclid(146_097);
  let doe = days - era * 146_097;
  let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let month = if mp < 10 { mp + 3 } else { mp - 9 };
  yoe + era * 400 + i64::from(month <= 2)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn urls_join_like_a_browser() {
    let base = "https://e.com/docs/guide/page.html?x=1#top";
    assert_eq!(join(base, "other.html"), "https://e.com/docs/guide/other.html");
    assert_eq!(join(base, "../api/"), "https://e.com/docs/api/");
    assert_eq!(join(base, "./a/../b?q=2"), "https://e.com/docs/guide/b?q=2");
    assert_eq!(join(base, "/root"), "https://e.com/root");
    assert_eq!(join(base, "//cdn.e.com/x.js"), "https://cdn.e.com/x.js");
    assert_eq!(join(base, "?y=2"), "https://e.com/docs/guide/page.html?y=2");
    assert_eq!(join(base, "#s"), "https://e.com/docs/guide/page.html?x=1#s");
    assert_eq!(join(base, "mailto:a@e.com"), "mailto:a@e.com");
    assert_eq!(join("https://e.com", "a"), "https://e.com/a");
    assert_eq!(normal_url("HTTP://Localhost:3000").as_deref(), Some("http://Localhost:3000/"));
    assert_eq!(origin("https://u:p@e.com:8443/a?b").as_deref(), Some("https://u:p@e.com:8443"));
  }

  #[test]
  fn private_addresses_are_told_apart_from_public_ones() {
    for url in [
      "http://localhost:3000/",
      "http://127.0.0.1/",
      "http://10.1.2.3/",
      "http://192.168.1.1/",
      "http://172.20.0.1/",
      "http://169.254.169.254/latest/meta-data",
      "http://100.100.1.1/",
      "http://[::1]:8080/",
      "http://[fd00::1]/",
      "http://[::ffff:127.0.0.1]/",
      "http://0.0.0.0/",
      "http://2130706433/",
      "http://0x7f.1/",
      "http://router/",
      "http://printer.local/",
      "http://api.localhost/",
      "http://wiki.corp/",
    ] {
      assert!(private_name(url), "{url}");
    }
    for url in ["https://example.com/", "https://8.8.8.8/", "https://[2606:4700::1111]/", "https://x.com/a/status/1", "http://172.32.0.1/"] {
      assert!(!private_name(url), "{url}");
    }
    assert_eq!(port_of("https", "e.com"), 443);
    assert_eq!(port_of("http", "[::1]:8080"), 8080);
  }

  #[test]
  fn arguments_are_checked() {
    assert!(prepare_fetch(&json!({ "url": "file:///etc/passwd" })).is_err());
    assert!(prepare_fetch(&json!({ "url": "https://e.com", "format": "pdf" })).is_err());
    let a = prepare_fetch(&json!({ "url": " https://e.com ", "format": "TEXT", "timeout": "500" })).unwrap();
    assert_eq!(a, Action::Fetch { url: "https://e.com/".into(), format: Format::Text, timeout_s: MAX_TIMEOUT_S, private: false });
    assert!(matches!(prepare_fetch(&json!({ "url": "http://localhost:5173" })).unwrap(), Action::Fetch { private: true, .. }));
    assert!(prepare_search(&json!({ "query": "  " })).is_err());
    assert_eq!(prepare_search(&json!({ "query": "acp", "count": 99 })).unwrap(), Action::Search { query: "acp".into(), count: MAX_RESULTS });
  }

  #[test]
  fn search_answers_parse_from_json_and_event_streams() {
    let inner = json!({ "results": [
      { "url": "https://a.dev", "title": "A", "publish_date": "2026-10-13", "excerpts": ["one", "two"] },
      { "url": "https://b.dev", "title": "", "excerpts": ["x".repeat(3000)] },
      { "url": "https://c.dev", "title": "C", "excerpts": [] },
    ] });
    let text = parallel_results(&inner.to_string(), 2).unwrap();
    assert!(text.starts_with("## 1. [A](https://a.dev)\nPublished 2026-10-13\n\none\n…\ntwo\n\n## 2. [https://b.dev](https://b.dev)"), "{text}");
    assert!(text.ends_with(" …") && !text.contains("c.dev"));
    let reply = json!({ "jsonrpc": "2.0", "id": 1, "result": { "content": [{ "type": "text", "text": "hi" }] } });
    assert_eq!(mcp_reply(&reply.to_string()).unwrap(), reply);
    assert_eq!(mcp_reply(&format!("event: message\ndata: {reply}\n\n")).unwrap(), reply);
    assert!(mcp_reply("<html>").is_none());
    // Jina's no-content answer, cut from a real one in keke's tests
    let body = r#"{"code":200,"data":[{"title":"Announcing Rust 1.90.0","url":"https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/","description":"Get it with rustup.","date":"Sep 18, 2025","content":""},{"title":"Redirect","url":"","description":""},{"title":" ","url":"https://releases.rs/docs/1.90.0/","description":""}]}"#;
    assert_eq!(
      jina_results(body, 8).unwrap(),
      "## 1. [Announcing Rust 1.90.0](https://blog.rust-lang.org/2025/09/18/Rust-1.90.0/)\nPublished Sep 18, 2025\n\nGet it with rustup.\n\n## 2. [https://releases.rs/docs/1.90.0/](https://releases.rs/docs/1.90.0/)"
    );
    assert_eq!(jina_results(r#"{"data":null}"#, 8).unwrap(), "No results.");
  }

  /// A local server answering each request by its path, until the test ends
  fn serve() -> String {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
      for stream in listener.incoming().flatten() {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        // Drain the headers
        let mut h = String::new();
        while reader.read_line(&mut h).is_ok_and(|n| n > 2) {
          h.clear();
        }
        let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let (status, headers, body): (&str, String, Vec<u8>) = match path.as_str() {
          "/page" => ("200 OK", "Content-Type: text/html; charset=utf-8\r\n".into(), b"<title>P</title><h1>Hi</h1><p>See <a href='/b'>b</a></p>".to_vec()),
          "/moved" => ("302 Found", "Location: page\r\n".into(), vec![]),
          "/away" => ("302 Found", format!("Location: http://{addr}/page\r\n"), vec![]),
          "/img" => ("200 OK", "Content-Type: image/png\r\n".into(), vec![0x89, b'P', b'N', b'G', 0, 1, 2]),
          "/json" => ("200 OK", "Content-Type: application/json\r\n".into(), br#"{"a":1}"#.to_vec()),
          _ => ("404 Not Found", "Content-Type: text/html\r\n".into(), b"<p>No such <b>page</b></p>".to_vec()),
        };
        let mut s = stream;
        let _ = write!(s, "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = s.write_all(&body);
      }
    });
    format!("http://{addr}")
  }

  #[test]
  fn fetching_follows_redirects_converts_pages_and_saves_files() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = Ctx { cwd: dir.path().to_owned(), outputs: dir.path().join("outputs"), call_id: "f:1".into(), progress: Box::new(|_| {}), jobs: Default::default(), search: Default::default() };
    let base = serve();
    let out = fetch(&format!("{base}/moved"), Format::Markdown, 10, true, &ctx);
    assert!(!out.is_error, "{}", out.model);
    assert!(out.model.starts_with(&format!("URL: {base}/page\nTitle: P\n\n# Hi\n\nSee [b]({base}/b)")), "{}", out.model);
    // Little text: the note about script-built pages
    assert!(out.model.contains("probably built by scripts"));
    let out = fetch(&format!("{base}/page"), Format::Html, 10, true, &ctx);
    assert!(out.model.starts_with("<title>P</title>"));
    let out = fetch(&format!("{base}/json"), Format::Markdown, 10, true, &ctx);
    assert_eq!(out.model, r#"{"a":1}"#);
    let out = fetch(&format!("{base}/img"), Format::Markdown, 10, true, &ctx);
    assert!(!out.is_error && out.model.contains("not text (image/png, 7 bytes)"), "{}", out.model);
    assert_eq!(std::fs::read(dir.path().join("outputs/f_1.png")).unwrap().len(), 7);
    let out = fetch(&format!("{base}/nope"), Format::Markdown, 10, true, &ctx);
    assert!(out.is_error && out.model.contains("HTTP 404") && out.model.contains("No such page"), "{}", out.model);
    // Without the approval a private address is refused before any request
    let out = fetch(&format!("{base}/away"), Format::Markdown, 10, false, &ctx);
    assert!(out.is_error && out.model.contains("private network"), "{}", out.model);
  }

  #[test]
  fn the_year_is_plausible_and_downloads_get_an_extension() {
    assert!((2026..2100).contains(&current_year()));
    assert_eq!(extension("application/pdf", "https://e.com/x"), "pdf");
    assert_eq!(extension("application/octet-stream", "https://e.com/a/b.tar.zst?x=1"), "zst");
    assert_eq!(extension("", "https://e.com/download"), "bin");
    assert_eq!(percent_encode("a b/+"), "a%20b%2F%2B");
  }
}
