// Single seam between the UI and the backend. Everything in the UI imports `invoke`/`listen` from
// here rather than directly from `@tauri-apps/api`, so the same build runs against three backends:
//
// - Tauri v2 injects `window.__TAURI_INTERNALS__` into its webview → the real Tauri IPC.
// - fretwire-serve injects `window.__FRETWIRE_SERVE__` into index.html (an inline script, so it
//   runs before this module evaluates) → HTTP + a WebSocket, see ./serve.js.
// - Neither present — e.g. the Vite dev server in a plain browser (`npm run dev`), with no
//   hardware or Rust toolchain — → the in-memory mock backend.
//
// The unused backends are imported but never called; all are harmless.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import * as mock from "../mock/backend.js";
import * as serve from "./serve.js";

const HAS_WINDOW = typeof window !== "undefined";
export const IS_SERVE = HAS_WINDOW && "__FRETWIRE_SERVE__" in window;
export const IS_MOCK = !IS_SERVE && !(HAS_WINDOW && "__TAURI_INTERNALS__" in window);

if (IS_MOCK) {
  console.info(
    "%c[fretwire]%c no Tauri runtime — using the mock device backend. " +
      "Simulate live-follow pushes from here via window.fretwireMock (e.g. fretwireMock.bypass(1, false)).",
    "color:#3f8ae0;font-weight:bold",
    "color:inherit",
  );
}

export const invoke = IS_SERVE ? serve.invoke : IS_MOCK ? mock.invoke : tauriInvoke;
export const listen = IS_SERVE ? serve.listen : IS_MOCK ? mock.listen : tauriListen;

/// Native file/folder picker, behind the same seam. Tauri routes to the dialog plugin; the mock
/// (a plain browser, which can't read arbitrary paths) falls back to typing a path. Returns the
/// chosen path, or null if the user cancelled.
export async function pickPath({ directory = false, title, filters, save = false } = {}) {
  if (IS_SERVE) {
    // The paths that matter are on the machine running the daemon, which the browser can't
    // walk. A typed server-side path is the honest v1 — the same model the backup dialogs
    // already use; a server-side directory browser is planned (docs/serve-mode.md §3).
    const answer = window.prompt(
      `${title ?? "Choose a path"}\n\n(Type a path on the machine running fretwire-serve.)`,
    );
    return answer?.trim() ? answer.trim() : null;
  }
  if (IS_MOCK) {
    const answer = window.prompt(
      `${title ?? "Choose a path"}\n\n(The browser mock can't open a native picker — type a path.)`,
    );
    return answer?.trim() ? answer.trim() : null;
  }
  const dialog = await import("@tauri-apps/plugin-dialog");
  // A save dialog is a different call, not a flag: `open` will not offer a name for a file that
  // does not exist yet, which is exactly what exporting an IR needs.
  const chosen = save
    ? await dialog.save({ title, filters })
    : await dialog.open({ directory, multiple: false, title, filters });
  return typeof chosen === "string" ? chosen : null;
}
