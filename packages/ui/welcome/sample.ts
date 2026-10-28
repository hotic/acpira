import type { AgentInfo, QueuedPrompt, SessionCategories, SessionSummary, Turn } from '@acpira/shared/transcript';
import { t } from '../i18n';

// The staged session the interface tour plays over (Welcome.tsx). Nothing here touches the engine or the disk, so the
// tour runs before the data folder exists and replays from Settings without disturbing real sessions. It holds what
// there is to teach: sessions running side by side, an approval waiting, a follow-up queued, the composer
export interface SampleSession {
  agents: AgentInfo[];
  sessions: SessionSummary[];
  categories: SessionCategories;
  turns: Turn[];
  queued: QueuedPrompt[];
  cwd: string;
  home: string;
}

const HOME = '/Users/dev';
const PROJECT = `${HOME}/demo-app`;
const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

// Built per call: the copy follows the locale in force, and the times are relative to now
export function sampleSession(now = Date.now()): SampleSession {
  const at = (ago: number) => new Date(now - ago).toISOString();
  return {
    agents: [
      { id: 'claude', name: 'Claude Code', available: true },
      { id: 'codex', name: 'Codex', available: true },
      { id: 'gemini', name: 'Gemini CLI', available: true },
    ],
    sessions: [
      { id: 'sample', title: t('welcome.sample.title1'), agent: 'claude', cwd: PROJECT, updatedAt: at(0), state: 'working' },
      { id: 'sample-2', title: t('welcome.sample.title2'), agent: 'codex', cwd: PROJECT, updatedAt: at(MINUTE), state: 'working' },
      { id: 'sample-3', title: t('welcome.sample.title3'), agent: 'gemini', cwd: PROJECT, updatedAt: at(HOUR) },
      { id: 'sample-4', title: t('welcome.sample.title4'), agent: 'claude', cwd: PROJECT, updatedAt: at(DAY) },
    ],
    categories: { categories: [], collapsedProjects: [], projects: [PROJECT] },
    turns: [
      { role: 'user', text: t('welcome.sample.prompt') },
      { role: 'agent', blocks: [
        { type: 'thought', text: t('welcome.sample.thought'), durationSec: 3 },
        { type: 'tool_call', id: 'sample-read', kind: 'read', verb: t('verb.read'), target: 'src/SessionList.tsx', targetMono: true, status: 'completed' },
        { type: 'tool_call', id: 'sample-run', kind: 'execute', verb: t('verb.execute'), target: 'pnpm test sidebar', status: 'completed' },
        { type: 'text', markdown: t('welcome.sample.reply') },
        // The title is the agent's own words, which arrive in English; the approvals step finds the card by it
        { type: 'permission', id: 'sample-permission', title: 'Edit src/SessionList.tsx', options: [
          { id: 'once', label: t('permission.once'), kind: 'allow_once' },
          { id: 'session', label: t('permission.session'), kind: 'allow_always' },
          { id: 'reject', label: t('permission.reject'), kind: 'reject_once' },
        ] },
      ] },
    ],
    queued: [{ id: 'sample-queued', text: t('welcome.sample.queued'), attachments: [] }],
    cwd: PROJECT,
    home: HOME,
  };
}
