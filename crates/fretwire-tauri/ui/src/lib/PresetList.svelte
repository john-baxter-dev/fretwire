<script>
  import { numbering, otherMode, slotLabel, canBank } from "./numbering.svelte.js";

  // Preset browser sidebar: the full setlist, current highlighted, click to load. Save and Save As
  // (choose target slot + name, covering copy and overwrite) stay on the surface with Copy/Paste,
  // whose label doubles as the clipboard readout. The rest live under the ⋯ menu — the row had
  // grown to seven buttons, and Rename/Export/Restore are the ones you reach for rarely and open a
  // dialog anyway. New entries belong there rather than as another button.
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
    onExport,
    onRestore,
    // Whole-device backup and restore — presets, IRs and global settings in one file.
    onBackupDevice,
    onRestoreDevice,
    onCopyPreset,
    onPastePreset,
    // Empty the loaded preset (edit buffer only — App.svelte confirms first).
    onClearPreset,
    onRevertPreset,
    // Switch the preset-numbering form. On a pedal that answers setting 27 this writes the pedal's
    // own Global Setting, so the menu item is that setting rather than a second opinion about it.
    onNumbering,
    // Name of the preset on the paste buffer, or null when it's empty.
    presetClip = null,
  } = $props();

  // How the pedal writes a slot on its own screen — `01A`, `01B`, `01C`, `02A` — when the backend
  // knows this device's banking. Matching the panel is the point: someone reading a preset off the
  // hardware should find the same string here. Which form the panel uses is a Global Setting on the
  // device, and the menu item below edits that setting. See `lib/numbering.svelte.js`.
  const pad = (p) => slotLabel(p);
  // Only worth offering where the backend actually knows the banking — otherwise both settings
  // render the same flat number and the menu item is a lie.
  const bankable = $derived(canBank(presets));

  // Only a device with more than one setlist gets the picker — an HX Stomp has a single flat
  // preset list and HX Edit shows no setlist control at all for it.
  const hasSetlists = $derived(setlists.length > 1);
  // The highlighted row means "this is what's loaded", which is only true while looking at the
  // setlist it was loaded from. Browsing another one highlights nothing.
  const viewingCurrent = $derived(!hasSetlists || viewBank === currentBank);

  let menuOpen = $state(false);
  const run = (fn) => {
    menuOpen = false;
    fn?.();
  };
  // Close on any click that isn't inside the menu, and on Escape. Registered on the window rather
  // than a backdrop element so the menu never covers the list underneath it.
  function dismissable(node) {
    const away = (e) => {
      if (!node.contains(e.target)) menuOpen = false;
    };
    const esc = (e) => e.key === "Escape" && (menuOpen = false);
    window.addEventListener("pointerdown", away, true);
    window.addEventListener("keydown", esc);
    return {
      destroy() {
        window.removeEventListener("pointerdown", away, true);
        window.removeEventListener("keydown", esc);
      },
    };
  }
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
    <div class="menuwrap" use:dismissable>
      <button
        class="more"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        title="More preset actions"
        onclick={() => (menuOpen = !menuOpen)}>⋯</button>
      {#if menuOpen}
        <div class="menu" role="menu">
          <button role="menuitem" onclick={() => run(onRename)}>Rename preset…</button>
          <button role="menuitem" onclick={() => run(onExport)}>Export presets to file…</button>
          <button role="menuitem" onclick={() => run(onRestore)}>Restore preset from file…</button>
          <div class="sep" role="separator"></div>
          <button role="menuitem" onclick={() => run(onBackupDevice)}>Back up device to file…</button>
          <button role="menuitem" onclick={() => run(onRestoreDevice)}>Restore device from file…</button>
          <div class="sep" role="separator"></div>
          <button
            role="menuitem"
            class="danger"
            title="Discard unsaved edits and reload the preset as last saved — the discarded state stays one Undo away"
            onclick={() => run(onRevertPreset)}>Revert to saved…</button>
          <button
            role="menuitem"
            class="danger"
            title="Delete every block and reset the snapshot names — edit buffer only, Save to keep it"
            onclick={() => run(onClearPreset)}>Clear preset…</button>
          {#if bankable}
            <div class="sep" role="separator"></div>
            <button
              role="menuitemcheckbox"
              aria-checked={numbering.mode === "banked"}
              title={numbering.deviceBacked
                ? "The pedal's own setting (Global Settings ▸ Displays ▸ Preset numbering). Changing it here changes it on the pedal."
                : "This pedal doesn't report its numbering setting, so this only changes how fretwire shows them."}
              onclick={() => run(() => onNumbering(otherMode()))}
              >Number presets: {numbering.mode === "banked" ? "01A" : "000"}</button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
  <div class="tools sub last">
    <button onclick={onCopyPreset} title="Copy the loaded preset, to paste onto another slot">Copy</button>
    <!-- Carries the copied preset's name, so it gets the wider share of the row and its own line;
         a name still longer than that ellipsises rather than pushing out of the sidebar. -->
    <button
      class="wide"
      onclick={onPastePreset}
      disabled={!presetClip || writeBlocked}
      title={presetClip ? `Replace the loaded preset with "${presetClip}" (edit buffer — Save to keep it)` : "Copy a preset first"}
    >Paste{presetClip ? ` "${presetClip}"` : ""}</button>
  </div>
  <div class="list">
    {#each presets as p (p.index)}
      <button class="row" class:current={viewingCurrent && p.index === currentIndex} onclick={() => onGoto(p.index)}>
        <span class="idx">{pad(p)}</span>
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
    /* Without `min-width: 0` a flex item refuses to shrink below its content, so one long label
       (the Paste button carries a preset name) pushes the whole row out of the sidebar. */
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: #2b7de0;
    color: #fff;
    border: 0;
    border-radius: 6px;
    padding: 6px 4px;
    cursor: pointer;
  }
  .tools button.wide {
    flex: 2;
  }
  /* The ⋯ menu: an icon-width button in the flex row, with the popup anchored under it. The wrapper
     is the flex item, so the popup can be absolutely positioned without leaving the row. */
  .menuwrap {
    position: relative;
    flex: 0 0 auto;
  }
  .tools .more {
    flex: 0 0 auto;
    width: 28px;
    padding: 0;
    line-height: 24px;
    font-size: 15px;
    background: #363b46;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 20;
    min-width: 210px;
    display: flex;
    flex-direction: column;
    background: #1b1e25;
    border: 1px solid #3a4150;
    border-radius: 8px;
    padding: 4px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.5);
  }
  .menu button {
    font: inherit;
    font-size: 12px;
    text-align: left;
    padding: 7px 9px;
    border: 0;
    border-radius: 5px;
    background: none;
    color: #e6e8ec;
    cursor: pointer;
    white-space: nowrap;
  }
  .menu button.danger {
    color: #c9403c;
  }
  .menu button:hover {
    background: #2a2f3a;
  }
  .menu .sep {
    height: 1px;
    margin: 4px 2px;
    background: #3a4150;
  }
  /* The numbering item is a setting, not an action: show the form it is currently on rather than
     leaving the reader to guess whether the label is the state or the destination. */
  .menu button[role="menuitemcheckbox"] {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    color: #aab2c0;
  }
  .tools button:nth-child(n + 2) {
    background: #363b46;
  }
  /* The tool rows read as one group: the separator lives under the last row only. */
  .tools:not(.last) {
    border-bottom: 0;
  }
  .tools:not(.sub) {
    padding-bottom: 4px;
  }
  .tools.sub {
    padding-top: 0;
    padding-bottom: 4px;
  }
  .tools.sub.last {
    padding-bottom: 10px;
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
