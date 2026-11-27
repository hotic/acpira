//! The page's side of theme and style plugins: the style sheets every page gets and the skin's settings

use std::path::Path;

use serde_json::Value;

use super::Loaded;
use crate::plugins::manifest;

/// The most of one plugin's theme style sheet a page gets
const THEME_CSS_MAX_BYTES: usize = manifest::CSS_MAX_BYTES as usize;

/// A file of a plugin's frozen copy as text, none when missing or over `max` bytes
pub(super) fn theme_css_of(loaded: &Loaded) -> String {
  let mut out = String::new();
  if let Some(a) = loaded.skin()
    && let Some(theme) = &a.manifest.contributes.theme
  {
    out.push_str(&format!("/* Acpira plugin {} */\n", a.id));
    // Checked when it was enabled; checked again here, since this text goes into every page
    let tokens: Vec<String> = theme
      .tokens
      .iter()
      .filter(|(k, _)| manifest::THEME_TOKENS.contains(&k.as_str()))
      .filter_map(|(k, v)| v.as_str().filter(|v| manifest::safe_css_value(v)).map(|v| format!("  {k}: {v};")))
      .collect();
    if !tokens.is_empty() {
      out.push_str(&format!(":root, .acp-shell, [data-theme], .acp-shell.acp-shell[data-skin] {{\n{}\n}}\n", tokens.join("\n")));
    }
    // A skin that names its own colour lends it to the page's landing marks (--land-ink, skin.css); without, they stay
    // the foreground's, since the default --brand is Acpira's own, not the skin's
    if theme.tokens.get("--brand").and_then(Value::as_str).is_some_and(manifest::safe_css_value) {
      out.push_str(":root, .acp-shell, [data-theme], .acp-shell.acp-shell[data-skin] { --skin-ink: var(--brand); }\n");
    }
    if let Some(css) = theme.css.as_deref() {
      push_css(&mut out, &a.dir, css);
    }
  }
  // The styles stack over the skin in reverse load order, so plugins earlier in the user's tile order win the CSS cascade
  for a in loaded.active.iter().rev() {
    if let Some(css) = a.manifest.contributes.style.as_deref() {
      out.push_str(&format!("/* Acpira plugin {} style */\n", a.id));
      push_css(&mut out, &a.dir, css);
    }
  }
  out
}

/// The `STYLE_HOOKS` a stylesheet names, in the order they first appear, so the card's mark can land on the parts it
/// restyles. Comments are skipped, and quotes and spaces inside a selector do not count (`[data-part=user-message]`
/// names the same hook)
pub(super) fn restyled_hooks(css: &str) -> Vec<String> {
  let mut text = String::with_capacity(css.len());
  let mut rest = css;
  while let Some((before, after)) = rest.split_once("/*") {
    text.push_str(before);
    rest = after.split_once("*/").map_or("", |(_, tail)| tail);
  }
  text.push_str(rest);
  let bare = |s: &str| s.chars().filter(|c| !matches!(c, '"' | '\'' | ' ' | '\t' | '\n' | '\r')).collect::<String>();
  let text = bare(&text);
  let mut found: Vec<(usize, &str)> = acpira_shared::plugins::STYLE_HOOKS.iter().filter_map(|h| text.find(&bare(h)).map(|at| (at, *h))).collect();
  found.sort();
  found.into_iter().map(|(_, h)| h.to_owned()).collect()
}

/// A plugin's stylesheet, cut at `THEME_CSS_MAX_BYTES`
fn push_css(out: &mut String, dir: &Path, rel: &str) {
  let Some(css) = manifest::inside(dir, rel).and_then(|p| std::fs::read_to_string(p).ok()) else { return };
  let mut end = css.len().min(THEME_CSS_MAX_BYTES);
  while !css.is_char_boundary(end) {
    end -= 1;
  }
  out.push_str(css.get(..end).unwrap_or_default());
  // Closes a comment the sheet (or the cut) left open, which would swallow the next plugin's sheet
  out.push_str("\n/* */\n");
}

pub(super) fn skin_of(loaded: &Loaded) -> Option<acpira_shared::plugins::PluginSkin> {
  let a = loaded.skin()?;
  let theme = a.manifest.contributes.theme.as_ref()?;
  Some(acpira_shared::plugins::PluginSkin {
    plugin: a.id.clone(),
    name: a.manifest.display_name().to_owned(),
    snapshot: a.hash.clone(),
    base: theme.base.clone().filter(|b| manifest::THEME_BASES.contains(&b.as_str())),
    glass: theme.glass.clone().filter(|g| manifest::GLASS.contains(&g.as_str())),
    brand: theme.tokens.get("--brand").and_then(Value::as_str).filter(|v| manifest::safe_css_value(v)).map(str::to_owned),
    backdrop: backdrop_doc(&a.dir, theme),
  })

}

/// A theme's backdrop document: its own, or its scene inside the running Acpira's renderer
pub(super) fn backdrop_doc(dir: &std::path::Path, theme: &manifest::Theme) -> Option<String> {
  match (&theme.scene, &theme.backdrop) {
    (Some(scene), _) => read_capped(dir, scene, manifest::SCENE_MAX_BYTES).map(|s| crate::plugins::sdk::backdrop_html(&s)),
    (None, Some(b)) => read_capped(dir, b, manifest::BACKDROP_MAX_BYTES),
    (None, None) => None,
  }
}

pub(super) fn read_capped(dir: &std::path::Path, rel: &str, max: u64) -> Option<String> {
  let path = manifest::inside(dir, rel)?;
  if std::fs::metadata(&path).ok()?.len() > max {
    return None;
  }
  std::fs::read_to_string(path).ok()
}
