// HTTP transport to a fretwire-serve daemon — the third backend behind the ipc.js seam.
// invoke() is a POST mirroring Tauri's shape; listen() rides one WebSocket carrying the three
// backend events. The daemon injects `window.__FRETWIRE_SERVE__` into index.html, which is how
// ipc.js knows to route here.

// The single-editor lease key: fretwire-serve refuses a second concurrent page (the clipboards
// and undo history are single-editor state), keyed on this random id. (`randomUUID` is a
// secure-context API, absent on a plain-HTTP LAN address — hence the fallback.)
const clientId = crypto.randomUUID?.() ?? String(Math.random()).slice(2);

// The bearer token a bind beyond loopback requires. It arrives in the link the daemon prints
// (`#token=…` — a fragment, so it never reaches the server or a Referer), is kept per origin, and
// is dropped from the address bar once read. The daemon's marker says whether one is needed at
// all, so a page with none can ask up front rather than on its first 401.
const TOKEN_KEY = "fretwire.serve.token";
const token = (() => {
  const m = location.hash.match(/(?:^#|&)token=([^&]+)/);
  if (m) {
    const t = decodeURIComponent(m[1]);
    try {
      localStorage.setItem(TOKEN_KEY, t);
    } catch {
      /* private mode: the token lives for this page load only */
    }
    history.replaceState(null, "", location.pathname + location.search);
    return t;
  }
  try {
    return localStorage.getItem(TOKEN_KEY);
  } catch {
    return null;
  }
})();
if (window.__FRETWIRE_SERVE__?.auth && !token) tokenPrompt();

export async function invoke(cmd, args = {}) {
  const headers = { "Content-Type": "application/json", "X-Fretwire-Client": clientId };
  if (token) headers.Authorization = `Bearer ${token}`;
  const resp = await fetch(`/invoke/${cmd}`, { method: "POST", headers, body: JSON.stringify(args ?? {}) });
  // A rotated or mistyped token: forget it and ask, rather than failing every call the same way.
  if (resp.status === 401) {
    forgetToken();
    tokenPrompt();
  }
  // Non-2xx bodies are plain strings; throwing the string matches how Tauri rejects an invoke,
  // so the app's `String(e)` error handling renders both identically.
  if (!resp.ok) throw await resp.text();
  return resp.json();
}

function forgetToken() {
  try {
    localStorage.removeItem(TOKEN_KEY);
  } catch {
    /* nothing stored */
  }
}

const handlers = new Map(); // event name -> Set of callbacks
let socket = null;
let retryMs = 1000;

function connectSocket() {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const auth = token ? `&token=${encodeURIComponent(token)}` : "";
  socket = new WebSocket(`${proto}//${location.host}/events?client=${clientId}${auth}`);
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
    // 4401: the token was missing or wrong. Ask; a reload with the right one reconnects.
    if (e.code === 4401) {
      forgetToken();
      tokenPrompt();
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

const OVERLAY_CSS =
  "position:fixed;inset:0;z-index:9999;display:flex;flex-direction:column;gap:1rem;align-items:center;" +
  "justify-content:center;background:#1b1e23;color:#e8eaed;font:16px/1.6 system-ui,sans-serif;" +
  "text-align:center;padding:2rem;";

// A full-page state, deliberately distinct from `device-lost` (that's the *pedal* going away,
// an ordinary event): this page lost the argument over who the editor is.
function refusedOverlay() {
  const d = document.createElement("div");
  d.style.cssText = OVERLAY_CSS;
  d.textContent =
    "This editor is already open in another browser window. Close that one, then reload this page.";
  document.body.appendChild(d);
}

// The daemon wants a token this page doesn't have. The usual way in is the link it printed; this
// is the fallback for a pasted token — which is stored and the page reloaded, so every transport
// path starts over with it.
let prompting = false;
function tokenPrompt() {
  if (prompting) return;
  prompting = true;
  const show = () => {
    const d = document.createElement("div");
    d.style.cssText = OVERLAY_CSS;
    d.innerHTML =
      "<div>This fretwire-serve needs a token. Open the link it printed at startup, or paste the token here.</div>" +
      '<form style="display:flex;gap:.5rem;width:min(32rem,90vw)">' +
      '<input name="token" autocomplete="off" spellcheck="false" style="flex:1;font:inherit;padding:.4rem .6rem;border-radius:4px;border:1px solid #555;background:#111;color:inherit" />' +
      '<button type="submit" style="font:inherit;padding:.4rem .9rem;border-radius:4px;border:0;background:#3f8ae0;color:#fff;cursor:pointer">Open</button>' +
      "</form>";
    d.querySelector("form").addEventListener("submit", (e) => {
      e.preventDefault();
      const t = d.querySelector("input").value.trim();
      if (!t) return;
      try {
        localStorage.setItem(TOKEN_KEY, t);
      } catch {
        /* fall through: the fragment route works even without storage */
      }
      location.replace(`${location.pathname}${location.search}#token=${encodeURIComponent(t)}`);
      location.reload();
    });
    document.body.appendChild(d);
    d.querySelector("input").focus();
  };
  if (document.body) show();
  else document.addEventListener("DOMContentLoaded", show, { once: true });
}
