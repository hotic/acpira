import { AgentInfo, FileHit, SettingsView, SlashCommand, SubagentGroup, SubagentPersona } from '../wire/Wire';
import { ComposerControls } from './ModelCatalog';
import { t } from '../store/L10n';

// apps/ios Logic/SlashCommands.swift (slashCommands.ts, Mention.tsx, personaHits.ts): the `/` command and `@` mention
// tokens under the caret, the command list's ranking, completion and input hint, and the summonable personas offered
// ahead of the files. Offsets are string indices, as the field reports its caret.

/** A `/` or `@` token under the caret: where its sigil sits and what has been typed after it, up to the caret */
export interface SigilSpan {
  start: number;
  query: string;
}

/** The prompt after a pick, and where the caret goes */
export interface PromptEdit {
  text: string;
  caret: number;
}

/** A cross-harness subagent (or a group) as the `@` list and the summon bean offer it (personaHits.ts MentionPersona) */
export interface MentionPersona {
  id: string;
  name: string;
  /** The CLI it runs in (empty for a group) */
  agent: string;
  /** The CLI's name, or a group's members joined */
  cli?: string;
  model?: string;
  effort?: string;
  group: boolean;
}

/** MentionPersona.from (SlashCommands.swift): the enabled personas, then the enabled groups with an enabled member */
export function personasOf(settings: SettingsView, agents: AgentInfo[]): MentionPersona[] {
  const on = (settings.subagents ?? []).filter((p: SubagentPersona) => p.enabled);
  const personas = on.map((p: SubagentPersona): MentionPersona => {
    const out: MentionPersona = {
      id: p.id, name: p.name, agent: p.agent, cli: agents.find((a: AgentInfo) => a.id === p.agent)?.name ?? p.agent,
      model: p.model, effort: p.effort !== undefined ? ComposerControls.effortLabel({ id: p.effort, name: p.effort }) : undefined,
      group: false
    };
    return out;
  });
  const groups: MentionPersona[] = [];
  for (const g of settings.subagentGroups ?? []) {
    const names = g.members.map((m: string) => on.find((p: SubagentPersona) => p.id === m)?.name)
      .filter((n: string | undefined) => n !== undefined) as string[];
    if (g.enabled && names.length > 0) {
      groups.push({ id: g.id, name: g.name, agent: '', cli: names.join(t('common.listSep')), group: true });
    }
  }
  return personas.concat(groups);
}

/** A persona's hit travels as a FileHit with this scheme */
const PERSONA_SCHEME = 'acpira-agent:';

function isSpace(c: string): boolean {
  return /\s/.test(c);
}

/**
 * What may sit right before a sigil for it to count (SIGIL_LEAD): whitespace, or CJK text and its full-width
 * punctuation, which those languages write without spaces. Latin letters, digits and ASCII punctuation keep
 * `a@b.com`, `a/b` and `https://` plain
 */
function leads(c: string): boolean {
  return isSpace(c) ||
    /[ᄀ-ᇿ⺀-⿟　-ヿ㄰-㆏ㇰ-ㇿ㐀-䶿一-鿿가-힯豈-﫿＀-￯]/.test(c);
}

export class SlashCommands {
  /** Where the run of non-blank characters ending at the caret starts */
  private static tailFrom(head: string): number {
    let from = head.length;
    while (from > 0 && !isSpace(head.charAt(from - 1))) {
      from--;
    }
    return from;
  }

  /** commandAt: the first `/` in the run before the caret that leads; the query runs from it to the caret */
  static commandAt(text: string, caret: number): SigilSpan | undefined {
    const head = text.slice(0, Math.max(0, caret));
    for (let i = SlashCommands.tailFrom(head); i < head.length; i++) {
      if (head.charAt(i) === '/' && (i === 0 || leads(head.charAt(i - 1)))) {
        const span: SigilSpan = { start: i, query: head.slice(i + 1) };
        return span;
      }
    }
    return undefined;
  }

  /** mentionAt: the last `@` in the run before the caret, when it leads; the query holds no further `@` */
  static mentionAt(text: string, caret: number): SigilSpan | undefined {
    const head = text.slice(0, Math.max(0, caret));
    const from = SlashCommands.tailFrom(head);
    const i = head.lastIndexOf('@');
    if (i < from || (i > 0 && !leads(head.charAt(i - 1)))) {
      return undefined;
    }
    const span: SigilSpan = { start: i, query: head.slice(i + 1) };
    return span;
  }

  /** completeCommand: the token becomes `/name `, the text after it stays as the arguments */
  static complete(text: string, span: SigilSpan, caret: number, name: string): PromptEdit {
    const head = text.slice(0, Math.min(span.start, text.length)) + `/${name} `;
    const rest = text.slice(Math.min(caret, text.length)).replace(/^\S*/, '').replace(/^\s+/, '');
    const edit: PromptEdit = { text: head + rest, caret: head.length };
    return edit;
  }

  /**
   * matchCommands: name prefixes first, then names containing the query or descriptions with a word starting with it,
   * each tier in the agent's order; case-insensitive. An empty query lists everything
   */
  static match(commands: SlashCommand[], query: string): SlashCommand[] {
    const q = query.toLowerCase();
    if (q.length === 0) {
      return SlashCommands.mergeSkillCopies(commands);
    }
    const wordStart = (s: string): boolean => s.toLowerCase().split(/[^\p{L}\p{N}]+/u).some((w: string) => w.startsWith(q));
    const prefix = commands.filter((c: SlashCommand) => c.name.toLowerCase().startsWith(q));
    const rest = commands.filter((c: SlashCommand) => !c.name.toLowerCase().startsWith(q) &&
      (c.name.toLowerCase().includes(q) || wordStart(c.description)));
    return SlashCommands.mergeSkillCopies(prefix.concat(rest));
  }

  /** A skill listed from both `~/.agents/skills` and `~/.claude/skills` (`agents:dig`, `claude:dig`) shows once */
  private static mergeSkillCopies(commands: SlashCommand[]): SlashCommand[] {
    const seen: Set<string> = new Set<string>();
    return commands.filter((c: SlashCommand) => {
      const m = c.name.match(/^(?:agents|claude):/);
      if (m === null) {
        return true;
      }
      const bare = c.name.slice(m[0].length);
      if (seen.has(bare)) {
        return false;
      }
      seen.add(bare);
      return true;
    });
  }

  /** commandHint: the input hint of the command the prompt's last token names, while its arguments are still empty */
  static hint(commands: SlashCommand[], text: string): string | undefined {
    const m = text.match(/(?:^|\s)\/(\S+)\s*$/);
    if (m === null) {
      return undefined;
    }
    const name = m[1];
    const hint = commands.find((c: SlashCommand) => c.name === name)?.input?.hint;
    return hint !== undefined && hint.length > 0 ? hint : undefined;
  }

  /** A summon (`@name `) written at `at`, replacing `cut` characters there; a space goes before it when needed */
  static summon(text: string, name: string, at: number, cut: number): PromptEdit {
    const before = text.slice(0, Math.min(at, text.length));
    const lead = before.length === 0 || isSpace(before.charAt(before.length - 1)) ? '' : ' ';
    const token = `${lead}@${name} `;
    const after = text.slice(Math.min(at + cut, text.length));
    const edit: PromptEdit = { text: before + token + after, caret: before.length + token.length };
    return edit;
  }

  /** A file picked for the `@` token: the token leaves the text (the file rides along as an attachment) */
  static dropToken(text: string, span: SigilSpan, caret: number): PromptEdit {
    const start = Math.min(span.start, text.length);
    const edit: PromptEdit = { text: text.slice(0, start) + text.slice(Math.min(caret, text.length)), caret: start };
    return edit;
  }

  /** personaHits: the personas whose name or id holds the query, leading the file list */
  static personaHits(personas: MentionPersona[], query: string): FileHit[] {
    const q = query.toLowerCase();
    return personas.filter((p: MentionPersona) => q.length === 0 || p.name.toLowerCase().includes(q) || p.id.includes(q))
      .map((p: MentionPersona) => {
        const hit: FileHit = { uri: PERSONA_SCHEME + p.id, path: p.name };
        return hit;
      });
  }

  /** personaOfHit */
  static personaOf(hit: FileHit, personas: MentionPersona[]): MentionPersona | undefined {
    if (!hit.uri.startsWith(PERSONA_SCHEME)) {
      return undefined;
    }
    const id = hit.uri.slice(PERSONA_SCHEME.length);
    return personas.find((p: MentionPersona) => p.id === id);
  }
}
