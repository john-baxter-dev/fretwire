<script>
  // The pedal's global settings, as an overlay for the same reason the IR panel is one: these are
  // not part of the preset being edited, and the list is longer than any sidebar.
  //
  // **These write the pedal itself.** There is no edit buffer behind them and no undo — the same
  // hazard class as an IR write, but continuous rather than destructive, so the honest design is a
  // standing warning rather than a confirmation on every knob. Changing one back is the undo.
  //
  // Ids fretwire has not identified are shown and **not** writable: a value nobody has explained is
  // worth seeing (this panel is how the rest of the space gets mapped) and is not worth writing.

  import GlobalEq from "./GlobalEq.svelte";

  let {
    settings = [],
    busy = false,
    showRaw = false,
    onClose,
    onRefresh,
    onToggleRaw,
    onWrite,
  } = $props();

  // Section order is the backend's to decide: rows arrive sorted by
  // `(group_rank, menu_rank, id)`, so building the map in arrival order reproduces the pedal's own
  // menu. This used to keep its own copy of the list, which silently disagreed with
  // `settings::GROUPS` — a group order that renders is worth more than one that only compiles.

  // The EQ has its own tab, so it must not also appear as a table of eleven numbers below.
  let tab = $state("eq");
  const eqRows = $derived(settings.filter((s) => s.group === "Global EQ"));

  const groups = $derived.by(() => {
    const by = new Map();
    for (const s of settings) {
      if (s.group === "Global EQ") continue;
      if (!by.has(s.group)) by.set(s.group, []);
      by.get(s.group).push(s);
    }
    // A Map iterates in insertion order, and the rows were already in the pedal's order.
    return [...by.entries()];
  });

  const namedCount = $derived(settings.filter((s) => s.writable).length);

  // A number that reads as its "off" sentinel is off, not 19.9 Hz — the device has no separate
  // enable for the EQ cuts.
  function shown(s) {
    if (s.off != null && Math.abs(Number(s.value) - s.off) < 0.05) return "Off";
    if (typeof s.value === "number") {
      return Number.isInteger(s.value) ? String(s.value) : s.value.toFixed(3).replace(/0+$/, "");
    }
    return String(s.value);
  }

  const write = (s, v) => {
    if (!s.writable || busy) return;
    const n = Number(v);
    if (!Number.isFinite(n)) return;
    onWrite?.(s.id, n);
  };
</script>

<div
  class="gl-overlay"
  role="presentation"
  onclick={(e) => e.target === e.currentTarget && onClose?.()}
>
  <div class="gl-card" role="dialog" aria-label="Global settings" aria-modal="true">
    <div class="gl-head">
      <span class="gl-title">Global settings</span>
      <span class="gl-count">
        {tab === "eq"
          ? `${eqRows.length} EQ parameters`
          : `${namedCount} named${
              showRaw && settings.length > namedCount
                ? ` · ${settings.length - namedCount} unidentified`
                : ""
            }`}
      </span>
      <span class="spacer"></span>
      {#if showRaw && tab === "settings"}
        <span class="hint-inline">unidentified ids are read-only</span>
      {/if}
      {#if tab === "settings"}
        <label class="check" title="Also list the ids that answer but have never been explained">
          <input type="checkbox" checked={showRaw} onchange={() => onToggleRaw?.()} disabled={busy} />
          Show unidentified
        </label>
      {/if}
      <button class="row" onclick={() => onRefresh?.()} disabled={busy}>Refresh</button>
      <button class="row" onclick={() => onClose?.()}>Close</button>
    </div>

    <div class="gl-tabs" role="tablist">
      <button class="tab" class:on={tab === "eq"} role="tab" aria-selected={tab === "eq"}
        onclick={() => (tab = "eq")} disabled={!eqRows.length}>Global EQ</button>
      <button class="tab" class:on={tab === "settings"} role="tab" aria-selected={tab === "settings"}
        onclick={() => (tab = "settings")}>All settings</button>
    </div>

    <div class="gl-body">
      {#if tab === "eq" && settings.length}
        <GlobalEq settings={eqRows} {busy} {onWrite} />
      {:else if !settings.length}
        <div class="empty">
          {busy ? "Reading the pedal…" : "Nothing read yet — press Refresh."}
        </div>
      {:else}
        {#each groups as [group, rows] (group)}
          <div class="grp">{group}</div>
          <table>
            <tbody>
              {#each rows as s (s.id)}
                <tr class:raw={!s.writable}>
                  <td class="nm">
                    {s.name}
                    <span class="id">#{s.id}</span>
                  </td>
                  <td class="ctl">
                    {#if s.kind === "flag"}
                      <select
                        class="field"
                        value={s.value ? "1" : "0"}
                        disabled={busy}
                        onchange={(e) => write(s, e.currentTarget.value)}
                      >
                        <option value="1">{s.labels?.[0] ?? "On"}</option>
                        <option value="0">{s.labels?.[1] ?? "Off"}</option>
                      </select>
                    {:else if s.kind === "choice" && s.options.length}
                      <select
                        class="field"
                        value={String(s.value)}
                        disabled={busy}
                        onchange={(e) => write(s, e.currentTarget.value)}
                      >
                        {#each s.options as [v, label] (v)}
                          <option value={String(v)}>{label}</option>
                        {/each}
                        {#if !s.options.some(([v]) => v === s.value)}
                          <!-- The device is on a value nobody has named. Show it rather than
                               silently snapping the picker to a neighbour. -->
                          <option value={String(s.value)}>{s.value} (unnamed)</option>
                        {/if}
                      </select>
                    {:else if s.kind === "raw"}
                      <span class="val">{shown(s)}</span>
                    {:else}
                      <input
                        class="field num"
                        type="number"
                        step="any"
                        value={s.value}
                        disabled={busy}
                        onchange={(e) => write(s, e.currentTarget.value)}
                      />
                      {#if s.unit}<span class="unit">{s.unit}</span>{/if}
                      {#if s.off != null && Math.abs(Number(s.value) - s.off) < 0.05}
                        <span class="unit">— off</span>
                      {/if}
                    {/if}
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        {/each}
      {/if}
    </div>

    <div class="gl-foot">
      These change the pedal itself and take effect at once — there is no undo and no reload that
      takes one back. Setting it back is the way back.
      {#if showRaw && tab === "settings"}
        <code>fretwire settings-diff</code> names an unidentified id in about thirty seconds, and
        then it becomes editable here.
      {/if}
    </div>
  </div>
</div>

<style>
  .gl-overlay {
    position: fixed;
    inset: 0;
    background: rgba(8, 10, 14, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 105;
  }
  .gl-card {
    display: flex;
    flex-direction: column;
    width: 660px;
    max-width: calc(100vw - 40px);
    max-height: calc(100vh - 80px);
    background: #1b1e25;
    border: 1px solid #3a4150;
    border-radius: 8px;
    box-shadow: 0 18px 48px rgba(0, 0, 0, 0.5);
  }
  .gl-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 14px;
    border-bottom: 1px solid #2b303a;
  }
  .gl-title {
    font-weight: 600;
  }
  .gl-count {
    color: #8b93a3;
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .gl-tabs {
    display: flex;
    gap: 2px;
    padding: 8px 14px 0;
    border-bottom: 1px solid #2b303a;
  }
  .tab {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: #8b93a3;
    padding: 5px 10px 7px;
    font-size: 12px;
    cursor: pointer;
  }
  .tab.on {
    color: #e6e9ef;
    border-bottom-color: #5b8def;
  }
  .tab:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .hint-inline {
    color: #6b7280;
    font-size: 11px;
  }
  .gl-body {
    overflow: auto;
    padding: 4px 14px 12px;
  }
  .grp {
    margin: 14px 0 4px;
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #8b93a3;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  td {
    padding: 5px 0;
    border-bottom: 1px solid #23272f;
    vertical-align: middle;
  }
  .nm {
    font-size: 13px;
  }
  .id {
    color: #6b7280;
    font-size: 11px;
    margin-left: 6px;
  }
  .ctl {
    text-align: right;
    white-space: nowrap;
  }
  tr.raw .nm {
    color: #8b93a3;
  }
  .val {
    color: #c9d1d9;
    font-variant-numeric: tabular-nums;
  }
  .field {
    background: #12151a;
    border: 1px solid #3a4150;
    border-radius: 4px;
    color: #e6e9ef;
    padding: 3px 6px;
    font-size: 13px;
  }
  .num {
    width: 96px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .unit {
    color: #8b93a3;
    font-size: 12px;
    margin-left: 5px;
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: #c9d1d9;
  }
  .empty {
    padding: 24px 0;
    text-align: center;
    color: #8b93a3;
  }
  .gl-foot {
    padding: 10px 14px;
    border-top: 1px solid #2b303a;
    color: #8b93a3;
    font-size: 12px;
  }
  code {
    color: #c9d1d9;
  }
  button.row {
    background: #232833;
    border: 1px solid #3a4150;
    color: #e6e9ef;
    border-radius: 4px;
    padding: 3px 9px;
    font-size: 12px;
    cursor: pointer;
  }
  button.row:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
