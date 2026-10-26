import { CODE_FONT_SIZE, DESKTOP_LAYOUTS, DIFF_MARKERS, SESSION_LIST_POSITIONS, THEMES, UI_FONT_SIZE, type SettingsView } from '@acpira/shared/settings';
import type { Appearance } from '../appearance';
import { t } from '../i18n';
import { useDesktop } from '../desktop';
import { Field, NumberField, Section, Select, Switch } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// Motion (an accessibility preference) and auto-expand are the appearance axes surfaced as user settings, plain on / off
// switches; the other axes stay LAB design decisions
// Appearance: how the panels render — scheme, motion, diff markers, then the type sizes. Every edit previews in place, this shell included
export function AppearancePage({ settings, appearance, on }: { settings: SettingsView; appearance: Appearance; on: SettingsHandlers }) {
  const desktop = useDesktop();
  return (
    <>
      <Section title={t('settings.group.ui')} desc={t('settings.group.ui.desc')}>
        {/* The desktop app keeps its column on the left and folds it from the title row instead; what it offers is the
            window's layout, the conversation or the files in the middle */}
        {desktop
          ? <Field label={t('settings.desktopLayout')} desc={t('settings.desktopLayout.desc')}>
            <Select options={DESKTOP_LAYOUTS.map(v => ({ value: v, label: t(`settings.desktopLayout.${v}`) }))} value={settings.desktopLayout} onChange={v => on.setSetting('desktopLayout', v)} label={t('settings.desktopLayout')} />
          </Field>
          : <Field label={t('settings.sessionListPosition')} desc={t('settings.sessionListPosition.desc')}>
            <Select options={SESSION_LIST_POSITIONS.map(v => ({ value: v, label: t(`settings.sessionListPosition.${v}` as const) }))} value={settings.sessionListPosition} onChange={v => on.setSetting('sessionListPosition', v)} label={t('settings.sessionListPosition')} />
          </Field>}
        <Field label={t('settings.theme')} desc={t('settings.theme.desc')}>
          <Select options={THEMES.map(v => ({ value: v, label: t(desktop && v === 'auto' ? 'settings.theme.system' : `settings.theme.${v}` as const) }))} value={settings.theme} onChange={v => on.setSetting('theme', v)} label={t('settings.theme')} />
        </Field>
        <Field label={t('settings.motion')} desc={t('settings.motion.desc')}>
          <Switch checked={appearance.motion !== 'none'} onChange={v => on.setAppearance('motion', v ? 'on' : 'none')} label={t('settings.motion')} />
        </Field>
        <Field label={t('settings.autoExpand')} desc={t('settings.autoExpand.desc')}>
          <Switch checked={appearance.autoExpand !== 'off'} onChange={v => on.setAppearance('autoExpand', v ? 'on' : 'off')} label={t('settings.autoExpand')} />
        </Field>
        <Field label={t('settings.diffMarkers')} desc={t('settings.diffMarkers.desc')}>
          <Select options={DIFF_MARKERS.map(v => ({ value: v, label: t(`settings.diffMarkers.${v}` as const) }))} value={settings.diffMarkers} onChange={v => on.setSetting('diffMarkers', v)} label={t('settings.diffMarkers')} />
        </Field>
      </Section>

      <Section title={t('settings.text.title')} desc={t('settings.text.desc')}>
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
