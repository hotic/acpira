// True while a key belongs to an input method (IME) composition, including the Enter that commits it.
// Chromium (VS Code, Electron) reports that Enter with `isComposing`; WebKit (the desktop app's WKWebView, Safari)
// fires `compositionend` before its keydown, so `isComposing` is already false there and only keyCode 229 tells.
export function imeKey(e: KeyboardEvent): boolean {
  return e.isComposing || e.keyCode === 229;
}

// A pinyin composition shows its syllables split by spaces ("de v in p ro") while the keys typed were "devinpro"; a typed
// space would have picked a candidate, so those spaces are never the user's. Committing raw letters (Shift to switch to
// English with the macOS input method) should insert the keys, but the shown text with its spaces sometimes lands instead.
// Answers the text to put in place of such a commit, or undefined when the commit is anything else: a chosen candidate
// differs from the shown text, and a correct raw commit has no spaces.
export function spacedPinyinCommit(committed: string, shown: string | undefined): string | undefined {
  if (!committed || committed !== shown || !/^[A-Za-z]+(?:[ '][A-Za-z]+)+$/.test(committed) || !committed.includes(' ')) return undefined;
  return committed.replaceAll(' ', '');
}
