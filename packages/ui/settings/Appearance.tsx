import { CODE_FONT_SIZE, DESKTOP_LAYOUTS, DIFF_MARKERS, PANEL_SCOPES, SESSION_LIST_POSITIONS, THEMES, UI_FONT_SIZE, type SettingsView } from '@acpira/shared/settings';
import type { Appearance } from '../appearance';
import { t } from '../i18n';
import { useDesktop } from '../desktop';
import { Field, MultiSelect, NumberField, Section, Select, Switch } from '../chrome/controls';
import type { SettingsHandlers } from './SettingsShell';

// The appearance axes surfaced as user settings: motion (an accessibility preference), the folded projects' agent marks
// and the process and conversation groups; the other axes stay LAB design decisions
const WIDTHS = ['narrow', 'standard', 'wide', 'full'] as const;
// Process step kinds in reading order, each on its `show*` axis
const STEPS = [
  ['thought', 'showThoughts'], ['commentary', 'showCommentary'], ['read', 'showReads'], ['command', 'showCommands'], ['edit', 'showEdits'], ['other', 'showOtherTools'],
] as const;

// Appearance: how the panels render (scheme, motion, diff markers), the tool panel's modules, then the type sizes. Every edit previews in place, this shell included
export function AppearancePage({ settings, appearance, on }: { settings: SettingsView; appearance: Appearance; on: SettingsHandlers }) {
  const desktop = useDesktop();
  const shown = STEPS.filter(([, axis]) => appearance[axis] !== 'off').map(([kind]) => kind);
  return (
    <>
      <Section title={t('settings.group.ui')}>
        {/* The desktop app keeps its column on the left and folds it from the title row instead; what it offers is the
            window's layout, the conversation or the files in the middle */}
        {desktop
          ? <>
            {settings.panelFiles !== false && (
              <Field label={t('settings.desktopLayout')} desc={t('settings.desktopLayout.desc')}>
                <Select options={DESKTOP_LAYOUTS.map(v => ({ value: v, label: t(`settings.desktopLayout.${v}`) }))} value={settings.desktopLayout} onChange={v => on.setSetting('desktopLayout', v)} label={t('settings.desktopLayout')} />
              </Field>
            )}
          </>
          : <Field label={t('settings.sessionListPosition')} desc={t('settings.sessionListPosition.desc')}>
            <Select options={SESSION_LIST_POSITIONS.map(v => ({ value: v, label: t(`settings.sessionListPosition.${v}` as const) }))} value={settings.sessionListPosition} onChange={v => on.setSetting('sessionListPosition', v)} label={t('settings.sessionListPosition')} />
          </Field>}
        <Field label={t('settings.theme')} desc={t('settings.theme.desc')}>
          <Select options={THEMES.map(v => ({ value: v, label: t(desktop && v === 'auto' ? 'settings.theme.system' : `settings.theme.${v}` as const) }))} value={settings.theme} onChange={v => on.setSetting('theme', v)} label={t('settings.theme')} />
        </Field>
        <Field label={t('settings.motion')} desc={t('settings.motion.desc')}>
          <Switch checked={appearance.motion !== 'none'} onChange={v => on.setAppearance('motion', v ? 'on' : 'none')} label={t('settings.motion')} />
        </Field>
        <Field label={t('settings.projectMarks')} desc={t('settings.projectMarks.desc')}>
          <Switch checked={appearance.projectMarks === 'on'} onChange={v => on.setAppearance('projectMarks', v ? 'on' : 'off')} label={t('settings.projectMarks')} />
        </Field>
      </Section>

      {/* How a turn's work reads: which kinds of step it lists and when they open, shown on a sample turn below */}
      <Section title={t('settings.group.process')}>
        <Field label={t('settings.steps')} desc={t('settings.steps.desc')}>
          <MultiSelect options={STEPS.map(([kind]) => ({ value: kind, label: t(`settings.steps.${kind}`) }))} on={shown}
            summary={shown.length === STEPS.length ? t('settings.steps.all') : shown.length ? t('settings.steps.some', { n: shown.length }) : t('settings.steps.none')}
            onChange={(kind, next) => on.setAppearance(STEPS.find(([k]) => k === kind)![1], next ? 'on' : 'off')} label={t('settings.steps')} />
        </Field>
        <Field label={t('settings.stepIcons')} desc={t('settings.stepIcons.desc')}>
          <Switch checked={appearance.stepIcons === 'on'} onChange={v => on.setAppearance('stepIcons', v ? 'on' : 'off')} label={t('settings.stepIcons')} />
        </Field>
        <Field label={t('settings.autoExpand')} desc={t('settings.autoExpand.desc')}>
          <Switch checked={appearance.autoExpand !== 'off'} onChange={v => on.setAppearance('autoExpand', v ? 'on' : 'off')} label={t('settings.autoExpand')} />
        </Field>
        <Field label={t('settings.keepProcess')} desc={t('settings.keepProcess.desc')}>
          <Switch checked={appearance.keepProcess === 'on'} onChange={v => on.setAppearance('keepProcess', v ? 'on' : 'off')} label={t('settings.keepProcess')} />
        </Field>
      </Section>

      <Section title={t('settings.group.chat')}>
        <Field label={t('settings.chatWidth')} desc={t('settings.chatWidth.desc')}>
          <Select options={WIDTHS.map(v => ({ value: v, label: t(`settings.chatWidth.${v}`) }))} value={appearance.chatWidth} onChange={v => on.setAppearance('chatWidth', v)} label={t('settings.chatWidth')} />
        </Field>
        <Field label={t('settings.codeWrap')} desc={t('settings.codeWrap.desc')}>
          <Switch checked={appearance.codeWrap === 'on'} onChange={v => on.setAppearance('codeWrap', v ? 'on' : 'off')} label={t('settings.codeWrap')} />
        </Field>
        <Field label={t('settings.diffMarkers')} desc={t('settings.diffMarkers.desc')}>
          <Select options={DIFF_MARKERS.map(v => ({ value: v, label: t(`settings.diffMarkers.${v}` as const) }))} value={settings.diffMarkers} onChange={v => on.setSetting('diffMarkers', v)} label={t('settings.diffMarkers')} />
        </Field>
        <Field label={t('settings.reportLink')} desc={t('settings.reportLink.desc')}>
          <Switch checked={appearance.reportLink !== 'off'} onChange={v => on.setAppearance('reportLink', v ? 'on' : 'off')} label={t('settings.reportLink')} />
        </Field>
      </Section>

      {/* The tool panel's built-in modules, switched in place without a restart, and whether the panel follows the session */}
      <Section title={t('settings.panels')}>
        <Field label={t('settings.plugins.files')} desc={t('settings.plugins.filesDesc')}>
          <Switch checked={settings.panelFiles !== false} onChange={v => on.setSetting('panelFiles', v)} label={t('settings.plugins.files')} />
        </Field>
        <Field label={t('settings.plugins.terminal')} desc={t('settings.plugins.terminalDesc')}>
          <Switch checked={settings.panelTerminal !== false} onChange={v => on.setSetting('panelTerminal', v)} label={t('settings.plugins.terminal')} />
        </Field>
        <Field label={t('settings.plugins.browser')} desc={t('settings.plugins.browserDesc')}>
          <Switch checked={settings.panelBrowser !== false} onChange={v => on.setSetting('panelBrowser', v)} label={t('settings.plugins.browser')} />
        </Field>
        {desktop && (settings.panelFiles !== false || settings.panelTerminal !== false || settings.panelBrowser !== false) && (
          <Field label={t('settings.panelScope')} desc={t('settings.panelScope.desc')}>
            <Select options={PANEL_SCOPES.map(v => ({ value: v, label: t(`settings.panelScope.${v}`) }))} value={settings.panelScope ?? 'project'} onChange={v => on.setSetting('panelScope', v)} label={t('settings.panelScope')} />
          </Field>
        )}
      </Section>

      <Section title={t('settings.text.title')}>
        <Field label={t('settings.uiFontSize')} desc={t('settings.uiFontSize.desc')}>
          <NumberField value={settings.uiFontSize} min={UI_FONT_SIZE.min} max={UI_FONT_SIZE.max} step={1} unit={t('settings.fontSize.unit')} label={t('settings.uiFontSize')} onCommit={v => on.setSetting('uiFontSize', v)} />
        </Field>
        <Field label={t('settings.codeFontSize')} desc={t('settings.codeFontSize.desc')}>
          <NumberField value={settings.codeFontSize} min={CODE_FONT_SIZE.min} max={CODE_FONT_SIZE.max} step={1} unit={t('settings.fontSize.unit')} label={t('settings.codeFontSize')} onCommit={v => on.setSetting('codeFontSize', v)} />
        </Field>
        <Field label={t('settings.fontSmoothing')} desc={t('settings.fontSmoothing.desc')}>
          <Switch checked={settings.fontSmoothing} onChange={v => on.setSetting('fontSmoothing', v)} label={t('settings.fontSmoothing')} />
        </Field>
      </Section>
    </>
  );
}
