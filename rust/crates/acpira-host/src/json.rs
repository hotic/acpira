//! Readers for loosely typed peer JSON: the TS code checks `typeof x === 'string'` before trusting a field, so do these

use serde_json::{Map, Value};

pub fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
  v.get(key).filter(|x| !x.is_null())
}

pub fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
  v.get(key).and_then(Value::as_str)
}

/// A string field that is present and non-empty
pub fn text_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
  str_of(v, key).filter(|s| !s.is_empty())
}

pub fn obj<'a>(v: &'a Value, key: &str) -> Option<&'a Map<String, Value>> {
  v.get(key).and_then(Value::as_object)
}

pub fn arr<'a>(v: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
  v.get(key).and_then(Value::as_array)
}

/// JS truthiness
pub fn truthy(v: Option<&Value>) -> bool {
  match v {
    None | Some(Value::Null) => false,
    Some(Value::Bool(b)) => *b,
    Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
    Some(Value::String(s)) => !s.is_empty(),
    Some(_) => true,
  }
}

/// JS `s.slice(0, n)` counted in UTF-16 units, never splitting a code point
pub fn slice16(s: &str, n: usize) -> String {
  let mut units = 0;
  let mut end = 0;
  for (i, c) in s.char_indices() {
    units += c.len_utf16();
    if units > n {
      break;
    }
    end = i + c.len_utf8();
  }
  if end >= s.len() { s.to_owned() } else { s[..end].to_owned() }
}

/// UTF-16 length, the unit JS lengths are counted in
pub fn len16(s: &str) -> usize {
  s.chars().map(char::len_utf16).sum()
}

/// JSON.stringify(v, null, 2)
pub fn pretty(v: &Value) -> String {
  serde_json::to_string_pretty(v).unwrap_or_default()
}

/// node path.basename for either separator style the peers send
pub fn basename(p: &str) -> String {
  let trimmed = p.trim_end_matches(['/', '\\']);
  let seps: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };
  trimmed.rsplit(seps).next().unwrap_or(trimmed).to_owned()
}
