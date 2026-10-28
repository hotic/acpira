//! acpira-app://localhost/…: the page, the bundle, the theme sheet, attachment blobs (files only from their own roots) and
//! the tool panel's local media in byte ranges (acpira_desktop::page::route). `/blank` is the empty document a new webview
//! starts on, so WebKit's frame-rate switch is set before the real page loads

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use acpira_desktop::page::{ByteRange, PLUGIN_FRAME_HTML, PageConfig, PageHost, Route, THEME_CSS, byte_range, mime, page_html, route, unpack};
use tauri::http::{Request, Response, StatusCode};

/// The most a media response carries: a player asking for the rest of a long video gets this much and asks on
const MEDIA_CHUNK: u64 = 4 * 1024 * 1024;

fn respond(status: StatusCode, mime: &str, body: Vec<u8>) -> Response<Vec<u8>> {
  // The status and headers are the shell's own constants; a builder error leaves an empty 200 rather than a panic
  Response::builder().status(status).header("content-type", mime).header("cache-control", "no-store").body(body).unwrap_or_default()
}

/// A media file, or the asked range of it (206). Players ask in ranges, a chunk at most each, so a long video is never read
/// whole; a request without one (an image, a PDF) gets the whole file
fn media(file: &Path, mime: &str, range: Option<&str>) -> Response<Vec<u8>> {
  let read = || -> std::io::Result<Response<Vec<u8>>> {
    let mut f = std::fs::File::open(file)?;
    let len = f.metadata()?.len();
    let span = match byte_range(range, len, MEDIA_CHUNK) {
      ByteRange::Unsatisfiable => {
        let r = Response::builder()
          .status(StatusCode::RANGE_NOT_SATISFIABLE)
          .header("content-range", format!("bytes */{len}"))
          .body(Vec::new());
        return Ok(r.unwrap_or_default());
      }
      ByteRange::Span(start, end) => Some((start, end)),
      ByteRange::Whole => None,
    };
    let (start, end) = span.unwrap_or((0, len.saturating_sub(1)));
    let mut body = Vec::new();
    if len > 0 {
      f.seek(SeekFrom::Start(start))?;
      f.take(end - start + 1).read_to_end(&mut body)?;
    }
    let mut r = Response::builder()
      .header("content-type", mime)
      .header("accept-ranges", "bytes")
      .header("cache-control", "no-store")
      .header("x-content-type-options", "nosniff");
    r = match span {
      Some(_) => r.status(StatusCode::PARTIAL_CONTENT).header("content-range", format!("bytes {start}-{}/{len}", (start + body.len() as u64).saturating_sub(1))),
      None => r.status(StatusCode::OK),
    };
    Ok(r.body(body).unwrap_or_default())
  };
  read().unwrap_or_else(|_| respond(StatusCode::NOT_FOUND, "text/plain", b"not found".to_vec()))
}

pub fn handle(request: Request<Vec<u8>>, responder: tauri::UriSchemeResponder) {
  let uri = request.uri();
  if uri.host() != Some("localhost") {
    responder.respond(respond(StatusCode::NOT_FOUND, "text/plain", b"not found".to_vec()));
    return;
  }
  let path = uri.path().to_owned();
  if path == "/blank" {
    responder.respond(respond(
      StatusCode::OK,
      "text/html; charset=utf-8",
      b"<!doctype html><meta charset=\"utf-8\"><title>Acpira</title>".to_vec(),
    ));
    return;
  }
  let Some(app) = crate::app::try_app() else {
    responder.respond(respond(StatusCode::SERVICE_UNAVAILABLE, "text/plain", Vec::new()));
    return;
  };
  // The Windows exe carries the bundle itself (build.rs)
  if let Some(rest) = path.strip_prefix("/webview/")
    && let Some((body, packed)) = crate::embedded::file(rest)
  {
    let mime = mime(std::path::Path::new(rest));
    if packed {
      let name = rest.to_owned();
      std::thread::spawn(move || match zstd::stream::decode_all(body) {
        Ok(plain) => responder.respond(respond(StatusCode::OK, mime, plain)),
        Err(e) => {
          crate::app::log(&format!("serve: unpacking {name}: {e}"));
          responder.respond(respond(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Vec::new()));
        }
      });
    } else {
      responder.respond(respond(StatusCode::OK, mime, body.to_vec()));
    }
    return;
  }
  let range = request.headers().get("range").and_then(|v| v.to_str().ok()).map(str::to_owned);
  let blobs = app.client().sessions_dir().map(std::path::PathBuf::from);
  match route(&path, uri.query(), &app.paths.webview, blobs.as_deref()) {
    Route::Page { host, tab } => {
      // Only a chat window's page runs the first-run flow; a browser page has no engine view to wait for
      let first_run = if host == PageHost::Editor && tab.is_none() { app.first_run_page() } else { None };
      let html = page_html(&PageConfig {
        host,
        locale: app.host_language(),
        dark: crate::os::dark_mode(),
        font: crate::app::page_font(),
        tab,
        first_run,
      });
      responder.respond(respond(StatusCode::OK, "text/html; charset=utf-8", html.into_bytes()));
    }
    Route::Theme => responder.respond(respond(StatusCode::OK, "text/css; charset=utf-8", THEME_CSS.as_bytes().to_vec())),
    Route::PluginFrame => responder.respond(respond(StatusCode::OK, "text/html; charset=utf-8", PLUGIN_FRAME_HTML.as_bytes().to_vec())),
    Route::File(file) => {
      // Off the main thread: the bundle is megabytes, a blob can be more
      std::thread::spawn(move || match std::fs::read(&file) {
        Ok(body) => responder.respond(respond(StatusCode::OK, mime(&file), body)),
        Err(_) => responder.respond(respond(StatusCode::NOT_FOUND, "text/plain", b"not found".to_vec())),
      });
    }
    Route::Media { file, mime } => {
      std::thread::spawn(move || responder.respond(media(&file, mime, range.as_deref())));
    }
    Route::Packed { file, mime } => {
      std::thread::spawn(move || match unpack(&file) {
        Ok(body) => responder.respond(respond(StatusCode::OK, mime, body)),
        Err(e) => {
          crate::app::log(&format!("serve: unpacking {}: {e}", file.display()));
          responder.respond(respond(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Vec::new()));
        }
      });
    }
    Route::NotFound => responder.respond(respond(StatusCode::NOT_FOUND, "text/plain", b"not found".to_vec())),
  }
}
