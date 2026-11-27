import { SessionOption } from '../wire/Wire';
import { t } from '../store/L10n';

// apps/ios Chat/Modes.swift (modeIcons.ts): a mode's glyph, labels and hint, and the approval level's glyph and label

/** modeIcons.ts `modeIcon`: a mode's glyph from its kind, id and name; first match wins */
export function modeIcon(mode: SessionOption): string {
  if (mode.kind === 'full_access') {
    return 'shieldOff';
  }
  if (mode.kind === 'auto_review') {
    return 'sparkles';
  }
  if (mode.id === 'auto_edit' || mode.id === 'acceptEdits') {
    return 'code';
  }
  const key = `${mode.id} ${mode.name}`.toLowerCase();
  if (/bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe/.test(key)) {
    return 'shieldOff';
  }
  if (/debug/.test(key)) {
    return 'bug';
  }
  if (/plan/.test(key)) {
    return 'listTodo';
  }
  if (/\bask\b|chat|question/.test(key)) {
    return 'messageCircleQuestionMark';
  }
  if (/smart|\bauto\b/.test(key)) {
    return 'sparkles';
  }
  if (/code|edit|build/.test(key)) {
    return 'code';
  }
  if (/\bteam\b|multitask|orchestrat/.test(key)) {
    return 'users';
  }
  if (/agent|default|normal/.test(key)) {
    return 'infinity';
  }
  return 'circleDashed';
}

const MODE_SHORT: Map<string, string> = new Map<string, string>([
  ['agent', 'agent'], ['build', 'build'], ['plan', 'plan'], ['raw', 'raw'], ['team', 'team'],
  ['acceptEdits', 'autoEdit'], ['auto_edit', 'autoEdit'], ['auto', 'auto'], ['smart', 'smart'],
  ['bypassPermissions', 'full'], ['bypass', 'full'], ['yolo', 'full'], ['agent-full-access', 'full'],
  ['read-only', 'readOnly'], ['workspace-write', 'workspace'], ['ask', 'ask'], ['code', 'code'], ['accept-edits', 'code']
]);

/**
 * modeIcons.ts `modeLabel`: the short name of a mode the built-in integrations send, matched on the exact id, in the
 * app's language; anything unknown shows the agent's own name
 */
export function modeLabel(mode: SessionOption): string {
  if (mode.kind === 'full_access') {
    return t('composer.modeShort.full');
  }
  if (mode.kind === 'auto_review') {
    return t('composer.modeShort.auto');
  }
  // Every known agent's `default` asks before acting (modeIcons.ts)
  if (mode.id === 'default') {
    return t('composer.access.ask');
  }
  const short = MODE_SHORT.get(mode.id);
  return short !== undefined ? t(`composer.modeShort.${short}`) : mode.name;
}

/** A collaboration mode's name: Codex's `default` is its Build mode, not the permission default beside it */
export function workLabel(mode: SessionOption): string {
  return mode.id === 'default' ? t('composer.modeShort.build') : modeLabel(mode);
}

/**
 * modeIcons.ts `modeName`: a working mode where it is picked by name (the `/` list, the bar's pill), the bare name
 * without the bean labels' padding to one length; unknown modes keep the label
 */
export function modeName(mode: SessionOption): string {
  if (mode.id === 'plan' || mode.id === 'team' || mode.id === 'raw') {
    return t(`composer.modeName.${mode.id}`);
  }
  return modeLabel(mode);
}

/** modeIcons.ts `modeHint`: one line on what the mode does, else the agent's own description */
export function modeHint(mode: SessionOption): string | undefined {
  if (mode.id === 'plan' || mode.id === 'team' || mode.id === 'raw') {
    return t(`composer.modeHint.${mode.id}`);
  }
  return mode.description;
}

const ACCESS_ICONS: Map<string, string> = new Map<string, string>([
  ['ask', 'shieldEllipsis'], ['auto-edit', 'shieldCheck'], ['auto', 'shieldHalf'], ['full', 'shieldOff']
]);

/**
 * modeIcons.ts `accessIcon`: the approval levels (the built-in agent's, and `auto` from the engine's split of Claude's
 * modes) as one shield family, so the bean reads as a scale; other levels keep their mode glyph
 */
export function accessIcon(levelOption: SessionOption): string {
  return ACCESS_ICONS.get(levelOption.id) ?? modeIcon(levelOption);
}

/** The built-in agent's approval levels by id, any other level its short mode label */
export function accessLabel(levelOption: SessionOption): string {
  if (levelOption.id === 'ask') {
    return t('composer.access.ask');
  }
  if (levelOption.id === 'auto-edit') {
    return t('composer.access.autoEdit');
  }
  if (levelOption.id === 'full') {
    return t('composer.access.full');
  }
  return modeLabel(levelOption);
}
