import { Download } from 'lucide-react';
import { Button } from '../ui/Button';
import { t } from '../i18n';
import { DialogBody, SettingsDialog } from './controls';

export const NODEJS_DOWNLOAD_URL = 'https://nodejs.org/';

// Confirmation dialog shown when an npm-backed agent install fails because Node.js / npm is missing: explains that
// Acpira downloads an official portable Node.js LTS into ~/.acpira/runtime/node without changing the system PATH, or
// opens nodejs.org for a manual install
export function NodeInstallDialog({ agentName, onConfirm, onManual, onClose }: {
  agentName: string;
  onConfirm: () => void;
  onManual?: () => void;
  onClose: () => void;
}) {
  return (
    <SettingsDialog icon={<Download />} title={t('settings.install.nodeTitle')} onClose={onClose}
      footer={<>
        {onManual && <Button onClick={() => { onManual(); onClose(); }}>{t('settings.install.nodeManual')}</Button>}
        <span className="flex-1" />
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        <Button variant="primary" onClick={() => { onConfirm(); onClose(); }}>{t('settings.install.nodeConfirm')}</Button>
      </>}>
      <DialogBody>
        <p className="m-0 text-2 leading-(--text-2-lh) text-fg-2 [overflow-wrap:anywhere]">
          {t('settings.install.nodePrompt', { agent: agentName })}
        </p>
      </DialogBody>
    </SettingsDialog>
  );
}
