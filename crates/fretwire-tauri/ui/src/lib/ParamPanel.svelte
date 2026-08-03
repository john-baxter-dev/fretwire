<script>
  // Editor for the selected block: bypass toggle + a control per parameter (slider / dropdown /
  // switch), for both the main model and any paired cab/IR. Commits go up via callbacks; the parent
  // re-reads the preset so values reflect the device.
  import ModelPicker from "./ModelPicker.svelte";

  let {
    block,
    dspLoad = 0,
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
  } = $props();

  // Routing nodes (split/mixer) and controller assignments aren't category-swappable or deletable
  // here — the split *type* is changed elsewhere, controllers are footswitch bindings.
  const editable = $derived(!isNode && !block?.is_controller);

  const BUDGET = 100;
  let swapping = $state(false);
  let swappingCab = $state(false);
  // Reset the swap pickers when the selected block changes.
  $effect(() => {
    block?.slot;
    swapping = false;
    swappingCab = false;
  });
  // DSP budget available if this block were replaced (exclude its own current load).
  const swapRemaining = $derived(BUDGET - (dspLoad - (block?.dsp_load ?? 0)));

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

  // ---- scroll-wheel nudging ----
  // A slider responds to the wheel while Shift is held (hover anywhere on it) or after it's been
  // clicked (focused). Each notch nudges one step; the commit is debounced so a burst of notches
  // is one USB write (and one undo entry). Svelte 5 registers `onwheel` passively, so this is an
  // action attaching a non-passive listener — preventDefault must work to stop the page scrolling.
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
    const step = isInt ? 1 : (r.max - r.min) / 100;
    const cur = live[k] ?? p.value;
    const v = Math.min(r.max, Math.max(r.min, cur + dir * step));
    live[k] = v;
    if (!isInt) preview(k, paired, p, v); // audible ramp per notch (float wire path only)
    clearTimeout(wheelTimers[k]);
    wheelTimers[k] = setTimeout(() => {
      const val = live[k];
      delete live[k];
      delete wheelTimers[k];
      cancelPreview(k);
      if (isInt) onEnum(block.slot, paired, p.index, Math.round(val));
      else onFloat(block.slot, paired, p.index, val);
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
</script>

{#if block}
  <div class="panel">
    <div class="head">
      <div class="title">
        {block.user_label || block.model_name}
        {#if block.paired_model_name}<span class="paired">+ {block.paired_model_name}</span>{/if}
        <span class="slot">slot {block.slot}</span>
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
  <div class="grid">
    {#each params as p (p.index)}
      {@const k = key(paired, p)}
      {@const c = control(p)}
      <div class="ctrl">
        <span class="cap">{p.name}</span>
        {#if c === "enum"}
          <select value={p.value} onchange={(e) => onEnum(block.slot, paired, p.index, Number(e.currentTarget.value))}>
            {#each p.enum_labels as lbl, i}<option value={i}>{lbl}</option>{/each}
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
            >{p.kind === "bool" ? (p.value >= 0.5 ? "On" : "Off") : fmt(p.value)}</span
          >
        {:else if c === "unranged"}
          <span
            class="val unranged"
            title="No range for this parameter in the reference data, so fretwire won't send a value it can't bound — an out-of-range integer can hang the device."
            >{fmt(p.value)}</span
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
              oninput={(e) => {
                const v = e.currentTarget.valueAsNumber;
                live[k] = v;
                if (!isInt) preview(k, paired, p, v);
              }}
              onchange={(e) => {
                const v = e.currentTarget.valueAsNumber;
                delete live[k];
                cancelPreview(k);
                if (isInt) onEnum(block.slot, paired, p.index, Math.round(v));
                else onFloat(block.slot, paired, p.index, v);
              }}
            />
            <span class="val">{fmt(live[k] ?? p.value)}</span>
          </div>
        {/if}
      </div>
    {/each}
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
    min-width: 48px;
    text-align: right;
    white-space: nowrap;
    color: #e6e8ec;
    font-variant-numeric: tabular-nums;
    font-size: 13px;
  }
  select {
    font: inherit;
    background: #232833;
    color: #e6e8ec;
    border: 1px solid #3a4150;
    border-radius: 6px;
    padding: 4px 8px;
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
