import { useMemo } from 'react';
import { imageMimeOf } from '@acpira/shared/attachments';
import type { FileStager } from '../markdown/fileLinks';

// What `fileStager` last read, by session and path (null: unreadable), kept for the page's life: a session switched
// back to, or a fold opened again, shows it at once while the host checks the file again. Reads under way are shared
const stagedFiles = new Map<string, string | null>();
const stagingFiles = new Map<string, Promise<string | undefined>>();
// Resolves once the image is decoded (or failed to), so the reserved frame gives way to pixels, not to an empty box
const decodeImage = (src: string) => { const img = new Image(); img.src = src; return img.decode().catch(() => {}); };

// Files rows show from their paths. Each mount asks the host again (a stat while the file is unchanged), so a file
// rewritten and shown again is never the old one; an image is decoded before it replaces its placeholder
export function useFileStager(
  sid: string | undefined,
  imageBlob: ((sessionId: string, path: string, shown?: boolean) => Promise<string | undefined>) | undefined,
  blobUrl: ((blob: string) => string) | undefined,
): FileStager | undefined {
  return useMemo<FileStager | undefined>(() => {
    if (!sid || !imageBlob) return undefined;
    return {
      known: path => stagedFiles.get(`${sid}\0${path}`),
      load: path => {
        const key = `${sid}\0${path}`;
        let pending = stagingFiles.get(key);
        if (!pending) {
          pending = imageBlob(sid, path, true)
            .then(async blob => {
              if (blob && blobUrl && imageMimeOf(path)) await decodeImage(blobUrl(blob));
              return blob;
            })
            .then(blob => {
              stagingFiles.delete(key);
              stagedFiles.set(key, blob ?? null);
              return blob;
            });
          stagingFiles.set(key, pending);
        }
        return pending;
      },
    };
  }, [sid, imageBlob, blobUrl]);
}
