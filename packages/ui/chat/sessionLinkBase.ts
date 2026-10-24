import { SESSION_LINK_PREFIX } from '@acpira/shared/sessionLinks';

// The base a copied session link starts with: the VS Code shell injects its clickable one (`vscode://hotic.acpira/session/`,
// `cursor://…` under Cursor); IDEA, the harness and the LAB copy the canonical `acpira://session/`
export function sessionLinkBase(): string {
  const base = window.__acpira?.sessionLinkBase;
  return typeof base === 'string' && base.endsWith('/session/') ? base : SESSION_LINK_PREFIX;
}
