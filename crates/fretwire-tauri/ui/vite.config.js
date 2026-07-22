import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The Svelte UI builds into ../dist, which crates/fretwire-tauri/tauri.conf.json embeds as `frontendDist`
// at compile time. So the flow is: `npm run build` (here) → `cargo run -p fretwire-tauri`.
//
// `tauri dev` instead points the real app at this dev server (`devUrl` in tauri.conf.json), so the
// UI hot-reloads against the live backend. That URL is fixed, hence strictPort: if 5173 is busy,
// fail loudly rather than drift to 5174 and leave the app loading a dead address.
export default defineConfig({
  plugins: [svelte()],
  root: ".",
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "esnext",
  },
});
