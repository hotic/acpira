import { BASE_APPEARANCE, type Appearance } from '@shared/appearance';

// The UI face is known from init (appearance), and the browser requests a web font only once text first needs it: an
// opened transcript's first frame drew in the fallback face, then laid every line out again when Inter arrived (~70 ms
// through VS Code's webview resource loader, after the page had painted). The face the host is configured with (the
// default when a host does not say) is requested as the bundle starts, well before init comes
const FAMILIES: Partial<Record<Appearance['font'], string>> = { inter: 'Inter Variable', geist: 'Geist Variable' };

const family = FAMILIES[window.__acpira?.font ?? BASE_APPEARANCE.font];
// Without a text argument this loads the latin faces (the ones holding a space); other subsets load when their text shows
if (family) document.fonts?.load(`1em "${family}"`).catch(() => { /* the face then loads on first use */ });
