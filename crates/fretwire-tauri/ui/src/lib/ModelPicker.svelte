<script>
  // Category + model chooser, used for both "change model" (swap) and "add block". Lists every
  // model in the chosen category with its DSP cost; the current model is marked, and models that
  // wouldn't fit the remaining DSP budget are disabled — mirroring HX Edit's grey-out.
  import { invoke } from "./ipc.js";
  import { onMount, untrack } from "svelte";

  let {
    title = "Change model",
    variant = null,
    currentSymbolicId = null,
    initialCategory = null,
    // Pin the picker to `initialCategory` (e.g. the change-cab flow only offers cabs).
    lockCategory = false,
    // Both in raw model-file units, as `dsp_load` comes off the wire — the *comparison* stays in
    // that scale, only the numbers on screen are converted.
    remaining = 75,
    budget = 75,
    onpick,
    oncancel,
  } = $props();

  // Shown as a percentage of what the pedal will actually accept, so the ceiling reads 100% and a
  // model's cost adds up against the meter next to it. See `editor::DSP_CEILING`.
  const pct = (raw) => (raw / budget) * 100;

  let cats = $state([]);
  // Intentionally a one-time snapshot of the prop (the picker's starting category, then
  // user-driven); `untrack` says so and silences the "only the initial value" lint.
  let categoryId = $state(untrack(() => initialCategory));
  let models = $state([]);
  let loading = $state(false);
  let err = $state(null);

  async function loadModels() {
    if (categoryId == null) return;
    loading = true;
    err = null;
    try {
      models = await invoke("models_in_category", { category: categoryId, variant });
    } catch (e) {
      err = String(e);
    }
    loading = false;
  }

  onMount(async () => {
    try {
      cats = await invoke("categories");
      if (categoryId == null && cats.length) categoryId = cats[0].id;
      await loadModels();
    } catch (e) {
      err = String(e);
    }
  });

  function onCat(e) {
    categoryId = Number(e.currentTarget.value);
    loadModels();
  }

  const fits = (m) => (m.dsp_load ?? 0) <= remaining + 0.001;
</script>

<div class="picker">
  <div class="row">
    <strong>{title}</strong>
    {#if lockCategory}
      <span class="dim">{cats.find((c) => c.id === categoryId)?.name ?? ""}</span>
    {:else}
      <select value={categoryId} onchange={onCat}>
        {#each cats as c}<option value={c.id}>{c.name}</option>{/each}
      </select>
    {/if}
    <span class="dim">{pct(remaining).toFixed(1)}% DSP free</span>
    <span class="spacer"></span>
    <button class="x" onclick={oncancel}>✕</button>
  </div>
  {#if err}<div class="err">{err}</div>{/if}
  {#if loading}<div class="dim">loading…</div>{/if}
  <div class="list">
    {#each models as m (m.index)}
      {@const isCurrent = currentSymbolicId && m.symbolic_id === currentSymbolicId}
      {@const tooBig = !isCurrent && !fits(m)}
      <button
        class="item"
        class:current={isCurrent}
        disabled={tooBig}
        title={tooBig
          ? `Needs ${pct(m.dsp_load ?? 0).toFixed(1)}% DSP; only ${pct(remaining).toFixed(1)}% is free. Remove or simplify a block to make room.`
          : m.name}
        onclick={() => onpick(m.index, m.default_paired_index ?? null)}
      >
        <span class="name">{m.name}{isCurrent ? " ✓" : ""}</span>
        <span class="dsp">{m.dsp_load != null ? pct(m.dsp_load).toFixed(1) + "%" : "—"}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .picker {
    margin-top: 12px;
    border: 1px solid #3a4150;
    border-radius: 8px;
    background: #12141a;
    padding: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 10px;
  }
  .spacer {
    flex: 1;
  }
  .dim {
    color: #9aa3b2;
    font-size: 12px;
  }
  .err {
    color: #ff8a8a;
    margin-bottom: 8px;
  }
  .list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 6px;
    max-height: 300px;
    overflow-y: auto;
  }
  .item {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    font: inherit;
    text-align: left;
    background: #232833;
    color: #e6e8ec;
    border: 1px solid #2a2e37;
    border-radius: 6px;
    padding: 7px 10px;
    cursor: pointer;
  }
  .item:hover:not(:disabled) {
    border-color: #3f8ae0;
  }
  .item.current {
    border-color: #f0c245;
  }
  /* Won't fit the DSP that's left. Dimmed rather than hidden, so the cost stays readable and it's
     obvious the model exists — and its cost is the one number that explains the greying. */
  .item:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .item:disabled .dsp {
    color: #e0785f;
  }
  .item .dsp {
    color: #9aa3b2;
    font-variant-numeric: tabular-nums;
  }
  select {
    font: inherit;
    background: #232833;
    color: #e6e8ec;
    border: 1px solid #3a4150;
    border-radius: 6px;
    padding: 4px 8px;
  }
  .x {
    font: inherit;
    background: #363b46;
    color: #e6e8ec;
    border: 0;
    border-radius: 6px;
    padding: 4px 10px;
    cursor: pointer;
  }
</style>
