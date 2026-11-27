import { useEffect, useRef, useState, type DragEvent } from 'react';
import type { CategoryOp } from '@acpira/shared/transcript';
import { droppedPathResolver, hostDragPaths } from '../vscodeApi';

// A folder dragged in from the OS onto the column joins the projects (the first one opens); only a drop carries the
// entries, so the frame shows for any file dragged over. No handlers outside the desktop app
export function useFolderDrop(desktop: boolean, categoryOp: ((op: CategoryOp) => void) | undefined, openProject: (cwd: string) => void) {

  const [folderOver, setFolderOver] = useState(false);
  const pathForFile = desktop ? droppedPathResolver() : undefined;
  // Linux: the drag's files exist only as the shell's paths (VsCodeApi.dragPaths)
  const dragPaths = desktop ? hostDragPaths() : undefined;
  // A drag that began on the page (a session or project row, selected text) never carries the OS's files, yet the desktop
  // app's WKWebView lists "Files" in its types: the frame lit for every row drag and took its drop. Only a page drag fires
  // dragstart here; a press clears a flag whose dragend never arrived (a source removed mid-drag)
  const pageDrag = useRef(false);
  useEffect(() => {
    const on = () => { pageDrag.current = true; };
    const off = () => { pageDrag.current = false; };
    addEventListener('dragstart', on, true);
    addEventListener('dragend', off, true);
    addEventListener('pointerdown', off, true);
    return () => { removeEventListener('dragstart', on, true); removeEventListener('dragend', off, true); removeEventListener('pointerdown', off, true); };
  }, []);
  const carriesFiles = (dt: DataTransfer) => !pageDrag.current && (dt.types.includes('Files') || !!dragPaths?.().length);
  const folderDrop = pathForFile && {
    onDragOver: (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = 'copy';
      if (!folderOver) setFolderOver(true);
    },
    onDragLeave: (e: DragEvent) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setFolderOver(false); },
    onDrop: (e: DragEvent) => {
      if (!carriesFiles(e.dataTransfer)) return;
      e.preventDefault();
      setFolderOver(false);
      // The entries are only readable during the drop; their paths may come later (the Windows shell answers async)
      const listed = dragPaths?.() ?? [];
      const asked = listed.length ? listed.filter(d => d.endsWith('/')).map(d => d.slice(0, -1)) : [...e.dataTransfer.items].flatMap(item => {
        const file = item.kind === 'file' && item.webkitGetAsEntry()?.isDirectory ? item.getAsFile() : null;
        return file ? [pathForFile(file)] : [];
      });
      void Promise.all(asked).then(paths => {
        const dirs = paths.filter((d): d is string => !!d);
        for (const dir of dirs) categoryOp?.({ op: 'addProject', cwd: dir });
        if (dirs[0]) openProject(dirs[0]);
      });
    },
  };
  return { folderOver, folderDrop };
}
