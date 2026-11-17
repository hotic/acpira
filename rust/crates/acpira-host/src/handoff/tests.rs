use super::*;

fn sh(dir: &Path, args: &[&str]) {
  let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().map(|o| o.status.success());
  assert_eq!(ok.ok(), Some(true), "git {args:?}");
}

fn repo() -> Option<tempfile::TempDir> {
  std::process::Command::new("git").arg("--version").output().ok()?;
  let dir = tempfile::tempdir().unwrap();
  sh(dir.path(), &["init", "-q"]);
  sh(dir.path(), &["config", "user.email", "t@t"]);
  sh(dir.path(), &["config", "user.name", "t"]);
  sh(dir.path(), &["config", "commit.gpgsign", "false"]);
  Some(dir)
}

fn write(dir: &Path, path: &str, body: &str) {
  let p = dir.join(path);
  std::fs::create_dir_all(p.parent().unwrap()).unwrap();
  std::fs::write(p, body).unwrap();
}

fn commit(dir: &Path) {
  sh(dir, &["add", "-A"]);
  sh(dir, &["commit", "-q", "-m", "base"]);
}

const LIB: &str = "use std::fmt;\n\npub fn alpha(a: u32) -> u32 {\n    let b = a + 1;\n    let c = b * 2;\n    let d = c * 3;\n    let e = d * 4;\n    let f = e * 5;\n    let g = f * 6;\n    g\n}\n\npub fn beta() -> u32 {\n    alpha(1)\n}\n";

#[tokio::test]
async fn a_hunk_widens_to_its_function_and_a_small_file_comes_in_whole() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  write(root, "src/lib.rs", LIB);
  write(root, "src/user.rs", "fn caller() -> u32 { crate::alpha(2) }\n");
  commit(root);
  write(root, "src/lib.rs", &LIB.replace("let g = f * 6;", "let g = f * 7;"));
  let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
  assert_eq!(pack.level, Level::Full);
  assert_eq!(pack.files.len(), 1);
  assert_eq!((pack.files[0].added, pack.files[0].removed), (Some(1), Some(1)));
  // -W: the whole of `alpha`, its first line included, though the change is seven lines below it
  let diff = pack.text.split("## Diff").nth(1).unwrap();
  assert!(diff.contains("\n pub fn alpha(a: u32) -> u32 {\n") && diff.contains("+    let g = f * 7;"), "{}", pack.text);
  assert!(pack.text.contains("### src/lib.rs (15 lines)"), "{}", pack.text);
  // Off by default: references and read paths
  assert!(!pack.text.contains("Where changed definitions are used"));
  // A plain recipe keeps three lines of context, and no full text
  let plain = Recipe { function_context: false, full_text_lines: 0, ..Recipe::default() };
  let pack = build(root, None, None, &plain).await.unwrap().unwrap();
  assert!(!pack.text.contains("\n pub fn alpha(a: u32) -> u32 {\n") && !pack.text.contains("Full text"), "{}", pack.text);
}

#[tokio::test]
async fn new_renamed_deleted_and_binary_files_are_listed_and_a_lockfile_only_counted() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  let long: String = (0..60).map(|i| format!("line {i}\n")).collect();
  write(root, "old_name.txt", &long);
  write(root, "gone.txt", "bye\n");
  write(root, "Cargo.lock", "a\n");
  std::fs::write(root.join("logo.bin"), [0u8, 1, 2, 3]).unwrap();
  commit(root);
  sh(root, &["mv", "old_name.txt", "new_name.txt"]);
  std::fs::remove_file(root.join("gone.txt")).unwrap();
  write(root, "fresh.py", "def hello():\n    return 1\n");
  write(root, "Cargo.lock", "a\nb\n");
  std::fs::write(root.join("logo.bin"), [0u8, 9, 9]).unwrap();
  let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
  let by: BTreeMap<&str, &PackFile> = pack.files.iter().map(|f| (f.path.as_str(), f)).collect();
  assert_eq!(by["new_name.txt"].status, "R");
  assert_eq!(by["new_name.txt"].old_path.as_deref(), Some("old_name.txt"));
  assert_eq!(by["gone.txt"].status, "D");
  assert_eq!((by["fresh.py"].status.as_str(), by["fresh.py"].added), ("A", Some(2)));
  assert_eq!(by["logo.bin"].added, None, "binary");
  assert!(pack.text.contains("old_name.txt -> new_name.txt"));
  assert!(pack.text.contains("Cargo.lock  +1 -0  (diff left out)"), "{}", pack.text);
  assert!(!pack.text.contains("+b\n"), "the lockfile's diff stays out");
  assert!(pack.text.contains("+def hello():"), "an untracked file shows as new: {}", pack.text);
  assert!(pack.text.contains("Binary files"), "{}", pack.text);
}

#[tokio::test]
async fn references_are_found_capped_and_definitions_left_out() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  write(root, "src/lib.rs", LIB);
  let users: String = (0..30).map(|i| format!("fn use{i}() -> u32 {{ crate::alpha({i}) }}\n")).collect();
  write(root, "src/user.rs", &users);
  commit(root);
  write(root, "src/lib.rs", &LIB.replace("pub fn alpha(a: u32) -> u32 {", "pub fn alpha(a: u32, b: u32) -> u32 {"));
  let recipe = Recipe { refs: true, refs_max: 5, ..Recipe::default() };
  let pack = build(root, None, None, &recipe).await.unwrap().unwrap();
  let refs = pack.text.split("## Where changed definitions are used").nth(1).unwrap();
  assert!(refs.contains("`alpha`:"), "{refs}");
  // The use in the same file comes first (git grep goes by path), then four of the thirty in user.rs
  assert_eq!(refs.matches("src/user.rs:").count(), 4, "capped at refs_max: {refs}");
  assert!(refs.contains("and 26 more"), "the rest counted: {refs}");
  assert!(!refs.contains("src/lib.rs:3:pub fn alpha"), "its own definition is not a use: {refs}");
  assert!(refs.contains("src/lib.rs:14:    alpha(1)"), "a use in the same file is: {refs}");
}

#[test]
fn definitions_are_read_off_changed_lines_in_several_languages() {
  let diff = "+pub(crate) async fn load_all(x: u32) {\n-def parse_header(line):\n+export class SessionView {\n+  async refresh(force: boolean): Promise<void> {\n+    const local = 1;\n+export const LIMIT = 5;\n+func (s *Server) Handle(w http.ResponseWriter) {\n+    if (ready()) {\n+fn ab() {}\n context fn not_changed() {}\n";
  let names: Vec<String> = changed_definitions(diff).into_iter().map(|d| d.name).collect();
  assert_eq!(names, ["load_all", "parse_header", "SessionView", "refresh", "LIMIT", "Handle"]);
  // Removed, changed in place, added; and the names every codebase defines are skipped
  let defs = changed_definitions("-fn gone() {}\n-fn kept(a: u8) {}\n+fn kept(a: u16) {}\n+fn fresh() {}\n+    fn default() -> Self {\n");
  assert_eq!(defs.iter().map(|d| (d.name.as_str(), d.removed, d.added)).collect::<Vec<_>>(), [("gone", true, false), ("kept", true, true), ("fresh", false, true)]);
  let rank = |i: usize, new_file| definition_rank(&defs[i], new_file);
  assert!(rank(0, false) < rank(1, false) && rank(1, false) < rank(2, false) && rank(2, false) < rank(2, true));
}

#[tokio::test]
async fn over_the_cap_full_texts_go_first_then_functions_then_all_but_the_stat() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  // One long function edited at its end, a small file, and a file whose every line changes
  let body: String = (0..300).map(|i| format!("    let v{i} = {i};\n")).collect();
  let src = format!("pub fn big() {{\n{body}}}\n");
  write(root, "big.rs", &src);
  write(root, "small.rs", "pub fn small() -> u8 {\n    1\n}\n");
  write(root, "huge.txt", &(0..150).map(|i| format!("x{i}\n")).collect::<String>());
  commit(root);
  write(root, "big.rs", &src.replace("let v299 = 299;", "let v299 = 300;"));
  write(root, "small.rs", "pub fn small() -> u8 {\n    2\n}\n");
  write(root, "huge.txt", &(0..150).map(|i| format!("y{i}\n")).collect::<String>());
  let full = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
  assert_eq!(full.level, Level::Full);
  assert!(full.text.contains("### big.rs (302 lines)"));
  let cap = |max_bytes| Recipe { max_bytes, ..Recipe::default() };
  // Just under the full size: the largest full text goes, the others stay
  let trimmed = build(root, None, None, &cap(full.bytes - 200)).await.unwrap().unwrap();
  assert_eq!(trimmed.level, Level::Trimmed, "{}", trimmed.text);
  assert!(trimmed.text.contains("### small.rs") && trimmed.text.contains("### huge.txt") && !trimmed.text.contains("### big.rs"));
  // The whole 300-line function does not fit: plain hunks, no full texts
  let plain = build(root, None, None, &cap(4_000)).await.unwrap().unwrap();
  assert_eq!(plain.level, Level::Plain, "{}", plain.text);
  assert!(plain.text.contains("+    let v299 = 300;") && !plain.text.contains("    let v10 = 10;") && !plain.text.contains("### small.rs"));
  assert!(plain.bytes <= 4_000);
  // At the floor: every path with its counts, the diffs of the two small changes, not the 150-line one
  let stat = build(root, None, None, &cap(2_000)).await.unwrap().unwrap();
  assert_eq!(stat.level, Level::Stat, "{}", stat.text);
  assert!(stat.bytes <= 2_000, "{}", stat.bytes);
  assert!(stat.text.contains("M huge.txt  +150 -150") && stat.text.contains("M big.rs  +1 -1"), "{}", stat.text);
  assert!(stat.text.contains("+    2\n") && stat.text.contains("+    let v299 = 300;") && !stat.text.contains("+y1\n"), "{}", stat.text);
}

#[tokio::test]
async fn a_scope_keeps_the_named_files_unless_the_session_could_write_anywhere() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  write(root, "a.txt", "a\n");
  write(root, "b.txt", "b\n");
  commit(root);
  write(root, "a.txt", "a2\n");
  write(root, "b.txt", "b2\n");
  let named = Scope { files: vec![root.join("a.txt").to_string_lossy().into_owned(), "nowhere/else.txt".into()], ..Default::default() };
  let pack = build(root, None, Some(&named), &Recipe::default()).await.unwrap().unwrap();
  assert_eq!(pack.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
  let anywhere = Scope { anywhere: true, ..named.clone() };
  let pack = build(root, None, Some(&anywhere), &Recipe::default()).await.unwrap().unwrap();
  assert_eq!(pack.files.len(), 2);
  // A named file with no change left: nothing to hand off
  let clean = Scope { files: vec!["nowhere/else.txt".into()], ..Default::default() };
  assert!(build(root, None, Some(&clean), &Recipe::default()).await.unwrap().is_none());
  // Read paths only when the recipe asks
  let read = Scope { read: vec!["docs/spec.md".into(), root.join("b.txt").to_string_lossy().into_owned()], ..named };
  let pack = build(root, None, Some(&read), &Recipe { read_paths: true, ..Recipe::default() }).await.unwrap().unwrap();
  assert!(pack.text.contains("## Files the author read\n\n- docs/spec.md\n- b.txt\n"), "{}", pack.text);
}

#[tokio::test]
async fn a_base_can_be_named_and_a_repository_without_commits_diffs_against_nothing() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  write(root, "x.txt", "one\n");
  let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
  assert_eq!(pack.base, EMPTY_TREE);
  commit(root);
  write(root, "x.txt", "two\n");
  commit(root);
  // Clean against HEAD, one change against HEAD~1
  assert!(build(root, None, None, &Recipe::default()).await.unwrap().is_none());
  let pack = build(root, Some("HEAD~1"), None, &Recipe::default()).await.unwrap().unwrap();
  assert!(pack.text.contains("-one\n+two"), "{}", pack.text);
  assert!(build(root, Some("no-such-rev"), None, &Recipe::default()).await.is_err());
  // Not a work tree at all
  let plain = tempfile::tempdir().unwrap();
  assert!(build(plain.path(), None, None, &Recipe::default()).await.unwrap().is_none());
}

#[test]
fn a_recipe_file_names_only_what_it_changes_and_is_clamped() {
  let r = Recipe::from_json(r#"{ "refs": true, "maxBytes": 10, "unknown": 1 }"#).unwrap();
  assert!(r.refs && r.function_context);
  assert_eq!(r.max_bytes, 2_000);
  assert!(Recipe::from_json("not json").is_err());
  // The data root's file, or the default when it is missing or broken
  let dir = tempfile::tempdir().unwrap();
  assert_eq!(recipe_in(dir.path()), Recipe::default());
  std::fs::write(dir.path().join(RECIPE_FILE), r#"{ "refs": true }"#).unwrap();
  assert!(recipe_in(dir.path()).refs);
  std::fs::write(dir.path().join(RECIPE_FILE), "{").unwrap();
  assert_eq!(recipe_in(dir.path()), Recipe::default());
}

#[tokio::test]
async fn a_later_listing_against_the_packs_base_names_only_what_moved() {
  let Some(dir) = repo() else { return };
  let root = dir.path();
  write(root, "a.txt", "one\n");
  write(root, "b.txt", "one\n");
  commit(root);
  write(root, "a.txt", "two\n");
  write(root, "b.txt", "two\n");
  let pack = build(root, None, None, &Recipe::default()).await.unwrap().unwrap();
  // A commit in between does not move the base: the listing stays against the pack's own
  sh(root, &["commit", "-q", "-am", "wip"]);
  write(root, "a.txt", "three\nfour\n");
  write(root, "b.txt", "one\n");
  write(root, "c.txt", "new\n");
  let now = file_stats(root, Some(&pack.base), None).await.unwrap().unwrap();
  assert_eq!(changed_since(&pack.files, &now), ["a.txt (+2 -1 now)", "c.txt (newly changed, +1 -0)", "b.txt (no longer changed)"]);
  assert!(changed_since(&now, &now).is_empty());
  // A scope keeps to its files
  let scope = Scope { files: vec!["a.txt".into()], ..Default::default() };
  let only = file_stats(root, Some(&pack.base), Some(&scope)).await.unwrap().unwrap();
  assert_eq!(only.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["a.txt"]);
}
