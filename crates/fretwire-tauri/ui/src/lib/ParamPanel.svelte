<script>
  // Editor for the selected block: bypass toggle + a control per parameter (slider / dropdown /
  // switch), for both the main model and any paired cab/IR. Commits go up via callbacks; the parent
  // re-reads the preset so values reflect the device.
  import ModelPicker from "./ModelPicker.svelte";

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
  } = $props();

  // Routing nodes (split/mixer) and controller assignments aren't category-swappable or deletable
  // here — the split *type* is changed elsewhere, controllers are footswitch bindings.
  const editable = $derived(!isNode && !block?.is_controller);

  let swapping = $state(false);
  let swappingCab = $state(false);
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
        {#if block.footswitch > 0}
          <span
            class="fs"
            title="This block's bypass is on footswitch {block.footswitch}. Read from the preset; fretwire can't change the binding yet."
          >FS{block.footswitch}</span>
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
