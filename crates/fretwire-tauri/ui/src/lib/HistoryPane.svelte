<script>
  // Expandable edit-history timeline: every edit is an entry (newest at the top); the highlighted
  // row is the state currently in the edit buffer. Clicking a row jumps the device there (op-21
  // write — pure navigation until the next edit truncates the future). Mark any two rows A and B,
  // then toggle between them to A/B-compare by ear.
  let { history = [], cursor = 0, onjump } = $props();

  let open = $state(false);
  let aIdx = $state(null);
  let bIdx = $state(null);

  // Entries can disappear (new edit truncates the future; preset switch clears) — drop stale marks.
  $effect(() => {
    if (aIdx != null && aIdx >= history.length) aIdx = null;
    if (bIdx != null && bIdx >= history.length) bIdx = null;
  });

  const canAB = $derived(aIdx != null && bIdx != null && aIdx !== bIdx);
  // Toggle to whichever mark we're not on (from an unmarked row, start at A).
  const toggleAB = () => onjump(cursor === aIdx ? bIdx : aIdx);
  const hearing = $derived(cursor === aIdx ? "A" : cursor === bIdx ? "B" : null);

  // Newest first for display; keep the real index for jumps.
  const rows = $derived(history.map((label, i) => ({ label, i })).reverse());
</script>

{#if history.length}
  <div class="hist">
    <button class="head" onclick={() => (open = !open)}>
      <span class="tri">{open ? "▾" : "▸"}</span>
      History
      <span class="count">{history.length}</span>
      {#if hearing}<span class="ab-chip">hearing {hearing}</span>{/if}
      <span class="spacer"></span>
      {#if canAB}
        <span
          class="ab-btn"
          role="button"
          tabindex="0"
          onclick={(e) => {
            e.stopPropagation();
            toggleAB();
          }}
          onkeydown={(e) => e.key === "Enter" && (e.stopPropagation(), toggleAB())}
        >A ⇄ B</span>
      {/if}
    </button>
    {#if open}
      <div class="rows">
        {#each rows as r (r.i)}
          <div class="row" class:cur={r.i === cursor}>
            <button class="jump" onclick={() => onjump(r.i)}>
              <span class="idx">{r.i}</span>
              <span class="lbl">{r.label}</span>
            </button>
            <button class="mark" class:on={aIdx === r.i} title="Mark as A" onclick={() => (aIdx = aIdx === r.i ? null : r.i)}>A</button>
            <button class="mark b" class:on={bIdx === r.i} title="Mark as B" onclick={() => (bIdx = bIdx === r.i ? null : r.i)}>B</button>
          </div>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .hist {
    margin-bottom: 14px;
    border: 1px solid #2a2e37;
    border-radius: 10px;
    background: #1b1e25;
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    font: inherit;
    font-size: 13px;
    text-align: left;
    background: transparent;
    color: #c3c9d4;
    border: 0;
    padding: 8px 12px;
    cursor: pointer;
  }
  .tri {
    color: #6b7280;
    font-size: 11px;
  }
  .count {
    color: #6b7280;
    font-size: 12px;
    background: #232833;
    border-radius: 8px;
    padding: 1px 7px;
  }
  .ab-chip {
    color: #f0c245;
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .ab-btn {
    font-size: 12px;
    background: #26333f;
    border: 1px solid #3f8ae0;
    color: #fff;
    border-radius: 6px;
    padding: 3px 10px;
    cursor: pointer;
  }
  .ab-btn:hover {
    background: #2b4a66;
  }
  .rows {
    max-height: 220px;
    overflow-y: auto;
    border-top: 1px solid #2a2e37;
    padding: 4px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 6px;
  }
  .row.cur {
    background: #26333f;
  }
  .row.cur .lbl {
    color: #fff;
  }
  .jump {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    font: inherit;
    font-size: 13px;
    text-align: left;
    background: transparent;
    color: #c3c9d4;
    border: 0;
    border-radius: 6px;
    padding: 5px 8px;
    cursor: pointer;
    min-width: 0;
  }
  .jump:hover {
    background: #232833;
  }
  .row.cur .jump:hover {
    background: #2b3a49;
  }
  .idx {
    color: #6b7280;
    font-variant-numeric: tabular-nums;
    font-size: 11px;
    min-width: 18px;
  }
  .lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mark {
    flex: 0 0 auto;
    font: inherit;
    font-size: 11px;
    width: 22px;
    height: 20px;
    background: transparent;
    color: #4b5563;
    border: 1px solid #2a2e37;
    border-radius: 5px;
    cursor: pointer;
    margin-right: 2px;
  }
  .mark:hover {
    color: #c3c9d4;
    border-color: #3a4150;
  }
  .mark.on {
    background: #3b2f14;
    border-color: #f0c245;
    color: #f0c245;
  }
</style>
