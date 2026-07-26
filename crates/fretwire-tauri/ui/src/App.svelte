<script>
  import { invoke, listen, IS_MOCK } from "./lib/ipc.js";
  import { onMount } from "svelte";
  import Chain from "./lib/Chain.svelte";
  import ParamPanel from "./lib/ParamPanel.svelte";
  import ModelPicker from "./lib/ModelPicker.svelte";
  import PresetList from "./lib/PresetList.svelte";
  import HistoryPane from "./lib/HistoryPane.svelte";
  import Dialog from "./lib/Dialog.svelte";
  import Toast from "./lib/Toast.svelte";
  import FirstRun from "./lib/FirstRun.svelte";

  const BUDGET = 100;

  // First-run gating: until we've checked whether the Line 6 reference data is imported, show
  // nothing (avoids flashing the editor then the setup screen). `null` = checking, then a status
  // object. `dataReady` also flips true when the user chooses to skip the import.
  let dataStatus = $state(null);
  let dataReady = $state(false);
  onMount(async () => {
    try {
      dataStatus = await invoke("data_status");
    } catch {
      dataStatus = { present: false, dir: "", files: 0 };
    }
    dataReady = dataStatus.present;
  });

  let status = $state("Ready — WebKitGTK webview is painting.");
  let statusErr = $state(false);
  let preset = $state(null);
  let presets = $state([]);
  let splitTypes = $state([]);
  let selectedSlot = $state(null);
  // Add-block picker target: null = closed, -1 = append to the chain (header button), a slot
  // number = add into that exact empty grid cell (clicked in the grid, like HX Edit).
  let addTarget = $state(null);
  // Tracked optimistically: the device applies a snapshot switch but the read-back's active-snapshot
  // field lags, so we set this on click and sync it only when a preset is loaded/switched.
  let activeSnapshot = $state(null);

  // ---- toasts (errors) & in-app dialogs (replacing native prompt/confirm) ----
  let toasts = $state([]);
  let toastSeq = 0;
  const dismissToast = (id) => (toasts = toasts.filter((t) => t.id !== id));
  function toast(msg, kind = "error") {
    const id = ++toastSeq;
    toasts = [...toasts, { id, msg: String(msg), kind }];
    setTimeout(() => dismissToast(id), 6000);
  }
  let saveAsDlg = $state(null); // { slot, name }
  let renameDlg = $state(null); // { name }
  let deleteDlg = $state(null); // slot number
  let backupDlg = $state(null); // { path }
  let backupProgress = $state(null); // { done, total, name } while a backup sweep runs
  let restoreDlg = $state(null); // { path, entries, index, slot } — entries load from the file
  let snapRenameDlg = $state(null); // { index, name }
  const deleteName = $derived.by(() => {
    const b = deleteDlg != null ? preset?.blocks.find((b) => b.slot === deleteDlg) : null;
    return b ? (b.user_label ?? b.model_name) : "this block";
  });
  // Focus (and select) a dialog's primary input when it opens.
  const autofocus = (node) => {
    node.focus();
    node.select?.();
  };

  // Each DSP's routing view (one on the HX Stomp, two on the Helix Floor). Fall back to the
  // preset's flat DSP-0 fields when the backend didn't send `dsps` (older payloads / the mock).
  const dspViews = $derived.by(() => {
    if (!preset) return [];
    if (preset.dsps?.length) return preset.dsps;
    return [
      {
        dsp: 0,
        split: preset.split,
        split_pos: preset.split_pos,
        mixer_pos: preset.mixer_pos,
        split_node: preset.split_node,
        mixer_node: preset.mixer_node,
        input_node: preset.input_node,
        output_node: preset.output_node,
        grid: preset.grid ?? [],
        dsp_load: preset.dsp_load,
      },
    ];
  });
  // Every structural node across all DSPs — so a DSP-2 split/mixer/IO node is selectable too.
  const allNodes = $derived(
    dspViews.flatMap((v) => [v.split_node, v.mixer_node, v.input_node, v.output_node]),
  );
  // Each DSP carries its own ~100% budget, so a block's fit is judged against *its* DSP's load,
  // not the combined total (which can exceed 100% on the Floor). Slots are global: `dsp*20+index`.
  const loadForSlot = (slot) => {
    const v = dspViews.find((x) => x.dsp === Math.floor(slot / 20));
    return v ? v.dsp_load : (preset?.dsp_load ?? 0);
  };
  // Header readout: "38.4%" on the Stomp, "38.4% · 58.9%" (per DSP) on the Floor.
  const dspLoadLabel = $derived(dspViews.map((v) => v.dsp_load.toFixed(1) + "%").join(" · "));

  // Whether the selected slot is a structural node (split/mixer/input/output) rather than a normal
  // block — nodes aren't swappable or deletable.
  const selectedIsNode = $derived(
    !!preset && selectedSlot != null && allNodes.some((n) => n?.slot === selectedSlot),
  );
  const selectedIsSplit = $derived(
    !!preset && dspViews.some((v) => v.split_node?.slot === selectedSlot),
  );

  // The selected block, looked up fresh from the current preset (so it reflects live edits).
  const selectedBlock = $derived.by(() => {
    if (!preset || selectedSlot == null) return null;
    return (
      preset.blocks.find((b) => b.slot === selectedSlot) ??
      allNodes.find((n) => n?.slot === selectedSlot) ??
      null
    );
  });

  // Live-follow: apply device-originated changes pushed by the heartbeat (footswitch bypass, panel
  // snapshot/preset switch) so the GUI mirrors the hardware.
  onMount(() => {
    const unlisten = listen("device-pushes", (e) => handlePushes(e.payload));
    const unProgress = listen("backup-progress", (e) => (backupProgress = e.payload));
    return () => {
      unlisten.then((f) => f());
      unProgress.then((f) => f());
    };
  });

  // ---- undo / redo / history ----
  const onUndo = () => preset?.undo_depth > 0 && apply(invoke("undo"));
  const onRedo = () => preset?.redo_depth > 0 && apply(invoke("redo"));
  const onHistoryJump = (index) => apply(invoke("history_jump", { index }));

  // Ctrl/Cmd+Z undoes, Ctrl/Cmd+Shift+Z or Ctrl/Cmd+Y redoes; Space toggles the selected block's
  // bypass (like HX Edit) — except while typing in a field or with a dialog open.
  onMount(() => {
    const onKey = (e) => {
      if (!connected) return;
      const tag = e.target?.tagName;
      if (tag === "INPUT" || tag === "SELECT" || tag === "TEXTAREA") return;
      if (e.key === " " && !e.ctrlKey && !e.metaKey && !e.altKey) {
        if (saveAsDlg || renameDlg || deleteDlg || backupDlg || restoreDlg || snapRenameDlg) return;
        const b = selectedBlock;
        if (b && (b.bypassed === true || b.bypassed === false)) {
          e.preventDefault(); // also keeps Space from "clicking" a focused button
          onBypass(b.slot, !b.bypassed);
        }
        return;
      }
      if (!(e.ctrlKey || e.metaKey)) return;
      const k = e.key.toLowerCase();
      if (k === "z") {
        e.preventDefault();
        (e.shiftKey ? onRedo : onUndo)();
      } else if (k === "y") {
        e.preventDefault();
        onRedo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  async function refreshPreset() {
    try {
      preset = await invoke("read_preset");
    } catch (e) {
      /* transient; heartbeat will catch up */
    }
  }

  async function handlePushes(pushes) {
    if (!preset || !connected) return;
    let presetChanged = false;
    const bypasses = new Map(); // slot → bypassed, from footswitch pushes
    for (const p of pushes) {
      if (p.kind === "Snapshot") activeSnapshot = p.index; // read-back lags; trust the push
      else if (p.kind === "Preset") presetChanged = true;
      else if (p.kind === "Bypass") bypasses.set(p.slot, !p.enabled);
    }
    // Any device-side change (bypass, snapshot, preset) is reflected by re-reading the preset —
    // reassigning `preset` is a clean reactive update (nested in-place mutation wasn't refreshing).
    if (presetChanged) {
      selectedSlot = null;
      addTarget = null;
      bypasses.clear(); // those pushes belonged to the preset we just left
    }
    await refreshPreset();
    if (presetChanged) {
      activeSnapshot = preset?.active_snapshot ?? 0;
      // Follow the device into whichever setlist it landed in — switching presets from the panel
      // can cross setlists, and the sidebar would otherwise keep listing the old one with nothing
      // highlighted. Re-list even within the same bank, since names may have changed.
      const bank = preset?.bank ?? 0;
      if (bank !== viewBank) viewBank = bank;
      await refreshPresets(bank);
    }
    // Footswitch bypass: like snapshots, the device's readable stream lags its own push, so the
    // re-read can still carry the pre-toggle state. Overlay the pushed values onto the fresh read.
    if (bypasses.size && preset) {
      const patch = (b) => (b && bypasses.has(b.slot) ? { ...b, bypassed: bypasses.get(b.slot) } : b);
      preset = {
        ...preset,
        blocks: preset.blocks.map(patch),
        split_node: patch(preset.split_node),
        mixer_node: patch(preset.mixer_node),
      };
    }
  }

  // Run a command that returns the updated preset; keep the selection. Errors surface as toasts.
  async function apply(promise) {
    try {
      preset = await promise;
    } catch (e) {
      toast(e);
    }
  }

  const onFloat = (slot, paired, index, value) =>
    apply(invoke(paired ? "set_paired_param" : "set_param", { slot, paramIndex: index, value }));
  // Fire-and-forget mid-drag preview — audio follows the slider; errors are ignored (the commit
  // on release is authoritative and will surface real failures).
  const onPreview = (slot, paired, index, value) =>
    invoke(paired ? "preview_paired_param" : "preview_param", { slot, paramIndex: index, value }).catch(() => {});
  const onEnum = (slot, paired, index, value) =>
    apply(invoke("set_param_enum", { slot, paired, paramIndex: index, value }));
  const onBypass = (slot, bypassed) => apply(invoke("set_bypass", { slot, bypassed }));
  const onSwap = (slot, modelIndex, pairedIndex) =>
    apply(invoke("swap_model", { slot, modelIndex, pairedIndex }));
  const onSplitType = (slot, modelIndex) =>
    apply(invoke("set_split_type", { splitSlot: slot, modelIndex }));
  const onSnapshot = (index) => {
    activeSnapshot = index;
    apply(invoke("set_snapshot", { index }));
  };
  const onSnapRename = (index) =>
    (snapRenameDlg = { index, name: preset?.snapshot_names?.[index] || `SNAPSHOT ${index + 1}` });
  async function confirmSnapRename() {
    const { index, name: rawName } = snapRenameDlg;
    const name = rawName.trim();
    snapRenameDlg = null;
    if (!name) return;
    await apply(invoke("rename_snapshot", { index, name }));
    status = `Renamed snapshot ${index + 1} to "${name}".`;
  }
  const onPlace = (srcSlot, dstSlot) => apply(invoke("place_block", { srcSlot, dstSlot }));
  // Drop onto an occupied cell: insert before/after it (by drop half), shifting neighbors.
  const onInsert = (srcSlot, dstSlot, before) =>
    apply(invoke("insert_block", { srcSlot, dstSlot, before }));
  // Drag the split (⋔) / join (⋉) node to a new signal column — re-classifies top-row blocks
  // between common / path A / common-after without moving any block.
  const onMoveNode = (node, pos, dsp = 0) => apply(invoke("set_node_pos", { node, pos, dsp }));
  const onDelete = (slot) => (deleteDlg = slot);
  function confirmDelete() {
    const slot = deleteDlg;
    deleteDlg = null;
    selectedSlot = null;
    apply(invoke("delete_block", { slot }));
  }
  function onAdd(modelIndex, defaultPaired) {
    const slot = addTarget;
    addTarget = null;
    const pairedIndex = defaultPaired ?? -1; // Amp+Cab picks arrive with their matched cab
    if (slot != null && slot >= 0) {
      apply(invoke("add_block_at", { slot, modelIndex, pairedIndex }));
    } else {
      apply(invoke("add_block", { modelIndex, pairedIndex }));
    }
  }

  // ---- preset management ----
  // A device may hold several setlists (the Helix Floor has eight; the Stomp one flat list).
  // `viewBank` is the setlist the sidebar is showing, which is not necessarily the one the loaded
  // preset came from — you can browse User 2 while sitting on a Factory 1 preset.
  let setlists = $state([]);
  let viewBank = $state(0);
  const presetBank = $derived(preset?.bank ?? 0);

  async function refreshPresets(bank = viewBank) {
    try {
      presets = await invoke("list_presets", { bank });
    } catch (e) {
      toast("preset list: " + e);
    }
  }

  // Switch which setlist the sidebar lists. Browsing only — the device stays on its preset until
  // one is actually clicked.
  async function onPickSetlist(bank) {
    viewBank = bank;
    presets = [];
    await refreshPresets(bank);
  }

  async function onGoto(index) {
    selectedSlot = null;
    await apply(invoke("goto_preset", { bank: viewBank, preset: index }));
    activeSnapshot = preset?.active_snapshot ?? 0;
  }

  async function onSave() {
    if (!preset) return;
    // Overwrite in place — the bank the preset was read from, not the one being browsed.
    await apply(
      invoke("save_preset", { bank: presetBank, slot: preset.index, name: preset.name ?? "" }),
    );
    await refreshPresets();
    status = `Saved to slot ${preset.index}.`;
  }

  const onSaveAs = () => preset && (saveAsDlg = { slot: preset.index, name: preset.name ?? "" });
  // What the chosen Save As slot currently holds — shown so overwriting is always a visible choice.
  const saveAsTarget = $derived(
    saveAsDlg ? presets.find((p) => p.index === saveAsDlg.slot) : null,
  );
  async function confirmSaveAs() {
    const { slot, name: rawName } = saveAsDlg;
    const name = rawName.trim();
    saveAsDlg = null;
    // Save As picks a slot out of the visible list, so it targets the browsed setlist.
    await apply(invoke("save_preset", { bank: viewBank, slot, name }));
    await refreshPresets();
    status = `Saved to slot ${slot} as "${name}".`;
  }

  const onRename = () => preset && (renameDlg = { name: preset.name ?? "" });
  async function confirmRename() {
    const name = renameDlg.name.trim();
    renameDlg = null;
    if (!name) return;
    try {
      await invoke("rename_preset", { bank: presetBank, slot: preset.index, name });
      await refreshPresets();
      status = `Renamed slot ${preset.index} to "${name}".`;
    } catch (e) {
      toast("rename: " + e);
    }
  }

  // ---- backup / restore ----
  const onBackup = () =>
    (backupDlg = { path: `~/fretwire-backup-${new Date().toISOString().slice(0, 10)}.json` });
  async function confirmBackup() {
    const path = backupDlg.path.trim();
    backupDlg = null;
    if (!path) return;
    backupProgress = { done: 0, total: 0, name: "starting…" };
    try {
      const count = await invoke("backup_setlist", { path });
      toast(`Backed up ${count} presets to ${path}`, "info");
      status = `Backed up ${count} presets.`;
    } catch (e) {
      toast("backup: " + e);
    } finally {
      backupProgress = null;
    }
    await refreshPreset(); // the sweep reloaded the current preset and cleared the history
  }

  const onRestore = () => (restoreDlg = { path: "~/", entries: null, index: null, slot: null });
  // What the chosen restore target currently holds — overwriting must be a visible choice.
  const restoreTarget = $derived(
    restoreDlg?.slot != null ? presets.find((p) => p.index === restoreDlg.slot) : null,
  );
  async function loadBackupEntries() {
    try {
      const entries = await invoke("backup_show", { path: restoreDlg.path.trim() });
      restoreDlg = { ...restoreDlg, entries, index: null, slot: null };
    } catch (e) {
      toast("restore: " + e);
    }
  }
  async function confirmRestore() {
    const { path, index, slot } = restoreDlg;
    restoreDlg = null;
    if (index == null || slot == null) return;
    selectedSlot = null;
    await apply(invoke("restore_preset", { path: path.trim(), index, slot }));
    await refreshPresets();
    activeSnapshot = preset?.active_snapshot ?? 0;
    status = `Restored to slot ${slot}.`;
  }

  async function detect() {
    status = "Detecting…";
    statusErr = false;
    try {
      const present = await invoke("detect");
      status = present ? "HX Stomp: present ✓" : "HX Stomp: not found";
      statusErr = !present;
    } catch (e) {
      status = "Ready.";
      toast("detect: " + e);
    }
  }

  let connected = $state(false);

  async function connect() {
    status = "Connecting + reading live preset…";
    statusErr = false;
    try {
      preset = await invoke("connect");
      connected = true;
      activeSnapshot = preset.active_snapshot ?? 0;
      // Open the sidebar on the setlist the device is actually sitting in, not always Factory 1 —
      // otherwise a Floor parked in User 1 lists names that have nothing to do with its screen.
      try {
        setlists = await invoke("setlists");
      } catch (e) {
        setlists = [];
      }
      viewBank = preset.bank ?? 0;
      await refreshPresets(viewBank);
      try {
        splitTypes = await invoke("split_types");
      } catch (e) {
        /* non-fatal */
      }
      status = `Connected — ${preset.blocks.length} blocks. Session held for editing.`;
    } catch (e) {
      status = "Not connected.";
      toast("connect: " + e);
    }
  }

  async function disconnect() {
    try {
      await invoke("disconnect");
    } catch (e) {
      toast("disconnect: " + e);
      return;
    }
    connected = false;
    preset = null;
    presets = [];
    setlists = [];
    viewBank = 0;
    selectedSlot = null;
    status = "Disconnected — pedal back to standalone.";
  }

  // ---- mock-only device switch ----
  // Which unit the mock backend is pretending to be. Only ever rendered under IS_MOCK; in a real
  // Tauri build `window.fretwireMock` doesn't exist and none of this runs.
  let mockDevice = $state(IS_MOCK ? (window.fretwireMock?.device() ?? "floor") : "floor");

  // Switching device rebuilds the mock's setlists and current preset, so the open session's state
  // (setlist names, preset, grids) is stale afterwards. Reconnect for the user rather than leaving
  // a half-updated UI behind — that footgun is the whole reason this control exists.
  async function onPickMockDevice(mode) {
    if (mode === mockDevice) return;
    mockDevice = window.fretwireMock?.device(mode) ?? mode;
    if (connected) {
      await disconnect();
      await connect();
    } else {
      status = `Mock is now a ${mode === "floor" ? "Helix Floor" : "HX Stomp"}. Connect to see it.`;
    }
  }
</script>

{#if dataStatus && !dataReady}
  <FirstRun
    status={dataStatus}
    onready={(r) => {
      dataStatus = { ...dataStatus, present: true, files: r.copied };
      dataReady = true;
      status = `Imported ${r.copied} reference file(s) — model names are available.`;
      statusErr = false;
    }}
    onskip={() => {
      dataReady = true;
      status = "No reference data — blocks and parameters show numeric indices.";
      statusErr = false;
    }}
  />
{/if}

{#if dataReady}
<header>
  <h1>fretwire</h1>
  <span class="spacer"></span>
  {#if IS_MOCK}
    <!-- Mock builds only: which unit to pretend to be. Never present in a real Tauri build. -->
    <label class="mockdev" title="Mock backend only — which device to simulate">
      <span>Mock</span>
      <select value={mockDevice} onchange={(e) => onPickMockDevice(e.currentTarget.value)}>
        <option value="floor">Helix Floor</option>
        <option value="stomp">HX Stomp</option>
      </select>
    </label>
  {/if}
  <button class="secondary" onclick={detect}>Detect</button>
  {#if connected}
    <button
      class="secondary"
      disabled={!preset || preset.undo_depth === 0}
      title="Undo (Ctrl+Z)"
      onclick={onUndo}
    >↶ Undo</button>
    <button
      class="secondary"
      disabled={!preset || preset.redo_depth === 0}
      title="Redo (Ctrl+Shift+Z)"
      onclick={onRedo}
    >↷ Redo</button>
    <button class="secondary" onclick={() => (addTarget = addTarget == null ? -1 : null)}>＋ Add block</button>
    <button class="secondary" onclick={disconnect}>Disconnect</button>
  {:else}
    <button onclick={connect}>Connect</button>
  {/if}
</header>

<div class="status" class:err={statusErr}>{status}</div>

<main>
  {#if preset}
    <div class="workspace">
      <PresetList {presets} currentIndex={preset.index} dirty={preset.dirty} {setlists} {viewBank} currentBank={presetBank} {onPickSetlist} {onGoto} {onSave} {onSaveAs} {onRename} {onBackup} {onRestore} />
      <div class="content">
        <div class="meta">
          <span>
            preset <b>{preset.name ?? "—"}</b>{preset.index != null ? " #" + preset.index : ""}
            {#if preset.dirty}<span class="edited" title="Edited — not saved to the device">● edited</span>{/if}
          </span>
          <span>device <b>{preset.device_model ?? "—"}</b></span>
          <span>fw <b>{preset.firmware ?? "—"}</b></span>
          <span>routing <b>{preset.split ? "parallel (split)" : "serial"}</b></span>
          <span>DSP <b>{dspLoadLabel}</b></span>
        </div>
        {#if preset.snapshot_names.length}
          <div class="snapshots">
            <span class="snap-label">Snapshots</span>
            {#each preset.snapshot_names as name, i}
              <button
                class="snap"
                class:active={i === activeSnapshot}
                title="Click to switch — double-click to rename"
                onclick={() => onSnapshot(i)}
                ondblclick={() => onSnapRename(i)}
              >
                {name || `SS${i + 1}`}
              </button>
            {/each}
          </div>
        {/if}
        <HistoryPane
          history={preset.history ?? []}
          cursor={preset.history_cursor ?? 0}
          onjump={onHistoryJump}
        />
        {#if addTarget != null}
          <ModelPicker
            title={addTarget >= 0 ? `Add block — slot ${addTarget}` : "Add block"}
            remaining={BUDGET -
              (addTarget >= 0 ? loadForSlot(addTarget) : Math.min(...dspViews.map((v) => v.dsp_load)))}
            onpick={onAdd}
            oncancel={() => (addTarget = null)}
          />
        {/if}
        {#each dspViews as dspView (dspView.dsp)}
          {#if dspViews.length > 1}
            <div class="dsp-head">
              DSP {dspView.dsp + 1}
              <span class="dsp-load">{dspView.dsp_load.toFixed(1)}%</span>
            </div>
          {/if}
          <Chain
            {preset}
            dsp={dspView}
            {selectedSlot}
            onselect={(slot) => (selectedSlot = slot)}
            onplace={onPlace}
            oninsert={onInsert}
            onmovenode={onMoveNode}
            onaddat={(slot) => (addTarget = slot)}
          />
        {/each}
        {#if selectedBlock}
          <ParamPanel
            block={selectedBlock}
            dspLoad={selectedBlock ? loadForSlot(selectedBlock.slot) : preset.dsp_load}
            isNode={selectedIsNode}
            isSplit={selectedIsSplit}
            {splitTypes}
            {onFloat}
            {onEnum}
            {onPreview}
            {onBypass}
            {onSwap}
            {onSplitType}
            {onDelete}
          />
        {:else}
          <p class="hint">Click a block to edit its parameters.</p>
        {/if}
      </div>
    </div>
  {:else}
    <p class="hint">Click <b>Connect</b> to open a session and read the current preset from the HX Stomp.</p>
  {/if}
</main>
{/if}

{#if saveAsDlg}
  <Dialog
    title="Save As"
    confirmLabel={saveAsDlg.slot === preset?.index ? "Overwrite current slot" : `Save to slot ${saveAsDlg.slot}`}
    danger={saveAsDlg.slot !== preset?.index}
    width={420}
    onconfirm={confirmSaveAs}
    oncancel={() => (saveAsDlg = null)}
  >
    <label class="dlg-field">
      Preset name
      <input type="text" maxlength="32" bind:value={saveAsDlg.name} use:autofocus />
    </label>
    <!-- Pick the destination from the setlist, seeing exactly what each slot holds. -->
    <div class="dlg-label">Save to slot</div>
    <div class="dlg-slots">
      {#each presets as p (p.index)}
        <button
          type="button"
          class="dlg-slot"
          class:chosen={p.index === saveAsDlg.slot}
          onclick={() => (saveAsDlg.slot = p.index)}
        >
          <span class="idx">{String(p.index).padStart(3, "0")}</span>
          <span class="nm">{p.name}</span>
          {#if p.index === preset?.index}<span class="cur">current</span>{/if}
        </button>
      {/each}
    </div>
    {#if saveAsTarget && saveAsDlg.slot !== preset?.index}
      <p class="dlg-warn">Overwrites <b>{saveAsTarget.name}</b> in slot {saveAsDlg.slot}.</p>
    {/if}
  </Dialog>
{/if}

{#if renameDlg}
  <Dialog
    title="Rename preset"
    confirmLabel="Rename"
    onconfirm={confirmRename}
    oncancel={() => (renameDlg = null)}
  >
    <label class="dlg-field">
      Preset name
      <input type="text" maxlength="32" bind:value={renameDlg.name} use:autofocus />
    </label>
  </Dialog>
{/if}

{#if snapRenameDlg}
  <Dialog
    title={`Rename snapshot ${snapRenameDlg.index + 1}`}
    confirmLabel="Rename"
    width={360}
    onconfirm={confirmSnapRename}
    oncancel={() => (snapRenameDlg = null)}
  >
    <label class="dlg-field">
      Snapshot name
      <input type="text" maxlength="14" bind:value={snapRenameDlg.name} use:autofocus />
    </label>
  </Dialog>
{/if}

{#if backupDlg}
  <Dialog
    title="Backup setlist"
    confirmLabel="Start backup"
    width={420}
    onconfirm={confirmBackup}
    oncancel={() => (backupDlg = null)}
  >
    <label class="dlg-field">
      Backup file
      <input type="text" bind:value={backupDlg.path} use:autofocus />
    </label>
    <p class="dlg-text">
      Reads every preset to the file — nothing on the device is written. The pedal steps through
      the whole setlist (audio follows along), and unsaved edits to the current preset are
      reloaded from flash.
    </p>
  </Dialog>
{/if}

{#if restoreDlg}
  <Dialog
    title="Restore preset"
    confirmLabel={restoreDlg.slot != null ? `Restore to slot ${restoreDlg.slot}` : "Restore"}
    confirmDisabled={restoreDlg.index == null || restoreDlg.slot == null}
    danger
    width={560}
    onconfirm={confirmRestore}
    oncancel={() => (restoreDlg = null)}
  >
    <label class="dlg-field">
      Backup file
      <span class="dlg-row">
        <input type="text" bind:value={restoreDlg.path} use:autofocus />
        <button type="button" class="dlg-btn" onclick={loadBackupEntries}>Load</button>
      </span>
    </label>
    {#if restoreDlg.entries}
      <div class="dlg-cols">
        <div class="dlg-col">
          <div class="dlg-label">Preset in backup</div>
          <div class="dlg-slots">
            {#each restoreDlg.entries as e (e.index)}
              <button
                type="button"
                class="dlg-slot"
                class:chosen={e.index === restoreDlg.index}
                onclick={() => (restoreDlg = { ...restoreDlg, index: e.index, slot: e.index })}
              >
                <span class="idx">{String(e.index).padStart(3, "0")}</span>
                <span class="nm">{e.name}</span>
              </button>
            {/each}
          </div>
        </div>
        <div class="dlg-col">
          <div class="dlg-label">Restore to slot</div>
          <div class="dlg-slots">
            {#each presets as p (p.index)}
              <button
                type="button"
                class="dlg-slot"
                class:chosen={p.index === restoreDlg.slot}
                onclick={() => (restoreDlg = { ...restoreDlg, slot: p.index })}
              >
                <span class="idx">{String(p.index).padStart(3, "0")}</span>
                <span class="nm">{p.name}</span>
                {#if p.index === preset?.index}<span class="cur">current</span>{/if}
              </button>
            {/each}
          </div>
        </div>
      </div>
      {#if restoreTarget && restoreDlg.index != null}
        <p class="dlg-warn">Overwrites <b>{restoreTarget.name}</b> in slot {restoreDlg.slot} on the device.</p>
      {/if}
    {:else}
      <p class="dlg-text">Enter the backup file's path and press <b>Load</b> to list its presets.</p>
    {/if}
  </Dialog>
{/if}

{#if backupProgress}
  <div class="bk-overlay">
    <div class="bk-card">
      <div class="bk-title">Backing up setlist…</div>
      <div class="bk-bar">
        <div
          class="bk-fill"
          style="width:{backupProgress.total ? (100 * backupProgress.done) / backupProgress.total : 0}%"
        ></div>
      </div>
      <div class="bk-line">
        {backupProgress.done}/{backupProgress.total || "…"} — {backupProgress.name}
      </div>
    </div>
  </div>
{/if}

{#if deleteDlg != null}
  <Dialog
    title="Delete block"
    confirmLabel="Delete"
    danger
    onconfirm={confirmDelete}
    oncancel={() => (deleteDlg = null)}
  >
    <p class="dlg-text">Delete <b>{deleteName}</b> from the chain?</p>
  </Dialog>
{/if}

<Toast {toasts} ondismiss={dismissToast} />

<style>
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px 16px;
    border-bottom: 1px solid #2a2e37;
    background: #1b1e25;
  }
  h1 {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
    letter-spacing: 0.2px;
  }
  .spacer {
    flex: 1;
  }
  /* Mock-only device switch. Deliberately understated and dashed — it must never read as a real
     control of the hardware. */
  .mockdev {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px 3px 10px;
    border: 1px dashed #3a4050;
    border-radius: 6px;
  }
  .mockdev span {
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #7a8296;
  }
  .mockdev select {
    font: inherit;
    font-size: 12px;
    background: #23262e;
    color: #c8cdd8;
    border: 1px solid #2a2e37;
    border-radius: 5px;
    padding: 3px 5px;
    cursor: pointer;
  }
  button {
    font: inherit;
    color: #e6e8ec;
    background: #2b7de0;
    border: 0;
    padding: 7px 14px;
    border-radius: 7px;
    cursor: pointer;
  }
  button.secondary {
    background: #363b46;
  }
  button:active {
    transform: translateY(1px);
  }
  button:disabled {
    opacity: 0.4;
    cursor: not-allowed;
    transform: none;
  }
  .status {
    padding: 8px 16px;
    color: #9aa3b2;
    border-bottom: 1px solid #2a2e37;
  }
  .status.err {
    color: #ff8a8a;
  }
  main {
    padding: 20px 16px;
  }
  .workspace {
    display: flex;
    gap: 16px;
    align-items: flex-start;
  }
  .content {
    flex: 1;
    min-width: 0;
  }
  .snapshots {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 14px;
    flex-wrap: wrap;
  }
  .snap-label {
    color: #9aa3b2;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
    margin-right: 4px;
  }
  .snap {
    font: inherit;
    font-size: 13px;
    background: #232833;
    color: #c3c9d4;
    border: 1px solid #3a4150;
    border-radius: 6px;
    padding: 5px 12px;
    cursor: pointer;
  }
  .snap.active {
    background: #26333f;
    border-color: #3f8ae0;
    color: #fff;
  }
  .meta {
    color: #9aa3b2;
    margin-bottom: 14px;
    display: flex;
    gap: 18px;
    flex-wrap: wrap;
  }
  .meta b {
    color: #e6e8ec;
    font-weight: 600;
  }
  .meta .edited {
    color: #f0c245;
    font-size: 12px;
    margin-left: 6px;
  }
  .hint {
    color: #9aa3b2;
  }
  /* Per-DSP label above each routing grid — only shown on multi-DSP devices (the Floor). */
  .dsp-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 14px 0 6px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.5px;
    color: #9aa3b2;
    text-transform: uppercase;
  }
  .dsp-head .dsp-load {
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0;
    color: #6d7688;
    text-transform: none;
  }
  .dlg-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: #c3c9d4;
    font-size: 13px;
    margin-bottom: 10px;
  }
  .dlg-field input {
    font: inherit;
    background: #232833;
    color: #e6e8ec;
    border: 1px solid #3a4150;
    border-radius: 6px;
    padding: 6px 10px;
  }
  .dlg-field input:focus {
    outline: none;
    border-color: #3f8ae0;
  }
  .dlg-text {
    color: #c3c9d4;
    font-size: 13px;
    margin: 0;
  }
  .dlg-text b {
    color: #e6e8ec;
  }
  .dlg-label {
    color: #c3c9d4;
    font-size: 13px;
    margin-bottom: 4px;
  }
  .dlg-slots {
    max-height: 260px;
    overflow-y: auto;
    border: 1px solid #2a2e37;
    border-radius: 8px;
    padding: 4px;
    background: #12141a;
  }
  .dlg-slot {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    text-align: left;
    font: inherit;
    font-size: 13px;
    background: transparent;
    color: #c3c9d4;
    border: 0;
    border-radius: 6px;
    padding: 5px 8px;
    cursor: pointer;
  }
  .dlg-slot:hover {
    background: #232833;
  }
  .dlg-slot.chosen {
    background: #26333f;
    color: #fff;
    outline: 1px solid #3f8ae0;
  }
  .dlg-slot .idx {
    color: #6b7280;
    font-variant-numeric: tabular-nums;
    font-size: 12px;
  }
  .dlg-slot.chosen .idx {
    color: #9ec5f0;
  }
  .dlg-slot .nm {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dlg-slot .cur {
    color: #f0c245;
    font-size: 11px;
  }
  .dlg-warn {
    color: #e0a83f;
    font-size: 13px;
    margin: 10px 0 0;
  }
  .dlg-warn b {
    color: #ffd27a;
  }
  .dlg-row {
    display: flex;
    gap: 6px;
  }
  .dlg-row input {
    flex: 1;
    min-width: 0;
  }
  .dlg-btn {
    font: inherit;
    font-size: 13px;
    background: #363b46;
    color: #e6e8ec;
    border: 0;
    border-radius: 6px;
    padding: 6px 14px;
    cursor: pointer;
  }
  .dlg-cols {
    display: flex;
    gap: 10px;
  }
  .dlg-col {
    flex: 1;
    min-width: 0;
  }
  /* Backup-progress overlay: modal but button-less — the sweep isn't cancellable mid-flight. */
  .bk-overlay {
    position: fixed;
    inset: 0;
    background: rgba(8, 10, 14, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 110;
  }
  .bk-card {
    width: 380px;
    background: #1b1e25;
    border: 1px solid #3a4150;
    border-radius: 12px;
    padding: 16px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.5);
  }
  .bk-title {
    font-weight: 600;
    font-size: 14px;
    margin-bottom: 12px;
  }
  .bk-bar {
    height: 8px;
    background: #232833;
    border-radius: 4px;
    overflow: hidden;
    margin-bottom: 8px;
  }
  .bk-fill {
    height: 100%;
    background: #2b7de0;
    transition: width 120ms linear;
  }
  .bk-line {
    color: #9aa3b2;
    font-size: 13px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
