<script>
  // Editor for the selected block: bypass toggle + a control per parameter (slider / dropdown /
  // switch), for both the main model and any paired cab/IR. Commits go up via callbacks; the parent
  // re-reads the preset so values reflect the device.
  import ModelPicker from "./ModelPicker.svelte";
  import CabMicView from "./CabMicView.svelte";

  let {
    block,
    dspLoad = 0,
    // What this DSP fills up at, on `dspLoad`'s scale — ~75, not 100. Passed down so the swap
    // picker greys the same models the pedal would refuse (see `editor::DSP_CEILING`).
    budget = 75,
    isNode = false,
    isSplit = false,
    splitTypes = [],
    onFloat,
    onEnum,
    onPreview,
    onBypass,
    onSwap,
    onSplitType,
    onDelete,
    onCopyBlock,
    onPasteBlock,
    // Display name of whatever block is on the paste buffer, or null when it's empty.
    blockClip = null,
    // Every parameter the preset has under a controller, and how many footswitch positions this
    // device has. Both come straight from the preset — see `PresetDto`.
    assignments = [],
    // The pedal's IR directory, as the IR panel lists it (`{index, display_name, used}`), for
    // naming the IR block's `IR Select` choices. Empty until the parent has read it.
    irSlots = [],
    footswitchCount = 0,
    onBypassSwitch,
    onSwitchLabel,
    onSwitchColor,
    onSwitchType,
    onAssignParam,
    onAssignTravel,
  } = $props();

  // The device's footswitch colour palette, by wire index (layout key `16` / `@fs_customcolor`).
  // The names are Line 6's own — the `footswitchLED` control in HelixControls.json, whose enum
  // ordinals match the wire values (0 = Auto Color, the virgin default; 11 = Off). Confirmed by
  // sweeping 1-10 on a live HX Stomp ring, 2026-08-27; the CSS approximates what the LED shows.
  const FS_PALETTE = [
    { index: 1, css: "#ffffff", name: "White" },
    { index: 2, css: "#ff1a10", name: "Red" },
    { index: 3, css: "#ff6a00", name: "Dark Orange" },
    { index: 4, css: "#ffb340", name: "Light Orange" },
    { index: 5, css: "#ffe64d", name: "Yellow" },
    { index: 6, css: "#06ff00", name: "Green" },
    { index: 7, css: "#00e5cc", name: "Turquoise" },
    { index: 8, css: "#0a68ff", name: "Blue" },
    { index: 9, css: "#7a1fd0", name: "Violet" },
    { index: 10, css: "#ff4ad8", name: "Pink" },
    { index: 11, css: "#14171e", name: "Off" },
  ];

  // ---- controller assignments ----
  //
  // The device keeps two mechanisms apart and so does this panel: a block's **bypass** on a
  // footswitch is a property of the block, so it sits in the header next to the bypass button; a
  // **parameter** under a controller belongs to that parameter's row.
  //
  // Sources: 0 none, 1-2 the expression inputs, then one ordinal per footswitch from 3, then MIDI,
  // then snapshots. MIDI is left out of the picker — it needs a CC number, which is a separate
  // opcode we have not built.
  //
  // **The ordinals above the footswitches move with the device.** Key `4` is a table of
  // `footswitchCount + 5`, so a Stomp runs 3..=7 with MIDI at 8 and snapshots at 9, while an XL runs
  // 3..=10 with 11 and 12 — its FS6 sits on the index a Stomp calls MIDI. This was capped at 5 with
  // snapshots hard-coded to 9 until an XL owner diffed one (issue #13); see
  // `fretwire_protocol::edit::source`, which is the same arithmetic and is pinned to both devices'
  // captures in `fretwire-core/tests/controller_table.rs`.
  //
  // A count of 0 means no preset is loaded, so there is no device to size this against and the
  // footswitch and snapshot entries are simply absent rather than guessed.
  const SOURCES = $derived([
    { value: 0, label: "—" },
    { value: 1, label: "EXP1" },
    { value: 2, label: "EXP2" },
    ...Array.from({ length: footswitchCount }, (_, i) => ({
      value: 3 + i,
      label: `FS${i + 1}`,
    })),
    ...(footswitchCount > 0 ? [{ value: 4 + footswitchCount, label: "Snapshots" }] : []),
  ]);

  // A bypass this block has on an **expression pedal**, or undefined.
  //
  // Bypass has two destinations, chosen by the source: a footswitch writes the footswitch layout
  // and arrives as `block.footswitch` (the picker above), while an expression pedal writes key `4`
  // and arrives here, as an entry with a target slot and **no parameter**. `assignmentFor` matches
  // on `param_index`, so before this existed such an entry matched no row and no badge and the
  // block simply looked unassigned — an XL owner's preset with two bypasses on EXP1/EXP2 rendered
  // as if it had none [issue #13, 2026-08-25].
  //
  // Read-only: which opcode writes one is unread — ops 56/57 take a plain switch index and nothing
  // we hold shows one accepting an expression input — so it is shown, not offered.
  const bypassOnPedal = $derived(
    assignments.find((a) => a.target_slot === block?.slot && a.param_index === null),
  );

  // The assignment driving one parameter of the selected block, or undefined.
  function assignmentFor(paired, index) {
    return assignments.find(
      (a) => a.target_slot === block?.slot && a.param_index === index && a.paired === paired,
    );
  }

  // Which parameter's assignment editor is open. Only one at a time — the travel controls are tall
  // enough that several open at once turns the grid into a wall.
  let openAssign = $state(null);
  $effect(() => {
    block?.slot;
    openAssign = null;
  });

  // Routing nodes (split/mixer) and controller assignments aren't category-swappable or deletable
  // here — the split *type* is changed elsewhere, controllers are footswitch bindings.
  const editable = $derived(!isNode && !block?.is_controller);

  let swapping = $state(false);
  let swappingCab = $state(false);
  // Custom label/colour mini-editor for the switch this block sits on.
  let fsEditing = $state(false);
  let fsLabelDraft = $state("");

  function commitFsLabel() {
    const text = fsLabelDraft.trim();
    // An empty label is a clear — the device keeps the stale string behind a false gate anyway.
    onSwitchLabel(block.footswitch, text || null);
  }
  // Reset the swap pickers when the selected block changes.
  $effect(() => {
    block?.slot;
    swapping = false;
    swappingCab = false;
  });
  // DSP available if this block were replaced (exclude its own current load).
  const swapRemaining = $derived(budget - (dspLoad - (block?.dsp_load ?? 0)));

  // Live values shown while dragging a slider (committed on release; previews stream meanwhile).
  let live = $state({});
  const key = (paired, p) => `${block?.slot}:${paired ? "p" : "m"}:${p.index}`;

  // ---- live audio preview while dragging ----
  // Stream the value to the device during a drag so the change is audible as a smooth ramp (like
  // HX Edit), instead of one jump on release. Latest-wins, pumped at most every PREVIEW_MS per
  // param; the gesture still ends with the ordinary commit (history entry + authoritative re-read).
  const PREVIEW_MS = 60;
  let pvLatest = {};
  let pvTimer = {};
  function preview(k, paired, p, v) {
    pvLatest[k] = { slot: block.slot, paired, index: p.index, value: v };
    if (pvTimer[k]) return;
    const pump = () => {
      const job = pvLatest[k];
      delete pvLatest[k];
      if (!job) {
        delete pvTimer[k];
        return;
      }
      onPreview?.(job.slot, job.paired, job.index, job.value);
      pvTimer[k] = setTimeout(pump, PREVIEW_MS);
    };
    pump();
  }
  // Called on commit: the final value is being sent anyway, so drop any queued preview.
  const cancelPreview = (k) => delete pvLatest[k];

  // Send the gesture's final value, and keep it on screen until the commit's re-read lands.
  //
  // Releasing it at send time instead is what made a scroll "go, flash back, then land": for the
  // round trip the slider falls back to `p.value`, which is whatever the panel last heard from the
  // device — the pre-edit value, or a status push still catching up — and then jumps forward again
  // when the re-read arrives. `apply` never rejects, so `finally` also covers a refused edit, where
  // snapping back to the device's value is the right answer.
  function commit(k, v, paired, p, isInt) {
    cancelPreview(k);
    const sent = isInt
      ? onEnum(block.slot, paired, p.index, Math.round(v))
      : onFloat(block.slot, paired, p.index, v);
    Promise.resolve(sent).finally(() => {
      // Unless a newer gesture has taken the display over in the meantime.
      if (live[k] === v) delete live[k];
    });
  }

  // ---- scroll-wheel nudging ----
  // A slider responds to the wheel while Shift is held (hover anywhere on it) or after it's been
  // clicked (focused). Each notch nudges one step; the commit is debounced so a burst of notches
  // is one USB write (and one undo entry). Svelte 5 registers `onwheel` passively, so this is an
  // action attaching a non-passive listener — preventDefault must work to stop the page scrolling.
  // The value box is only in the DOM once it's been clicked, so focus it as it appears.
  function focusOnMount(node) {
    node.focus();
    node.select();
  }

  function wheelable(node, handler) {
    node.addEventListener("wheel", handler, { passive: false });
    return { destroy: () => node.removeEventListener("wheel", handler) };
  }

  let wheelTimers = {};
  function nudge(e, k, p, paired, isInt, r) {
    if (!e.deltaY) return;
    if (!e.shiftKey && document.activeElement !== e.currentTarget) return;
    e.preventDefault();
    const dir = e.deltaY < 0 ? 1 : -1;
    // One notch = one of the increments HX Edit uses, where the reference data states one. Pan's is
    // half of what the old blanket 1/100th was, which is why it scrolled two display units a notch.
    const step = isInt ? 1 : (p.step ?? (r.max - r.min) / 100);
    const cur = live[k] ?? p.value;
    const v = Math.min(r.max, Math.max(r.min, cur + dir * step));
    live[k] = v;
    if (!isInt) preview(k, paired, p, v); // audible ramp per notch (float wire path only)
    clearTimeout(wheelTimers[k]);
    wheelTimers[k] = setTimeout(() => {
      delete wheelTimers[k];
      commit(k, v, paired, p, isInt);
    }, 300);
  }

  // Which control a param needs. Enums with labels → dropdown; bools → switch; segmented floats
  // (discrete stops, e.g. cab mic Angle 0°/45°) → button group; value_type 1 or float kind →
  // slider; anything else integer → stepped slider (int wire path).
  //
  // An integer we have *no declared range* for is shown read-only instead of guessing one. Integer
  // params index tables in the firmware, and the device does not range-check: the old fallback span
  // (0..=127) let a 0..=3 head selector be set to 77, which hung the pedal hard enough to drop it
  // off USB. A value we can't bound is one we have no business sending.
  // The IR block's `IR Select` is a bare slot number on the wire, and it is **one-based**: the
  // parameter's range starts at 1, and value 6 is the IR the panel lists as 006 — the directory
  // record at zero-based index 5. (Shown as `index + 1` at first, off the guess that the wire
  // counted from 0 like the IR ops do; the owner of a POD Go with seven IRs loaded read the names
  // one row out, 2026-09-03.) The reference data has no labels for it, so without this it is a
  // slider and the user has to know which number holds which file. Named from the directory the
  // parent read; slots it hasn't heard of still get their number, so an IR the list doesn't cover
  // is never unreachable.
  const isIrSelect = (p) =>
    p.name === "IR Select" && /^HD2_ImpulseResponse/.test(block?.symbolic_id ?? "");
  const irLabel = (i) => {
    const num = String(i).padStart(3, "0");
    const slot = irSlots.find((s) => s.index === i - 1);
    if (!slot) return num;
    return slot.used ? `${num}  ${slot.display_name}` : `${num}  (empty)`;
  };
  const irChoices = (p) => {
    const lo = p.min ?? 1;
    const hi = p.max ?? 128;
    return Array.from({ length: Math.max(0, hi - lo + 1) }, (_, k) => lo + k);
  };

  // The governed member of a tempo-sync group, resolved: whether the switch is on and the note
  // param to show while it is. `null` for every other param — including the switch and the note
  // themselves, which have no rows.
  //
  // `list` is the parameter list being rendered — the block's own or its paired cab's — and it is
  // an argument rather than a lookup because the two lists index separately: a sync group only
  // ever names members of its own. It was written as a bare `params`, which is the *snippet's*
  // parameter and not in scope here, so this threw `ReferenceError: params is not defined` the
  // moment it reached a governed param — taking the whole panel down with it. Every block with a
  // tempo-sync group (a delay's Time, a chorus's Rate) could be selected but never opened.
  // [owner's report, 2026-09-05: Trinity Chorus in preset 26, Bucket Brigade in preset 4]
  function syncOf(p, list) {
    if (!p.sync || p.sync.role !== "governed") return null;
    const tempo = list.find((q) => q.index === p.sync.tempo);
    const note = list.find((q) => q.index === p.sync.note);
    if (!tempo || !note) return null;
    return { on: tempo.value >= 0.5, tempoIndex: tempo.index, note };
  }

  function control(p) {
    // A block carrying *several* values past the end of its symbol's param list. The lone trailing
    // value (`Trails`, a legacy cab's mic index) is reachable through the extras addressing and
    // stays editable; a second one has no wire evidence for its index, so don't offer a control.
    if (p.settable === false) return "unsettable";
    if (p.enum_labels && p.enum_labels.length) return "enum";
    if (p.value_type === 2 || p.kind === "bool") return "bool";
    if (p.stops && p.stops.length) return "seg";
    if (p.value_type === 1 || p.kind === "float") return "float";
    if (p.min == null || p.max == null) return "unranged";
    return "int";
  }

  // The stop nearest the current value — segmented floats highlight it as active.
  const nearestStop = (p) =>
    p.stops.reduce((a, s) => (Math.abs(s.value - p.value) < Math.abs(a.value - p.value) ? s : a));

  function range(p, isInt) {
    const min = p.min ?? 0;
    const max = p.max ?? (isInt ? 127 : 1);
    const step = isInt ? 1 : (max - min) / 200 || 0.001;
    return { min, max, step };
  }

  function fmt(v) {
    return Number.isInteger(v) ? String(v) : v.toFixed(2);
  }

  // The device stores DSP values — a delay time is 1.3728 — and HX Edit shows them scaled with a
  // unit ("1.373 s"). `p.format` carries that recipe from HelixControls.json; this applies it. Done
  // here rather than in Rust because a slider re-renders on every drag frame, before any value has
  // been sent. Falls back to the bare number when the reference data doesn't describe the control.
  function fmtVal(p, v) {
    const f = p.format;
    if (!f || !Number.isFinite(v)) return fmt(v);
    const s = v * f.scale + (f.offset ?? 0);
    const r =
      f.rules.find((r) => (r.lo == null || s >= r.lo) && (r.hi == null || s < r.hi)) ??
      f.rules[f.rules.length - 1];
    if (!r) return fmt(v);
    return printf(r.template, s * r.mult);
  }

  // The literal text of a format rule with the number taken out ("Left %.0f" -> "left"): what the
  // user sees next to the digits, and so what they may type back.
  const ruleWord = (t) =>
    t.replace(/%%|%\+?(?:\.\d+)?f/g, " ").trim().toLowerCase();

  // Parse what was typed in the value box back into a stored value. The box shows what the label
  // shows, so it has to accept that: "L100" comes back as "Left 100" here, and the rule that
  // rendered it is the one that reverses it — `Left`'s unitsMultiplier is -1, so 100 means -100
  // display, which is 0.0 stored. A bare number is taken as the display value directly, so "-50"
  // and "Left 50" both land on the same place. Returns null if there's nothing usable in the text.
  function parseDisplay(p, text) {
    const t = text.trim().toLowerCase();
    if (!t) return null;
    const f = p.format;
    const num = t.match(/-?\d*\.?\d+/);
    if (!f) return num ? Number(num[0]) : null;

    if (!num) {
      // A word-only rule ("Center", "Off"): aim at the middle of the band it covers, which is what
      // renders that word back. Unbounded ends have no middle to aim at.
      const r = f.rules.find((r) => ruleWord(r.template) === t);
      if (!r || r.lo == null || r.hi == null) return null;
      return fromDisplay(p, (r.lo + r.hi) / 2);
    }
    const n = Number(num[0]);
    // A rule whose word is present in the text reverses its own multiplier; otherwise the number is
    // already the display value. The initial-only form matters: the *pedal* writes pan as "L100",
    // so that is what someone reads off its screen and types, and without it "L100" has no word to
    // match, is taken as a bare +100, and lands hard right — the opposite of what was asked for.
    const initial = t.match(/^([a-z])\s*-?\d/)?.[1];
    const named =
      f.rules.find((r) => { const w = ruleWord(r.template); return w && t.includes(w); }) ??
      (initial && f.rules.find((r) => ruleWord(r.template).startsWith(initial)));
    return fromDisplay(p, named ? n / (named.mult || 1) : n);
  }

  // Display value -> stored value, undoing `fmtVal`'s scale/offset, clamped to the param's range.
  function fromDisplay(p, display) {
    const f = p.format;
    const v = f ? (display - (f.offset ?? 0)) / f.scale : display;
    if (!Number.isFinite(v)) return null;
    return Math.min(p.max ?? v, Math.max(p.min ?? v, v));
  }

  // Double-click a slider to put the param back where the model says it starts — pan to Center,
  // a mix to its stock blend. `.models` carries that default for every param it describes; the
  // routing nodes aren't in those files, so their sliders simply don't offer it.
  function resetToDefault(k, p, paired, isInt) {
    if (p.default == null || p.default === p.value) return;
    live[k] = p.default;
    commit(k, p.default, paired, p, isInt);
  }

  // ---- typed values ----
  // Which value box is open for editing, and its text. Clicking the readout turns it into a field:
  // sliders are hard to land on an exact number, and the pedal shows exact numbers.
  let typing = $state(null);
  let typed = $state("");
  const openTyping = (k, p) => {
    typing = k;
    typed = fmtVal(p, live[k] ?? p.value);
  };
  function commitTyped(k, p, paired, isInt) {
    const v = parseDisplay(p, typed);
    typing = null;
    if (v == null || v === p.value) return;
    live[k] = v;
    commit(k, v, paired, p, isInt);
  }

  // printf-ish `%[+][.N]f`, with `%%` a literal percent — the only forms the reference data uses.
  function printf(template, v) {
    let used = false;
    return template.replace(/%%|%(\+?)(?:\.(\d+))?f/g, (m, plus, prec) => {
      if (m === "%%") return "%";
      if (used) return m;
      used = true;
      const s = v.toFixed(prec === undefined ? 0 : Number(prec));
      return plus && v >= 0 ? `+${s}` : s;
    });
  }
</script>

{#if block}
  <div class="panel">
    <div class="head">
      <div class="title">
        {block.user_label || block.model_name}
        {#if block.paired_model_name}<span class="paired">+ {block.paired_model_name}</span>{/if}
        <span class="slot">slot {block.slot}</span>
        {#if editable && footswitchCount > 0}
          <!-- Re-sending op 56 for a different switch *moves* the binding, so this is one select
               and not an unassign-then-assign pair. -->
          <label class="fspick" title="Which footswitch toggles this block's bypass">
            <span>FS</span>
            <select
              value={block.footswitch}
              onchange={(e) =>
                onBypassSwitch(block.slot, Number(e.currentTarget.value), block.footswitch)}
            >
              <option value={0}>&mdash;</option>
              <!-- Not capped: `footswitchCount` is the preset's own layout length, so it is already
                   this device's number — 5 on a Stomp, 8 on an XL. Capping it at 5 here was issue
                   #13, where an XL showed FS6 in the chain and could not pick it. Unlike the
                   controller SOURCES above, ops 56/57 take a plain switch index with no table to
                   overrun. -->
              {#each Array.from({ length: footswitchCount }, (_, i) => i + 1) as n}
                <option value={n}>{n}</option>
              {/each}
            </select>
          </label>
          {#if block.footswitch > 0 && !block.is_controller}
            <button
              class="fsedit"
              class:open={fsEditing}
              title="Custom label and LED colour for FS{block.footswitch}"
              onclick={() => {
                fsEditing = !fsEditing;
                fsLabelDraft = block.user_label || "";
              }}>✎</button
            >
          {/if}
        {:else if block.footswitch > 0}
          <span class="fs" title="This block's bypass is on footswitch {block.footswitch}."
            >FS{block.footswitch}</span
          >
        {/if}
        {#if bypassOnPedal}
          <span
            class="fs"
            title="This block's bypass is on {bypassOnPedal.source_name}. Assigned on the pedal — fretwire can read this binding but not write one."
            >{bypassOnPedal.source_name}</span
          >
        {/if}
      </div>
      <div class="actions">
        {#if !isNode && (block.bypassed === true || block.bypassed === false)}
          <button
            class="bypass"
            class:on={!block.bypassed}
            onclick={() => onBypass(block.slot, !block.bypassed)}
          >
            {block.bypassed ? "Bypassed" : "Active"}
          </button>
        {/if}
        {#if editable}
          <button class="act" onclick={() => { swapping = !swapping; swappingCab = false; }}>Change model ▾</button>
          {#if block.paired_index != null && block.model_index != null}
            <button class="act" onclick={() => { swappingCab = !swappingCab; swapping = false; }}>Change cab ▾</button>
          {/if}
          <button class="act" onclick={() => onCopyBlock(block.slot)} title="Copy this block, with all its settings">Copy</button>
          <!-- The copied block's name lives in the tooltip, not the label: this row sits opposite
               the block title in a `space-between` header, and a long model name in a button
               pushes the title out of shape. -->
          <button
            class="act"
            disabled={!blockClip}
            onclick={() => onPasteBlock(block.slot)}
            title={blockClip ? `Replace this block with the copied "${blockClip}"` : "Copy a block first"}
          >
            Paste
          </button>
          <button class="act danger" onclick={() => onDelete(block.slot)}>Delete</button>
        {/if}
      </div>
    </div>

    {#if fsEditing && editable && block.footswitch > 0}
      <!-- Both write the preset document (there is no surgical op) — edit-buffer only, undoable,
           and the ring/scribble on the pedal repaint immediately [verified live 2026-08-27]. -->
      <div class="fseditor">
        <span class="cap">FS{block.footswitch}</span>
        <input
          placeholder={block.model_name}
          value={fsLabelDraft}
          maxlength="16"
          oninput={(e) => (fsLabelDraft = e.currentTarget.value)}
          onkeydown={(e) => e.key === "Enter" && commitFsLabel()}
        />
        <button class="act" onclick={commitFsLabel}>Set label</button>
        <button
          class="act"
          disabled={!block.user_label}
          onclick={() => onSwitchLabel(block.footswitch, null)}>Clear</button
        >
        <span class="swatches">
          {#each FS_PALETTE as p}
            <button
              class="swatch"
              class:sel={block.custom_color === p.index}
              style="background: {p.css}"
              title={p.name}
              aria-label="LED colour {p.name}"
              onclick={() => onSwitchColor(block.footswitch, p.index)}
            ></button>
          {/each}
          <button
            class="swatch none"
            class:sel={block.custom_color == null}
            title="No custom colour — the block's own"
            aria-label="Clear custom LED colour"
            onclick={() => onSwitchColor(block.footswitch, null)}>×</button
          >
        </span>
        <!-- Latching: a press toggles. Momentary: the block flips only while the switch is held.
             Layout key 12 on every binding of the switch, like the label and colour. -->
        <span class="fstype" role="group" aria-label="Switch type">
          <button class="act" class:sel={!block.momentary} onclick={() => block.momentary && onSwitchType(block.footswitch, false)}>Latching</button>
          <button class="act" class:sel={block.momentary} onclick={() => !block.momentary && onSwitchType(block.footswitch, true)}>Momentary</button>
        </span>
      </div>
    {/if}

    {#if isSplit && splitTypes.length}
      <div class="splittype">
        <span class="cap">Split type</span>
        <select
          value={block.symbolic_id}
          onchange={(e) => {
            const t = splitTypes.find((t) => t.symbolic_id === e.currentTarget.value);
            if (t) onSplitType(block.slot, t.index);
          }}
        >
          {#each splitTypes as t}<option value={t.symbolic_id}>{t.label}</option>{/each}
        </select>
      </div>
    {/if}

    {#if swapping && editable}
      <ModelPicker
        title="Change model"
        variant={block.variant}
        currentSymbolicId={block.symbolic_id}
        initialCategory={block.category}
        remaining={swapRemaining}
        {budget}
        onpick={(idx, defaultPaired) => {
          swapping = false;
          // An Amp+Cab pick brings its own matched cab; otherwise keep the current pairing.
          onSwap(block.slot, idx, defaultPaired ?? block.paired_index ?? -1);
        }}
        oncancel={() => (swapping = false)}
      />
    {/if}

    {#if swappingCab && editable}
      <!-- Change only the paired cab: same op as a model swap, re-sending the block's own model.
           LIVE: whether the device keeps the amp's knob values through this is unverified. -->
      <ModelPicker
        title="Change cab"
        currentSymbolicId={block.paired_symbolic_id}
        initialCategory={block.paired_category ?? 19}
        lockCategory
        remaining={swapRemaining}
        {budget}
        onpick={(idx) => {
          swappingCab = false;
          onSwap(block.slot, block.model_index, idx);
        }}
        oncancel={() => (swappingCab = false)}
      />
    {/if}

    {@render controls(block.params, false)}

    {#if block.paired_params.length}
      <div class="subhead">{block.paired_model_name ?? "Cab / IR"}</div>
      {@render controls(block.paired_params, true)}
    {/if}
  </div>
{/if}

{#snippet controls(params, paired)}
  <!-- A cab's params get the mic drawing *beside* them. Detected from the params themselves rather
       than the category, because the two cab families differ (the legacy one has no Position or
       Angle) and because a paired cab and a standalone Cab block are the same list either way.
       Above the grid it was a band of dead space — the drawing is wider than it is tall and the
       grid is not, so side by side is what actually fills the panel. It wraps under the params when
       there isn't room for both. -->
  {@const showMic =
    params.some((p) => p.name === "Mic") && params.some((p) => p.name === "Distance")}
  <div class="controls" class:withmic={showMic}>
    <div class="grid">
      {#each params as p (p.index)}
        {@const k = key(paired, p)}
        {@const c = control(p)}
        {@const asg = assignmentFor(paired, p.index)}
        {@const sync = syncOf(p, params)}
        <!-- A tempo-sync group is one control here, as in HX Edit and on the pedal: the
             `Tempo Sync` switch and the `Note Sync` value have no rows of their own; the knob
             they govern carries the switch, and shows the note value while it is on. -->
        {#if !(p.sync && p.sync.role !== "governed")}
        <div class="ctrl" class:assigned={!!asg}>
          <span class="cap">
            {p.name}
            {#if sync}
              <button
                class="syncbtn"
                class:on={sync.on}
                title={sync.on ? "Synced to tempo — click for a free value" : "Sync to the tempo (note values)"}
                onclick={() => onEnum(block.slot, paired, sync.tempoIndex, sync.on ? 0 : 1)}>♩</button>
            {/if}
            <button
              class="asgbtn"
              class:on={!!asg}
              title={asg
                ? `Driven by ${asg.source_name} — click to change`
                : "Put this parameter under a footswitch or expression pedal"}
              onclick={() => (openAssign = openAssign === k ? null : k)}>{asg ? asg.source_name : "\u21e2"}</button
            >
          </span>
          {#if sync?.on}
            <!-- The note value in place of the knob, written to the `SyncSelect` param — the
                 same list and the same `enum_base` offset the plain enum branch uses. -->
            <select value={sync.note.value} onchange={(e) => onEnum(block.slot, paired, sync.note.index, Number(e.currentTarget.value))}>
              {#each sync.note.enum_labels as lbl, i}<option value={i + (sync.note.enum_base ?? 0)}>{lbl}</option>{/each}
            </select>
          {:else if isIrSelect(p)}
            <!-- Sized to its cell, not to its longest option: an IR name is as wide as its author
                 made it, and a select that grows to fit one runs under the next parameter's label
                 (owner screenshot, 2026-09-03). The closed control clips; the open list is the
                 browser's own popup and shows the full names. -->
            <select
              class="irsel"
              value={p.value}
              onchange={(e) => onEnum(block.slot, paired, p.index, Number(e.currentTarget.value))}
              title={irSlots.length ? "The pedal's IR slots, by number and name" : "IR slot number — names appear once the IR list has been read (IRs… in the toolbar)"}
            >
              {#each irChoices(p) as i (i)}<option value={i}>{irLabel(i)}</option>{/each}
            </select>
          {:else if c === "enum"}
            <!-- The option's value is the wire value, which starts at `enum_base` and not at 0 —
                 `Note Sync` labels 1..=19. Offsetting here keeps read and write on the same entry;
                 indexing the list from 0 displayed one note past the pedal and wrote one short of
                 the pick (issue #8). -->
            <select value={p.value} onchange={(e) => onEnum(block.slot, paired, p.index, Number(e.currentTarget.value))}>
              {#each p.enum_labels as lbl, i}<option value={i + (p.enum_base ?? 0)}>{lbl}</option>{/each}
            </select>
          {:else if c === "bool"}
            <label class="switch">
              <input
                type="checkbox"
                checked={p.value >= 0.5}
                onchange={(e) => onEnum(block.slot, paired, p.index, e.currentTarget.checked ? 1 : 0)}
              />
              <span>{p.value >= 0.5 ? "On" : "Off"}</span>
            </label>
          {:else if c === "unsettable"}
            <span
              class="val unranged"
              title="The device carries this value but fretwire has no confirmed way to address it, so it is read-only here rather than a control that would be refused."
              >{p.kind === "bool" ? (p.value >= 0.5 ? "On" : "Off") : fmtVal(p, p.value)}</span
            >
          {:else if c === "unranged"}
            <span
              class="val unranged"
              title="No range for this parameter in the reference data, so fretwire won't send a value it can't bound — an out-of-range integer can hang the device."
              >{fmtVal(p, p.value)}</span
            >
          {:else if c === "seg"}
            {@const active = nearestStop(p)}
            <div class="seg">
              {#each p.stops as s (s.value)}
                <button
                  class="segbtn"
                  class:active={s.value === active.value}
                  onclick={() => s.value !== active.value && onFloat(block.slot, paired, p.index, s.value)}
                >{s.label}</button>
              {/each}
            </div>
          {:else}
            {@const isInt = c === "int"}
            {@const r = range(p, isInt)}
            <div class="slider">
              <input
                type="range"
                min={r.min}
                max={r.max}
                step={r.step}
                value={live[k] ?? p.value}
                use:wheelable={(e) => nudge(e, k, p, paired, isInt, r)}
                ondblclick={() => resetToDefault(k, p, paired, isInt)}
                title={p.default != null
                  ? `Double-click to reset to ${fmtVal(p, p.default)}`
                  : "Shift+scroll to nudge"}
                oninput={(e) => {
                  const v = e.currentTarget.valueAsNumber;
                  live[k] = v;
                  if (!isInt) preview(k, paired, p, v);
                }}
                onchange={(e) => commit(k, e.currentTarget.valueAsNumber, paired, p, isInt)}
              />
              {#if typing === k}
                <input
                  class="val typing"
                  value={typed}
                  oninput={(e) => (typed = e.currentTarget.value)}
                  onblur={() => commitTyped(k, p, paired, isInt)}
                  onkeydown={(e) => {
                    if (e.key === "Enter") e.currentTarget.blur();
                    else if (e.key === "Escape") { typing = null; e.currentTarget.blur(); }
                  }}
                  use:focusOnMount
                />
              {:else}
                <button class="val" title="Click to type a value" onclick={() => openTyping(k, p)}>
                  {fmtVal(p, live[k] ?? p.value)}
                </button>
              {/if}
            </div>
          {/if}
          {#if openAssign === k}
            <div class="asgrow">
              <label>
                Controlled by
                <select
                  value={asg?.source ?? 0}
                  onchange={(e) =>
                    onAssignParam(block.slot, paired, p.index, Number(e.currentTarget.value))}
                >
                  {#each SOURCES as src}<option value={src.value}>{src.label}</option>{/each}
                </select>
              </label>
              {#if asg}
                <!-- The travel ends are in the parameter's own units, which is why they reuse the
                     parameter's own range rather than a 0..1 sweep: a pitch block's ends are
                     semitones. Bools have no meaningful middle, so they get a two-state select. -->
                {@const tr = range(p, control(p) === "int")}
                {#each [["Min", false], ["Max", true]] as [lbl, isMax]}
                  {@const v = (isMax ? asg.max : asg.min) ?? (isMax ? tr.max : tr.min)}
                  <label class="travel">
                    {lbl}
                    {#if p.kind === "bool"}
                      <select
                        value={v >= 0.5 ? 1 : 0}
                        onchange={(e) =>
                          onAssignTravel(block.slot, paired, p.index, isMax, Number(e.currentTarget.value))}
                      >
                        <option value={0}>Off</option>
                        <option value={1}>On</option>
                      </select>
                    {:else}
                      <input
                        type="range"
                        min={tr.min}
                        max={tr.max}
                        step={tr.step}
                        value={v}
                        onchange={(e) =>
                          onAssignTravel(block.slot, paired, p.index, isMax, e.currentTarget.valueAsNumber)}
                      />
                      <span class="tval">{fmtVal(p, v)}</span>
                    {/if}
                  </label>
                {/each}
              {/if}
            </div>
          {/if}
        </div>
        {/if}
      {/each}
    </div>
    {#if showMic}
      <CabMicView {params} {paired} slot={block.slot} {onFloat} {onPreview} {fmtVal} />
    {/if}
  </div>
{/snippet}

<style>
  .panel {
    margin-top: 18px;
    border: 1px solid #2a2e37;
    border-radius: 10px;
    background: #1b1e25;
    padding: 14px 16px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 12px;
  }
  .title {
    font-weight: 600;
    font-size: 15px;
  }
  .title .paired {
    color: #9aa3b2;
    font-weight: 400;
    margin-left: 6px;
  }
  .title .slot {
    color: #6b7280;
    font-weight: 400;
    font-size: 12px;
    margin-left: 8px;
  }
  /* Matches the chain cell's badge — same binding, same colour, so the two read as one fact. */
  .title .fs {
    margin-left: 8px;
    padding: 1px 6px;
    border-radius: 8px;
    background: #24405e;
    color: #9fc4ee;
    font-size: 11px;
    font-weight: 700;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    /* Six buttons is enough to crowd a narrow panel — wrap rather than overflow. */
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .act {
    font: inherit;
    white-space: nowrap;
    border: 1px solid #3a4150;
    background: #232833;
    color: #c3c9d4;
    padding: 5px 12px;
    border-radius: 6px;
    cursor: pointer;
  }
  .act:hover {
    border-color: #3f8ae0;
  }
  .act.danger:hover {
    border-color: #d9534f;
    color: #ffb3b0;
  }
  .bypass {
    font: inherit;
    border: 1px solid #3a4150;
    background: #232833;
    color: #9aa3b2;
    padding: 5px 12px;
    border-radius: 6px;
    cursor: pointer;
  }
  .bypass.on {
    background: #1e5a2f;
    border-color: #2f8a47;
    color: #d7f5e0;
  }
  .splittype {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 12px;
  }
  .title .fsedit {
    margin-left: 4px;
    background: none;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #9aa3b2;
    font-size: 12px;
    cursor: pointer;
    padding: 0 4px;
  }
  .title .fsedit:hover,
  .title .fsedit.open {
    color: #e6e9ef;
    border-color: #3a4150;
  }
  .fseditor {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-bottom: 12px;
  }
  .fseditor .cap {
    color: #c3c9d4;
    font-size: 13px;
  }
  .fseditor input {
    width: 140px;
    font-size: 12px;
    padding: 3px 6px;
  }
  .fseditor .swatches {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-left: 4px;
  }
  .fseditor .swatch {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    border: 2px solid #2a2f3a;
    padding: 0;
    cursor: pointer;
  }
  .fseditor .swatch.sel {
    border-color: #e6e9ef;
  }
  .fseditor .swatch.none {
    background: none;
    border-style: dashed;
    border-width: 1px;
    color: #9aa3b2;
    font-size: 11px;
    line-height: 1;
  }
  .fseditor .swatch.none.sel {
    border-color: #e6e9ef;
    border-style: solid;
  }
  .splittype .cap {
    color: #c3c9d4;
    font-size: 13px;
  }
  .subhead {
    margin: 16px 0 8px;
    color: #9aa3b2;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 10px 32px;
  }
  /* Cab blocks only: the drawing sits beside the params instead of banding across the top. The
     grid keeps its own auto-fill columns in whatever width is left, and `min-width: 0` is what
     lets it actually give width back — a grid item defaults to its content's min size and would
     otherwise shove the drawing off the panel. Below ~600px there is no room for both and the
     drawing wraps underneath. */
  .controls.withmic {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: 12px 24px;
  }
  .controls.withmic .grid {
    flex: 1 1 300px;
    min-width: 0;
  }
  .ctrl {
    display: grid;
    grid-template-columns: 110px 1fr;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .ctrl .cap {
    color: #c3c9d4;
    font-size: 13px;
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  /* The assign affordance stays quiet until a parameter is actually assigned: every row carries
     one, and a row of bright badges would read as "these are all controlled". */
  .fstype .act.sel {
    background: #3a6ea5;
    border-color: #3a6ea5;
    color: #fff;
  }
  /* The tempo-sync switch, folded onto the knob it governs. */
  .syncbtn {
    font: inherit;
    font-size: 0.85rem;
    line-height: 1;
    padding: 0 0.3rem;
    margin-left: 0.25rem;
    border: 1px solid #555;
    border-radius: 4px;
    background: transparent;
    color: #9aa3b2;
    cursor: pointer;
  }
  .syncbtn.on {
    background: #3a6ea5;
    border-color: #3a6ea5;
    color: #fff;
  }
  .asgbtn {
    flex: none;
    border: 1px solid #33405a;
    background: transparent;
    color: #6d7a90;
    border-radius: 8px;
    padding: 0 5px;
    font-size: 10px;
    line-height: 15px;
    cursor: pointer;
  }
  .asgbtn:hover {
    color: #cfe0f5;
    border-color: #4a6a95;
  }
  .asgbtn.on {
    background: #24405e;
    border-color: #35618f;
    color: #9fc4ee;
    font-weight: 700;
  }
  .ctrl.assigned .cap {
    color: #e2e8f2;
  }
  .asgrow {
    grid-column: 1 / -1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
    margin: 6px 0 2px;
    padding: 8px 10px;
    border-radius: 8px;
    background: #1a2334;
    border: 1px solid #2b3a52;
    font-size: 12px;
    color: #9aa3b2;
  }
  .asgrow label {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .asgrow .travel input[type="range"] {
    width: 96px;
  }
  .asgrow .tval {
    min-width: 52px;
    color: #c3c9d4;
    font-variant-numeric: tabular-nums;
  }
  .title .fspick {
    margin-left: 8px;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: #9aa3b2;
  }
  .title .fspick select {
    font-size: 11px;
    padding: 1px 4px;
  }
  .slider {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  .slider input[type="range"] {
    flex: 1 1 auto;
    min-width: 0;
  }
  .val.unranged {
    color: #6b7280;
    cursor: help;
    text-align: left;
    border-bottom: 1px dotted #3a4150;
  }
  .val {
    flex: 0 0 auto;
    /* Wide enough for the unit-bearing forms ("-14.4 dB", "1.373 s") so the column doesn't jump
       width mid-drag as a value crosses from one format range into the next. */
    min-width: 68px;
    text-align: right;
    white-space: nowrap;
    color: #e6e8ec;
    font-variant-numeric: tabular-nums;
    font-size: 13px;
  }
  /* The readout doubles as a click-to-type field. Strip the button/input chrome so it still reads
     as a value until you're in it, and pin the font rather than inheriting — `.val` sets 13px and a
     bare `font: inherit` would take the row's size instead. */
  button.val,
  input.val {
    font-family: inherit;
    font-size: 13px;
    background: none;
    border: 0;
    padding: 0;
    color: inherit;
    cursor: text;
  }
  button.val:hover {
    color: #fff;
    text-decoration: underline dotted #4a5265;
  }
  input.val.typing {
    width: 76px;
    background: #232833;
    border: 1px solid #4a5265;
    border-radius: 4px;
    padding: 1px 4px;
    color: #e6e8ec;
  }
  select {
    font: inherit;
    background: #232833;
    color: #e6e8ec;
    border: 1px solid #3a4150;
    border-radius: 6px;
    padding: 4px 8px;
  }
  select.irsel {
    width: 100%;
    min-width: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .switch {
    display: flex;
    align-items: center;
    gap: 8px;
    color: #c3c9d4;
    font-size: 13px;
  }
  .seg {
    display: inline-flex;
    border: 1px solid #3a4150;
    border-radius: 6px;
    overflow: hidden;
    width: fit-content;
  }
  .segbtn {
    font: inherit;
    font-size: 13px;
    background: #232833;
    color: #9aa3b2;
    border: 0;
    padding: 4px 12px;
    cursor: pointer;
  }
  .segbtn + .segbtn {
    border-left: 1px solid #3a4150;
  }
  .segbtn.active {
    background: #2b7de0;
    color: #fff;
  }
</style>
