import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// The Svelte UI builds into ../dist, which crates/fretwire-tauri/tauri.conf.json embeds as `frontendDist`
// at compile time. So the flow is: `npm run build` (here) → `cargo run -p fretwire-tauri`.
export default defineConfig({
  plugins: [svelte()],
  root: ".",
  clearScreen: false,
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "esnext",
  },
});
