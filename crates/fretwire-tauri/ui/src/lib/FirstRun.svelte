<script>
  // First-run screen: fretwire ships none of Line 6's model/preset data, so on a fresh install the
  // editor can't name a single block. Rather than failing with "run `fretwire import-data`" — which
  // assumes the user found the CLI — offer the import here, with a native picker.
  //
  // Skipping is allowed on purpose: the wire protocol edits by raw parameter index, so the editor
  // genuinely works without the data. You just get numbers instead of names.
  import { invoke, pickPath } from "./ipc.js";

  let { status, onready, onskip } = $props();

  let busy = $state(false);
  let error = $state(null);
  let warning = $state(null);

  const INSTALLER_FILTERS = [
    { name: "HX Edit installer", extensions: ["exe", "msi", "pkg", "dmg"] },
    { name: "All files", extensions: ["*"] },
  ];

  async function choose(directory) {
    error = null;
    warning = null;
    const source = await pickPath({
      directory,
      title: directory ? "Choose an HX Edit `res` folder" : "Choose an HX Edit installer",
      filters: directory ? undefined : INSTALLER_FILTERS,
    });
    if (!source) return; // cancelled
    busy = true;
    try {
      const result = await invoke("import_data", { source });
      if (result.missing?.length) {
        // Import succeeded but the source was incomplete — usable, just degraded.
        warning = `Imported ${result.copied} file(s), but ${result.missing.join(" and ")} ${
          result.missing.length > 1 ? "were" : "was"
        } missing. Model names and parameter ordering may be incomplete.`;
      }
      onready?.(result);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="wrap">
  <div class="card">
    <h1>One-time setup</h1>
    <p class="lead">
      fretwire doesn't ship Line 6's model data — that's theirs, not ours. Point it at your own copy
      of HX Edit once and it'll cache what it needs locally.
    </p>

    <div class="choices">
      <button class="primary" disabled={busy} onclick={() => choose(false)}>
        Choose an HX Edit installer…
      </button>
      <button class="secondary" disabled={busy} onclick={() => choose(true)}>
        Choose an extracted folder…
      </button>
    </div>

    <p class="hint">
      An installer (<code>.exe</code>, <code>.msi</code>, <code>.pkg</code>, <code>.dmg</code>)
      is unpacked with <code>7z</code>. If you already have HX Edit installed somewhere — including
      on a Windows or macOS machine you can reach — its <code>res</code> folder works too, and needs
      no <code>7z</code>.
    </p>
    <p class="path" title="Set $FRETWIRE_DATA_DIR to change this">Cached in {status?.dir}</p>

    {#if busy}
      <div class="note busy">Importing… unpacking an installer can take a moment.</div>
    {/if}
    {#if error}
      <div class="note error">{error}</div>
    {/if}
    {#if warning}
      <div class="note warn">{warning}</div>
    {/if}

    <div class="foot">
      <button class="link" disabled={busy} onclick={() => onskip?.()}>
        Skip — use the editor without model names
      </button>
    </div>
  </div>
</div>

<style>
  .wrap {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .card {
    width: 520px;
    max-width: 100%;
    background: #1b1e25;
    border: 1px solid #3a4150;
    border-radius: 12px;
    padding: 24px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
  }
  h1 {
    margin: 0 0 8px;
    font-size: 18px;
  }
  .lead {
    margin: 0 0 18px;
    color: #b9c0cc;
  }
  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .choices button {
    font: inherit;
    border: 0;
    border-radius: 8px;
    padding: 10px 14px;
    cursor: pointer;
    text-align: left;
  }
  .primary {
    background: #2b7de0;
    color: #fff;
  }
  .secondary {
    background: #363b46;
    color: #e6e8ec;
  }
  .choices button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .hint {
    margin: 14px 0 0;
    font-size: 12.5px;
    color: #8b93a1;
  }
  code {
    background: #23272f;
    border-radius: 4px;
    padding: 1px 4px;
  }
  .note {
    margin-top: 14px;
    padding: 9px 11px;
    border-radius: 8px;
    font-size: 13px;
    white-space: pre-wrap;
  }
  .busy {
    background: #23272f;
    color: #b9c0cc;
  }
  .error {
    background: #3a2224;
    color: #ffb4b0;
  }
  .warn {
    background: #3a3322;
    color: #ffd79a;
  }
  .foot {
    margin-top: 20px;
    padding-top: 14px;
    border-top: 1px solid #2b3038;
  }
  .link {
    font: inherit;
    background: none;
    border: 0;
    padding: 0;
    color: #8b93a1;
    text-decoration: underline;
    cursor: pointer;
  }
  .link:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .path {
    margin: 8px 0 0;
    font-size: 11.5px;
    color: #6b7280;
    overflow-wrap: anywhere;
  }
</style>
