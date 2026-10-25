import { Folder, FolderOpen, Globe } from 'lucide-react';
import { cn } from '../ui/cn';

// A folder on an SSH host: the plain folder glyph with a small globe on its lower right corner (picked 2026-10-25 in
// lab/remote.preview.tsx `set=list`, B over a single redrawn glyph). The folder is masked away under the globe
// (`.remote-folder-cut`), so the badge reads on any fill behind it, a hovered row's included. The badge hangs past the
// icon box into the gap beside it, like the LAB's. `pulse`: the host is connecting, the glyph breathes (on an inner box,
// so the outer one's own opacity, which the column's fold cross-fades, stays the caller's)
export function RemoteFolderIcon({ open, pulse, className }: { open?: boolean; pulse?: boolean; className?: string }) {
  const Glyph = open ? FolderOpen : Folder;
  return (
    <span aria-hidden className={cn('inline-flex size-icon shrink-0', className)}>
      <span className={cn('relative flex size-full', pulse && 'remote-breathe')}>
        <Glyph strokeWidth={1.5} className="remote-folder-cut size-full" />
        <span className="absolute -right-1 -bottom-1 flex size-(--remote-badge) items-center justify-center">
          {/* Important: icon slots size every svg inside them (`[&_svg]:size-icon`), the badge's included */}
          <Globe strokeWidth={2} className="size-2!" />
        </span>
      </span>
    </span>
  );
}
