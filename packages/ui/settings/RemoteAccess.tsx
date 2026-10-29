import { useEffect, useState } from 'react';
import { Plus, QrCode, RotateCw, Unlink } from 'lucide-react';
import type { DeviceStatus, Relays, RemoteAccessView } from '@acpira/shared/devices';
import { Button } from '../ui/Button';
import { StableText } from '../ui/StableText';
import { t } from '../i18n';
import { deviceOrder, remoteAccessOp } from '../remote/access';
import { DeviceGlyph, KIND_ICON, PairingCore, PathTag, usePairing } from '../remote/Pairing';
import { seenLine } from '../remote/seen';
import { StateCell } from './ConnectionsPage';
import { DangerAction, DialogBody, Dot, Field, ItemRow, Note, Section, SectionAction, Select, SettingsDialog, Switch, TextBox } from './controls';

// Settings → Connections, at the top: remote access (rust/crates/acpira-host/src/devices/). The switch that installs the
// gateway, its relays and keep-awake, then the paired devices; "Add device" opens the pairing core (remote/Pairing.tsx)
// in a dialog, and a device's dialog renames or revokes it. The IDE shells reach pairing only here; the desktop app also
// from its foot card (remote/DevicesCard.tsx). Every edit goes to the engine, which pushes the whole view back

type RelayPick = Relays['mode'];

export function RemoteAccessSection({ view }: { view: RemoteAccessView }) {
  const [pairing, setPairing] = useState(false);
  const [open, setOpen] = useState<string>();
  const gateway = view.gateway;
  const devices = deviceOrder(gateway?.devices ?? []);
  const running = !!gateway;
  // The switch shows the pick at once; the pushed view confirms it
  const [on, setOn] = useState(view.enabled);
  useEffect(() => setOn(view.enabled), [view.enabled]);
  const relay: RelayPick = gateway?.relays.mode ?? 'n0';
  const state = !view.enabled ? 'off' : view.busy ? 'busy' : running ? 'running' : 'stopped';
  const STATE = {
    off: t('devices.allowDesc.off'),
    busy: t('devices.starting'),
    running: t('devices.allowDesc.running', { name: gateway?.name ?? '' }),
    stopped: t('devices.allowDesc.stopped'),
  };
  const device = devices.find(d => d.id === open);
  // A device revoked (here or elsewhere) closes its dialog
  useEffect(() => { if (open && !device) setOpen(undefined); }, [open, device]);
  return <>
    <Section title={t('devices.section')} desc={t('devices.sectionDesc')}
      action={state === 'stopped' ? <SectionAction icon={<RotateCw />} onClick={() => remoteAccessOp({ op: 'enable' })}>{t('devices.restart')}</SectionAction> : undefined}>
      {view.available
        ? <Field label={t('devices.allow')}
          desc={<span className="flex items-center gap-1.5">{state === 'stopped' && <Dot ok={false} warn />}
            <StableText current={state} items={Object.entries(STATE).map(([key, text]) => ({ key, text }))} /></span>}>
          <Switch checked={on} disabled={view.busy} label={t('devices.allow')}
            onChange={v => { setOn(v); remoteAccessOp({ op: v ? 'enable' : 'disable' }); }} />
        </Field>
        : <Note>{t(view.unavailable === 'component' ? 'devices.unavailable.component' : 'devices.unavailable.platform')}</Note>}
      {view.enabled && <Field label={t('devices.relays')} desc={t('devices.relaysDesc')}>
        <Select<RelayPick> label={t('devices.relays')} value={relay}
          onChange={mode => { if (mode !== 'custom') remoteAccessOp({ op: 'setRelays', relays: { mode } }); }}
          options={[
            { value: 'n0', label: t('devices.relays.n0') },
            // Own relays are set in gateway.json; offered only to show them when they are in use
            ...(relay === 'custom' ? [{ value: 'custom' as const, label: t('devices.relays.custom') }] : []),
            { value: 'off', label: t('devices.relays.off') },
          ]} />
      </Field>}
      {view.enabled && <Field label={t('devices.keepAwake')}>
        <Switch checked={view.keepAwake ?? true} label={t('devices.keepAwake')} onChange={v => remoteAccessOp({ op: 'setKeepAwake', on: v })} />
      </Field>}
    </Section>
    {view.enabled && <Section title={t('devices.title')} count={devices.length} desc={t('devices.listDesc')}
      action={<SectionAction icon={<Plus />} disabled={!running} onClick={() => setPairing(true)}>{t('devices.add')}</SectionAction>}>
      {devices.length === 0 && <Note>{t('devices.empty')}</Note>}
      {devices.map(d => <ItemRow key={d.id} lead={<DeviceGlyph kind={d.kind} online={d.online} />} title={d.name} editLabel={d.name}
        desc={d.online && d.path ? <PathTag path={d.path} className="justify-items-start" /> : seenLine(d)}
        trailing={<Presence online={d.online} />} onEdit={() => setOpen(d.id)} />)}
    </Section>}
    {pairing && <PairDialog onClose={() => setPairing(false)} />}
    {device && <DeviceDialog device={device} onClose={() => setOpen(undefined)} />}
  </>;
}

// Online or not at a row's right edge, in the connections page's one state column
function Presence({ online }: { online: boolean }) {
  return <StateCell ok={online} word={online ? 'devices.state.online' : 'devices.state.offline'} />;
}

// Add device: the pairing core in the dialog's body; a device that pairs turns the same block into its success, the
// dialog keeping its size
function PairDialog({ onClose }: { onClose: () => void }) {
  const pairing = usePairing(true);
  // No footer: the ring counts down, the close button ends it, as the desktop's card
  return <SettingsDialog icon={<QrCode />} title={t('devices.add')} onClose={onClose}>
    <DialogBody>
      <div className="dv-on-dialog flex justify-center">
        <PairingCore pairing={pairing} className="w-(--dv-card-w) py-gap" />
      </div>
    </DialogBody>
  </SettingsDialog>;
}

// One device: how it is connected, its name, and revoking it
function DeviceDialog({ device, onClose }: { device: DeviceStatus; onClose: () => void }) {
  const Kind = KIND_ICON[device.kind];
  const since = t('devices.pairedOn', { date: new Date(device.pairedAt).toLocaleDateString() });
  return <SettingsDialog icon={<Kind />} title={device.name} onClose={onClose}
    footer={<>
      <DangerAction icon={<Unlink strokeWidth={1.75} />} onClick={() => { remoteAccessOp({ op: 'revoke', id: device.id }); onClose(); }}>{t('devices.revoke')}</DangerAction>
      <span className="flex-1" />
      <Button variant="primary" onClick={onClose}>{t('common.done')}</Button>
    </>}>
    <DialogBody>
      <Section title={<span className="flex items-center gap-2"><Dot ok={device.online} />{t(device.online ? 'devices.state.online' : 'devices.state.offline')}</span>}
        desc={device.online ? since : `${seenLine(device)}${t('common.metaSep')}${since}`}>
        {device.online && device.path && <Field label={t('devices.pathLabel')}><PathTag path={device.path} className="justify-items-end" /></Field>}
        <Field label={t('devices.name')}>
          <TextBox value={device.name} label={t('devices.name')} onCommit={name => { if (name.trim()) remoteAccessOp({ op: 'rename', id: device.id, name: name.trim() }); }} />
        </Field>
      </Section>
    </DialogBody>
  </SettingsDialog>;
}
