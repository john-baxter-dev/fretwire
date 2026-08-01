<script>
  // Preset browser sidebar: the full setlist, current highlighted, click to load. Header buttons
  // save/rename. Save As (choose target slot + name) covers copy and overwrite. Backup/Restore
  // round-trip the whole setlist through a JSON file on disk.
  let {
    presets,
    currentIndex,
    dirty = false,
    setlists = [],
    viewBank = 0,
    currentBank = 0,
    // True while browsing a setlist the app may not write into — see `foreignSetlist` in App.svelte.
    writeBlocked = false,
    onPickSetlist,
    onGoto,
    onSave,
    onSaveAs,
    onRename,
    onBackup,
    onRestore,
    onCopyPreset,
    onPastePreset,
    // Name of the preset on the paste buffer, or null when it's empty.
    presetClip = null,
  } = $props();

  const pad = (n) => String(n).padStart(3, "0");

  // Only a device with more than one setlist gets the picker — an HX Stomp has a single flat
  // preset list and HX Edit shows no setlist control at all for it.
  const hasSetlists = $derived(setlists.length > 1);
  // The highlighted row means "this is what's loaded", which is only true while looking at the
  // setlist it was loaded from. Browsing another one highlights nothing.
  const viewingCurrent = $derived(!hasSetlists || viewBank === currentBank);
</script>

<div class="sidebar">
  {#if hasSetlists}
    <div class="setlist">
      <label for="setlist-pick">Setlist</label>
      <select
        id="setlist-pick"
        value={viewBank}
        onchange={(e) => onPickSetlist?.(Number(e.currentTarget.value))}
      >
        {#each setlists as name, i}
          <option value={i}>{name}</option>
        {/each}
      </select>
    </div>
  {/if}
  <div class="tools">
    <button onclick={onSave} title="Overwrite the current preset">Save</button>
    <button
      onclick={onSaveAs}
      disabled={writeBlocked}
      title={writeBlocked
        ? `Writing into ${setlists[viewBank] ?? "another setlist"} is untested on this hardware — switch back to ${setlists[currentBank] ?? "the device's setlist"}, or set FRETWIRE_SETLISTS=1`
        : "Save to a chosen slot / copy / overwrite"}>Save As…</button>
    <button onclick={onRename} title="Rename the current preset (name only)">Rename…</button>
  </div>
  <div class="tools sub">
    <button onclick={onBackup} title="Save every preset to a file (reads only)">Backup…</button>
    <button onclick={onRestore} title="Restore a preset from a backup file into a slot">Restore…</button>
    <button onclick={onCopyPreset} title="Copy the loaded preset, to paste onto another slot">Copy</button>
    <button
      onclick={onPastePreset}
      disabled={!presetClip || writeBlocked}
      title={presetClip ? `Replace the loaded preset with "${presetClip}" (edit buffer — Save to keep it)` : "Copy a preset first"}
    >Paste{presetClip ? ` "${presetClip}"` : ""}</button>
  </div>
  <div class="list">
    {#each presets as p (p.index)}
      <button class="row" class:current={viewingCurrent && p.index === currentIndex} onclick={() => onGoto(p.index)}>
        <span class="idx">{pad(p.index)}</span>
        <span class="nm">{p.name}</span>
        {#if dirty && viewingCurrent && p.index === currentIndex}<span class="dirty" title="Edited — not saved">●</span>{/if}
      </button>
    {/each}
    {#if !presets.length}<div class="empty">no presets loaded</div>{/if}
  </div>
</div>

<style>
  .sidebar {
    width: 230px;
    flex: 0 0 auto;
    border: 1px solid #2a2e37;
    border-radius: 10px;
    background: #1b1e25;
    display: flex;
    flex-direction: column;
    max-height: 70vh;
  }
  .setlist {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 10px 0;
  }
  .setlist label {
    font-size: 11px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: #8b93a3;
  }
  .setlist select {
    font: inherit;
    font-size: 12px;
    flex: 1;
    min-width: 0;
    background: #23262e;
    color: #e6e8ec;
    border: 1px solid #2a2e37;
    border-radius: 6px;
    padding: 5px 6px;
    cursor: pointer;
  }
  .tools {
    display: flex;
    gap: 6px;
    padding: 10px;
    border-bottom: 1px solid #2a2e37;
  }
  .tools button {
    font: inherit;
    font-size: 12px;
    flex: 1;
    background: #2b7de0;
    color: #fff;
    border: 0;
    border-radius: 6px;
    padding: 6px 4px;
    cursor: pointer;
  }
  .tools button:nth-child(n + 2) {
    background: #363b46;
  }
  /* The two tool rows read as one group: the separator lives under the second row only. */
  .tools:not(.sub) {
    border-bottom: 0;
    padding-bottom: 4px;
  }
  .tools.sub {
    padding-top: 0;
  }
  .tools.sub button {
    background: #2a2f3a;
    color: #c3c9d4;
  }
  .list {
    overflow-y: auto;
    padding: 6px;
  }
  .row {
    display: flex;
    gap: 8px;
    width: 100%;
    text-align: left;
    font: inherit;
    background: transparent;
    color: #c3c9d4;
    border: 0;
    border-radius: 6px;
    padding: 6px 8px;
    cursor: pointer;
  }
  .row:hover {
    background: #232833;
  }
  .row.current {
    background: #26333f;
    color: #fff;
  }
  .idx {
    color: #6b7280;
    font-variant-numeric: tabular-nums;
    font-size: 12px;
  }
  .row.current .idx {
    color: #9ec5f0;
  }
  .nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dirty {
    margin-left: auto;
    color: #f0c245;
    font-size: 10px;
  }
  .empty {
    color: #6b7280;
    padding: 10px;
    font-style: italic;
  }
</style>
