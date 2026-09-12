# Context usage and automatic compaction

## Sources and ownership

| Value | Source | Meaning |
| --- | --- | --- |
| Context used / size | ACP `usage_update`; Grok `_x.ai/session/info` fallback | Latest native context snapshot and model window |
| Automatic threshold | Host platform `compactAtTokens`, default 300,000 | Acpira's turn-boundary `/compact` policy |
| History categories | Retained transcript text estimates | Full displayed history, including messages already compacted by the CLI |

The threshold does not configure the CLI's own compactor, enlarge the model window, or interrupt an active `session/prompt`. A threshold at or above the reported model window is flagged in the panel. Native compaction can happen earlier. The context ring and panel percentage both use the reported model window; history estimates never override that total or drive automatic compaction.

VS Code and Cursor read their own `acpira.*` settings. IntelliJ persists an application-level settings document and forwards it to `SidecarPlatform`. Both entry points construct the same `HostRuntime`, `AcpSession`, and webview. Shared logic does not imply shared IDE settings or that an installed plugin already contains the latest source changes.

## Refresh and ordering

1. Standard usage notifications replace the displayed snapshot, including reductions after native compaction.
2. Grok polls context throughout a running request, including intervals with no streamed text, then refreshes at the turn boundary. Poll requests are serialized with the final refresh; standard notifications supersede this fallback.
3. Kimi 0.41.0 emits usage asynchronously after acknowledging an ordinary prompt. With auto-compaction enabled, the host holds the turn and queue until that notification, with a five-second bound for unavailable usage. Cancellation and disposal release the wait. Late usage is also checked after a completed user turn.
4. Once reported usage meets the enabled threshold and `/compact` is advertised, automatic compaction runs before queued work. The existing completion latch waits for Devin/Kimi background completion or structured compaction events.
5. A missing usage notification is not replaced with transcript estimates. The UI identifies totals as the latest agent report. Some CLIs refresh only after model calls or the next ordinary turn.

## Verified on 2026-09-12

| CLI | Version | Window reported by the default probe session | Real automatic dispatch and follow-up |
| --- | --- | --- | --- |
| Grok | 1.0.18 | 250,000 | Passed |
| Devin | 3000.10.21 | 262,000 | Passed |
| Kimi Code | 0.41.0 | 262,144 | Passed |

These are observed sessions, not universal model limits. All three windows were below 300,000. Small synthetic conversations exercised automatic dispatch at an explicitly lowered threshold and verified completion plus `FOLLOWUP_OK`; they do not constitute a real 300,000-token load test. Fake ACP regression tests separately exercise the configured 300,000 threshold, including a 500,000 → 300,000 settings change shared by sidebar and editor views.

```sh
pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-context-policy.ts grok --exercise
pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-context-policy.ts devin --after-turn
pnpm exec tsx --tsconfig tsconfig.host.json scripts/probe-context-policy.ts kimi --after-turn
pnpm exec vitest run test/usage-breakdown.test.ts test/grok-usage.test.ts test/compaction-queue.test.ts test/hostRuntime.test.ts
pnpm exec vite --config vite.lab.config.ts --port 5207
```

`lab/context.preview.html` retains approximately 996K tool-history tokens while reporting a 24K / 200K native context. Running/idle transitions must remain at 12%. The over-threshold scene reports 350K / 1M, displays 35% consistently, and explains that compaction waits for the turn boundary. Both scenes support light/dark and narrow-screen checks.
