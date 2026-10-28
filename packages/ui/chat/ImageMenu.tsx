import { useCallback, useContext, useState, type ReactElement } from 'react';
import { Copy, ExternalLink, FolderOpen } from 'lucide-react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { macOS, windowsOS } from '../desktop';
import { t } from '../i18n';
import { ShellLayerContext, popupClass } from '../ui/Overlay';
import { optionClass } from '../ui/DropdownMenu';
import { useCopyAction } from '../ui/useCopied';
import { copyImage } from './copyImage';
import { OpenToolFileContext, ToolFileActionsContext } from './fileLinks';

const contextMenuCollision = { side: 'flip', align: 'flip', fallbackAxisSide: 'none' } as const;
const positionerClass = 'z-40 w-max min-w-pop-sm';
const revealFileLabel = () => t(macOS ? 'session.project.revealFinder' : windowsOS ? 'session.project.revealExplorer' : 'session.project.revealFolder');

// The shell's own right-click menu for an image (Copy image, Open in editor, Reveal in file manager, Copy path),
// replacing the webview's native Cut / Copy / Paste. Shared by transcript images and the Lightbox, so a previewed
// image keeps the same actions.
// The child element itself becomes the trigger (no wrapper), so it keeps its own sizing inside the Lightbox.
// `mimeType` may be omitted when unknown: copyImage then trusts the fetched bytes' own type.
export function ImageMenu({ src, mimeType = '', path, line, openInEditor, children }: {
  src: string;
  mimeType?: string;
  path?: string;
  line?: number;
  openInEditor?: () => void;
  children: ReactElement<Record<string, unknown>>;
}) {
  const layer = useContext(ShellLayerContext);
  const actions = useContext(ToolFileActionsContext);
  const writeImage = useCallback(() => copyImage(src, mimeType, path), [src, mimeType, path]);
  const { state: copyState, copy } = useCopyAction(src, writeImage);
  const open = openInEditor ?? (path && actions?.openInEditor ? () => actions.openInEditor!(path, line) : undefined);
  const reveal = path && actions?.revealInOS ? () => actions.revealInOS!(path) : undefined;
  const copyFilePath = path && actions?.copyPath ? () => actions.copyPath!(path) : undefined;
  return (
    <>
      <ContextMenu.Root>
        <ContextMenu.Trigger render={children} />
        <ContextMenu.Portal container={layer?.current ?? undefined}>
          <ContextMenu.Positioner className={positionerClass} collisionAvoidance={contextMenuCollision} collisionBoundary={layer?.current ?? undefined}>
            {/* Portaled, but React events still bubble up the component tree: a menu click must not reach the
                Lightbox's backdrop handler and close the preview */}
            <ContextMenu.Popup finalFocus={false} className={popupClass} onClick={e => e.stopPropagation()}>
              <ContextMenu.Item onClick={() => void copy()} className={optionClass}>
                <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                {t('image.copy')}
              </ContextMenu.Item>
              {open && (
                <ContextMenu.Item onClick={open} className={optionClass}>
                  <ExternalLink className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                  {t('image.openInEditor')}
                </ContextMenu.Item>
              )}
              {reveal && (
                <ContextMenu.Item onClick={reveal} className={optionClass}>
                  <FolderOpen className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                  {revealFileLabel()}
                </ContextMenu.Item>
              )}
              {copyFilePath && (
                <ContextMenu.Item title={path} onClick={copyFilePath} className={optionClass}>
                  <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                  {t('session.project.copyPath')}
                </ContextMenu.Item>
              )}
            </ContextMenu.Popup>
          </ContextMenu.Positioner>
        </ContextMenu.Portal>
      </ContextMenu.Root>
      {/* Copy feedback is for screen readers only: a visible line under the image would push the transcript down */}
      <span className="sr-only" role="status">{copyState === 'idle' ? '' : t(copyState === 'copied' ? 'code.copied' : 'code.copyFailed')}</span>
    </>
  );
}

// Right-click menu for a file reference in the transcript (Open in editor, Reveal in file manager, Copy path).
// A left-click on an image reference previews it in the Lightbox; right-click always offers the file and path actions.
export function FileMenu({ path, line, openInEditor, children }: {
  path: string;
  line?: number;
  openInEditor?: () => void;
  children: ReactElement<Record<string, unknown>>;
}) {
  const layer = useContext(ShellLayerContext);
  const actions = useContext(ToolFileActionsContext);
  const openFile = useContext(OpenToolFileContext);
  const [menuOpen, setMenuOpen] = useState(false);
  if (!actions && !openInEditor) return children;
  const open = openInEditor ?? (actions?.openInEditor ? () => actions.openInEditor!(path, line) : openFile ? () => openFile(path, line) : undefined);
  const reveal = actions?.revealInOS ? () => actions.revealInOS!(path) : undefined;
  const copyFilePath = actions?.copyPath ? () => actions.copyPath!(path) : undefined;
  if (!open && !reveal && !copyFilePath) return children;
  return (
    <ContextMenu.Root onOpenChange={setMenuOpen}>
      <ContextMenu.Trigger render={children} />
      <ContextMenu.Portal container={layer?.current ?? undefined}>
        <ContextMenu.Positioner className={positionerClass} collisionAvoidance={contextMenuCollision} collisionBoundary={layer?.current ?? undefined}>
          <ContextMenu.Popup finalFocus={false} className={popupClass} onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
            {menuOpen && (
              <>
                {open && (
                  <ContextMenu.Item onClick={open} className={optionClass}>
                    <ExternalLink className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                    {t('image.openInEditor')}
                  </ContextMenu.Item>
                )}
                {reveal && (
                  <ContextMenu.Item onClick={reveal} className={optionClass}>
                    <FolderOpen className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                    {revealFileLabel()}
                  </ContextMenu.Item>
                )}
                {copyFilePath && (
                  <ContextMenu.Item title={path} onClick={copyFilePath} className={optionClass}>
                    <Copy className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                    {t('session.project.copyPath')}
                  </ContextMenu.Item>
                )}
              </>
            )}
          </ContextMenu.Popup>
        </ContextMenu.Positioner>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
