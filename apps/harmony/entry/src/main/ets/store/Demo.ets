import { AgentInfo, SessionSummary, SessionView } from '../wire/Wire';
import { readRaw } from './Assets';

// The synthetic sessions the iOS and Android demos show (apps/ios AcpiraFixtures/synthetic, synced by scripts/build.sh):
// the same files, so a screenshot of a fixture on any phone compares like for like.

function read<T>(name: string): T | undefined {
  const text = readRaw(`fixtures/${name}.json`);
  return text === undefined ? undefined : JSON.parse(text) as T;
}

export function demoAgents(): AgentInfo[] {
  return read<AgentInfo[]>('agents') ?? [];
}

export function demoSessions(): SessionSummary[] {
  return read<SessionSummary[]>('sessions') ?? [];
}

/** The fixture a demo session opens on: one named after it, else the showcase under the session's own title */
export function demoView(id: string, title: string): SessionView {
  const own = read<SessionView>(id);
  const view: SessionView = own ?? read<SessionView>('showcase') ?? {
    id: id, agent: 'claude', title: title, cwd: '', status: 'ready', running: false, turns: []
  };
  if (own === undefined) {
    view.id = id;
    view.title = title;
  }
  return view;
}

/** A fixture by name, for launches that ask for one (`fixture` in the want) */
export function fixtureView(name: string): SessionView | undefined {
  return read<SessionView>(name);
}

interface Clock {
  now: string;
}

/** The fixtures' "now" (clock.json): their timestamps are written relative to it */
export function demoNow(): number | undefined {
  const clock = read<Clock>('clock');
  return clock === undefined ? undefined : Date.parse(clock.now);
}
