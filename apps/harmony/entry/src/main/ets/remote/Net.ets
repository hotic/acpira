import { hilog } from '@kit.PerformanceAnalysisKit';
import { Module } from '../../cpp/types/libacpira_mobile_ohos/index';

export { Computer, DialReport, Greeting, Link, LinkEvent, Module, NamedComputer, Node, Relays } from '../../cpp/types/libacpira_mobile_ohos/index';

// The paired-remote library (rust/crates/acpira-mobile-ohos, `libacpira_mobile_ohos.so` from scripts/build-net.sh) as
// ArkTS sees it: the N-API surface of its src/napi.rs, typed by hand in cpp/types/libacpira_mobile_ohos/index.d.ts.
// It is loaded at run time, so a build without the library (scripts/build-net.sh not run, the Previewer) still runs:
// `net()` is undefined there and the app keeps the demo and the engine by address. Every failure is an Error whose
// message is `Kind` or `Kind: detail` (src/bridge.rs).

const DOMAIN = 0xacb1;
const TAG = 'AcpiraPair';

let loaded: Module | undefined = undefined;
let tried: boolean = false;

/** The library, or undefined when this build has none */
export function net(): Module | undefined {
  if (tried) {
    return loaded;
  }
  tried = true;
  try {
    // By OHM URL: a bare `libacpira_mobile_ohos.so` must be a dependency in oh-package.json5 (which needs ohpm) or the
    // runtime refuses the name. This form names the HAP's own libs/<abi> library whatever the bundle
    const m = loadNativeModule('@normalized:Y&&&libacpira_mobile_ohos.so&') as Module;
    if (m !== undefined && m !== null && typeof m.startNode === 'function') {
      loaded = m;
    }
  } catch (e) {
    hilog.info(DOMAIN, TAG, 'no remote access library: %{public}s', `${e}`);
  }
  return loaded;
}

/** The failure's kind: the message up to its first ": " (`Offline`, `Incompatible`, …) */
export function failureKind(e: Object | undefined): string {
  const text = failureText(e);
  const at = text.indexOf(': ');
  return at < 0 ? text : text.slice(0, at);
}

/** The failure's detail after its kind, or the whole message when it has none */
export function failureDetail(e: Object | undefined): string {
  const text = failureText(e);
  const at = text.indexOf(': ');
  return at < 0 ? text : text.slice(at + 2);
}

function failureText(e: Object | undefined): string {
  if (e instanceof Error) {
    return e.message;
  }
  return e === undefined ? 'Other' : `${e}`;
}
