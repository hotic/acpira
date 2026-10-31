// Request/response pairs over postMessage: each request carries a fresh `seq` and the host's reply with the same seq
// settles it. A reply that never comes (host gone, an engine from before the message) settles with `fallback` after
// `timeoutMs`, so nothing waits forever; a reply after that, or for a seq nobody asked, is dropped
export interface RequestTable<T> {
  // Posts through `send` with the request's seq; resolves with the reply or the fallback
  ask(send: (seq: number) => void): Promise<T>;
  // The host's reply for `seq`
  settle(seq: number, value: T): void;
}

export function requestTable<T>(timeoutMs: number, fallback: T): RequestTable<T> {
  let seq = 0;
  const waits = new Map<number, { resolve: (value: T) => void; timer: ReturnType<typeof setTimeout> }>();
  const settle = (mine: number, value: T) => {
    const wait = waits.get(mine);
    if (!wait) return;
    waits.delete(mine);
    clearTimeout(wait.timer);
    wait.resolve(value);
  };
  return {
    ask: send => new Promise<T>(resolve => {
      const mine = ++seq;
      waits.set(mine, { resolve, timer: setTimeout(() => settle(mine, fallback), timeoutMs) });
      send(mine);
    }),
    settle,
  };
}
