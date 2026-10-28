import { execFile } from 'node:child_process';
import { mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';

// An image the webview's clipboard would flatten (Chromium writes PNG only, so a GIF pastes as its first frame) goes on
// the system clipboard here instead: written to a temporary file, then that file, as Finder / Explorer copy one. Chat
// apps and browsers paste a copied file as the animated image. VS Code has no clipboard API for files, so the system's
// own tools do it; Linux has none to rely on and keeps the page's PNG copy

// The folder holds the one file the clipboard points at; each copy replaces the last
const CLIPBOARD_DIR = join(tmpdir(), 'acpira-clipboard');

// macOS: the file URL (what a Finder copy carries) plus the GIF bytes, for apps that read image data rather than files.
// JXA, the path passed as an argument so it is never part of the script's text
const MAC_SCRIPT = `function run(argv) {
  ObjC.import('AppKit');
  const path = argv[0];
  const data = $.NSData.dataWithContentsOfFile(path);
  if (data.isNil()) throw new Error('cannot read ' + path);
  const item = $.NSPasteboardItem.alloc.init;
  item.setStringForType($.NSURL.fileURLWithPath(path).absoluteString, 'public.file-url');
  item.setDataForType(data, 'com.compuserve.gif');
  const board = $.NSPasteboard.generalPasteboard;
  board.clearContents;
  if (!board.writeObjects($([item]))) throw new Error('the pasteboard refused the image');
}`;

// Windows: a file drop list, what Explorer's Copy puts there. The path goes through the environment, since
// powershell -Command appends extra arguments to the command text
const WIN_SCRIPT = 'Set-Clipboard -LiteralPath $env:ACPIRA_CLIPBOARD_FILE';

// Only where the extension host shares the user's desktop: a remote host's clipboard is not the one they paste from
export function imageClipboardSupported(remote: boolean): boolean {
  return !remote && (process.platform === 'darwin' || process.platform === 'win32');
}

export async function copyImageFile(data: Uint8Array, name: string): Promise<void> {
  // Only a plain file name: anything path-like from the page is cut to its last part
  const file = join(CLIPBOARD_DIR, basename(name.replace(/\\/g, '/')) || 'image.gif');
  await rm(CLIPBOARD_DIR, { recursive: true, force: true });
  await mkdir(CLIPBOARD_DIR, { recursive: true });
  await writeFile(file, data);
  if (process.platform === 'darwin') await run('osascript', ['-l', 'JavaScript', '-e', MAC_SCRIPT, file]);
  else if (process.platform === 'win32') await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', WIN_SCRIPT], { ACPIRA_CLIPBOARD_FILE: file });
  else throw new Error(`No image clipboard on ${process.platform}`);
}

function run(command: string, args: string[], env?: Record<string, string>): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile(command, args, { env: { ...process.env, ...env }, windowsHide: true, timeout: 10_000 }, (error, _stdout, stderr) => {
      if (error) reject(new Error(stderr.trim() || error.message));
      else resolve();
    });
  });
}
