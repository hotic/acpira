// The browser stand-in for an IDE shell: speaks the envelope protocol (src/shared/sidecar.ts) to the sidecar over /ws, presents
// exactly the transport the webview bundle expects (window.__acpiraApi.postMessage up, window `message` events down), and answers
// the platform requests a shell must (writeSetting, toast, openExternal); the rest is logged. Plain JS on purpose: nothing to build.
//
// Query string: ?host=editor (default sidebar) · ?theme=light (default dark) · ?agent=<id> (default setting) · ?cwd=<abs path>
(() => {
  const params = new URLSearchParams(location.search);
  const host = params.get('host') === 'editor' ? 'editor' : 'sidebar';
  if (params.get('theme') === 'light') document.body.className = 'vscode-light';
  window.__acpira = { host };

  // Settings live in the shell (an IDE would persist them); a snapshot of flat acpira.* keys goes with hello
  const SETTINGS_KEY = 'acpira-host-preview.settings';
  const settings = Object.assign({ defaultAgent: 'grok' }, JSON.parse(localStorage.getItem(SETTINGS_KEY) || '{}'));
  if (params.get('agent')) settings.defaultAgent = params.get('agent');
  const persist = () => localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));

  const VIEW_ID = 'preview';
  const pendingUp = [];
  let ws;
  let attached = false;

  const send = m => ws.send(JSON.stringify(m));
  const toast = (level, text) => {
    const el = document.createElement('div');
    el.className = level;
    el.textContent = text;
    document.getElementById('harness-toasts').appendChild(el);
    setTimeout(() => el.remove(), 8000);
    console[level === 'error' ? 'error' : 'info']('[shell toast]', text);
  };

  // The bundle posts `ready` the moment App mounts, usually before the socket is open: queue until the view is attached
  window.__acpiraApi = {
    postMessage(message) {
      if (attached) send({ type: 'webviewMessage', viewId: VIEW_ID, message });
      else pendingUp.push(message);
    },
  };

  const deliver = message => window.dispatchEvent(new MessageEvent('message', { data: message }));

  const respond = (requestId, result, error) => {
    if (!requestId) return;
    send(error ? { type: 'platformResponse', requestId, error } : { type: 'platformResponse', requestId, result });
  };

  const onPlatformRequest = ({ requestId, request }) => {
    switch (request.method) {
      case 'toast': toast(request.level, request.text); return;
      case 'openExternal': window.open(request.url, '_blank', 'noopener'); return;
      case 'writeSetting': {
        settings[request.key] = request.value;
        persist();
        respond(requestId, null);
        send({ type: 'platformEvent', event: { type: 'settingsChanged', keys: [request.key], settings } });
        return;
      }
      case 'openResolvedFile': toast('info', `open ${request.path}${request.line ? `:${request.line}` : ''}`); respond(requestId, null); return;
      case 'openPlanDocument': toast('info', 'path' in request.target ? `open plan ${request.target.path}` : 'open plan (markdown)'); respond(requestId, null); return;
      case 'revealInOS': toast('info', `reveal ${request.path}`); respond(requestId, null); return;
      case 'runInTerminal': toast('info', `terminal: ${[request.command, ...request.args].join(' ')}`); return;
      case 'openInEditor': window.open(`${location.pathname}?host=editor${request.sessionId ? `&session=${request.sessionId}` : ''}`, '_blank'); return;
      default: respond(requestId, undefined, `${request.method} is not supported by the browser harness`);
    }
  };

  const token = params.get('token');
  const connect = () => {
    if (!token) {
      toast('error', 'Missing harness token — open the URL printed by pnpm harness');
      return;
    }
    ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws?token=${encodeURIComponent(token)}`);
    ws.onopen = () => {
      send({
        type: 'hello', protocolVersion: 1, requestId: 'hello-1',
        client: { name: 'host-preview', version: '0', capabilities: ['toast', 'openExternal', 'writeSetting', 'openResolvedFile', 'openPlanDocument', 'revealInOS', 'runInTerminal', 'openInEditor'] },
        env: { hostLanguage: navigator.language, cwd: params.get('cwd') || undefined, blobBase: `${location.origin}/blobs` },
        settings,
      });
    };
    ws.onmessage = e => {
      for (const line of String(e.data).split('\n')) {
        if (!line.trim()) continue;
        const m = JSON.parse(line);
        switch (m.type) {
          case 'helloOk': {
            const session = params.get('session');
            send({ type: 'attachView', viewId: VIEW_ID, host, initial: session || (host === 'sidebar' ? { mostRecent: true } : undefined) });
            attached = true;
            for (const message of pendingUp.splice(0)) send({ type: 'webviewMessage', viewId: VIEW_ID, message });
            break;
          }
          case 'helloReject': toast('error', `sidecar rejected hello: ${m.reason}`); break;
          case 'hostMessage': if (m.viewId === VIEW_ID) deliver(m.message); break;
          case 'platformRequest': onPlatformRequest(m); break;
        }
      }
    };
    ws.onclose = () => { attached = false; toast('error', 'sidecar connection closed — reload the page'); };
  };
  window.addEventListener('focus', () => { if (attached) send({ type: 'platformEvent', event: { type: 'windowFocus' } }); });
  connect();
})();
