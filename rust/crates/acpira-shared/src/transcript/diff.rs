//! Diffs: the whole files behind one, kept in the record and sent to a page as text blobs, and its lines

use super::*;

/// The whole files behind a diff. The record keeps both texts; a page is sent the path only (`omitted`), since an edit
/// carries every byte of the file twice and a long session's view ran to megabytes of them over Remote-SSH. The page
/// renders the diff from its `lines` and asks for the texts when it needs them (`diffSource`)
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSource {
  pub path: String,
  #[serde(default)]
  pub old_text: String,
  #[serde(default)]
  pub new_text: String,
  /// Read back from a page encoding: the texts were left out and are empty here
  #[serde(default)]
  pub omitted: bool,
  /// Big texts live in the session's blob folder instead of the record (`collect_text_blobs`): these name them, and with
  /// either set a missing one is an empty text. The record also says `omitted`, so an older engine reads such a source
  /// as one without texts rather than as two empty files
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub old_blob: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub new_blob: Option<String>,
  /// The blob names the inline texts were stored under, worked out once per source
  #[serde(skip)]
  pub stored: StoredNames,
}

impl DiffSource {
  /// A source with both texts inline
  pub fn whole(path: String, old_text: String, new_text: String) -> Self {
    DiffSource { path, old_text, new_text, ..Default::default() }
  }

  /// Whether the texts are blobs to be read back (`old_blob` / `new_blob`)
  pub fn in_blobs(&self) -> bool {
    self.old_blob.is_some() || self.new_blob.is_some()
  }
}

/// A cache the source's equality ignores: two sources with the same texts are the same diff
#[derive(Debug, Clone, Default)]
pub struct StoredNames(std::sync::Arc<std::sync::OnceLock<(Option<String>, Option<String>)>>);

impl PartialEq for StoredNames {
  fn eq(&self, _: &Self) -> bool {
    true
  }
}

impl Eq for StoredNames {}

/// Where a record write puts diff texts too big to inline (`collect_text_blobs`)
pub struct TextBlobSink {
  /// Below this many bytes (both texts together) a source stays inline: a blob file costs a disk block of its own
  pub min_len: usize,
  /// The content-derived blob name for a text
  pub name: Box<dyn Fn(&str) -> String>,
  /// Whether the session's folder already holds the named blob
  pub known: Box<dyn Fn(&str) -> bool>,
  /// The blobs to write before the record: name and text
  pub out: Vec<(String, String)>,
}

thread_local! {
  static VIEW_ENCODING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
  static TEXT_SINK: std::cell::RefCell<Option<TextBlobSink>> = const { std::cell::RefCell::new(None) };
}

/// Serialize what `f` serializes the way the store writes a record: diff texts of `sink.min_len` or more become blob
/// names, and the texts not yet stored are handed back with the result. Scoped to this thread like `encode_for_view`
pub fn collect_text_blobs<R>(sink: TextBlobSink, f: impl FnOnce() -> R) -> (R, Vec<(String, String)>) {
  // Holds the sink that was there before until it is put back (also when `f` panics)
  struct Restore(Option<Option<TextBlobSink>>);
  impl Drop for Restore {
    fn drop(&mut self) {
      if let Some(prev) = self.0.take() {
        TEXT_SINK.with(|s| *s.borrow_mut() = prev);
      }
    }
  }
  let mut restore = Restore(Some(TEXT_SINK.with(|s| s.borrow_mut().replace(sink))));
  let out = f();
  let prev = restore.0.take().flatten();
  let taken = TEXT_SINK.with(|s| std::mem::replace(&mut *s.borrow_mut(), prev));
  (out, taken.map(|s| s.out).unwrap_or_default())
}

impl DiffSource {
  /// The blob names for this source's inline texts when a record write is collecting them and they are big enough
  fn stash(&self) -> Option<(Option<String>, Option<String>)> {
    TEXT_SINK.with(|s| {
      let mut guard = s.borrow_mut();
      let sink = guard.as_mut()?;
      if self.old_text.len() + self.new_text.len() < sink.min_len {
        return None;
      }
      let names = self.stored.0.get_or_init(|| {
        let name = |t: &str| (!t.is_empty()).then(|| (sink.name)(t));
        (name(&self.old_text), name(&self.new_text))
      });
      for (name, text) in [(&names.0, &self.old_text), (&names.1, &self.new_text)] {
        if let Some(n) = name
          && !(sink.known)(n)
          && !sink.out.iter().any(|(o, _)| o == n)
        {
          sink.out.push((n.clone(), text.clone()));
        }
      }
      Some(names.clone())
    })
  }
}

/// Serialize what `f` serializes the way a page gets it: diff sources without their texts. Scoped to this thread and
/// restored on return (also when `f` panics), so a record written elsewhere is never affected
pub fn encode_for_view<R>(f: impl FnOnce() -> R) -> R {
  struct Restore(bool);
  impl Drop for Restore {
    fn drop(&mut self) {
      VIEW_ENCODING.with(|v| v.set(self.0));
    }
  }
  let _restore = Restore(VIEW_ENCODING.with(|v| v.replace(true)));
  f()
}

impl Serialize for DiffSource {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    // A page gets the path only; a record keeps blob names when the texts live in blobs; otherwise both texts inline
    let blobs = if VIEW_ENCODING.with(|v| v.get()) {
      None
    } else if self.in_blobs() {
      Some((self.old_blob.clone(), self.new_blob.clone()))
    } else if self.omitted {
      None
    } else {
      self.stash()
    };
    let omit = blobs.is_some() || self.omitted || VIEW_ENCODING.with(|v| v.get());
    let mut m = s.serialize_struct("DiffSource", 4)?;
    m.serialize_field("path", &self.path)?;
    if omit {
      m.serialize_field("omitted", &true)?;
      if let Some((old, new)) = blobs {
        if let Some(o) = old {
          m.serialize_field("oldBlob", &o)?;
        }
        if let Some(n) = new {
          m.serialize_field("newBlob", &n)?;
        }
      }
    } else {
      m.serialize_field("oldText", &self.old_text)?;
      m.serialize_field("newText", &self.new_text)?;
    }
    m.end()
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
  Hunk,
  Add,
  Del,
  Ctx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
  pub kind: DiffKind,
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_u64")]
  pub old_line: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_u64")]
  pub new_line: Option<u64>,
}

#[cfg(test)]
mod diff_source_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_page_encoding_leaves_the_file_texts_out_and_a_record_keeps_them() {
    let src = DiffSource::whole("/a.ts".into(), "old".into(), "new".into());
    let diff = ToolContent::Diff { lines: vec![], source: Some(src) };
    assert_eq!(serde_json::to_value(&diff).unwrap()["source"], json!({ "path": "/a.ts", "oldText": "old", "newText": "new" }));
    let page = encode_for_view(|| serde_json::to_value(&diff).unwrap());
    assert_eq!(page["source"], json!({ "path": "/a.ts", "omitted": true }));
    // the scope ends with the call: the next record write is whole again
    assert_eq!(serde_json::to_value(&diff).unwrap()["source"]["newText"], "new");
    // read back, an omitted source stays omitted rather than passing for two empty files
    let back: ToolContent = serde_json::from_value(page).unwrap();
    let ToolContent::Diff { source: Some(s), .. } = back else { panic!("diff expected") };
    assert!(s.omitted && s.old_text.is_empty());
    assert_eq!(serde_json::to_value(&s).unwrap(), json!({ "path": "/a.ts", "omitted": true }));
  }

  fn sink(known: &'static [&'static str]) -> TextBlobSink {
    TextBlobSink { min_len: 8, name: Box::new(|t: &str| format!("{}.zst", t.len())), known: Box::new(move |n: &str| known.contains(&n)), out: vec![] }
  }

  #[test]
  fn a_record_write_moves_big_texts_into_blobs_and_older_readers_see_them_omitted() {
    let big = DiffSource::whole("/a.ts".into(), String::new(), "0123456789".into());
    let small = DiffSource::whole("/b.ts".into(), "a".into(), "b".into());
    let diff = |s: &DiffSource| ToolContent::Diff { lines: vec![], source: Some(s.clone()) };
    let (v, out) = collect_text_blobs(sink(&[]), || serde_json::to_value([diff(&big), diff(&small)]).unwrap());
    // An empty text needs no blob: the other name alone says the texts are blobs
    assert_eq!(v[0]["source"], json!({ "path": "/a.ts", "omitted": true, "newBlob": "10.zst" }));
    assert_eq!(v[1]["source"], json!({ "path": "/b.ts", "oldText": "a", "newText": "b" }));
    assert_eq!(out, vec![("10.zst".to_owned(), "0123456789".to_owned())]);
    // A blob the folder already holds is not handed back again, and outside a record write the texts stay inline
    let (_, again) = collect_text_blobs(sink(&["10.zst"]), || serde_json::to_value(diff(&big)).unwrap());
    assert!(again.is_empty());
    assert_eq!(serde_json::to_value(&big).unwrap()["newText"], "0123456789");
    // Read back by this engine the names survive a rewrite; a page still gets the path only
    let back: DiffSource = serde_json::from_value(v[0]["source"].clone()).unwrap();
    assert!(back.in_blobs() && back.omitted && back.new_text.is_empty());
    assert_eq!(serde_json::to_value(&back).unwrap(), v[0]["source"]);
    assert_eq!(encode_for_view(|| serde_json::to_value(&back).unwrap()), json!({ "path": "/a.ts", "omitted": true }));
    // The cached names are no part of equality
    assert_eq!(big, DiffSource::whole("/a.ts".into(), String::new(), "0123456789".into()));
  }
}
