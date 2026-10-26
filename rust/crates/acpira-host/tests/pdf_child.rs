//! The built-in agent's PDF parser in a child process (`acpira pdf-pages`), which shipped builds use because a panic
//! there aborts: the real binary answers a window of pages, a parser error comes back as text, and a child that dies
//! reads as a file the parser failed on

#![allow(clippy::unwrap_used, reason = "test code")]

use std::path::Path;
use std::process::Command;

use acpira_agent::tools::pdf::{CHILD_COMMAND, in_child};
use pdf_extract::{Dictionary, Document, Object, Stream};

const EXE: &str = env!("CARGO_BIN_EXE_acpira");

/// A PDF with one Helvetica text line per page
fn pdf(pages: &[&str]) -> Vec<u8> {
  let mut doc = Document::with_version("1.5");
  let pages_id = doc.new_object_id();
  let mut font = Dictionary::new();
  font.set("Type", Object::Name(b"Font".to_vec()));
  font.set("Subtype", Object::Name(b"Type1".to_vec()));
  font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
  font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
  let font_id = doc.add_object(font);
  let mut fonts = Dictionary::new();
  fonts.set("F1", Object::Reference(font_id));
  let mut resources = Dictionary::new();
  resources.set("Font", Object::Dictionary(fonts));
  let resources_id = doc.add_object(resources);
  let mut kids = vec![];
  for text in pages {
    let content_id = doc.add_object(Stream::new(Dictionary::new(), format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET").into_bytes()));
    let mut page = Dictionary::new();
    page.set("Type", Object::Name(b"Page".to_vec()));
    page.set("Parent", Object::Reference(pages_id));
    page.set("Contents", Object::Reference(content_id));
    page.set("Resources", Object::Reference(resources_id));
    page.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
    kids.push(Object::Reference(doc.add_object(page)));
  }
  let mut tree = Dictionary::new();
  tree.set("Type", Object::Name(b"Pages".to_vec()));
  tree.set("Count", Object::Integer(kids.len() as i64));
  tree.set("Kids", Object::Array(kids));
  doc.objects.insert(pages_id, Object::Dictionary(tree));
  let mut catalog = Dictionary::new();
  catalog.set("Type", Object::Name(b"Catalog".to_vec()));
  catalog.set("Pages", Object::Reference(pages_id));
  let catalog_id = doc.add_object(catalog);
  doc.trailer.set("Root", Object::Reference(catalog_id));
  let mut out = vec![];
  doc.save_to(&mut out).unwrap();
  out
}

#[test]
fn the_child_reads_a_window_of_pages() {
  let pages = in_child(Path::new(EXE), &pdf(&["First page here", "Second page here"]), 2, 5, 50_000).unwrap();
  assert_eq!(pages.total, 2);
  assert_eq!(pages.rows.len(), 1);
  assert!(pages.rows[0].starts_with("--- page 2 ---") && pages.rows[0].contains("Second page here"), "{:?}", pages.rows);
}

#[test]
fn a_parser_error_comes_back_as_text() {
  let err = in_child(Path::new(EXE), b"%PDF-1.5 not really a pdf", 1, 5, 50_000).unwrap_err();
  assert!(!err.contains("failed on this file"), "{err}");
}

#[cfg(unix)]
#[test]
fn a_child_that_dies_reads_as_a_failed_parse() {
  // `false` takes any arguments, reads nothing and exits 1, as a child ended by a panic would
  let err = in_child(Path::new("/usr/bin/false"), &pdf(&["x"]), 1, 5, 50_000).unwrap_err();
  assert_eq!(err, "the PDF parser failed on this file");
}

#[test]
fn the_child_refuses_bad_arguments() {
  let out = Command::new(EXE).args([CHILD_COMMAND, "0", "5", "100"]).output().unwrap();
  assert_eq!(out.status.code(), Some(2));
}
