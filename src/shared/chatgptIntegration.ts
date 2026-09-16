// Transport facts are separate from conversation activity and cloud account authorization.
export interface ChatGptIntegrationStatus {
  checkedAt: string;
  bridgeAvailable: boolean;
  desktopCommander: {
    installation: 'detected' | 'not_detected' | 'unknown';
    process: 'detected' | 'not_detected' | 'unknown';
    // No cloud credentials are read to manufacture a pairing result.
    pairing: 'unknown';
    evidence: 'process' | 'executable' | 'configuration' | 'none';
  };
  project: {
    mirrors: number;
    observedMirrors: number;
    latestSessionId?: string;
    lastEventAt?: string;
  };
}
