import { realpathSync } from 'node:fs';
import { resolve, sep } from 'node:path';

// A file under `root` named by a URL path (already split off its route prefix), or undefined when the name is not valid,
// does not exist, or leaves the root once `..`, percent-escapes and symlinks are resolved. Every file the app protocol
// serves (the webview bundle, attachment blobs) goes through here
export function confine(root: string, urlPath: string): string | undefined {
  let name: string;
  try { name = decodeURIComponent(urlPath); } catch { return undefined; }
  if (!name || name.includes('\0')) return undefined;
  let base: string, real: string;
  try {
    base = realpathSync(root);
    real = realpathSync(resolve(base, `.${sep}${name}`));
  } catch {
    return undefined;
  }
  return real.startsWith(base + sep) ? real : undefined;
}
