import { useContext, useEffect, useState } from 'react';
import { FileStagerContext } from './fileLinks';

// A source the host can read as a file: a file: URL, an absolute path (POSIX, drive letter, UNC) or one relative to the
// session folder. Web, data: and blob: URLs are the renderer's
const isFileSource = (uri: string) => /^(?:file:|[a-z]:[\\/])/i.test(uri) || !/^[a-z][a-z0-9+.-]*:|^#|^\/\//i.test(uri);

// The blob a row's image, video or widget shows: its saved copy, else the file its `uri` names, read by the host when
// it mounts. What this page read before shows at once while the host checks the file again (a stat while unchanged),
// so a file rewritten and shown again under the same path never keeps the old copy.
// undefined while the first read runs, null when there is nothing to show
export function useShownBlob(blob: string | undefined, uri: string | undefined): string | null | undefined {
  const stager = useContext(FileStagerContext);
  const path = !blob && uri && isFileSource(uri) ? uri : undefined;
  const [loaded, setLoaded] = useState<{ path: string; blob: string | null }>();
  useEffect(() => {
    if (!path || !stager) return;
    let alive = true;
    void stager.load(path).then(b => { if (alive) setLoaded({ path, blob: b ?? null }); });
    return () => { alive = false; };
  }, [path, stager]);
  if (blob) return blob;
  if (!path || !stager) return null;
  return loaded?.path === path ? loaded.blob : stager.known(path);
}
