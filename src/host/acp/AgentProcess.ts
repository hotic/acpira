import { spawn, type ChildProcessByStdio } from 'node:child_process';
import { createInterface, type Interface } from 'node:readline';
import { Readable, Writable } from 'node:stream';
import * as acp from '@agentclientprotocol/sdk';
import type { AgentDef } from './AgentRegistry';
import { t } from '../i18n';
import { VERSION } from '../version';
import { approveGrokPlan, GROK_EXIT_PLAN, parseGrokExitPlan } from './grokPlan';
import { GROK_ASK_QUESTION, parseGrokQuestion, type GrokQuestionRequest, type GrokQuestionResponse } from './grokQuestions';
import { spawnSpec } from './launch';
import { rewriteExtension } from './subagents/wire';

// What the client side has to accept: updates / permission requests / file reads & writes / questions the agent sends on its own initiative.
// The optional handlers double as capability switches: a handler present is advertised in initialize, an absent one answers method-not-found
export interface ClientHandlers {
  onUpdate: (n: acp.SessionNotification) => void;
  onPermission: (req: acp.RequestPermissionRequest, signal: AbortSignal) => Promise<acp.RequestPermissionResponse>;
  onReadFile?: (req: acp.ReadTextFileRequest) => Promise<acp.ReadTextFileResponse>;
  onWriteFile?: (req: acp.WriteTextFileRequest) => Promise<void>;
  // Form elicitation (elicitation/create with mode=form): the agent asks the user for structured input, e.g. Devin's / Kimi's ask_user_question
  onElicitation?: (req: acp.CreateElicitationRequest, signal: AbortSignal) => Promise<acp.CreateElicitationResponse>;
  // Grok's private question request (`_x.ai/ask_user_question`); without a handler the CLI reports the tool as failed with method-not-found
  onGrokQuestion?: (req: GrokQuestionRequest, signal: AbortSignal) => Promise<GrokQuestionResponse>;
  onStderr?: (line: string) => void;
  onExit?: (code: number | null, signal: NodeJS.Signals | null) => void;
}

export const CLIENT_INFO = { name: 'acpira', version: VERSION };

// The executable would not spawn at all (ENOENT, EACCES, a bad shim): the child 'error' event fires before any
// output. Kept distinct from a process that started and then failed the handshake so health can name the stage
export class AgentSpawnError extends Error {
  constructor(readonly cause: Error) {
    super(cause.message, { cause });
    this.name = 'AgentSpawnError';
  }
}

// A CLI that ignores the polite signal is force-killed after this long (DSH's own graceful-exit window is 5 s)
const KILL_GRACE_MS = 5_000;

// initialize has to answer within this long; a CLI that hangs the handshake would otherwise stall the session forever
const INIT_TIMEOUT_MS = 30_000;

// One agent subprocess = one long-lived ACP connection. stdio carries ndjson; stderr goes line by line to the Output Channel.
// Handlers are rebindable so a warm (initialize-only) process can be handed to a session without a second spawn
export class AgentProcess {
  private constructor(
    readonly def: AgentDef,
    readonly child: ChildProcessByStdio<Writable, Readable, Readable>,
    readonly conn: acp.ClientConnection,
    readonly init: acp.InitializeResponse,
    private readonly stderr: Interface,
    private readonly box: { h: ClientHandlers },
  ) {}

  get agent(): acp.ClientContext { return this.conn.agent; }
  get alive(): boolean { return this.child.exitCode === null && !this.child.killed; }

  bind(h: ClientHandlers) { this.box.h = h; }

  // extraEnv: variables injected by the account layer per identity, layered on top of the agent definition's env.
  // opts.initTimeoutMs bounds the initialize handshake (default INIT_TIMEOUT_MS)
  static async spawn(def: AgentDef, binary: string, cwd: string, h: ClientHandlers, extraEnv?: Record<string, string>, opts?: { initTimeoutMs?: number }): Promise<AgentProcess> {
    const box = { h };
    const spec = spawnSpec(binary, def.args, process.platform, process.env);
    const child = spawn(spec.command, spec.args, {
      cwd,
      env: { ...process.env, ...def.env, ...extraEnv },
      stdio: ['pipe', 'pipe', 'pipe'],
      ...(spec.windowsVerbatimArguments ? { windowsVerbatimArguments: true } : {}),
    });
    const stderr = createInterface({ input: child.stderr });
    stderr.on('line', line => box.h.onStderr?.(line));
    child.on('exit', (code, signal) => box.h.onExit?.(code, signal));

    const raw = acp.ndJsonStream(
      Writable.toWeb(child.stdin) as WritableStream<Uint8Array>,
      Readable.toWeb(child.stdout) as ReadableStream<Uint8Array>,
    );
    // Extension session/update kinds are rewritten into session_info_update before the SDK's closed-union parse drops them
    const stream: acp.Stream = {
      writable: raw.writable,
      readable: raw.readable.pipeThrough(new TransformStream<acp.AnyMessage, acp.AnyMessage>({
        transform(msg, controller) { controller.enqueue(rewriteExtension(msg) as acp.AnyMessage); },
      })),
    };
    const app = acp.client({ name: CLIENT_INFO.name })
      .onNotification(acp.methods.client.session.update, ctx => { box.h.onUpdate(ctx.params); })
      .onRequest(acp.methods.client.session.requestPermission, ctx => box.h.onPermission(ctx.params, ctx.signal))
      .onRequest(GROK_EXIT_PLAN, parseGrokExitPlan, ctx => approveGrokPlan(ctx.params, ctx.signal, box.h.onPermission))
      .onRequest(GROK_ASK_QUESTION, parseGrokQuestion, ctx => {
        if (!box.h.onGrokQuestion) throw acp.RequestError.methodNotFound(GROK_ASK_QUESTION);
        return box.h.onGrokQuestion(ctx.params, ctx.signal);
      })
      .onRequest(acp.methods.client.fs.readTextFile, ctx => {
        if (!box.h.onReadFile) throw acp.RequestError.methodNotFound(acp.methods.client.fs.readTextFile);
        return box.h.onReadFile(ctx.params);
      })
      .onRequest(acp.methods.client.fs.writeTextFile, async ctx => {
        if (!box.h.onWriteFile) throw acp.RequestError.methodNotFound(acp.methods.client.fs.writeTextFile);
        await box.h.onWriteFile(ctx.params);
        return {};
      })
      .onRequest(acp.methods.client.elicitation.create, ctx => {
        if (!box.h.onElicitation) throw acp.RequestError.methodNotFound(acp.methods.client.elicitation.create);
        return box.h.onElicitation(ctx.params, ctx.signal);
      });
    const conn = app.connect(stream);

    const exited = new Promise<never>((_, reject) => {
      child.once('exit', (code, signal) => reject(new Error(t('host.spawnExited', { command: def.command, code: code ?? '-', signal: signal ?? '-' }))));
      child.once('error', e => reject(new AgentSpawnError(e)));
    });
    const initReq = {
      protocolVersion: acp.PROTOCOL_VERSION,
      clientInfo: CLIENT_INFO,
      clientCapabilities: {
        fs: { readTextFile: !!h.onReadFile, writeTextFile: !!h.onWriteFile },
        terminal: false,
        // The host can reproduce the agent's invocation in an interactive terminal, so `type: 'terminal'`
        // authMethods may be offered (claude-agent-acp only advertises its logins to clients that declare this);
        // an agent whose ACP process ignores the local login opts out (AgentDef.auth.terminal, Devin)
        ...(def.auth?.terminal === false ? {} : { auth: { terminal: true } }),
        ...(h.onElicitation ? { elicitation: { form: {} } } : {}),
        // ACP boolean session config options (RFD boolean-config-option); the codex / claude adapters degrade
        // an option like Codex's fast-mode to an on/off select when the client does not declare this
        session: { configOptions: { boolean: {} } },
        ...(def.subagents === false ? {} : { subagents: {} }),
        _meta: {
          // codex-acp streams shell output as _meta.terminal_output_delta and only then drops its JSON rawOutput receipt;
          // claude-agent-acp switches Bash to an agent-managed terminal on either terminal_output flag
          terminal_output_delta: true,
          // JetBrains AIR bridge (RFD #1992 + adapter extensions): nativeSubagentSessions follows the def's subagents
          // gate; sessionFailure and asyncTasks are independent capabilities the host always supports.
          // recommendedValue: claude-agent-acp drops its placeholder `default` effort row (and the `default` model row when
          // a concrete model matches it) and reports concrete current values instead
          jetbrains: { air: { version: 1, capabilities: [...(def.subagents === false ? [] : ['nativeSubagentSessions']), 'sessionFailure', 'asyncTasks', 'recommendedValue'] } },
        },
      },
    } as acp.InitializeRequest;
    const initTimeoutMs = opts?.initTimeoutMs ?? INIT_TIMEOUT_MS;
    let initTimer: ReturnType<typeof setTimeout> | undefined;
    const timedOut = new Promise<never>((_, reject) => {
      initTimer = setTimeout(() => reject(new Error(t('host.initTimeout', { command: def.command, seconds: initTimeoutMs / 1000 }))), initTimeoutMs);
    });
    // A CLI that answers initialize with an error is still running; without this it would sit there as an orphan behind the error notice
    try {
      const init: acp.InitializeResponse = await Promise.race([conn.agent.request(acp.methods.agent.initialize, initReq), exited, timedOut]);
      return new AgentProcess(def, child, conn, init, stderr, box);
    } catch (e) {
      stderr.close();
      conn.close();
      terminate(child);
      throw e;
    } finally {
      clearTimeout(initTimer);
    }
  }

  kill() {
    const exited = this.child.exitCode !== null || this.child.signalCode !== null
      ? Promise.resolve()
      : new Promise<void>(resolve => { this.child.once('exit', () => resolve()); });
    this.stderr.close();
    this.conn.close();
    terminate(this.child);
    return exited;
  }
}

function terminate(child: ChildProcessByStdio<Writable, Readable, Readable>) {
  if (child.exitCode !== null || child.killed) return;
  child.kill();
  const force = setTimeout(() => { if (child.exitCode === null) child.kill('SIGKILL'); }, KILL_GRACE_MS);
  force.unref();
  child.once('exit', () => clearTimeout(force));
}
