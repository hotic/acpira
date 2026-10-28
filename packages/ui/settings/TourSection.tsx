import { Compass } from 'lucide-react';
import { t } from '../i18n';
import { Button } from '../ui/Button';
import { ItemRow, Section } from './controls';

// Getting started: the interface tour again, over the sample session (welcome/). The desktop's About page and VS Code's
// General page carry it; where it plays is the shell's business (App.tsx startTour)
export function TourSection({ onStart }: { onStart: () => void }) {
  return (
    <Section title={t('welcome.replay.group')}>
      <ItemRow lead={<Compass />} title={t('welcome.replay.title')} desc={t('welcome.replay.desc')}
        trailing={<Button onClick={onStart}>{t('welcome.replay.start')}</Button>} />
    </Section>
  );
}
