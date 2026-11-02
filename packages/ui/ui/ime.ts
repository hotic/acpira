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

// Applies spacedPinyinCommit to every text field of the page, so the composer, question answers, renames and any
// later field share one fix. The terminal is left alone: xterm reads its own helper textarea's composition.
export function fixSpacedPinyinCommits(doc: Document = document): void {
  // The input method's last shown text, to tell a raw commit of it (spaces and all) from a chosen candidate
  let shown: string | undefined;
  doc.addEventListener('compositionupdate', e => { shown = e.data; }, true);
  doc.addEventListener('compositionend', e => {
    const committed = shown;
    shown = undefined;
    const data = e.data;
    const fixed = spacedPinyinCommit(data, committed);
    const el = e.target;
    if (fixed === undefined || !(el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) || el.closest('.xterm')) return;
    // After the commit settles: WebKit fires compositionend before the input that inserts the text
    setTimeout(() => {
      // selectionStart is null on input types without a caret (number, email), which never take pinyin anyway
      if (el.selectionStart === null || el.selectionStart !== el.selectionEnd) return;
      const end = el.selectionStart;
      const start = end - data.length;
      if (start < 0 || el.value.slice(start, end) !== data) return;
      el.setSelectionRange(start, end);
      // insertText keeps the native undo stack and fires input, so a controlled value follows through onChange
      if (!doc.execCommand('insertText', false, fixed)) el.setSelectionRange(end, end);
    }, 0);
  }, true);
}
