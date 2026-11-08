import { i18n } from '@kit.LocalizationKit';
import { common } from '@kit.AbilityKit';
import { AppModel } from '../store/AppModel';
import { initL10n } from '../store/L10n';
import { PairingModel, PairLink } from '../store/PairingModel';
import { PairingService, Presence } from '../remote/PairingService';
import { ConnectionError } from '../remote/PairedTransport';
import { PairedComputer } from '../remote/Stores';
import { ComputersPage, PairConnect } from './Pairing';
import { PairSheet } from './PairSheet';
import { SessionList } from './SessionList';
import { WelcomeFlow } from './Welcome';

// The pairing screens as phone-wide frames side by side for the Previewer (scripts/preview.sh <out> 3780 1170 dark
// pages/PreviewPair 240): the connect frame empty and offline, the add sheet, the code page, the computers page, the
// first run's pairing step and the list following a computer. The Previewer has no remote access library, so the camera
// card shows its unavailable state and nothing dials. Not reached from the app's own pages.

/** A phone's width in vp (1080 px at 480 dpi) */
const PHONE: number = 360;

const STUDIO: PairedComputer = { id: 'a1b2c3d4e5f6', name: 'Mac Studio', addrs: ['192.168.1.20:7790'], pairedAt: 0 };
const AIR: PairedComputer = { id: 'f6e5d4c3b2a1', name: 'MacBook Air', addrs: [], pairedAt: 0 };

@ComponentV2
struct PairScene {
  @Param scene: string = 'none';
  @Local model: AppModel = new AppModel();
  @Local pairing: PairingModel | undefined = undefined;

  aboutToAppear(): void {
    const model = new AppModel();
    model.attach(undefined, i18n.System.getSystemLanguage());
    model.demo();
    // Preferences do not open here: the service's store lives in memory and nothing is dialed (no library)
    const pairing = new PairingModel(new PairingService(this.getUIContext().getHostContext() as common.Context));
    pairing.loaded = true;
    pairing.pinnedNearby = { id: STUDIO.id, name: STUDIO.name, addrs: STUDIO.addrs };
    if (this.scene !== 'none' && this.scene !== 'sheet' && this.scene !== 'code' && this.scene !== 'welcome') {
      pairing.computers = [STUDIO, AIR];
      const away: Presence = { kind: 'offline' };
      pairing.presence = new Map<string, Presence>([[AIR.id, away]]);
    }
    if (this.scene === 'offline') {
      pairing.link = new PairLink('failed', undefined, new ConnectionError('Offline', ''));
    } else if (this.scene === 'computers' || this.scene === 'list') {
      pairing.link = new PairLink('online', 'lan');
    }
    model.pairing = pairing;
    this.model = model;
    this.pairing = pairing;
  }

  build() {
    Stack({ alignContent: Alignment.Top }) {
      if (this.pairing !== undefined) {
        if (this.scene === 'none' || this.scene === 'offline') {
          PairConnect({ pairing: this.pairing })
        } else if (this.scene === 'sheet') {
          PairSheet({ pairing: this.pairing })
        } else if (this.scene === 'code') {
          PairSheet({ pairing: this.pairing, start: 'code' })
        } else if (this.scene === 'computers') {
          ComputersPage({ pairing: this.pairing })
        } else if (this.scene === 'welcome') {
          WelcomeFlow({ model: this.model, start: 'pair' })
        } else if (this.scene === 'list') {
          SessionList({ model: this.model })
        }
      }
    }
    .width(PHONE)
    .height('100%')
    .clip(true)
    .backgroundColor($r('app.color.bg0'))
  }
}

@Entry
@ComponentV2
struct PreviewPair {
  aboutToAppear(): void {
    initL10n(i18n.System.getSystemLanguage());
  }

  build() {
    Row() {
      ForEach(['none', 'offline', 'sheet', 'code', 'computers', 'welcome', 'list'], (scene: string) => {
        PairScene({ scene: scene })
      }, (scene: string) => scene)
    }
    .height('100%')
    .alignItems(VerticalAlign.Top)
  }
}
