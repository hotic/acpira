#!/usr/bin/env node
// A minimal stdio MCP server for probes: every request method it receives is appended to $MCP_MARKER,
// so a probe can tell whether an agent really launched a server handed over in session/new `mcpServers`.
// $MCP_LIST_CHANGED_MS: that long after its first tools/list answer the server adds a second tool and sends
// notifications/tools/list_changed (noted as `sent list_changed`), so a probe can see whether the client lists again
import { appendFileSync } from 'node:fs';
import { createInterface } from 'node:readline';

const marker = process.env.MCP_MARKER;
const note = line => marker && appendFileSync(marker, `${line}\n`);
note(`spawned pid=${process.pid}`);

const send = msg => process.stdout.write(`${JSON.stringify(msg)}\n`);
const changeAfter = Number(process.env.MCP_LIST_CHANGED_MS ?? 0);
let listed = false;
let grown = false;

createInterface({ input: process.stdin }).on('line', line => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }
  if (typeof msg.method === 'string') note(msg.method);
  if (msg.id === undefined) return;
  if (msg.method === 'initialize') {
    send({
      jsonrpc: '2.0',
      id: msg.id,
      result: {
        protocolVersion: msg.params?.protocolVersion ?? '2025-06-18',
        capabilities: { tools: changeAfter > 0 ? { listChanged: true } : {} },
        serverInfo: { name: 'acpira-probe', version: '0.0.0' },
      },
    });
  } else if (msg.method === 'tools/list') {
    const tool = { name: 'acpira_probe_ping', description: 'Probe tool; returns pong', inputSchema: { type: 'object', properties: {} } };
    const added = { name: 'acpira_probe_added', description: 'Probe tool added after list_changed', inputSchema: { type: 'object', properties: {} } };
    send({ jsonrpc: '2.0', id: msg.id, result: { tools: grown ? [tool, added] : [tool] } });
    if (changeAfter > 0 && !listed) {
      listed = true;
      setTimeout(() => {
        grown = true;
        send({ jsonrpc: '2.0', method: 'notifications/tools/list_changed' });
        note('sent list_changed');
      }, changeAfter);
    }
  } else if (msg.method === 'tools/call') {
    send({ jsonrpc: '2.0', id: msg.id, result: { content: [{ type: 'text', text: 'pong' }] } });
  } else {
    send({ jsonrpc: '2.0', id: msg.id, result: {} });
  }
});
