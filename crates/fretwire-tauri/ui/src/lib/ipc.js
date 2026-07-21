// Single seam between the UI and the backend. Everything in the UI imports `invoke`/`listen` from
// here rather than directly from `@tauri-apps/api`, so we can transparently swap in a mock device
// when there's no Tauri runtime.
//
// Tauri v2 injects `window.__TAURI_INTERNALS__` into its webview. When that's absent — e.g. running
// the Vite dev server in a plain browser (`npm run dev`), with no hardware or Rust toolchain — we
// route to the in-memory mock backend instead. In a real Tauri build the mock is imported but never
// called, and vice versa; both are harmless.
import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import * as mock from "../mock/backend.js";

export const IS_MOCK = !(typeof window !== "undefined" && "__TAURI_INTERNALS__" in window);

if (IS_MOCK) {
  console.info(
    "%c[fretwire]%c no Tauri runtime — using the mock device backend. " +
      "Simulate live-follow pushes from here via window.fretwireMock (e.g. fretwireMock.bypass(1, false)).",
    "color:#3f8ae0;font-weight:bold",
    "color:inherit",
  );
}

export const invoke = IS_MOCK ? mock.invoke : tauriInvoke;
export const listen = IS_MOCK ? mock.listen : tauriListen;
