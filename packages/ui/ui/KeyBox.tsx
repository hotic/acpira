import { useState } from 'react';
import { Check, Eye, EyeOff } from 'lucide-react';
import { IconButton } from './Button';
import { cn } from './cn';
import { t } from '../i18n';

// A secret typed into a box: sent with the check or Enter, then the box empties, since the page never gets a stored
// secret back. `sendOnPaste` boxes have no check: a key is pasted far more often than typed, so a paste sends it at once
// (Enter still sends a typed one). `disabled` holds the send while the last one is still being handled; Escape calls
// `onCancel`. The visibility toggle and the check sit inside the box, so it keeps the width its place gives it (a
// settings column by default, `className` elsewhere); `sm` matches the dock card's small buttons
export function KeyBox({ label, placeholder, disabled, autoFocus, size, className, sendOnPaste, onSave, onCancel }: {
  label: string; placeholder: string; disabled?: boolean; autoFocus?: boolean; size?: 'sm'; className?: string; sendOnPaste?: boolean;
  onSave: (key: string) => void; onCancel?: () => void;
}) {
  const [key, setKey] = useState('');
  const [show, setShow] = useState(false);
  const save = (value = key) => {
    const k = value.trim();
    if (!k || disabled) return;
    onSave(k);
    setKey('');
    setShow(false);
  };
  const eyeLabel = show ? t('settings.providers.hideKey') : t('settings.providers.showKey');
  return (
    <div className={cn('flex w-(--ctl-w) max-w-full min-w-0 items-center rounded-md border border-line bg-hover pl-3 transition-colors focus-within:bg-active hover:border-line-strong',
      size === 'sm' ? 'h-ctl-sm' : 'h-ctl', className)}>
      <input type={show ? 'text' : 'password'} autoComplete="off" spellCheck={false} autoFocus={autoFocus} value={key}
        onChange={e => {
          setKey(e.target.value);
          if (sendOnPaste && (e.nativeEvent as InputEvent).inputType === 'insertFromPaste') save(e.target.value);
        }}
        onKeyDown={e => { if (e.key === 'Enter') save(); else if (e.key === 'Escape' && onCancel) { e.stopPropagation(); onCancel(); } }}
        aria-label={label} placeholder={placeholder}
        className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
      <IconButton size="sm" title={eyeLabel} aria-label={eyeLabel} aria-pressed={show} onClick={() => setShow(v => !v)} className="text-fg-3">
        {show ? <EyeOff strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
      </IconButton>
      {!sendOnPaste && <IconButton size="sm" title={t('settings.providers.save')} aria-label={t('settings.providers.save')} disabled={disabled || !key.trim()} onClick={() => save()} className="mr-0.5 text-fg-2 disabled:opacity-40">
        <Check strokeWidth={1.75} />
      </IconButton>}
    </div>
  );
}
