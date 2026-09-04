<script>
  import { invoke, listen, IS_MOCK, IS_SERVE, INLINE_FILES } from "./lib/ipc.js";
  import { base64ToBytes, bytesToBase64, pickFile, saveFile } from "./lib/files.js";
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
  import {
    slotLabel,
    numbering,
    setNumbering,
    applyDeviceNumbering,
    forgetDeviceNumbering,
    numberingFlag,
    formForFlag,
  } from "./lib/numbering.svelte.js";
  import GlobalsPanel from "./lib/GlobalsPanel.svelte";


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
    if (dataReady) updateStartup();
    // Under fretwire-serve the backend outlives the page, so a reload can land on a live device
    // session (it survives editor disconnects for a few minutes). Re-attach instead of showing a
    // disconnected view over a session that's still open — `connect` is an idempotent re-read,
    // not a second handshake, and the undo history rides along. Under Tauri and the mock a fresh
    // page always answers false here, so nothing changes for them.
    try {
      if (await invoke("is_connected")) await connect();
    } catch {
      /* the disconnected view is the honest fallback */
    }
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

  // ---- update check (fretwire_core::update) ----
  // `update` is the last UpdateStatusDto. The startup path never shows an error: the automatic
  // check is opt-in, once a day, and offline is silence. The ask bar appears once, while the
  // preference is unanswered (an install that predates the question, or a skipped first run);
  // the About dialog is where it can be changed and where "Check now" lives.
  let update = $state(null);
  let updateAsk = $state(false);
  let updateDlg = $state(false);
  let updateBusy = $state(false);
  let updateErr = $state(null);
  async function updateStartup() {
    try {
      update = await invoke("update_status");
    } catch {
      return; // an older backend without the command — nothing to show
    }
    if (update.enabled == null && !update.locked) {
      updateAsk = true;
      return;
    }
    if (update.enabled) {
      try {
        update = await invoke("update_check", { force: false });
      } catch {
        /* offline, or GitHub is down — the automatic check stays silent */
      }
    }
  }
  async function updateAnswer(enabled) {
    updateAsk = false;
    try {
      update = await invoke("update_pref", { enabled });
    } catch (e) {
      toast("update check: " + e);
      return;
    }
    if (enabled) await updateStartup();
  }
  async function updateNow() {
    updateBusy = true;
    updateErr = null;
    try {
      update = await invoke("update_check", { force: true });
    } catch (e) {
      updateErr = String(e);
    } finally {
      updateBusy = false;
    }
  }
  // The release page: a real link in a browser (serve, mock), `xdg-open` via the backend under
  // Tauri, whose webview does not open external links itself.
  async function openRelease(e) {
    if (IS_SERVE || IS_MOCK) return;
    e.preventDefault();
    try {
      await invoke("open_url", { url: update.url });
    } catch (err) {
      toast("open: " + err);
    }
  }
  const updateChecked = $derived(
    update?.checked_at ? new Date(update.checked_at * 1000).toLocaleString() : null,
  );

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
  let exportDlg = $state(null); // { path, all, onServer } — onServer: a serve-mode export to the daemon's disk
  // True from pressing Cancel until the sweep unwinds — the file is still written, holding what
  // was read up to that point, so the finished toast has to say which of the two happened.
  let exportCancelling = $state(false);
  let backupProgress = $state(null); // { done, total, name } while a backup sweep runs
  // { path | json+fileName, entries, index, slot } — entries load from the file, which is a path
  // for the backend to open or (INLINE_FILES) the file's text carried in the invoke.
  let restoreDlg = $state(null);
  // Whole-device backup: { path, irs, settings, onServer } — every setlist, plus the IR store and
  // the global settings unless unticked. The export dialog above is presets only.
  let backupDlg = $state(null);
  // Whole-device restore: { path | json+fileName, info, presets, irs, settings }. `info` is what
  // the file holds (backup_info), loaded before anything can be confirmed.
  let restoreDevDlg = $state(null);
  // What a device restore did, shown when it finishes — counts and every failure by name.
  let restoreReport = $state(null);
  // Which sweep the progress card is for: "export", "backup" or "restore" — the title and the
  // cancel wording differ.
  let progressKind = $state("export");
  let snapRenameDlg = $state(null); // { index, name }
  // Clear preset: empty the chain, reset the snapshot names. Edit buffer only and one undo entry,
  // but it throws away a whole preset's work in a click, so it confirms first.
  let clearDlg = $state(false);
  // Revert: reload the preset as last saved, discarding unsaved edits. Same stakes as Clear —
  // unsaved work gone in a click — so it confirms too, and it leaves an undo entry behind.
  let revertDlg = $state(false);
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
      if ((e.key === "Delete" || e.key === "Backspace") && !e.ctrlKey && !e.metaKey && !e.altKey) {
        if (saveAsDlg || renameDlg || deleteDlg || exportDlg || restoreDlg || backupDlg || restoreDevDlg || restoreReport || snapRenameDlg || clearDlg) return;
        // Routing nodes (split/mixer) have no model to delete — only real blocks answer this.
        if (selectedBlock?.model_index != null) {
          e.preventDefault();
          deleteDlg = selectedBlock.slot;
        }
        return;
      }
      if (e.key === " " && !e.ctrlKey && !e.metaKey && !e.altKey) {
        if (saveAsDlg || renameDlg || deleteDlg || exportDlg || restoreDlg || backupDlg || restoreDevDlg || restoreReport || snapRenameDlg || clearDlg) return;
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

  // A push addresses one of *three* index spaces in a slot — the block's own param list, its
  // paired cab's param list, or the block's extra values — and **all three start at 0**, so the
  // space has to be part of the key. Keying on the number alone delivered Trails (extra 0) to the
  // model's param 0, which is how a tester found the first half: turning Trails on the pedal swept
  // the Time slider. The paired axis is the same bug one over — a cab's Distance (paired 2) landing
  // on an amp's Mid (main 2), so moving mic distance appeared to move the amp's Mid. [issue #11]
  // See PushDto::Param / ParamDto::extra_index.
  // The two axes are independent — a legacy cab reaches its Mic through the *extras* table on the
  // *paired* model — so both go in the key rather than one shadowing the other.
  const paramKey = (slot, param, extra, paired) =>
    `${slot}:${paired ? "c" : "p"}${extra ? "x" : ""}${param}`;

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
    // One list at a time, told apart by `paired` — an amp+cab block carries two of them and a
    // push names which it meant.
    const patchList = (slot, list, paired) => {
      if (!list) return [list, false];
      let touched = false;
      const next = list.map((p) => {
        const isExtra = p.extra_index != null;
        const v = pendingParams.get(
          paramKey(slot, isExtra ? p.extra_index : p.index, isExtra, paired),
        );
        if (v === undefined || v === p.value) return p;
        touched = true;
        return { ...p, value: v };
      });
      return [next, touched];
    };
    const patch = (b) => {
      if (!b?.params) return b;
      const [params, hit] = patchList(b.slot, b.params, false);
      const [paired_params, pairedHit] = patchList(b.slot, b.paired_params, true);
      if (!hit && !pairedHit) return b;
      return { ...b, params, ...(b.paired_params ? { paired_params } : {}) };
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
        pendingParams.set(paramKey(p.slot, p.param, p.extra, p.paired), p.value);
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
  // Custom footswitch label/colour (layout keys 14/13 and 16/15, via the op-21 write; see
  // docs/preset-format.md). One-based here like `block.footswitch`, zero-based on the wire.
  const onSwitchLabel = (oneBased, label) =>
    apply(invoke("set_switch_label", { switch: oneBased - 1, label }));
  const onSwitchColor = (oneBased, color) =>
    apply(invoke("set_switch_color", { switch: oneBased - 1, color }));
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
  const onClearPreset = () => (clearDlg = true);
  function confirmClear() {
    clearDlg = false;
    selectedSlot = null;
    apply(invoke("clear_preset"));
  }
  const onRevertPreset = () => (revertDlg = true);
  function confirmRevert() {
    revertDlg = false;
    selectedSlot = null;
    apply(invoke("revert_preset"));
  }
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
  // "Export", not "Backup": this captures presets and nothing else. The device backup — presets,
  // global settings and IRs, what makes a wiped pedal whole — is the pair of dialogs further down.
  const exportDefault = `fretwire-presets-${new Date().toISOString().slice(0, 10)}.json`;
  const onExport = () =>
    (exportDlg = {
      // In a browser the file is a download and this is just its name.
      path: INLINE_FILES ? exportDefault : `~/${exportDefault}`,
      all: false,
      onServer: false,
    });
  const exportInline = $derived(INLINE_FILES && !exportDlg?.onServer);
  async function confirmExport() {
    const path = exportDlg.path.trim();
    const inline = exportInline;
    // Whatever setlist you are looking at, not whichever one the device happens to sit in — the
    // sidebar shows one list and the button used to export bank 0 regardless.
    const banks = exportDlg.all ? setlists.map((_, i) => i) : [viewBank];
    exportDlg = null;
    if (!path) return;
    exportCancelling = false;
    progressKind = "export";
    backupProgress = { done: 0, total: 0, name: "starting…" };
    try {
      let count;
      let where;
      if (inline) {
        const file = await invoke("export_setlists_inline", { banks });
        count = file.count;
        where = path.split(/[\\/]/).pop() || exportDefault;
        saveFile(where, new Blob([file.json], { type: "application/json" }));
      } else {
        count = await invoke("export_setlists", { path, banks });
        where = path;
      }
      const how = exportCancelling ? "Cancelled —" : "Exported";
      toast(`${how} ${count} presets to ${where}`, exportCancelling ? "warn" : "info");
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

  const onRestore = () =>
    (restoreDlg = { path: "~/", json: null, fileName: null, entries: null, index: null, slot: null });
  // What the chosen restore target currently holds — overwriting must be a visible choice.
  const restoreTarget = $derived(
    restoreDlg?.slot != null ? presets.find((p) => p.index === restoreDlg.slot) : null,
  );
  async function loadBackupEntries() {
    try {
      let entries;
      let picked = {};
      if (INLINE_FILES) {
        const file = await pickFile({ accept: ".json,application/json" });
        if (!file) return;
        const json = await file.text();
        entries = await invoke("backup_show_inline", { json });
        picked = { json, fileName: file.name };
      } else {
        entries = await invoke("backup_show", { path: restoreDlg.path.trim() });
      }
      restoreDlg = { ...restoreDlg, ...picked, entries, index: null, slot: null, bank: null };
    } catch (e) {
      toast("restore: " + e);
    }
  }
  async function confirmRestore() {
    const { path, json, index, slot, bank } = restoreDlg;
    restoreDlg = null;
    if (index == null || slot == null) return;
    selectedSlot = null;
    // The entry's own bank, so a preset exported from USER 1 goes back to USER 1 rather than
    // whichever list bank 0 happens to be. The file goes back with the choice when it was the
    // browser's: the backend keeps nothing between the listing and the restore.
    const target = { index, slot, bank: bank ?? 0 };
    await apply(
      json != null
        ? invoke("restore_preset_inline", { json, ...target })
        : invoke("restore_preset", { path: path.trim(), ...target }),
    );
    await refreshPresets();
    activeSnapshot = preset?.active_snapshot ?? 0;
    status = `Restored to slot ${slot}.`;
  }

  // ---- whole-device backup / restore ----
  // The three things a wiped pedal needs back — every preset, the IR store, the global settings —
  // in one file (format v3). Same sweep as the export for the presets, then the IRs and a
  // settings scan; `restore_device` writes only what the pedal does not already hold.
  const backupDefault = `fretwire-backup-${new Date().toISOString().slice(0, 10)}.json`;
  const onBackupDevice = () =>
    (backupDlg = {
      path: INLINE_FILES ? backupDefault : `~/${backupDefault}`,
      irs: true,
      settings: true,
      onServer: false,
    });
  const backupInline = $derived(INLINE_FILES && !backupDlg?.onServer);
  async function confirmBackupDevice() {
    const { path, irs, settings } = backupDlg;
    const inline = backupInline;
    const banks = setlists.map((_, i) => i);
    backupDlg = null;
    if (!path.trim()) return;
    exportCancelling = false;
    progressKind = "backup";
    backupProgress = { done: 0, total: 0, name: "starting…" };
    try {
      let counts;
      let where;
      if (inline) {
        const file = await invoke("backup_device_inline", { banks, irs, settings });
        counts = { presets: file.count, irs: file.irs, settings: file.settings };
        where = path.trim().split(/[\\/]/).pop() || backupDefault;
        saveFile(where, new Blob([file.json], { type: "application/json" }));
      } else {
        counts = await invoke("backup_device", { path: path.trim(), banks, irs, settings });
        where = path.trim();
      }
      const what = `${counts.presets} presets, ${counts.irs} IRs, ${counts.settings} settings`;
      const how = exportCancelling ? "Cancelled —" : "Backed up";
      toast(`${how} ${what} to ${where}`, exportCancelling ? "warn" : "info");
      status = `${how} ${what}.`;
    } catch (e) {
      toast("backup: " + e);
    } finally {
      backupProgress = null;
      exportCancelling = false;
    }
    await refreshPreset();
  }

  const onRestoreDevice = () =>
    (restoreDevDlg = { path: "~/", json: null, fileName: null, info: null, presets: true, irs: true, settings: true });
  // The pedal this session is talking to, for the file's device tag to be checked against. The
  // backend refuses a mismatch too; saying so before the button is pressed is the courtesy.
  const connectedName = $derived(preset?.device_name ?? preset?.device_model ?? null);
  const restoreDevMismatch = $derived(
    !!(restoreDevDlg?.info && connectedName && restoreDevDlg.info.device !== connectedName),
  );
  const restoreDevNothing = $derived(
    !restoreDevDlg?.info ||
      !(
        (restoreDevDlg.presets && restoreDevDlg.info.presets) ||
        (restoreDevDlg.irs && restoreDevDlg.info.irs) ||
        (restoreDevDlg.settings && restoreDevDlg.info.settings)
      ),
  );
  async function loadBackupInfo() {
    try {
      let info;
      let picked = {};
      if (INLINE_FILES) {
        const file = await pickFile({ accept: ".json,application/json" });
        if (!file) return;
        const json = await file.text();
        info = await invoke("backup_info_inline", { json });
        picked = { json, fileName: file.name };
      } else {
        info = await invoke("backup_info", { path: restoreDevDlg.path.trim() });
      }
      restoreDevDlg = { ...restoreDevDlg, ...picked, info };
    } catch (e) {
      toast("restore: " + e);
    }
  }
  async function confirmRestoreDevice() {
    const { path, json, presets: doPresets, irs, settings } = restoreDevDlg;
    restoreDevDlg = null;
    selectedSlot = null;
    exportCancelling = false;
    progressKind = "restore";
    backupProgress = { done: 0, total: 0, name: "starting…" };
    try {
      const args = { presets: doPresets, irs, settings };
      restoreReport =
        json != null
          ? await invoke("restore_device_inline", { json, ...args })
          : await invoke("restore_device", { path: path.trim(), ...args });
    } catch (e) {
      toast("restore: " + e);
    } finally {
      backupProgress = null;
      exportCancelling = false;
    }
    // Whatever was written, the device's state is new: presets, position, settings, IRs.
    await refreshPreset();
    await refreshPresets();
    activeSnapshot = preset?.active_snapshot ?? 0;
    status = "Device restore finished.";
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
      // Take the pedal's own preset-numbering form, before the first listing renders so the slot
      // column doesn't visibly re-label itself. This is the pedal's setting and we now write it
      // too, so it wins outright; a device that doesn't answer setting 27 leaves the local
      // preference in place and the menu toggle stays cosmetic for it.
      try {
        applyDeviceNumbering(await invoke("device_numbering"));
      } catch (e) {
        /* non-fatal — the manual toggle is still there */
      }
      viewBank = preset.bank ?? 0;
      await refreshPresets(viewBank);
      try {
        splitTypes = await invoke("split_types");
      } catch (e) {
        /* non-fatal */
      }
      await loadCategoryColors();
      await loadTempo();
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
    forgetDeviceNumbering();
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
    forgetDeviceNumbering();
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
  // HX Edit's category colours, id -> "#rrggbb", read from the reference data. Empty until the
  // catalog answers, and empty forever on an install that skipped the import — Chain falls back to
  // its own palette rather than showing a grey chain.
  let catColors = $state(null);

  async function loadCategoryColors() {
    try {
      const cats = await invoke("categories");
      const map = {};
      for (const c of cats) if (c.color) map[c.id] = c.color;
      catColors = Object.keys(map).length ? map : null;
    } catch (e) {
      catColors = null; // cosmetic; never worth surfacing
    }
  }

  // Tempo lives in the globals namespace (id 16), but it is the one global anyone reaches for
  // mid-session, so it gets a field in the toolbar rather than three clicks into an overlay.
  //
  // Id 14 says what a tempo write actually means — per snapshot, per preset, or global — and that
  // changes the consequence enough to show alongside rather than leave the user guessing.
  const TEMPO_ID = 16;
  const TEMPO_SCOPE_ID = 14;
  // Preset numbering — a Global Setting the preset sidebar edits directly as well as the Globals
  // panel's own row. See `lib/numbering.svelte.js`.
  const NUMBERING_ID = 27;
  const SCOPE_NAMES = { 0: "per snapshot", 1: "per preset", 2: "global" };

  let tempo = $state(null);
  let tempoScope = $state(null);
  let tempoBusy = $state(false);

  const tempoTitle = $derived(
    tempo == null
      ? "The device did not report a tempo"
      : `Tempo, ${SCOPE_NAMES[tempoScopeRaw] ?? "scope unknown"}. Writes the pedal immediately.`,
  );
  let tempoScopeRaw = $state(null);

  async function loadTempo() {
    try {
      const rows = await invoke("settings_read", { all: false });
      const t = rows.find((r) => r.id === TEMPO_ID);
      const sc = rows.find((r) => r.id === TEMPO_SCOPE_ID);
      tempo = typeof t?.value === "number" ? t.value : null;
      tempoScopeRaw = typeof sc?.value === "number" ? sc.value : null;
      tempoScope = SCOPE_NAMES[tempoScopeRaw] ?? null;
    } catch (e) {
      tempo = null; // a device that doesn't answer id 16 simply gets no field
    }
  }

  async function setTempo(v) {
    const n = Number(v);
    if (!Number.isFinite(n) || n === tempo) return;
    tempoBusy = true;
    try {
      const after = await invoke("settings_write", { id: TEMPO_ID, value: n });
      tempo = typeof after.value === "number" ? after.value : tempo;
      // Keep the globals panel honest if it is open behind this.
      globals = globals.map((s) => (s.id === after.id ? after : s));
    } catch (e) {
      toast("tempo: " + e);
      await loadTempo();
    } finally {
      tempoBusy = false;
    }
  }

  let showIrs = $state(false);
  // Global settings. Not on the undo timeline and not part of `preset` — these are the pedal's own
  // state, so they get their own fetch rather than riding a PresetDto.
  let showGlobals = $state(false);
  let globals = $state([]);
  let globalsRaw = $state(false);
  let globalsBusy = $state(false);

  async function refreshGlobals() {
    globalsBusy = true;
    try {
      globals = await invoke("settings_read", { all: globalsRaw });
    } catch (e) {
      toast("settings: " + e);
    } finally {
      globalsBusy = false;
    }
  }

  async function openGlobals() {
    showGlobals = true;
    if (!globals.length) await refreshGlobals();
  }

  /**
   * Switch the preset-numbering form from the preset sidebar's ⋯ menu.
   *
   * On any pedal that answers setting 27 this *is* the pedal's setting: it writes it, and the mode
   * follows the device's read-back rather than the click, so a refused write leaves the menu
   * showing what the pedal actually is. Where the pedal has no such setting to write there is
   * nothing to keep in sync and the choice is simply remembered locally.
   */
  async function setNumberingMode(mode) {
    if (!numbering.deviceBacked) {
      setNumbering(mode);
      return;
    }
    try {
      const after = await invoke("settings_write", {
        id: NUMBERING_ID,
        value: numberingFlag(mode),
      });
      applyDeviceNumbering(formForFlag(after.value));
      // Keep the Globals panel's row in step if it has been opened — same setting, two views.
      globals = globals.map((g) => (g.id === after.id ? after : g));
    } catch (e) {
      toast("preset numbering: " + e);
    }
  }

  async function toggleGlobalsRaw() {
    globalsRaw = !globalsRaw;
    await refreshGlobals();
  }

  async function writeGlobal(id, value) {
    globalsBusy = true;
    try {
      const after = await invoke("settings_write", { id, value });
      // Replace in place rather than re-reading everything: the reply is the device's own read-back
      // of that id, so it is already the authority on what landed.
      globals = globals.map((s) => (s.id === after.id ? after : s));
      // The preset list numbers itself from this one, so it has to follow immediately — and it
      // does so unconditionally now, which is what stops the sidebar's own toggle from outranking
      // this row (they are two views of setting 27, not two settings).
      if (after.id === NUMBERING_ID) applyDeviceNumbering(formForFlag(after.value));
      // …and the toolbar's BPM field is a second view of ids 16 and 14.
      if (after.id === TEMPO_ID && typeof after.value === "number") tempo = after.value;
      if (after.id === TEMPO_SCOPE_ID && typeof after.value === "number") {
        tempoScopeRaw = after.value;
        tempoScope = SCOPE_NAMES[after.value] ?? null;
      }
    } catch (e) {
      toast("settings: " + e);
      await refreshGlobals();
    } finally {
      globalsBusy = false;
    }
  }
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

  // Read the IR directory the first time an IR block is selected, so its `IR Select` shows names
  // rather than bare slot numbers. Once per connection: a directory that comes back empty is an
  // answer too, and re-asking on every click would put a listing round trip behind each selection.
  let irListed = $state(false);
  $effect(() => {
    if (!connected) {
      irListed = false;
      return;
    }
    const sym = selectedBlock?.symbolic_id ?? "";
    if (!irListed && !irBusy && irSlots.length === 0 && sym.startsWith("HD2_ImpulseResponse")) {
      irListed = true;
      refreshIrs();
    }
  });

  function openIrs() {
    showIrs = true;
    refreshIrs();
  }

  // `path` is null in a browser: the WAV comes back inline and is saved as a download.
  const onIrExport = (slot, path) =>
    irCall(async () => {
      if (path == null) {
        const file = await invoke("ir_export_inline", { slot });
        const name = `${file.name.trim() || `IR${String(slot + 1).padStart(3, "0")}`}.wav`;
        saveFile(name, new Blob([base64ToBytes(file.wav_base64)], { type: "audio/wav" }));
        toast(`Exported "${file.name}" — downloading ${name}`, "info");
      } else {
        const name = await invoke("ir_export", { slot, path });
        toast(`Exported "${name}" to ${path}`, "info");
      }
      return null;
    });

  // The job names a `path` (Tauri) or carries the `file` (a browser) — see IrPanel.startUpload.
  const onIrUpload = (job) =>
    irCall(async () => {
      const name = job.name.trim();
      const target = { slot: job.slot, name, overwrite: job.overwrite, force: job.force };
      const slots = job.file
        ? await invoke("ir_upload_inline", {
            ...target,
            wavBase64: bytesToBase64(await job.file.arrayBuffer()),
          })
        : await invoke("ir_upload", { ...target, path: job.path });
      toast(`Uploaded "${name}" to IR ${String(job.slot + 1).padStart(3, "0")}`, "info");
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
      updateStartup();
      status = `Imported ${r.copied} reference file(s) — model names are available.`;
      statusErr = false;
    }}
    onskip={() => {
      dataReady = true;
      updateStartup();
      status = "No reference data — blocks and parameters show numeric indices.";
      statusErr = false;
    }}
  />
{/if}

{#if dataReady}
<header>
  <h1>fretwire</h1>
  <button class="ver" onclick={() => (updateDlg = true)} title="Version and the update check">
    v{update?.current ?? "…"}
  </button>
  {#if update?.available}
    <a
      class="newver"
      href={update.url}
      target="_blank"
      rel="noreferrer"
      onclick={openRelease}
      title={`${update.install_label}: ${update.instruction}`}
    >v{update.latest} available</a>
  {/if}
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
    <button class="secondary" onclick={openGlobals} title="Read and change the pedal's global settings">Globals…</button>
    <span class="tempo" title={tempoTitle}>
      <label for="bpm">BPM</label>
      <input
        id="bpm"
        type="number"
        min="40"
        max="240"
        step="0.1"
        disabled={tempo == null || tempoBusy}
        value={tempo ?? ""}
        onchange={(e) => setTempo(e.currentTarget.value)}
      />
      {#if tempoScope}<span class="scope">{tempoScope}</span>{/if}
    </span>
    <button class="secondary" onclick={disconnect}>Disconnect</button>
  {:else}
    <button onclick={connect}>Connect</button>
  {/if}
</header>

{#if updateAsk}
  <div class="askbar">
    <span>
      Check for new fretwire versions once a day? One request to github.com for the latest
      release tag{IS_SERVE ? ", from the machine running fretwire-serve" : ""}; nothing about
      you is sent, and nothing is downloaded or installed for you.
    </span>
    <button onclick={() => updateAnswer(true)}>Yes</button>
    <button class="secondary" onclick={() => updateAnswer(false)}>No</button>
  </div>
{/if}

<div class="status" class:err={statusErr}>{status}</div>

<main>
  {#if preset}
    <div class="workspace">
      <PresetList {presets} currentIndex={preset.index} dirty={preset.dirty} {setlists} {viewBank} currentBank={presetBank} writeBlocked={foreignSetlist} {onPickSetlist} {onGoto} {onSave} {onSaveAs} {onRename} {onExport} {onRestore} {onBackupDevice} {onRestoreDevice} {onCopyPreset} {onPastePreset} {onClearPreset} {onRevertPreset} {presetClip} onNumbering={setNumberingMode} />
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
            {catColors}
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
            {irSlots}
            {onBypassSwitch}
            {onSwitchLabel}
            {onSwitchColor}
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

{#if updateDlg && update}
  <Dialog
    title="About fretwire"
    confirmLabel="Close"
    width={460}
    onconfirm={() => (updateDlg = false)}
    oncancel={() => (updateDlg = false)}
  >
    <div class="about">
      <p>
        fretwire <b>{update.current}</b> — {update.install_label}{IS_SERVE
          ? " (the fretwire-serve daemon)"
          : ""}
      </p>
      <label class="about-check">
        <input
          type="checkbox"
          checked={update.enabled === true}
          disabled={update.locked}
          onchange={(e) => updateAnswer(e.currentTarget.checked)}
        />
        Check for new versions once a day
      </label>
      <p class="dim">
        {#if update.locked}Pinned off by <code>$FRETWIRE_NO_UPDATE_CHECK</code>.{/if}
        One request to github.com for the latest release tag; nothing about you or your rig is
        sent, and fretwire never downloads or installs anything itself.
      </p>
      <div class="about-row">
        <button type="button" class="secondary" disabled={updateBusy} onclick={updateNow}>
          {updateBusy ? "Checking…" : "Check now"}
        </button>
        {#if update.latest}
          <span class="dim">
            {update.available ? `v${update.latest} is available` : `v${update.latest} is the latest release`}{updateChecked
              ? ` · checked ${updateChecked}`
              : ""}
          </span>
        {/if}
      </div>
      {#if update.available}
        <p>
          <a href={update.url} target="_blank" rel="noreferrer" onclick={openRelease}>Open the release page</a>
          — {update.instruction}
        </p>
      {/if}
      {#if updateErr}
        <p class="about-err">{updateErr}</p>
      {/if}
    </div>
  </Dialog>
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
      {exportInline ? "Download as" : "Export file"}
      <input type="text" bind:value={exportDlg.path} use:autofocus />
    </label>
    <!-- Serve mode only: the daemon's disk is a real destination too (a backup that lives with
         the rig, cron-able), just not the default one. -->
    {#if IS_SERVE}
      <label class="dlg-check">
        <input
          type="checkbox"
          checked={exportDlg.onServer}
          onchange={(e) =>
            (exportDlg = {
              ...exportDlg,
              onServer: e.currentTarget.checked,
              path: e.currentTarget.checked ? `~/${exportDefault}` : exportDefault,
            })}
        />
        Save on the machine running fretwire-serve instead of downloading
      </label>
    {/if}
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
      <b>Presets only.</b> Global settings and IRs are not in the file — for those, use
      <b>Back up device to file…</b> in the same menu.
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
    {#if INLINE_FILES}
      <div class="dlg-field">
        Backup file
        <span class="dlg-row">
          <span class="dlg-filename">{restoreDlg.fileName ?? "No file chosen"}</span>
          <button type="button" class="dlg-btn" onclick={loadBackupEntries} use:autofocus>
            Choose file…
          </button>
        </span>
      </div>
    {:else}
      <label class="dlg-field">
        Backup file
        <span class="dlg-row">
          <input type="text" bind:value={restoreDlg.path} use:autofocus />
          <button type="button" class="dlg-btn" onclick={loadBackupEntries}>Load</button>
        </span>
      </label>
    {/if}
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
      <p class="dlg-text">
        {#if INLINE_FILES}Choose an export file to list its presets.{:else}Enter the backup file's
          path and press <b>Load</b> to list its presets.{/if}
      </p>
    {/if}
  </Dialog>
{/if}

{#if backupDlg}
  <Dialog
    title="Back up device"
    confirmLabel="Start backup"
    width={460}
    onconfirm={confirmBackupDevice}
    oncancel={() => (backupDlg = null)}
  >
    <label class="dlg-field">
      {backupInline ? "Download as" : "Backup file"}
      <input type="text" bind:value={backupDlg.path} use:autofocus />
    </label>
    {#if IS_SERVE}
      <label class="dlg-check">
        <input
          type="checkbox"
          checked={backupDlg.onServer}
          onchange={(e) =>
            (backupDlg = {
              ...backupDlg,
              onServer: e.currentTarget.checked,
              path: e.currentTarget.checked ? `~/${backupDefault}` : backupDefault,
            })}
        />
        Save on the machine running fretwire-serve instead of downloading
      </label>
    {/if}
    <div class="dlg-field">
      What goes in the file
      <label class="dlg-check">
        <input type="checkbox" checked disabled />
        Every preset{#if setlists.length > 1} in all {setlists.length} setlists{/if}
      </label>
      <label class="dlg-check">
        <input type="checkbox" checked={backupDlg.irs} onchange={(e) => (backupDlg = { ...backupDlg, irs: e.currentTarget.checked })} />
        The user IR store
      </label>
      <label class="dlg-check">
        <input type="checkbox" checked={backupDlg.settings} onchange={(e) => (backupDlg = { ...backupDlg, settings: e.currentTarget.checked })} />
        Global settings
      </label>
    </div>
    <p class="dlg-text">
      Reads everything to the file — nothing on the device is written. The pedal may step through
      presets it cannot read in place (audio follows along), and unsaved edits to the current preset
      are reloaded from flash. You can cancel partway; the file keeps what was read.
    </p>
  </Dialog>
{/if}

{#if restoreDevDlg}
  <Dialog
    title="Restore device"
    confirmLabel="Restore device"
    confirmDisabled={restoreDevNothing || restoreDevMismatch}
    danger
    width={520}
    onconfirm={confirmRestoreDevice}
    oncancel={() => (restoreDevDlg = null)}
  >
    {#if INLINE_FILES}
      <div class="dlg-field">
        Backup file
        <span class="dlg-row">
          <span class="dlg-filename">{restoreDevDlg.fileName ?? "No file chosen"}</span>
          <button type="button" class="dlg-btn" onclick={loadBackupInfo} use:autofocus>
            Choose file…
          </button>
        </span>
      </div>
    {:else}
      <label class="dlg-field">
        Backup file
        <span class="dlg-row">
          <input type="text" bind:value={restoreDevDlg.path} use:autofocus />
          <button type="button" class="dlg-btn" onclick={loadBackupInfo}>Load</button>
        </span>
      </label>
    {/if}
    {#if restoreDevDlg.info}
      {@const info = restoreDevDlg.info}
      <p class="dlg-text">
        From a <b>{info.device}</b>: {info.presets} presets{#if info.setlists.length > 1} in {info.setlists.length} setlists{/if},
        {info.irs} IRs, {info.settings} settings.
      </p>
      {#if restoreDevMismatch}
        <p class="dlg-warn">
          This is a <b>{connectedName}</b>. A file from a different device is not restored here — its
          presets may not fit and its setting ids may mean something else. The command line has a
          <b>--force</b> for owners who know better.
        </p>
      {:else}
        <div class="dlg-field">
          What to restore
          <label class="dlg-check">
            <input type="checkbox" checked={restoreDevDlg.presets} disabled={!info.presets} onchange={(e) => (restoreDevDlg = { ...restoreDevDlg, presets: e.currentTarget.checked })} />
            Presets — each to the slot it came from{#if !info.presets} (none in the file){/if}
          </label>
          <label class="dlg-check">
            <input type="checkbox" checked={restoreDevDlg.irs} disabled={!info.irs} onchange={(e) => (restoreDevDlg = { ...restoreDevDlg, irs: e.currentTarget.checked })} />
            IRs — each to its slot{#if !info.irs} (none in the file){/if}
          </label>
          <label class="dlg-check">
            <input type="checkbox" checked={restoreDevDlg.settings} disabled={!info.settings} onchange={(e) => (restoreDevDlg = { ...restoreDevDlg, settings: e.currentTarget.checked })} />
            Global settings — the ones fretwire has identified{#if !info.settings} (none in the file){/if}
          </label>
        </div>
        <p class="dlg-warn">
          <b>Overwrites the device.</b> Every chosen preset slot, IR slot and setting in the file is
          written over what the pedal holds now — except where the pedal already holds the same
          thing, which is left alone. There is no undo for a flash write.
        </p>
      {/if}
    {:else}
      <p class="dlg-text">
        {#if INLINE_FILES}Choose a backup file to see what it holds.{:else}Enter the backup file's
          path and press <b>Load</b> to see what it holds.{/if}
      </p>
    {/if}
  </Dialog>
{/if}

{#if restoreReport}
  {@const r = restoreReport}
  <Dialog
    title="Device restored"
    confirmLabel="OK"
    width={520}
    onconfirm={() => (restoreReport = null)}
    oncancel={() => (restoreReport = null)}
  >
    <p class="dlg-text">
      Presets: <b>{r.presets_written}</b> written, {r.presets_unchanged} already matched.<br />
      IRs: <b>{r.irs_written}</b> written, {r.irs_unchanged} already matched.<br />
      Settings: <b>{r.settings_written}</b> written, {r.settings_unchanged} already matched{#if r.settings_skipped.length}, {r.settings_skipped.length} not written{/if}.
    </p>
    {#if r.failures.length}
      <p class="dlg-warn"><b>{r.failures.length} failed:</b></p>
      <ul class="dlg-list">
        {#each r.failures as f}<li>{f}</li>{/each}
      </ul>
    {/if}
    {#if r.skipped.length}
      <p class="dlg-text dim">{r.skipped.length} item(s) were not attempted: {r.skipped[0]}{#if r.skipped.length > 1}, …{/if}</p>
    {/if}
    {#if r.settings_skipped.length && !r.failures.length}
      <p class="dlg-text dim">
        Settings not written are ids nobody has identified yet, or ones this pedal holds in another
        type — recorded in the file, never sent.
      </p>
    {/if}
  </Dialog>
{/if}

{#if backupProgress}
  <div class="bk-overlay">
    <div class="bk-card">
      <div class="bk-title">
        {exportCancelling
          ? "Finishing up…"
          : progressKind === "backup"
            ? "Backing up device…"
            : progressKind === "restore"
              ? "Restoring device…"
              : "Exporting presets…"}
      </div>
      <div class="bk-bar">
        <div
          class="bk-fill"
          style="width:{backupProgress.total ? (100 * backupProgress.done) / backupProgress.total : 0}%"
        ></div>
      </div>
      <div class="bk-line">
        {backupProgress.done}/{backupProgress.total || "…"}
        {#if backupProgress.setlist && (setlists.length > 1 || backupProgress.stage === "irs" || backupProgress.stage === "settings")}— {backupProgress.setlist}{/if}
        — {backupProgress.name}
      </div>
      <!-- All eight of a Floor's setlists is 1024 presets and the better part of an hour. An
           un-cancellable modal over that is not a reasonable thing to put in front of someone. -->
      <button type="button" class="bk-cancel" disabled={exportCancelling} onclick={cancelExport}>
        {exportCancelling ? "Stopping after this one…" : "Cancel"}
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

{#if revertDlg}
  <Dialog
    title="Revert to saved"
    confirmLabel="Revert"
    danger
    onconfirm={confirmRevert}
    oncancel={() => (revertDlg = false)}
  >
    <p class="dlg-text">
      Discard unsaved changes to <b>{preset?.name ?? "this preset"}</b> and reload it as last
      saved?
    </p>
    <p class="dlg-text dim">
      This is the pedal's own switch-away-and-back, without leaving the preset. The state you're
      discarding stays one Undo away until you switch presets.
    </p>
  </Dialog>
{/if}

{#if clearDlg}
  <Dialog
    title="Clear preset"
    confirmLabel="Clear"
    danger
    onconfirm={confirmClear}
    oncancel={() => (clearDlg = false)}
  >
    <p class="dlg-text">
      Delete every block from <b>{preset?.name ?? "this preset"}</b> and reset its snapshot names?
    </p>
    <p class="dlg-text dim">
      Footswitch and controller assignments go with the blocks. This only changes the edit buffer —
      undo it, or reload the preset; the stored preset is untouched until you Save.
    </p>
  </Dialog>
{/if}

<Toast {toasts} ondismiss={dismissToast} />

{#if showGlobals}
  <GlobalsPanel
    settings={globals}
    busy={globalsBusy}
    showRaw={globalsRaw}
    onClose={() => (showGlobals = false)}
    onRefresh={refreshGlobals}
    onToggleRaw={toggleGlobalsRaw}
    onWrite={writeGlobal}
  />
{/if}

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
  /* Tempo sits in the toolbar because it is the one global anyone changes mid-session. Styled as a
     readout with an editable number rather than as another button, so it reads as device state. */
  .tempo {
    display: inline-flex;
    align-items: baseline;
    gap: 5px;
    padding: 2px 8px;
    border: 1px solid #3a4150;
    border-radius: 4px;
    background: #12151a;
    font-size: 12px;
    color: #8b93a3;
  }
  .tempo input {
    width: 58px;
    background: none;
    border: none;
    color: #e6e9ef;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    text-align: right;
    padding: 0;
  }
  .tempo input:disabled {
    opacity: 0.5;
  }
  .tempo input:focus {
    outline: 1px solid #5b8def;
    outline-offset: 2px;
    border-radius: 2px;
  }
  .tempo .scope {
    color: #6b7280;
    font-size: 11px;
  }
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
  .ver {
    font: inherit;
    font-size: 12px;
    background: none;
    border: 0;
    padding: 2px 6px;
    color: #6b7280;
    cursor: pointer;
  }
  .ver:hover {
    color: #b9c0cc;
  }
  .newver {
    font-size: 12px;
    padding: 2px 8px;
    border-radius: 999px;
    background: #2b4a2e;
    color: #b6f0bd;
    text-decoration: none;
  }
  .newver:hover {
    background: #356a3a;
  }
  .askbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 16px;
    background: #23272f;
    border-bottom: 1px solid #2a2e37;
    color: #b9c0cc;
    font-size: 13px;
  }
  .askbar span {
    flex: 1;
  }
  .about p {
    margin: 0 0 10px;
  }
  .about .dim {
    color: #8b93a1;
    font-size: 12.5px;
  }
  .about code {
    background: #23272f;
    border-radius: 4px;
    padding: 1px 4px;
  }
  .about-check {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 0 0 8px;
    cursor: pointer;
  }
  .about-row {
    display: flex;
    gap: 10px;
    align-items: center;
    margin: 0 0 10px;
  }
  .about a {
    color: #7fb4ff;
  }
  .about-err {
    color: #ff8a8a;
    white-space: pre-wrap;
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
  /* Secondary line under a confirmation — the consequences, not the question. */
  .dlg-text.dim {
    color: #9aa3b2;
    margin-top: 8px;
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
  .dlg-list {
    margin: 0.2rem 0 0.4rem 1.2rem;
    padding: 0;
    font-size: 0.85rem;
    max-height: 9rem;
    overflow: auto;
  }
  .dlg-warn b {
    color: #ffd27a;
  }
  .dlg-row {
    display: flex;
    gap: 6px;
  }
  .dlg-row input,
  .dlg-filename {
    flex: 1;
    min-width: 0;
  }
  .dlg-filename {
    align-self: center;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0.8;
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
