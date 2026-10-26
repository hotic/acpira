// True while a key belongs to an input method (IME) composition, including the Enter that commits it.
// Chromium (VS Code, Electron) reports that Enter with `isComposing`; WebKit (the desktop app's WKWebView, Safari)
// fires `compositionend` before its keydown, so `isComposing` is already false there and only keyCode 229 tells.
export function imeKey(e: KeyboardEvent): boolean {
  return e.isComposing || e.keyCode === 229;
}
