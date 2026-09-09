import { spawn, type ChildProcessByStdio } from 'node:child_process';
import { createInterface, type Interface } from 'node:readline';
import { Readable, Writable } from 'node:stream';
import * as acp from '@agentclientprotocol/sdk';
import type { AgentDef } from './AgentRegistry';
import { t } from '../i18n';
import { VERSION } from '../version';
import { approveGrokPlan, GROK_EXIT_PLAN, parseGrokExitPlan } from './grokPlan';
import { GROK_ASK_QUESTION, parseGrokQuestion, type GrokQuestionRequest, type GrokQuestionResponse } from './grokQuestions';

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

// A CLI that ignores the polite signal is force-killed after this long
const KILL_GRACE_MS = 2_000;

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

  // extraEnv: variables injected by the account layer per identity, layered on top of the agent definition's env
  static async spawn(def: AgentDef, binary: string, cwd: string, h: ClientHandlers, extraEnv?: Record<string, string>): Promise<AgentProcess> {
    const box = { h };
    const child = spawn(binary, def.args, {
      cwd,
      env: { ...process.env, ...def.env, ...extraEnv },
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    const stderr = createInterface({ input: child.stderr });
    stderr.on('line', line => box.h.onStderr?.(line));
    child.on('exit', (code, signal) => box.h.onExit?.(code, signal));

    const stream = acp.ndJsonStream(
      Writable.toWeb(child.stdin) as WritableStream<Uint8Array>,
      Readable.toWeb(child.stdout) as ReadableStream<Uint8Array>,
    );
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
      child.once('error', reject);
    });
    const initReq: acp.InitializeRequest = {
      protocolVersion: acp.PROTOCOL_VERSION,
      clientInfo: CLIENT_INFO,
      clientCapabilities: {
        fs: { readTextFile: !!h.onReadFile, writeTextFile: !!h.onWriteFile },
        terminal: false,
        ...(h.onElicitation ? { elicitation: { form: {} } } : {}),
      },
    };
    // A CLI that answers initialize with an error is still running; without this it would sit there as an orphan behind the error notice
    try {
      const init: acp.InitializeResponse = await Promise.race([conn.agent.request(acp.methods.agent.initialize, initReq), exited]);
      return new AgentProcess(def, child, conn, init, stderr, box);
    } catch (e) {
      stderr.close();
      conn.close();
      terminate(child);
      throw e;
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
