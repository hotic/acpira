import type { ChatGptIntegrationStatus } from '@shared/chatgptIntegration';
import { Button } from '../ui/Button';
import { t } from '../i18n';
import { FactRow, Field, Section, SectionDescription } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// External session setup has no model selector or independent agent login.
export function ChatGptPage({ status, on }: { status?: ChatGptIntegrationStatus; on: SettingsHandlers }) {
  const dc = status?.desktopCommander;
  const project = status?.project;
  return <div className="flex min-w-0 flex-col gap-pad" data-chatgpt-settings>
    <SectionDescription>{t('chatgpt.settingsIntro')}</SectionDescription>
    <Section title="Desktop Commander" desc={t('chatgpt.transportDesc')}>
      <FactRow label={t('chatgpt.installation')}><span>{t(dc ? `chatgpt.install.${dc.installation}` : 'chatgpt.checking')}</span></FactRow>
      <FactRow label={t('chatgpt.process')}><span>{t(dc ? `chatgpt.process.${dc.process}` : 'chatgpt.checking')}</span></FactRow>
      <Field label={t('chatgpt.pairing')} desc={t('chatgpt.pairingHint')}>
        <span className="text-2 text-fg-2">{t('chatgpt.pairingUnknown')}</span>
      </Field>
      <Field label={t('chatgpt.setup')}>
        <Button onClick={() => on.openExternal('https://mcp.desktopcommander.app')}>{t('chatgpt.dashboard')}</Button>
        <Button onClick={() => on.openExternal('https://github.com/desktop-commander/remote-desktop-commander/blob/main/docs/SETUP.md')}>{t('chatgpt.setupGuide')}</Button>
      </Field>
    </Section>
    <Section title={t('chatgpt.projectBinding')} desc={t('chatgpt.projectBindingHint')}>
      <FactRow label={t('chatgpt.receiver')}><span>{t(status ? status.bridgeAvailable ? 'chatgpt.receiverReady' : 'chatgpt.receiverMissing' : 'chatgpt.checking')}</span></FactRow>
      <FactRow label={t('chatgpt.boundSessions')}><span>{project ? project.observedMirrors : '—'}</span></FactRow>
      <FactRow label={t('chatgpt.lastEvent')}><span>{project?.lastEventAt ? new Date(project.lastEventAt).toLocaleString() : t('chatgpt.noEvent')}</span></FactRow>
      <Field label={project?.observedMirrors ? t('chatgpt.hasSession') : t('chatgpt.unbound')}>
        {project?.latestSessionId && <Button variant="primary" onClick={() => on.openChatgpt?.(project.latestSessionId!)}>{t('chatgpt.openMirror')}</Button>}
        <Button variant={project?.latestSessionId ? 'secondary' : 'primary'} disabled={!status?.bridgeAvailable || !on.connectChatgpt}
          onClick={() => on.connectChatgpt?.()}>{t('chatgpt.createBinding')}</Button>
      </Field>
    </Section>
    <SectionDescription>{t('chatgpt.sourceBoundary')}</SectionDescription>
    {status && <p className="m-0 text-3 text-fg-3">{t('chatgpt.checkedAt')} {new Date(status.checkedAt).toLocaleTimeString()}</p>}
  </div>;
}
