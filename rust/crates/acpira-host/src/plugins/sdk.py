# Acpira plugin SDK for Python: a resident plugin process (`contributes.runtime`) in one file, standard library only,
# Python 3.8 or newer. `acpira plugin new <id> --lang python` copies it into the plugin folder, where it is frozen with
# the plugin; the wire it speaks is docs/dev/plugins.md, Runtime (ndjson JSON-RPC 2.0 on stdin / stdout).
#
#   from acpira_plugin import Plugin
#   p = Plugin()
#
#   @p.hook("prompt")
#   def prompt(payload, ctx):
#       return {"context": "Deploys go through `make ship`."} if "deploy" in payload["text"] else None
#
#   @p.on("turn.ended")
#   def ended(event):
#       p.storage.set("last", event["data"]["session"])
#
#   @p.method("ping")
#   def ping(params, ctx):
#       return "pong"
#
#   @p.schedule("daily")
#   def daily(ctx):
#       p.ui.toast("Morning")
#
#   p.run()
#
# Every name used here must also be declared in plugin.json (`hooks`, `events`, `methods`, `tools`, `schedules`): the
# engine calls only what the manifest lists, which is what the user confirmed. Handlers run on worker threads, so one
# may wait on a request to the engine (`p.storage.get`) without holding up the others.

import json
import os
import sys
import threading
from concurrent.futures import Future, ThreadPoolExecutor

PROTOCOL_VERSION = 1


class EngineError(Exception):
    """An error answer of the engine to one of the plugin's requests"""

    def __init__(self, message, code=None):
        super().__init__(message)
        self.code = code


class _Calls:
    """A group of engine calls under one prefix (`storage/get`, ...)"""

    def __init__(self, table):
        for name, fn in table.items():
            setattr(self, name, fn)


class Plugin:
    def __init__(self, input=None, output=None, keep_print=False):
        self._in = input or sys.stdin
        self._out = output or sys.stdout
        # print() would corrupt the wire: it goes to stderr, Acpira's log
        if output is None and not keep_print:
            sys.stdout = sys.stderr
        self._write = threading.Lock()
        self._hooks = {}
        self._methods = {}
        self._tools = {}
        self._schedules = {}
        self._listeners = []
        self._closers = []
        self._waits = {}
        self._waits_lock = threading.Lock()
        self._cancelled = {}
        self._next = 1
        self._pool = ThreadPoolExecutor(max_workers=8)
        self.info = None
        r = self.request
        self.storage = _Calls({
            "get": lambda key: (r("storage/get", {"key": key}) or {}).get("value"),
            "set": lambda key, value: r("storage/set", {"key": key, "value": value}),
            "delete": lambda key: r("storage/delete", {"key": key}),
            "list": lambda prefix="": (r("storage/list", {"prefix": prefix}) or {}).get("keys", []),
        })
        self.session = _Calls({
            "list": lambda cwd=None: (r("session/list", {"cwd": cwd} if cwd else {}) or {}).get("sessions", []),
            "get": lambda session, turns=None: (r("session/get", {"session": session, **({} if turns is None else {"turns": turns})}) or {}).get("session"),
            "act": lambda message: r("session/act", {"message": message}),
            "send": lambda session_id, text: r("session/act", {"message": {"type": "send", "sessionId": session_id, "text": text}}),
        })
        self.ui = _Calls({
            "toast": lambda text, level="info": r("ui/toast", {"text": text, "level": level}),
            "ask": lambda **params: r("ui/ask", params),
            "post": lambda message: r("ui/post", {"message": message}),
            "composer": lambda text, replace=False, session=None: r("ui/composer", {"text": text, "replace": replace, **({"session": session} if session else {})}),
        })
        self.model = _Calls({"generate": lambda prompt, **rest: r("model/generate", {"prompt": prompt, **rest})})
        self.providers = _Calls({"set": lambda providers: r("providers/set", {"providers": providers})})

    # Registration

    def hook(self, name):
        """Answer a hook: `fn(payload, ctx)` returns the fields to change, or None for no opinion"""
        def wrap(fn):
            self._hooks[name] = fn
            return fn
        return wrap

    def on(self, pattern):
        """Hear events: an exact name (`turn.ended`, `text.delta`), a family (`tool.*`) or `*`"""
        def wrap(fn):
            self._listeners.append((pattern, fn))
            return fn
        return wrap

    def method(self, name):
        """Offer a method to pages and other plugins: `fn(params, ctx)`, `ctx["from"]` naming the caller"""
        def wrap(fn):
            self._methods[name] = fn
            return fn
        return wrap

    def tool(self, name):
        """Answer a tool of plugin.json that has no `command`: `fn(args, ctx)` returns its text (anything else is sent
        as JSON); `ctx` holds `workspace` and `session`"""
        def wrap(fn):
            self._tools[name] = fn
            return fn
        return wrap

    def schedule(self, id):
        """Run a schedule of plugin.json when the engine starts it: `fn(ctx)`, `ctx["due"]` the occurrence in
        milliseconds"""
        def wrap(fn):
            self._schedules[id] = fn
            return fn
        return wrap

    def on_close(self, fn):
        self._closers.append(fn)
        return fn

    # Engine calls

    def request(self, method, params=None, timeout=60):
        """A request to the engine; raises EngineError with the engine's message"""
        with self._waits_lock:
            rid = "sdk-%d" % self._next
            self._next += 1
            fut = Future()
            self._waits[rid] = fut
        self._send({"id": rid, "method": method, "params": params or {}})
        return fut.result(timeout=timeout)

    def call(self, plugin, method, params=None):
        """Another plugin's method"""
        return self.request("plugin/call", {"plugin": plugin, "method": method, "params": params})

    def log(self, message):
        self._send({"method": "log", "params": {"message": str(message)}})

    # The wire

    def _send(self, message):
        line = json.dumps({"jsonrpc": "2.0", **message}, ensure_ascii=False)
        with self._write:
            self._out.write(line + "\n")
            self._out.flush()

    @staticmethod
    def _matches(pattern, kind):
        if pattern == kind:
            return True
        # Streamed deltas come only to a listener that names them
        if kind.endswith(".delta"):
            return False
        return pattern == "*" or (pattern.endswith(".*") and kind.split(".")[0] == pattern[:-2])

    def _answer(self, m):
        rid, method, params = m["id"], m.get("method", ""), m.get("params") or {}
        flag = threading.Event()
        self._cancelled[rid] = flag
        try:
            if method == "initialize":
                self.info = params
                declared = params.get("hooks") or []
                result = {"protocolVersion": PROTOCOL_VERSION, "hooks": [h for h in declared if h in self._hooks]}
            elif method.startswith("hook/") and method[5:] in self._hooks:
                result = self._hooks[method[5:]](params, {"cancelled": flag})
            elif method.startswith("rpc/") and method[4:] in self._methods:
                result = self._methods[method[4:]](params.get("params"), {"from": params.get("from"), "cancelled": flag})
            elif method.startswith("tool/") and method[5:] in self._tools:
                ctx = {"workspace": params.get("workspace"), "session": params.get("session"), "cancelled": flag}
                out = self._tools[method[5:]](params.get("arguments") or {}, ctx)
                result = {"text": out if isinstance(out, str) else json.dumps(out, ensure_ascii=False)}
            elif method.startswith("schedule/") and method[9:] in self._schedules:
                result = self._schedules[method[9:]]({"due": params.get("due"), "cancelled": flag})
            else:
                self._send({"id": rid, "error": {"code": -32601, "message": "no handler for " + method}})
                return
            if not flag.is_set():
                self._send({"id": rid, "result": result})
        except Exception as e:  # a handler's failure is its answer, never the process's end
            if not flag.is_set():
                self._send({"id": rid, "error": {"code": -32000, "message": str(e) or type(e).__name__}})
        finally:
            self._cancelled.pop(rid, None)

    def _event(self, params):
        kind = params.get("type", "")
        for pattern, fn in self._listeners:
            if self._matches(pattern, kind):
                self._pool.submit(self._guard, fn, params)

    @staticmethod
    def _guard(fn, arg):
        try:
            fn(arg)
        except Exception as e:
            print("acpira-plugin: listener failed:", repr(e), file=sys.stderr)

    def _close(self):
        for fn in self._closers:
            self._guard(lambda _: fn(), None)
        # Handlers still waiting on the engine would hold a normal exit up: the process goes now
        sys.stderr.flush()
        os._exit(0)

    def run(self):
        """Serve until stdin closes or a new version takes over"""
        for line in self._in:
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if "id" in m and "method" not in m:
                with self._waits_lock:
                    fut = self._waits.pop(m["id"], None)
                if fut is None:
                    continue
                if m.get("error"):
                    fut.set_exception(EngineError(m["error"].get("message", "error"), m["error"].get("code")))
                else:
                    fut.set_result(m.get("result"))
            elif "id" in m:
                # initialize first and in order: the engine waits for it before anything else
                if m.get("method") == "initialize":
                    self._answer(m)
                else:
                    self._pool.submit(self._answer, m)
            elif m.get("method") == "event":
                self._event(m.get("params") or {})
            elif m.get("method") == "$/cancel_request":
                flag = self._cancelled.get((m.get("params") or {}).get("requestId"))
                if flag:
                    flag.set()
            elif m.get("method") == "shutdown":
                break
        self._close()
