import { CODE_FONT_SIZE, DIFF_MARKERS, THEMES, UI_FONT_SIZE, type SettingsView } from '@shared/settings';
import type { Appearance } from '../appearance';
import { t } from '../i18n';
import { Field, NumberField, Section, Segmented, Switch } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// Motion is the one appearance axis surfaced as a user setting (it is an accessibility preference), a plain on / off switch;
// the other axes stay LAB design decisions
// Appearance: how the panels render — scheme, motion, diff markers, then the type sizes. Every edit previews in place, this shell included
export function AppearancePage({ settings, appearance, on }: { settings: SettingsView; appearance: Appearance; on: SettingsHandlers }) {
  return (
    <>
      <Section>
        <Field label={t('settings.theme')} desc={t('settings.theme.desc')}>
          <Segmented options={THEMES.map(v => ({ value: v, label: t(`settings.theme.${v}` as const) }))} value={settings.theme} onChange={v => on.setSetting('theme', v)} label={t('settings.theme')} />
        </Field>
        <Field label={t('settings.motion')} desc={t('settings.motion.desc')}>
          <Switch checked={appearance.motion !== 'none'} onChange={v => on.setAppearance('motion', v ? 'on' : 'none')} label={t('settings.motion')} />
        </Field>
        <Field label={t('settings.diffMarkers')} desc={t('settings.diffMarkers.desc')}>
          <Segmented options={DIFF_MARKERS.map(v => ({ value: v, label: t(`settings.diffMarkers.${v}` as const) }))} value={settings.diffMarkers} onChange={v => on.setSetting('diffMarkers', v)} label={t('settings.diffMarkers')} />
        </Field>
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
