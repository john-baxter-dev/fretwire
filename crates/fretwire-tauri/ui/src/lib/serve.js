// HTTP transport to a fretwire-serve daemon — the third backend behind the ipc.js seam.
// invoke() is a POST mirroring Tauri's shape; listen() rides one WebSocket carrying the three
// backend events. The daemon injects `window.__FRETWIRE_SERVE__` into index.html, which is how
// ipc.js knows to route here.

// The single-editor lease key: fretwire-serve refuses a second concurrent page (the clipboards
// and undo history are single-editor state), keyed on this random id.
const clientId = crypto.randomUUID?.() ?? String(Math.random()).slice(2);

export async function invoke(cmd, args = {}) {
  const resp = await fetch(`/invoke/${cmd}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Fretwire-Client": clientId },
    body: JSON.stringify(args ?? {}),
  });
  // Non-2xx bodies are plain strings; throwing the string matches how Tauri rejects an invoke,
  // so the app's `String(e)` error handling renders both identically.
  if (!resp.ok) throw await resp.text();
  return resp.json();
}

const handlers = new Map(); // event name -> Set of callbacks
let socket = null;
let retryMs = 1000;

function connectSocket() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  socket = new WebSocket(`${proto}//${location.host}/events?client=${clientId}`);
  socket.onmessage = (m) => {
    let frame;
    try {
      frame = JSON.parse(m.data);
    } catch {
      return;
    }
    // Handlers expect Tauri's envelope: { event, payload, id }.
    const set = handlers.get(frame.event);
    if (set) for (const h of set) h({ event: frame.event, payload: frame.payload, id: 0 });
  };
  socket.onopen = () => {
    retryMs = 1000;
  };
  socket.onclose = (e) => {
    socket = null;
    // 4409: another page holds the editor lease. Don't retry — a reconnect loop would fight the
    // other editor for the lease forever.
    if (e.code === 4409) {
      refusedOverlay();
      return;
    }
    // Anything else (daemon restarted, network blip): retry with backoff. Missed pushes are
    // tolerable — every mutation returns fresh authoritative state anyway.
    setTimeout(connectSocket, retryMs);
    retryMs = Math.min(retryMs * 2, 5000);
  };
}

export function listen(event, handler) {
  if (!handlers.has(event)) handlers.set(event, new Set());
  handlers.get(event).add(handler);
  if (!socket) connectSocket();
  // Tauri's listen resolves to an unlisten function; mirror that.
  return Promise.resolve(() => handlers.get(event)?.delete(handler));
}

// A full-page state, deliberately distinct from `device-lost` (that's the *pedal* going away,
// an ordinary event): this page lost the argument over who the editor is.
function refusedOverlay() {
  const d = document.createElement("div");
  d.style.cssText =
    "position:fixed;inset:0;z-index:9999;display:flex;align-items:center;justify-content:center;" +
    "background:#1b1e23;color:#e8eaed;font:16px/1.6 system-ui,sans-serif;text-align:center;padding:2rem;";
  d.textContent =
    "This editor is already open in another browser window. Close that one, then reload this page.";
  document.body.appendChild(d);
}
