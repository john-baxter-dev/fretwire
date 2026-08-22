<script>
  // The user IR store, as an overlay rather than a pane in the editor: managing impulse responses
  // is a separate job from editing a preset, it needs the width of a 128-row list, and nothing in
  // the chain view relates to it.
  //
  // Every write in here is a **flash write with no undo** — unlike every other edit in this app,
  // which lands in the edit buffer and can be reloaded away. That is why delete and overwrite both
  // confirm, and why the confirmations name what is about to be lost rather than asking "are you
  // sure".
  import Dialog from "./Dialog.svelte";
  import { pickPath } from "./ipc.js";

  let {
    slots = [],
    busy = false,
    // Slots the device has, for the upload target picker. The Stomp has 128.
    slotCount = 128,
    onClose,
    onRefresh,
    onScan,
    onExport,
    onUpload,
    onDelete,
    onRename,
  } = $props();

  // The device numbers IRs from 1 in its own menus; the wire is zero-based. Show the device's.
  const label = (index) => String(index + 1).padStart(3, "0");

  let showEmpty = $state(false);
  let renaming = $state(null);
  let renameText = $state("");
  let deleting = $state(null);
  let uploading = $state(null);

  const used = $derived(slots.filter((s) => s.used));
  const shown = $derived(showEmpty ? slots : used);
  // Only meaningful once a scan has been run — the directory listing has no empty rows to count.
  const freeSlots = $derived(
    slots.length >= slotCount ? slots.filter((s) => !s.used).map((s) => s.index) : [],
  );

  function sizeOf(slot) {
    if (!slot.used) return "";
    return slot.samples ? `${slot.samples} samples` : "—";
  }

  // A populated slot whose samples are all zero is silence, not emptiness, and looks identical to
  // a real IR in every other column.
  const isSilent = (slot) => slot.used && slot.checksum === 0;

  async function startUpload(preferSlot = null) {
    const path = await pickPath({
      title: "Choose an impulse response",
      filters: [{ name: "WAV audio", extensions: ["wav"] }],
    });
    if (!path) return;
    const stem = path.split(/[\\/]/).pop().replace(/\.[^.]+$/, "");
    uploading = {
      path,
      // The device stores 31 characters; trimming here rather than letting it truncate silently.
      name: stem.slice(0, 31),
      slot: preferSlot ?? (freeSlots.length ? freeSlots[0] : 0),
      force: false,
    };
  }

  function confirmUpload() {
    const target = slots.find((s) => s.index === uploading.slot);
    const job = { ...uploading, overwrite: !!target?.used };
    uploading = null;
    onUpload?.(job);
  }

  async function exportSlot(slot) {
    const path = await pickPath({
      title: `Save IR ${label(slot.index)} as…`,
      filters: [{ name: "WAV audio", extensions: ["wav"] }],
      save: true,
    });
    if (path) onExport?.(slot.index, path);
  }
</script>

<div
  class="ir-overlay"
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && onClose?.()}
>
  <div class="ir-card" role="dialog" aria-modal="true" aria-label="Impulse responses">
    <div class="ir-head">
      <div class="ir-title">Impulse responses</div>
      <span class="count">{used.length} loaded</span>
      <span class="spacer"></span>
      <label class="toggle">
        <input
          type="checkbox"
          checked={showEmpty}
          disabled={busy}
          onchange={(e) => {
            showEmpty = e.currentTarget.checked;
            // The fast listing only returns populated slots, so showing the empties means asking
            // the device for all 128 — one request each, and worth doing only on demand.
            if (showEmpty) onScan?.();
          }}
        />
        Show empty slots
      </label>
      <button class="secondary" disabled={busy} onclick={() => onRefresh?.()}>Refresh</button>
      <button class="secondary" disabled={busy} onclick={startUpload}>Upload…</button>
      <button class="secondary close" onclick={() => onClose?.()} aria-label="Close">✕</button>
    </div>

    <div class="ir-body">
      {#if busy}
        <div class="empty">Talking to the pedal…</div>
      {:else if shown.length === 0}
        <div class="empty">
          No impulse responses loaded.
          <button class="linkish" onclick={startUpload}>Upload one</button>
        </div>
      {:else}
        <table>
          <thead>
            <tr>
              <th class="num">Slot</th>
              <th>Name</th>
              <th class="len">Length</th>
              <th class="acts"></th>
            </tr>
          </thead>
          <tbody>
            {#each shown as slot (slot.index)}
              <tr class:dim={!slot.used}>
                <td class="num">{label(slot.index)}</td>
                <td class="name" class:unnamed={!slot.name}>
                  {slot.display_name}
                  {#if isSilent(slot)}<span class="tag" title="This slot holds an IR whose samples are all zero — silence, not an empty slot.">silent</span>{/if}
                </td>
                <td class="len">{sizeOf(slot)}</td>
                <td class="acts">
                  {#if slot.used}
                    <button class="row" onclick={() => exportSlot(slot)}>Export</button>
                    <button
                      class="row"
                      onclick={() => {
                        renaming = slot;
                        renameText = slot.name;
                      }}>Rename</button>
                    <button class="row danger" onclick={() => (deleting = slot)}>Delete</button>
                  {:else}
                    <button class="row" onclick={() => startUpload(slot.index)}>Upload here</button>
                  {/if}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>

    <div class="ir-foot">
      Writes here go straight to the pedal's flash — there is no undo, and no reload that takes one
      back.
    </div>
  </div>
</div>

{#if renaming}
  <Dialog
    title={`Rename IR ${label(renaming.index)}`}
    confirmLabel="Rename"
    confirmDisabled={!renameText.trim()}
    onconfirm={() => {
      const slot = renaming.index;
      const name = renameText.trim();
      renaming = null;
      onRename?.(slot, name);
    }}
    oncancel={() => (renaming = null)}
  >
    <!-- svelte-ignore a11y_autofocus -->
    <input class="field" bind:value={renameText} maxlength="31" autofocus />
    <div class="hint">The device stores 31 characters.</div>
  </Dialog>
{/if}

{#if deleting}
  <Dialog
    title={`Delete IR ${label(deleting.index)}?`}
    confirmLabel="Delete"
    danger
    onconfirm={() => {
      const slot = deleting.index;
      deleting = null;
      onDelete?.(slot);
    }}
    oncancel={() => (deleting = null)}
  >
    <div class="warn">
      <b>{deleting.display_name}</b> is erased from the pedal. This cannot be undone, and the file
      is not recoverable from any preset that uses it — export it first if you do not have a copy.
    </div>
  </Dialog>
{/if}

{#if uploading}
  <Dialog
    title="Upload impulse response"
    confirmLabel={slots.find((s) => s.index === uploading.slot)?.used ? "Replace" : "Upload"}
    danger={!!slots.find((s) => s.index === uploading.slot)?.used}
    confirmDisabled={!uploading.name.trim()}
    width={420}
    onconfirm={confirmUpload}
    oncancel={() => (uploading = null)}
  >
    <div class="path" title={uploading.path}>{uploading.path}</div>
    <label class="row-field">
      <span>Name</span>
      <input class="field" bind:value={uploading.name} maxlength="31" />
    </label>
    <label class="row-field">
      <span>Slot</span>
      <select class="field" bind:value={uploading.slot}>
        {#each Array.from({ length: slotCount }, (_, i) => i) as i}
          {@const at = slots.find((s) => s.index === i)}
          <option value={i}>
            {label(i)}
            {at?.used ? `— replaces ${at.display_name}` : "— empty"}
          </option>
        {/each}
      </select>
    </label>
    {#if slots.find((s) => s.index === uploading.slot)?.used}
      <div class="warn">
        That slot holds <b>{slots.find((s) => s.index === uploading.slot).display_name}</b>, which
        is erased. Export it first if you do not have a copy.
      </div>
    {/if}
    <label class="check">
      <input type="checkbox" bind:checked={uploading.force} />
      Upload even if the file is not 48 kHz
    </label>
    <div class="hint">
      The file is trimmed or zero-padded to 2048 samples and its first channel is taken. Nothing
      here resamples: a 44.1 kHz file loaded at 48 kHz plays short and bright.
    </div>
  </Dialog>
{/if}

<style>
  .ir-overlay {
    position: fixed;
    inset: 0;
    background: rgba(8, 10, 14, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 105;
  }
  .ir-card {
    display: flex;
    flex-direction: column;
    width: 720px;
    max-width: calc(100vw - 40px);
    max-height: calc(100vh - 80px);
    background: #1b1e25;
    border: 1px solid #3a4150;
    border-radius: 8px;
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.5);
  }
  .ir-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid #2b303a;
  }
  .ir-title {
    font-weight: 600;
  }
  .count {
    color: #8b94a5;
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: #b6bece;
    user-select: none;
  }
  .close {
    padding: 4px 9px;
  }
  .ir-body {
    overflow: auto;
    flex: 1;
    min-height: 140px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  th {
    position: sticky;
    top: 0;
    background: #1b1e25;
    text-align: left;
    font-weight: 500;
    color: #8b94a5;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 8px 10px;
    border-bottom: 1px solid #2b303a;
  }
  td {
    padding: 6px 10px;
    border-bottom: 1px solid #23272f;
    vertical-align: middle;
  }
  tr.dim td {
    color: #6d7a90;
  }
  .num {
    width: 56px;
    font-variant-numeric: tabular-nums;
    color: #8b94a5;
  }
  .len {
    width: 110px;
    color: #8b94a5;
    font-variant-numeric: tabular-nums;
  }
  .name.unnamed {
    font-style: italic;
    color: #8b94a5;
  }
  .tag {
    margin-left: 8px;
    padding: 1px 6px;
    border-radius: 8px;
    background: #3a2f22;
    color: #d8a657;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .acts {
    width: 210px;
    text-align: right;
    white-space: nowrap;
  }
  button.row {
    background: none;
    border: 1px solid #3a4150;
    color: #b6bece;
    border-radius: 5px;
    padding: 3px 8px;
    font-size: 12px;
    margin-left: 5px;
    cursor: pointer;
  }
  button.row:hover {
    border-color: #566076;
    color: #e6ebf3;
  }
  button.row.danger:hover {
    border-color: #a04a4a;
    color: #f0a0a0;
  }
  .empty {
    padding: 34px 16px;
    text-align: center;
    color: #8b94a5;
  }
  .linkish {
    background: none;
    border: none;
    color: #6ea8f0;
    cursor: pointer;
    text-decoration: underline;
    font-size: inherit;
    padding: 0 0 0 4px;
  }
  .ir-foot {
    padding: 9px 14px;
    border-top: 1px solid #2b303a;
    color: #8b94a5;
    font-size: 11.5px;
  }
  .field {
    width: 100%;
    background: #12151a;
    border: 1px solid #3a4150;
    color: #e6ebf3;
    border-radius: 5px;
    padding: 6px 8px;
    font-size: 13px;
  }
  .row-field {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 8px 0;
  }
  .row-field span {
    width: 44px;
    color: #8b94a5;
    font-size: 12px;
  }
  .path {
    font-size: 11.5px;
    color: #8b94a5;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
    margin-bottom: 6px;
  }
  .warn {
    margin: 8px 0;
    padding: 8px 10px;
    border-radius: 5px;
    background: #2c2224;
    border: 1px solid #5a3a3f;
    color: #e8c3c3;
    font-size: 12px;
    line-height: 1.45;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: #b6bece;
    margin: 8px 0 4px;
  }
  .hint {
    color: #8b94a5;
    font-size: 11.5px;
    line-height: 1.45;
    margin-top: 6px;
  }
</style>
