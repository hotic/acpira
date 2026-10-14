// open_browser's reachability check (`probeUrl` from the VS Code shell). The page renders on the user's machine even when the
// extension host is remote, so a fetch from here travels the same way the browser tab will: through the forwarded port.
// no-cors leaves the response opaque, which is enough: any HTTP answer resolves, a refused or reset connection rejects.
// A tunnel that was set up a moment ago gets a few tries before it counts as down
const TRIES = 3;
const EACH_MS = 2500;
const BETWEEN_MS = 600;

export async function probeUrl(url: string): Promise<{ ok: boolean; error?: string }> {
  let error = 'no response';
  for (let i = 0; i < TRIES; i++) {
    if (i) await new Promise(r => setTimeout(r, BETWEEN_MS));
    try {
      await fetch(url, { mode: 'no-cors', cache: 'no-store', credentials: 'omit', signal: AbortSignal.timeout(EACH_MS) });
      return { ok: true };
    } catch (e) {
      error = e instanceof DOMException && e.name === 'TimeoutError' ? `no response within ${EACH_MS / 1000}s` : String(e instanceof Error ? e.message : e);
    }
  }
  return { ok: false, error };
}
