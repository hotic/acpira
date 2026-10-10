//! Server-sent events over a blocking reader: `event:` / `data:` fields, a blank line ends an event, `:` lines are
//! comments (keep-alives). Multi-line data joins with `\n`

use std::io::BufRead;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
  pub event: Option<String>,
  pub data: String,
}

pub struct SseReader<R> {
  inner: R,
  line: Vec<u8>,
}

impl<R: BufRead> SseReader<R> {
  pub fn new(inner: R) -> Self {
    SseReader { inner, line: Vec::with_capacity(4096) }
  }

  /// The next event; Ok(None) at the end of the stream (a trailing event without its blank line still counts)
  pub fn next_event(&mut self) -> std::io::Result<Option<SseEvent>> {
    let mut ev = SseEvent::default();
    let mut has_data = false;
    loop {
      self.line.clear();
      let n = self.inner.read_until(b'\n', &mut self.line)?;
      if n == 0 {
        return Ok(has_data.then_some(ev));
      }
      let text = String::from_utf8_lossy(&self.line);
      let line = text.trim_end_matches(['\n', '\r']);
      if line.is_empty() {
        if has_data {
          return Ok(Some(ev));
        }
        ev.event = None;
        continue;
      }
      if line.starts_with(':') {
        continue;
      }
      let (field, value) = match line.split_once(':') {
        Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
        None => (line, ""),
      };
      match field {
        "data" => {
          if has_data {
            ev.data.push('\n');
          }
          ev.data.push_str(value);
          has_data = true;
        }
        "event" => ev.event = Some(value.to_owned()),
        _ => {}
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn events_comments_and_a_trailing_event() {
    let raw = ": keep-alive\n\nevent: message_start\ndata: {\"a\":1}\n\ndata: line1\r\ndata: line2\r\n\r\ndata: [DONE]";
    let mut r = SseReader::new(raw.as_bytes());
    assert_eq!(r.next_event().unwrap(), Some(SseEvent { event: Some("message_start".into()), data: "{\"a\":1}".into() }));
    assert_eq!(r.next_event().unwrap().unwrap().data, "line1\nline2");
    assert_eq!(r.next_event().unwrap().unwrap().data, "[DONE]");
    assert_eq!(r.next_event().unwrap(), None);
  }
}
