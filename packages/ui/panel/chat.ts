import { useEffect, useRef } from 'react';
import { isChatFolder } from '../chat/sessionTree';
import { project } from './api';
import { panel, panelState } from './store';

// Chats (sessions in no project) run in a hidden folder of their own under InitState.chats (docs/dev/host-architecture.md
// "Chats"). That folder is an implementation detail: an empty one in the tool panel read as a stray project named by date
// and random. So the panel folds as a chat with no files comes up, and opens again as the chat is left; the file tree
// names a chat's folder for what it is (FileTree)

// Where chats' folders live; set by the shell that knows it (Shell via useChatPanel)
let chatsRoot: string | undefined;

/** Whether `root` is a chat's own folder */
export const isChatRoot = (root: string) => isChatFolder(chatsRoot, root);

/** Folds the window's tool panel while the shown session is a chat whose folder holds nothing, and opens it again once a
 *  project session is shown. A panel opened by hand inside a chat stays open */
export function useChatPanel(chats: string | undefined, chat: boolean, sessionId: string | undefined) {
  chatsRoot = chats;
  // The panel was open and this hook folded it: leaving the chats gives it back
  const folded = useRef(false);
  useEffect(() => {
    if (!chat) {
      if (folded.current) {
        folded.current = false;
        if (!panelState().open) panel.toggle(true);
      }
      return;
    }
    if (!sessionId || !panelState().open) return;
    let live = true;
    // A chat not started yet has no folder to read (noProject); only a read nobody answered says nothing
    void project.dir(sessionId, '').then(l => {
      if (!live || l.error === 'timeout' || (l.entries.length && !l.error) || !panelState().open) return;
      folded.current = true;
      panel.toggle(false);
    });
    return () => { live = false; };
  }, [chat, sessionId]);
}
