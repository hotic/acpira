import { useCallback, useEffect, useRef } from 'react';
import type { SessionCategories, SessionSummary, Turn } from '@acpira/shared/transcript';
import type { SubagentSummary } from '@acpira/shared/subagents';
import type { SessionScope } from '@acpira/shared/settings';
import type { ShellCommand } from '@acpira/shared/protocol';
import { t } from '../i18n';
import { imeKey } from '../ui/ime';
import { useShellCommands } from '../desktop';
import { hitShortcut } from '../keybindings';
import { focusMainComposer, mainComposerEmpty } from '../chat/useComposerDraft';
import { pendingQuickPermission } from '../chat/permissionOptions';
import { nextAttentionSession, orderedSessions, stepSession } from '../chat/sessionNav';
import type { SettingsPage } from '../chrome/settingsPage';

// What the desktop shortcuts and the menu bar's commands run, read at the moment a key arrives
export interface ShellKeyActions {
  newIn: () => void;
  addProject: () => void;
  focusSearch: () => void;
  leftSide: () => void;
  rightSide: () => void;
  back?: () => void;
  forward?: () => void;
  toggleTerminal: () => void;
  openChanges: () => void;
  toggleBrowser: () => void;
  toggleQuickOpen: () => void;
  closePanelOrWindow: () => void;
}

// What the cross-shell shortcuts read, as of the latest render
export interface ShellKeyState {
  sessions: SessionSummary[];
  categories?: SessionCategories;
  activeSessionId?: string;
  pendingSessionId?: string;
  workspace?: string;
  sessionScope?: SessionScope;
  chats?: string;
  running: boolean;
  stoppable: boolean;
  turns: Turn[];
  subagents?: SubagentSummary[];
  hasQuestion: boolean;
  overlayInspect: boolean;
  chatFindOpen: boolean;
  selectSession: (id: string) => void;
  stop: () => void;
  permission: (sessionId: string, blockId: string, optionId: string) => void;
  openChatFind: () => void;
  focusSearch: () => void;
  openSettings?: (page?: SettingsPage) => void;
  hasSessionPanel: boolean;
}

interface ShellKeysOptions {
  desktop: boolean;
  sample?: boolean;
  // Present when a folder can be picked: the open-folder shortcut only works then
  pickFolder?: () => Promise<string | undefined>;
  openSettings?: (page?: SettingsPage) => void;
  actions: ShellKeyActions;
  state: ShellKeyState;
  setChatFind: (closed: undefined) => void;
  setInspect: (closed: undefined) => void;
  dropToast: (key: string) => void;
  pushToast: (toast: { key: string; text: string; ttl?: number }) => void;
}

// The shell's keyboard: cross-shell shortcuts on every host, desktop shortcuts and the menu bar's commands in the app.
// Listeners are added once and read the latest actions and state through refs
export function useShellKeys({ desktop, sample, pickFolder, openSettings, actions, state, setChatFind, setInspect, dropToast, pushToast }: ShellKeysOptions) {
  const keys = useRef(actions);
  keys.current = actions;
  // Cross-shell shortcuts: session cycling (⌃Tab / ⌘⇧[ / ] / ⌘⌥A), find in chat (⌘F), permission quick approval (Enter / Esc), and Esc to stop
  const escStopArmed = useRef(0);
  useEffect(() => { escStopArmed.current = 0; dropToast('esc-stop'); }, [state.stoppable, state.activeSessionId, dropToast]);
  const shellKeys = useRef(state);
  shellKeys.current = state;
  useEffect(() => {
    if (sample) return;
    const onKey = (e: KeyboardEvent) => {
      if (imeKey(e) || e.defaultPrevented) return;
      const cur = shellKeys.current;
      const target = e.target instanceof HTMLElement ? e.target : null;
      const inDialog = !!document.querySelector('[data-settings-dialog], [data-quick-open], [role="menu"]');
      const inXterm = !!target?.closest('.xterm');

      if (!inDialog && cur.openSettings && hitShortcut(e, 'openShortcuts')) {
        e.preventDefault();
        cur.openSettings({ kind: 'shortcuts' });
        return;
      }

      if (!inDialog && !inXterm && hitShortcut(e, 'focusComposer')) {
        e.preventDefault();
        focusMainComposer();
        return;
      }

      if (!inDialog && !desktop && cur.hasSessionPanel && hitShortcut(e, 'searchSessions')) {
        e.preventDefault();
        cur.focusSearch();
        return;
      }

      // ⌃Tab / ⌃⇧Tab, and on macOS ⌘⇧[ / ⌘⇧]: cycle sessions in the visible list order
      const isPrevSession = !inDialog && hitShortcut(e, 'prevSession');
      const isNextSession = !inDialog && !isPrevSession && hitShortcut(e, 'nextSession');
      if (isPrevSession || isNextSession) {
        const list = orderedSessions({
          sessions: cur.sessions, categories: cur.categories, activeId: cur.activeSessionId,
          workspace: cur.workspace, scope: desktop ? 'all' : cur.sessionScope, desktop, chats: desktop ? cur.chats : undefined,
        });
        const next = stepSession(list, cur.pendingSessionId ?? cur.activeSessionId, isPrevSession ? -1 : 1);
        if (next) { e.preventDefault(); cur.selectSession(next); }
        return;
      }

      // ⌘⌥A (Ctrl+Alt+A elsewhere): jump to the next session needing attention (waiting / unread / error)
      if (!inDialog && hitShortcut(e, 'nextAttention')) {
        const list = orderedSessions({
          sessions: cur.sessions, categories: cur.categories, activeId: cur.activeSessionId,
          workspace: cur.workspace, scope: desktop ? 'all' : cur.sessionScope, desktop, chats: desktop ? cur.chats : undefined,
        });
        const next = nextAttentionSession(list, cur.pendingSessionId ?? cur.activeSessionId);
        if (next) { e.preventDefault(); cur.selectSession(next); }
        return;
      }

      // ⌘F / Ctrl+F: find in the active conversation (terminals keep their own find bar)
      if (!inDialog && !inXterm && hitShortcut(e, 'findInChat')) {
        if (!cur.overlayInspect && cur.turns.length > 0) {
          e.preventDefault();
          cur.openChatFind();
        }
        return;
      }

      if (inDialog || inXterm) return;

      // Enter: approve the unambiguous pending permission card while the main composer is empty
      if (e.key === 'Enter' && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey && !e.repeat) {
        if (!cur.running || !cur.activeSessionId || cur.hasQuestion || !mainComposerEmpty()) return;
        const onInteractive = target?.closest('button, a, input, select, [contenteditable="true"], [role="button"], [role="menuitem"], [role="option"], [role="tab"]');
        const onOtherTextarea = target?.tagName === 'TEXTAREA' && !target.hasAttribute('data-main-composer');
        if (onInteractive || onOtherTextarea) return;
        const quick = pendingQuickPermission(cur.turns, cur.subagents);
        if (quick?.primaryId) {
          e.preventDefault();
          cur.permission(cur.activeSessionId, quick.blockId, quick.primaryId);
        }
        return;
      }

      // Escape: close chat find -> close overlay inspector -> reject pending permission (empty composer) -> stop the turn and its summoned rounds
      if (e.key === 'Escape' && !e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey && !e.repeat) {
        if (target?.matches('input, select, [contenteditable="true"]') || (target?.tagName === 'TEXTAREA' && !target.hasAttribute('data-main-composer'))) return;
        if (cur.chatFindOpen) { e.preventDefault(); setChatFind(undefined); focusMainComposer(); return; }
        if (cur.overlayInspect) { e.preventDefault(); setInspect(undefined); return; }
        if (cur.running && cur.activeSessionId && !cur.hasQuestion && mainComposerEmpty()) {
          const quick = pendingQuickPermission(cur.turns, cur.subagents);
          if (quick?.rejectId) {
            e.preventDefault();
            cur.permission(cur.activeSessionId, quick.blockId, quick.rejectId);
            return;
          }
        }
        if (cur.stoppable) {
          e.preventDefault();
          if (mainComposerEmpty() || Date.now() - escStopArmed.current < 1500) {
            escStopArmed.current = 0;
            dropToast('esc-stop');
            cur.stop();
          } else {
            escStopArmed.current = Date.now();
            pushToast({ key: 'esc-stop', text: t('composer.escAgainToStop'), ttl: 1500 });
          }
        }
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop, sample, dropToast, pushToast]);
  useEffect(() => {
    if (!desktop) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.defaultPrevented) return;
      if (hitShortcut(e, 'toggleTerminal')) keys.current.toggleTerminal();
      else if (hitShortcut(e, 'openChanges')) keys.current.openChanges();
      else if (hitShortcut(e, 'toggleBrowser')) keys.current.toggleBrowser();
      else if (hitShortcut(e, 'newSession')) keys.current.newIn();
      else if (hitShortcut(e, 'searchSessions')) keys.current.focusSearch();
      else if (hitShortcut(e, 'toggleSidebar')) keys.current.leftSide();
      else if (hitShortcut(e, 'quickOpen')) keys.current.toggleQuickOpen();
      else if (hitShortcut(e, 'closeTab')) keys.current.closePanelOrWindow();
      else if (pickFolder && hitShortcut(e, 'openFolder')) keys.current.addProject();
      else if (openSettings && hitShortcut(e, 'openSettings')) openSettings();
      else if (hitShortcut(e, 'back')) keys.current.back?.();
      else if (hitShortcut(e, 'forward')) keys.current.forward?.();
      else return;
      e.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [desktop, pickFolder, openSettings]);
  // The menu bar's commands do what the shortcuts do (settings is App's: it switches the page)
  const runCommand = useCallback((command: ShellCommand) => {
    if (command === 'newSession') keys.current.newIn();
    else if (command === 'search') keys.current.focusSearch();
    else if (command === 'openFolder') keys.current.addProject();
    else if (command === 'quickOpen') keys.current.toggleQuickOpen();
    else if (command === 'close') keys.current.closePanelOrWindow();
    else if (command === 'sidebar') keys.current.leftSide();
    else if (command === 'toolPanel') keys.current.rightSide();
    else if (command === 'terminal') keys.current.toggleTerminal();
    else if (command === 'changes') keys.current.openChanges();
    else if (command === 'browser') keys.current.toggleBrowser();
  }, []);
  useShellCommands(desktop ? runCommand : undefined);
}
