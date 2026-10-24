import { spawn } from 'node:child_process';
import { accessSync, constants, statSync } from 'node:fs';
import { join } from 'node:path';
import type { FileOpener } from '@acpira/shared/settings';

// "Open files with" an editor (acpira.openFilesWith): its command line opens a file link or a plan at the line. The
// command comes from PATH as the login shell sets it (an app started from the Dock has almost none of its own), then
// from the app bundle where macOS installs put it, since "Install 'code' command in PATH" is a step many skip

export type Editor = Exclude<FileOpener, 'system'>;

export interface FileTarget {
  path: string;
  line?: number;
}

// The arguments that open a file at a line: `-g path:line` for VS Code and its forks, `path:line` for Zed and Sublime
// Text, `--line N path` for IntelliJ IDEA
export function editorArgs(editor: Editor, t: FileTarget): string[] {
  const at = t.line ? `${t.path}:${t.line}` : t.path;
  switch (editor) {
    case 'code':
    case 'cursor':
    case 'windsurf':
      return ['-g', at];
    case 'zed':
    case 'subl':
      return [at];
    case 'idea':
      return t.line ? ['--line', String(t.line), t.path] : [t.path];
  }
}

// The command line inside each editor's macOS bundle, under /Applications or ~/Applications (JetBrains Toolbox)
const MAC_BUNDLES: Record<Editor, string[]> = {
  code: ['Visual Studio Code.app/Contents/Resources/app/bin/code'],
  cursor: ['Cursor.app/Contents/Resources/app/bin/cursor'],
  windsurf: ['Windsurf.app/Contents/Resources/app/bin/windsurf'],
  zed: ['Zed.app/Contents/MacOS/cli'],
  subl: ['Sublime Text.app/Contents/SharedSupport/bin/subl'],
  idea: ['IntelliJ IDEA.app/Contents/MacOS/idea', 'IntelliJ IDEA Ultimate.app/Contents/MacOS/idea', 'IntelliJ IDEA CE.app/Contents/MacOS/idea'],
};

function isExecutable(path: string): boolean {
  try {
    if (!statSync(path).isFile()) return false;
    accessSync(path, constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

// Where the editor's command line is: PATH first, then the macOS bundles. Windows takes `.exe` files only: the `.cmd`
// launchers VS Code and its forks put on PATH need a shell, which a file path must not pass through
export function findEditor(editor: Editor, env: NodeJS.ProcessEnv, platform: NodeJS.Platform, home: string, exists = isExecutable): string | undefined {
  const names = platform === 'win32' ? [`${editor}.exe`] : [editor];
  for (const dir of (env.PATH ?? '').split(platform === 'win32' ? ';' : ':').filter(Boolean)) {
    for (const name of names) {
      const path = join(dir, name);
      if (exists(path)) return path;
    }
  }
  if (platform !== 'darwin') return undefined;
  for (const root of ['/Applications', join(home, 'Applications')]) {
    for (const rel of MAC_BUNDLES[editor]) {
      const path = join(root, rel);
      if (exists(path)) return path;
    }
  }
  return undefined;
}

// Starts the editor on the file, detached. False when its command line is nowhere to be found or does not start, so the
// caller can fall back to the system's default application
export function openInEditor(editor: Editor, target: FileTarget, env: NodeJS.ProcessEnv, home: string, log: (line: string) => void): Promise<boolean> {
  const bin = findEditor(editor, env, process.platform, home);
  if (!bin) return Promise.resolve(false);
  return new Promise(resolve => {
    const child = spawn(bin, editorArgs(editor, target), { env, detached: true, stdio: 'ignore' });
    child.once('spawn', () => resolve(true));
    child.once('error', e => { log(`editor ${bin} failed: ${String(e)}`); resolve(false); });
    child.unref();
  });
}
