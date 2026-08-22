<script>
  import { invoke, listen, IS_MOCK } from "./lib/ipc.js";
  import { onMount } from "svelte";
  import Chain from "./lib/Chain.svelte";
  import ParamPanel from "./lib/ParamPanel.svelte";
  import ModelPicker from "./lib/ModelPicker.svelte";
  import PresetList from "./lib/PresetList.svelte";
  import HistoryPane from "./lib/HistoryPane.svelte";
  import IrPanel from "./lib/IrPanel.svelte";
  import Dialog from "./lib/Dialog.svelte";
  import Toast from "./lib/Toast.svelte";
  import FirstRun from "./lib/FirstRun.svelte";
  import { slotLabel } from "./lib/numbering.svelte.js";


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

  // The status line reports the last thing that happened, which makes it a liar the moment the
  // loaded preset changes out from under it. Both halves showed up in the tester's screenshots: a
  // "Connected — 15 blocks" line still on screen beside a 9-block preset, and "Saved to slot 7."
  // sitting over a preset in a different setlist. So re-state it whenever the identity moves —
  // whichever path moved it (sidebar click, pedal knob, restore). Not $state: this is a marker for
  // the effect below, and tracking it would re-run the effect on its own write.
  let announced = null;
  const presetKey = (p) => (p ? `${p.bank ?? 0}/${p.index ?? 0}` : null);
  $effect(() => {
    if (!preset) {
      announced = null;
      return;
    }
    const key = presetKey(preset);
    if (key === announced) return;
    announced = key;
    const n = preset.blocks.length;
    status = `${preset.name || "Preset"} — ${n} block${n === 1 ? "" : "s"}.`;
  });

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
  // Errors linger: they carry device refusal codes and byte counts a tester has to copy down by
  // hand, and 6 s is not enough to transcribe one. Confirmations ("Backed up 12 presets") are
  // read at a glance, so they keep the shorter life. Temporary — asked for while the Floor
  // lockups are being chased; revisit once the errors stop being the interesting part.
  const TOAST_MS = { error: 18000, info: 6000 };
  function toast(msg, kind = "error") {
    const id = ++toastSeq;
    toasts = [...toasts, { id, msg: String(msg), kind }];
    setTimeout(() => dismissToast(id), TOAST_MS[kind] ?? TOAST_MS.error);
  }
  let saveAsDlg = $state(null); // { slot, name }
  let renameDlg = $state(null); // { name }
  let deleteDlg = $state(null); // slot number
  let exportDlg = $state(null); // { path, all }
  // True from pressing Cancel until the sweep unwinds — the file is still written, holding what
  // was read up to that point, so the finished toast has to say which of the two happened.
  let exportCancelling = $state(false);
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
  // What a DSP fills up at, on the same scale as `dsp_load`. **Not 100** — the pedal starts
  // refusing blocks a quarter short of it (measured on both devices; see `editor::DSP_CEILING`).
  // The backend sends the figure so there is one copy of it; the fallback covers the mock backend.
  const BUDGET = $derived(preset?.dsp_ceiling ?? 75);
  // Each DSP is budgeted on its own, so a block's fit is judged against *its* DSP's load, not the
  // combined total (which can exceed the ceiling on the Floor). Slots are global: `dsp*20+index`.
  const loadForSlot = (slot) => {
    const v = dspViews.find((x) => x.dsp === Math.floor(slot / 20));
    return v ? v.dsp_load : (preset?.dsp_load ?? 0);
  };
  // Every DSP figure on screen is a percentage of what the pedal will actually accept, so the
  // ceiling reads 100%. The raw `dsp_load` is a percentage of a budget the hardware never gives
  // you — 72.7 of it meant "nearly full", which is the opposite of how it read. Raw units stay in
  // the CLI and the docs; nothing in the GUI shows them.
  const pct = (raw) => (raw / BUDGET) * 100;
  // Header readout, per DSP. Rendered as one labelled chip each rather than a joined string: on
  // the Floor the two used to run together as "96.9% · 3.1% free · 0.0% · 100.0% free", with the
  // same separator between the DSPs as inside them and nothing saying which was which.
  const dspUsed = (v) => Math.min(100, Math.max(0, pct(v.dsp_load)));
  const dspFree = (v) => Math.max(pct(BUDGET - v.dsp_load), 0);
  // Within a few points of the ceiling — the picker is already greying most models out by here.
  const dspTight = (v) => dspFree(v) < 10;

  // Whether the selected slot is a structural node (split/mixer/input/output) rather than a normal
  // block — nodes aren't swappable or deletable.
  const selectedIsNode = $derived(
    !!preset && selectedSlot != null && allNodes.some((n) => n?.slot === selectedSlot),
  );
  const selectedIsSplit = $derived(
    !!preset && dspViews.some((v) => v.split_node?.slot === selectedSlot),
  );

  // How the pedal writes the loaded preset's slot on its own screen (`09A`). Read off the listing
  // rather than recomputed, so there is one source for it; falls back to `#24` on a device whose
  // banking we haven't seen, and to nothing at all before the listing arrives.
  const presetLabel = $derived.by(() => {
    if (preset?.index == null) return null;
    const row = presets.find((p) => p.index === preset.index && p.bank === presetBank);
    return row ? slotLabel(row) : "#" + preset.index;
  });

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
    // The pedal stopped answering and the backend closed the session out from under us. Fall back
    // to the disconnected view rather than leaving a UI whose every button will fail.
    const unLost = listen("device-lost", (e) => onDeviceLost(e.payload));
    return () => {
      unlisten.then((f) => f());
      unProgress.then((f) => f());
      unLost.then((f) => f());
      clearTimeout(pushTimer); // don't let a coalesced refresh fire into a torn-down session
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
        if (saveAsDlg || renameDlg || deleteDlg || exportDlg || restoreDlg || snapRenameDlg) return;
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

  // ---- device-push coalescing ----
  //
  // One turn of the preset knob emits a *flurry* of pushes — the preset change, then snapshot and
  // bypass pushes as the new preset settles — spread over about a second, and the heartbeat hands
  // them to us in 250 ms batches. Refreshing on every batch cost ~3 full preset streams plus a
  // preset-list re-read per knob turn (~530 KB across 21 preset changes in the tester's 2026-07-26
  // session), all fired at a Helix Floor that was still reconfiguring both DSPs — and twice it
  // stopped answering. So fold the batches together and read *once*, after the device goes quiet.
  const PUSH_QUIET_MS = 300; // no pushes for this long → treat the device as settled
  const PUSH_MAX_WAIT_MS = 1200; // ...but never defer a refresh longer than this
  let pushTimer = null;
  let pushDeadline = 0;
  let flushing = false; // a refresh is mid-flight; don't start a second
  let flushAgain = false; // ...and pushes arrived while it was, so re-arm afterwards
  let pendingPresetChange = false;
  const pendingBypasses = new Map(); // slot → bypassed, from footswitch pushes
  const pendingParams = new Map(); // paramKey(slot, param, extra) → value, from panel-knob pushes

  // A push addresses either the model's param list or the block's extra values, and **both spaces
  // start at 0** — so the space has to be part of the key. Keying on the number alone delivered
  // Trails (extra 0) to the model's param 0, which is how a tester found this: turning Trails on
  // the pedal swept the Time slider. See PushDto::Param / ParamDto::extra_index.
  const paramKey = (slot, param, extra) => `${slot}:${extra ? "x" : "p"}${param}`;

  // A footswitch bypass is fully described by its own push, so apply it directly. This is also why
  // the re-read can't be trusted for it: the device's readable stream lags its own push, so a fresh
  // read can still carry the pre-toggle state. Overlaying wins either way.
  function applyBypasses() {
    if (!preset || !pendingBypasses.size) return;
    const patch = (b) =>
      b && pendingBypasses.has(b.slot) ? { ...b, bypassed: pendingBypasses.get(b.slot) } : b;
    preset = {
      ...preset,
      blocks: preset.blocks.map(patch),
      split_node: patch(preset.split_node),
      mixer_node: patch(preset.mixer_node),
    };
    pendingBypasses.clear();
  }

  // Panel knobs. Keyed "slot:param" because a sweep pushes ~15 updates a second and only the last
  // one matters. Applied straight to the value we already hold — the push carries it, so re-reading
  // the whole preset for every notch would flood the device for no new information.
  function applyParams() {
    if (!preset || !pendingParams.size) return;
    const patch = (b) => {
      if (!b?.params) return b;
      let touched = false;
      const params = b.params.map((p) => {
        const isExtra = p.extra_index != null;
        const v = pendingParams.get(paramKey(b.slot, isExtra ? p.extra_index : p.index, isExtra));
        if (v === undefined || v === p.value) return p;
        touched = true;
        return { ...p, value: v };
      });
      return touched ? { ...b, params } : b;
    };
    preset = {
      ...preset,
      blocks: preset.blocks.map(patch),
      split_node: patch(preset.split_node),
      mixer_node: patch(preset.mixer_node),
    };
    pendingParams.clear();
  }

  function scheduleFlush() {
    // A flush is already talking to the device — let it finish and re-arm on the way out, rather
    // than firing a second read on top of the first. Overlapping them would reintroduce exactly the
    // read pile-up this whole mechanism exists to prevent.
    if (flushing) {
      flushAgain = true;
      return;
    }
    const now = Date.now();
    if (!pushDeadline) pushDeadline = now + PUSH_MAX_WAIT_MS;
    clearTimeout(pushTimer);
    pushTimer = setTimeout(flushPushes, Math.max(0, Math.min(PUSH_QUIET_MS, pushDeadline - now)));
  }

  async function flushPushes() {
    pushTimer = null;
    pushDeadline = 0;
    const presetChanged = pendingPresetChange;
    pendingPresetChange = false;
    if (!connected) return;
    flushing = true;
    try {
      await runFlush(presetChanged);
    } finally {
      flushing = false;
    }
    if (flushAgain) {
      flushAgain = false;
      scheduleFlush();
    }
  }

  async function runFlush(presetChanged) {
    await refreshPreset();
    if (presetChanged) {
      activeSnapshot = preset?.active_snapshot ?? 0;
      // Follow the device into whichever setlist it landed in — switching presets from the panel
      // can cross setlists, and the sidebar would otherwise keep listing the old one with nothing
      // highlighted. Only re-list when the bank actually changed: moving *within* a setlist can't
      // alter that setlist's contents, and the listing is a second multi-KB stream off the device.
      const bank = preset?.bank ?? 0;
      if (bank !== viewBank) {
        viewBank = bank;
        await refreshPresets(bank);
      }
    }
    applyBypasses();
    applyParams();
  }

  function handlePushes(pushes) {
    if (!preset || !connected) return;
    // Snapshot and preset changes rewrite state we can't derive from the push alone (a snapshot
    // carries its own bypass matrix and parameter values), so they need a re-read. A bypass does
    // not.
    let needsRead = false;
    for (const p of pushes) {
      if (p.kind === "Snapshot") {
        activeSnapshot = p.index; // read-back lags; trust the push
        needsRead = true;
      } else if (p.kind === "Preset") {
        pendingPresetChange = true;
        needsRead = true;
      } else if (p.kind === "Bypass") {
        pendingBypasses.set(p.slot, !p.enabled);
      } else if (p.kind === "Param") {
        pendingParams.set(paramKey(p.slot, p.param, p.extra), p.value);
      }
    }
    if (pendingPresetChange) {
      selectedSlot = null;
      addTarget = null;
      pendingBypasses.clear(); // those pushes belonged to the preset we just left
      pendingParams.clear();
    } else if (!needsRead) {
      // bypass/knob only: the push carries everything, so show it without touching the device
      applyBypasses();
      applyParams();
      return;
    }
    scheduleFlush();
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
  // ---- controller assignments ----
  // Two mechanisms the device keeps apart, so we do too: a block's *bypass* on a footswitch
  // (ops 56/57) and a *parameter* under a controller (op 37). See docs/protocol.md.
  //
  // `switch` is zero-based on the wire while `block.footswitch` is one-based, so the panel converts
  // at the boundary rather than leaving two conventions loose in the UI.
  const onBypassSwitch = (slot, oneBased, wasOneBased) =>
    apply(
      oneBased > 0
        ? invoke("assign_bypass", { slot, switch: oneBased - 1 })
        : invoke("unassign_bypass", { slot, switch: wasOneBased - 1 }),
    );
  const onAssignParam = (slot, paired, index, source) =>
    apply(invoke("assign_param", { slot, paired, paramIndex: index, source }));
  const onAssignTravel = (slot, paired, index, max, value) =>
    apply(invoke("set_assign_travel", { slot, paired, paramIndex: index, max, value }));
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

  // ---- copy/paste ----
  // HX Edit can copy a preset (or a block) and paste it onto another slot; without it a tester
  // rebuilding a case by hand does it click by click. The blobs live in the Rust side — the UI only
  // learns the name, for the button label.
  let presetClip = $state(null);
  let blockClip = $state(null);
  async function onCopyPreset() {
    try {
      presetClip = await invoke("copy_preset");
      toast(`Copied preset "${presetClip}"`, "info");
    } catch (e) {
      toast(e);
    }
  }
  // Paste lands in the edit buffer, like every other edit — Save commits it.
  const onPastePreset = () => apply(invoke("paste_preset"));
  async function onCopyBlock(slot) {
    try {
      blockClip = await invoke("copy_block", { slot });
      toast(`Copied block "${blockClip}"`, "info");
    } catch (e) {
      toast(e);
    }
  }
  const onPasteBlock = (slot) => apply(invoke("paste_block", { slot }));
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
  // Writing into a setlist the device isn't in is gated on the backend (FRETWIRE_SETLISTS=1) until
  // a Helix Floor gets through a session cleanly. Mirror it here so Save As greys out with a reason
  // rather than failing at the wire after the user has typed a name.
  let crossSetlistWrite = $state(true);
  const foreignSetlist = $derived(!crossSetlistWrite && viewBank !== presetBank);

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

  const onSaveAs = () => {
    if (!preset) return;
    if (foreignSetlist) {
      toast(
        `Save As is limited to ${setlists[presetBank] ?? "the device's setlist"} — writing into ` +
          `another setlist is untested on this hardware. Set FRETWIRE_SETLISTS=1 to allow it.`,
      );
      return;
    }
    saveAsDlg = { slot: preset.index, name: preset.name ?? "" };
  };
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

  // ---- export / import ----
  // "Export", not "Backup": this captures presets and nothing else. A device backup in HX Edit's
  // sense also carries global settings and IRs, neither of which we can read yet, and a file that
  // called itself a backup would be trusted to make a wiped pedal whole. It wouldn't.
  const onExport = () =>
    (exportDlg = {
      path: `~/fretwire-presets-${new Date().toISOString().slice(0, 10)}.json`,
      all: false,
    });
  async function confirmExport() {
    const path = exportDlg.path.trim();
    // Whatever setlist you are looking at, not whichever one the device happens to sit in — the
    // sidebar shows one list and the button used to export bank 0 regardless.
    const banks = exportDlg.all ? setlists.map((_, i) => i) : [viewBank];
    exportDlg = null;
    if (!path) return;
    exportCancelling = false;
    backupProgress = { done: 0, total: 0, name: "starting…" };
    try {
      const count = await invoke("export_setlists", { path, banks });
      const how = exportCancelling ? "Cancelled —" : "Exported";
      toast(`${how} ${count} presets to ${path}`, exportCancelling ? "warn" : "info");
      status = `${how} ${count} presets.`;
    } catch (e) {
      toast("export: " + e);
    } finally {
      backupProgress = null;
      exportCancelling = false;
    }
    await refreshPreset(); // the sweep reloaded the current preset and cleared the history
  }
  async function cancelExport() {
    exportCancelling = true;
    try {
      await invoke("cancel_export");
    } catch (e) {
      toast("cancel: " + e);
    }
  }

  const onRestore = () => (restoreDlg = { path: "~/", entries: null, index: null, slot: null });
  // What the chosen restore target currently holds — overwriting must be a visible choice.
  const restoreTarget = $derived(
    restoreDlg?.slot != null ? presets.find((p) => p.index === restoreDlg.slot) : null,
  );
  async function loadBackupEntries() {
    try {
      const entries = await invoke("backup_show", { path: restoreDlg.path.trim() });
      restoreDlg = { ...restoreDlg, entries, index: null, slot: null, bank: null };
    } catch (e) {
      toast("restore: " + e);
    }
  }
  async function confirmRestore() {
    const { path, index, slot, bank } = restoreDlg;
    restoreDlg = null;
    if (index == null || slot == null) return;
    selectedSlot = null;
    // The entry's own bank, so a preset exported from USER 1 goes back to USER 1 rather than
    // whichever list bank 0 happens to be.
    await apply(invoke("restore_preset", { path: path.trim(), index, slot, bank: bank ?? 0 }));
    await refreshPresets();
    activeSnapshot = preset?.active_snapshot ?? 0;
    status = `Restored to slot ${slot}.`;
  }

  async function detect() {
    status = "Detecting…";
    statusErr = false;
    try {
      // Name what is actually plugged in. This used to say "HX Stomp" whatever it found, which on
      // an HX Stomp XL reads as fretwire failing to tell the two apart.
      const found = await invoke("detect");
      if (found.length === 0) {
        status = "No HX device found";
        statusErr = true;
      } else {
        status =
          found.map((d) => d.name + (d.caveat ? ` (${d.caveat})` : "")).join(", ") + ": present ✓";
        statusErr = false;
      }
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
      // Claim this one so the re-state effect doesn't race the connect message below and replace it
      // with the plainer per-preset line. Everything after connect is fair game for the effect.
      announced = presetKey(preset);
      connected = true;
      activeSnapshot = preset.active_snapshot ?? 0;
      // Open the sidebar on the setlist the device is actually sitting in, not always Factory 1 —
      // otherwise a Floor parked in User 1 lists names that have nothing to do with its screen.
      try {
        setlists = await invoke("setlists");
      } catch (e) {
        setlists = [];
      }
      try {
        crossSetlistWrite = await invoke("cross_setlist_write_allowed");
      } catch (e) {
        crossSetlistWrite = true; // the mock has no such command; it can't touch hardware anyway
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

  // The backend dropped the session because the pedal went unresponsive. Same teardown as an
  // explicit disconnect, but the status says what happened and what to do about it — there is no
  // "reconnect" that works here until the unit is power-cycled.
  function onDeviceLost(message) {
    connected = false;
    preset = null;
    presets = [];
    setlists = [];
    viewBank = 0;
    selectedSlot = null;
    status = message ?? "The pedal stopped responding. Power-cycle it, then reconnect.";
    toast(status);
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

  // ---- user IR slots ----
  //
  // Deliberately not part of the preset flow: these do not touch the edit buffer, do not appear on
  // the undo timeline, and every write is flash. The panel owns its own confirmations; this side
  // only carries the calls and turns failures into toasts.
  let showIrs = $state(false);
  let irSlots = $state([]);
  let irBusy = $state(false);

  async function irCall(fn, after) {
    irBusy = true;
    try {
      const slots = await fn();
      if (slots) irSlots = slots;
      after?.();
    } catch (e) {
      toast("IR: " + e);
    } finally {
      irBusy = false;
    }
  }

  const refreshIrs = () => irCall(() => invoke("ir_list"));
  const scanIrs = () => irCall(() => invoke("ir_scan"));

  function openIrs() {
    showIrs = true;
    refreshIrs();
  }

  const onIrExport = (slot, path) =>
    irCall(
      async () => {
        const name = await invoke("ir_export", { slot, path });
        toast(`Exported "${name}" to ${path}`, "info");
        return null;
      },
    );

  const onIrUpload = (job) =>
    irCall(async () => {
      const slots = await invoke("ir_upload", {
        slot: job.slot,
        path: job.path,
        name: job.name.trim(),
        overwrite: job.overwrite,
        force: job.force,
      });
      toast(`Uploaded "${job.name.trim()}" to IR ${String(job.slot + 1).padStart(3, "0")}`, "info");
      return slots;
    });

  const onIrDelete = (slot) =>
    irCall(async () => {
      const slots = await invoke("ir_delete", { slot });
      toast(`Emptied IR ${String(slot + 1).padStart(3, "0")}`, "info");
      return slots;
    });

  const onIrRename = (slot, name) =>
    irCall(() => invoke("ir_rename", { slot, name }));
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
        <option value="xl">HX Stomp XL</option>
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
    <button class="secondary" onclick={openIrs} title="Manage the pedal's user impulse responses">IRs…</button>
    <button class="secondary" onclick={disconnect}>Disconnect</button>
  {:else}
    <button onclick={connect}>Connect</button>
  {/if}
</header>

<div class="status" class:err={statusErr}>{status}</div>

<main>
  {#if preset}
    <div class="workspace">
      <PresetList {presets} currentIndex={preset.index} dirty={preset.dirty} {setlists} {viewBank} currentBank={presetBank} writeBlocked={foreignSetlist} {onPickSetlist} {onGoto} {onSave} {onSaveAs} {onRename} {onExport} {onRestore} {onCopyPreset} {onPastePreset} {presetClip} />
      <div class="content">
        <div class="meta">
          <span>
            preset <b>{preset.name ?? "—"}</b>{presetLabel ? " " + presetLabel : ""}
            {#if preset.dirty}<span class="edited" title="Edited — not saved to the device">● edited</span>{/if}
          </span>
          <span>device <b>{preset.device_model ?? "—"}</b></span>
          <span
            title="A build id stamped inside the preset (key 7 → 37), not your pedal's firmware version — an HX Stomp on 3.80 reports v3.71-32-g1039661, and so does an HX Stomp XL on 3.80.0. The suffix reads as 32 commits past a tag named v3.71, so it names a build inside the firmware rather than a release."
          >preset build <b>{preset.build_stamp ?? "—"}</b></span>
          <span>routing <b>{preset.split ? "parallel (split)" : "serial"}</b></span>
          {#each dspViews as v (v.dsp)}
            <span
              class="dsp-chip"
              class:tight={dspTight(v)}
              title="{dspViews.length > 1 ? `DSP ${v.dsp + 1}` : 'DSP'}: {dspUsed(v).toFixed(
                1,
              )}% of what the pedal will accept, {dspFree(v).toFixed(1)}% free"
            >
              <span class="dsp-name">{dspViews.length > 1 ? `DSP ${v.dsp + 1}` : "DSP"}</span>
              <span class="dsp-bar"><span class="fill" style="width:{dspUsed(v)}%"></span></span>
              <b>{dspUsed(v).toFixed(1)}%</b>
              <span class="dsp-free">{dspFree(v).toFixed(1)}% free</span>
            </span>
          {/each}
        </div>
        {#if preset.snapshot_names.length}
          <div class="snapshots">
            <span class="snap-label">Snapshots</span>
            {#each preset.snapshot_names as name, i}
              <button
                class="snap"
                class:active={i === activeSnapshot}
                title="Click to switch — right-click (or double-click) to rename"
                onclick={() => onSnapshot(i)}
                ondblclick={() => onSnapRename(i)}
                oncontextmenu={(e) => {
                  // The webview would otherwise pop its own menu (Reload/Inspect), which is both
                  // useless here and hides ours. Double-click still works; right-click is what
                  // people reach for, and it was undiscoverable behind a tooltip.
                  e.preventDefault();
                  onSnapRename(i);
                }}
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
            budget={BUDGET}
            onpick={onAdd}
            oncancel={() => (addTarget = null)}
          />
        {/if}
        {#each dspViews as dspView (dspView.dsp)}
          {#if dspViews.length > 1}
            <div class="dsp-head" class:tight={dspTight(dspView)}>
              DSP {dspView.dsp + 1}
              <span class="dsp-bar"
                ><span class="fill" style="width:{dspUsed(dspView)}%"></span></span
              >
              <span class="dsp-load"
                >{dspUsed(dspView).toFixed(1)}% used · {dspFree(dspView).toFixed(1)}% free</span
              >
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
            budget={BUDGET}
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
            {onCopyBlock}
            {onPasteBlock}
            {blockClip}
            assignments={preset?.assignments ?? []}
            footswitchCount={preset?.footswitch_count ?? 0}
            {onBypassSwitch}
            {onAssignParam}
            {onAssignTravel}
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
          <span class="idx">{slotLabel(p)}</span>
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

{#if exportDlg}
  <Dialog
    title="Export presets"
    confirmLabel="Start export"
    width={460}
    onconfirm={confirmExport}
    oncancel={() => (exportDlg = null)}
  >
    <label class="dlg-field">
      Export file
      <input type="text" bind:value={exportDlg.path} use:autofocus />
    </label>
    <!-- Only worth asking on a device that has more than one list. -->
    {#if setlists.length > 1}
      <div class="dlg-field">
        What to export
        <label class="dlg-check">
          <input type="radio" checked={!exportDlg.all} onchange={() => (exportDlg = { ...exportDlg, all: false })} />
          This setlist — <b>{setlists[viewBank] ?? `bank ${viewBank}`}</b>
        </label>
        <label class="dlg-check">
          <input type="radio" checked={exportDlg.all} onchange={() => (exportDlg = { ...exportDlg, all: true })} />
          All {setlists.length} setlists — every preset on the device
        </label>
      </div>
    {/if}
    <p class="dlg-text">
      Reads presets to the file — nothing on the device is written. The pedal steps through every
      preset exported (audio follows along), and unsaved edits to the current preset are reloaded
      from flash. You can cancel partway; the file keeps what was read.
    </p>
    <p class="dlg-text">
      <b>Presets only.</b> Global settings and IRs are not in the file, so this is not a full
      device backup.
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
            <!-- Keyed by bank *and* slot: a multi-setlist file has eight entries numbered 000. -->
            {#each restoreDlg.entries as e (`${e.bank}:${e.index}`)}
              <button
                type="button"
                class="dlg-slot"
                class:chosen={e.index === restoreDlg.index && e.bank === restoreDlg.bank}
                onclick={() =>
                  (restoreDlg = { ...restoreDlg, index: e.index, bank: e.bank, slot: e.index })}
              >
                <span class="idx">{slotLabel(e)}</span>
                <span class="nm">{e.name}</span>
                {#if e.setlist && setlists.length > 1}<span class="cur">{e.setlist}</span>{/if}
              </button>
            {/each}
          </div>
        </div>
        <div class="dlg-col">
          <div class="dlg-label">
            Restore to slot{#if setlists.length > 1} in {setlists[viewBank] ?? `bank ${viewBank}`}{/if}
          </div>
          <div class="dlg-slots">
            {#each presets as p (p.index)}
              <button
                type="button"
                class="dlg-slot"
                class:chosen={p.index === restoreDlg.slot}
                onclick={() => (restoreDlg = { ...restoreDlg, slot: p.index })}
              >
                <span class="idx">{slotLabel(p)}</span>
                <span class="nm">{p.name}</span>
                {#if p.index === preset?.index}<span class="cur">current</span>{/if}
              </button>
            {/each}
          </div>
        </div>
      </div>
      {#if restoreTarget && restoreDlg.index != null}
        <p class="dlg-warn">
          Overwrites <b>{restoreTarget.name}</b> in slot {restoreDlg.slot}
          {#if setlists.length > 1}of <b>{setlists[restoreDlg.bank] ?? `bank ${restoreDlg.bank}`}</b>{/if}
          on the device.
        </p>
      {/if}
    {:else}
      <p class="dlg-text">Enter the backup file's path and press <b>Load</b> to list its presets.</p>
    {/if}
  </Dialog>
{/if}

{#if backupProgress}
  <div class="bk-overlay">
    <div class="bk-card">
      <div class="bk-title">
        {exportCancelling ? "Finishing up…" : "Exporting presets…"}
      </div>
      <div class="bk-bar">
        <div
          class="bk-fill"
          style="width:{backupProgress.total ? (100 * backupProgress.done) / backupProgress.total : 0}%"
        ></div>
      </div>
      <div class="bk-line">
        {backupProgress.done}/{backupProgress.total || "…"}
        {#if backupProgress.setlist && setlists.length > 1}— {backupProgress.setlist}{/if}
        — {backupProgress.name}
      </div>
      <!-- All eight of a Floor's setlists is 1024 presets and the better part of an hour. An
           un-cancellable modal over that is not a reasonable thing to put in front of someone. -->
      <button type="button" class="bk-cancel" disabled={exportCancelling} onclick={cancelExport}>
        {exportCancelling ? "Stopping after this preset…" : "Cancel"}
      </button>
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

{#if showIrs}
  <IrPanel
    slots={irSlots}
    busy={irBusy}
    onClose={() => (showIrs = false)}
    onRefresh={refreshIrs}
    onScan={scanIrs}
    onExport={onIrExport}
    onUpload={onIrUpload}
    onDelete={onIrDelete}
    onRename={onIrRename}
  />
{/if}

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
  /* One per DSP. The bar is what makes two of them tell apart at a glance — the numbers alone
     read as one run-on figure, which is how "96.9% · 3.1% free · 0.0% · 100.0% free" happened. */
  .dsp-chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .dsp-chip .dsp-name {
    font-variant-numeric: tabular-nums;
  }
  .dsp-bar {
    width: 60px;
    height: 6px;
    border-radius: 3px;
    background: #2a2f3a;
    overflow: hidden;
  }
  .dsp-bar .fill {
    display: block;
    height: 100%;
    background: #5b8dd6;
  }
  .dsp-chip b {
    font-variant-numeric: tabular-nums;
  }
  .dsp-chip .dsp-free {
    font-size: 12px;
    color: #6d7688;
    font-variant-numeric: tabular-nums;
  }
  /* Nearly full: the same warning colour the model picker uses on a model that won't fit. */
  .dsp-chip.tight .dsp-bar .fill {
    background: #e0785f;
  }
  .dsp-chip.tight b {
    color: #e0785f;
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
  .dsp-head.tight .dsp-bar .fill {
    background: #e0785f;
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
  /* Export-progress overlay: modal, with the one button that matters. */
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
    display: flex;
    flex-direction: column;
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
  .bk-cancel {
    margin-top: 12px;
    align-self: flex-end;
    font: inherit;
    font-size: 13px;
    padding: 5px 12px;
    border-radius: 6px;
    border: 1px solid #3a4150;
    background: #232833;
    color: #e6e8ec;
    cursor: pointer;
  }
  .bk-cancel:hover:not(:disabled) {
    border-color: #4a5265;
  }
  .bk-cancel:disabled {
    color: #6b7280;
    cursor: default;
  }
  .dlg-check {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
    font-weight: 400;
    color: #c8cdd6;
  }
</style>
