<script>
  // The signal chain as an interactive routing grid: HTML cells (draggable blocks + empty drop
  // targets) laid out by the device's real grid columns, with SVG wires behind. Each cell maps to
  // exactly one slot; dropping a block onto an empty cell is a single `place_block` to that slot,
  // and the device recomputes the split/mixer columns from where blocks land (we re-read after).
  // `preset` supplies the global block list (slots run `dsp * 20 + index`); `dsp` is the one DSP's
  // routing view this instance draws (grid + split/mixer/io nodes for that DSP). The Helix Floor
  // renders two Chains, one per DSP; a single-DSP device (or the mock) passes no `dsp` and we fall
  // back to the preset's flat, DSP-0 fields.
  import ModelIcon from "./icons/ModelIcon.svelte";

  let {
    preset, dsp = null, selectedSlot = null, catColors = null,
    onselect, onplace, oninsert, onmovenode, onaddat,
  } = $props();
  const d = $derived(dsp ?? preset);

  // Block accent colour by category. **HX Edit's own colours win**, read from the reference data at
  // runtime (`categories` carries them) and passed in as `catColors` — a block should be the same
  // shade in both editors, because two colour schemes for one preset is worse than either.
  //
  // The table below is the fallback for a fresh install with no reference data imported, where the
  // editor still runs with numeric names. It is our own approximation and deliberately *not* a copy
  // of Line 6's values, which are theirs and are never redistributed.
  const CAT_COLORS = {
    1: "#d94f4f", // Amp
    13: "#c96a4a", // Preamp
    2: "#a8783c", // Cab
    19: "#a8783c", // Cab (Mic+IR)
    16: "#94693a", // IR
    3: "#d9c53f", // Distortion
    4: "#4f8fe0", // Dynamics
    5: "#b04fe0", // Synth
    6: "#4fd9c9", // Filter
    7: "#9d6be0", // Pitch/Synth
    8: "#e06bbf", // Modulation
    9: "#4fc26b", // Delay
    10: "#e08a3f", // Reverb
    11: "#8fb83c", // Wah
    14: "#3ca88a", // EQ
    12: "#7a8494", // Send/Return
    15: "#7a8494", // Looper
    17: "#7a8494", // Volume/Pan
  };
  const catColor = (c) => catColors?.[c] ?? CAT_COLORS[c] ?? "#5a6478";

  // X0 leaves room left of column 1 for the input node glyph; the output glyph hangs after the
  // last column (width padded below).
  const STEP = 120, CW = 104, CH = 50, X0 = 48, TOP_Y = 8, BOT_Y = 90;
  const colX = (c) => X0 + (c - 1) * STEP;
  const midY = (y) => y + CH / 2;
  // The x of the inter-column gap *before* column c — where the bracket wires run and the split/
  // mixer node glyphs (and their drop zones) sit.
  const gapX = (c) => colX(c) - (STEP - CW) / 2;

  let dragSrc = $state(null);
  let dragOver = $state(null);
  // Which half of an occupied cell the drag is over: dropping inserts before ("l") / after ("r").
  let dragSide = $state(null);
  // A split/mixer node drag in flight ("split" | "mixer"), and the gap position hovered.
  let dragNode = $state(null);
  let nodeOver = $state(null);

  const view = $derived.by(() => {
    const allCells = d.grid ?? [];
    const bySlot = new Map(preset.blocks.map((b) => [b.slot, b]));
    const split = d.split && d.split_pos != null && d.mixer_pos != null;
    // The grid carries the empty row-B cells even on a serial preset (the split/mixer node slots
    // always exist in the device's fixed slot array). Normally we hide that row when serial — but
    // while a drag is in flight we reveal it as drop targets: dropping a block there is how the
    // split gets *created* (one place_block; the device activates the split and we re-read).
    const dragging = dragSrc != null || dragNode != null;
    const showB = split || dragSrc != null;
    // Trim trailing empty columns: at rest, show up to one spare column past the last block (and
    // never cut the bracket off — keep through the mixer column on a split preset). While a drag is
    // in flight every column is a potential drop target, so reveal them all.
    const maxAllCol = allCells.reduce((m, c) => Math.max(m, c.column), 1);
    const lastOcc = allCells.reduce((m, c) => (c.occupied ? Math.max(m, c.column) : m), 0);
    const maxCol = dragging
      ? maxAllCol
      : Math.max(1, Math.min(Math.max(lastOcc + 1, split ? d.mixer_pos : 1), maxAllCol));
    // Every row-B cell is a legal drop target, including the ones past the mixer column. It is
    // tempting to hide those — the bracket wire stops at the mixer, so they look disconnected — and
    // for a few hours on 2026-08-02 we did. The device says otherwise: a Floor preset with the
    // mixer before column 3 and its two loop blocks moved out to columns 3 and 4 accepted the move,
    // saved it, and both blocks still passed audio. Whatever key 13 means, it is not "signal stops
    // here". Don't act on the drawing. [refuted — `somehinged3_var1.bin` + `somehinged2.log`]
    const cells = allCells.filter(
      (c) => c.column <= maxCol && (showB || c.row === 0),
    );

    // A block on the B row outside the split→mixer bracket. HX Edit cannot draw this layout at all —
    // its path B always spans exactly the bracket — so it is worth marking. But the pedal stores it,
    // saves it, and **plays it**: the tester put a reverb left of the split and heard it fine.
    //
    // It briefly said "no feed" here, on the theory that a cell left of the split has nothing
    // branched into it yet. That was wrong, and the same evening's logs say why: every block he
    // called dead was an envelope filter (Tron Up, Mystery Filter, Autofilter — all sweep on input
    // level), and every block he called merely quiet was a delay or reverb. Split Y sits at Balance
    // 0.5 per leg, so path B runs ~6 dB down and an envelope filter there may never open, wherever
    // it sits. Position was a coincidence; level was the cause. Don't out-guard the pedal.
    // [refuted — 2026-08-03, `fretwire49`/`50` + `Somehinged4_var2.png`.]
    const strandedSide = (c) =>
      !split || c.row !== 1 || !c.occupied
        ? null
        : c.column < d.split_pos
          ? "before"
          : c.column >= d.mixer_pos
            ? "after"
            : null;

    const items = cells.map((c) => {
      const b = bySlot.get(c.slot);
      const name = b ? b.user_label || b.model_name : "";
      const side = strandedSide(c);
      return {
        slot: c.slot,
        occupied: c.occupied,
        x: colX(c.column),
        y: c.row === 1 ? BOT_Y : TOP_Y,
        name,
        symbolicId: b?.symbolic_id ?? null,
        modelName: b?.model_name ?? "",
        // An amp carrying a paired cab is drawn as the full stack, the way it is on stage.
        iconCategory: b?.paired_symbolic_id ? 100 : (b?.category ?? null),
        bypassed: b ? !!b.bypassed : false,
        // Which footswitch toggles this block's bypass, or 0 for unbound. Decoded from the
        // preset's footswitch layout (key `3 → 8`, position + 1) and carried on every block —
        // the editor simply never showed it. Read-only for now; see ROADMAP "Footswitch assign".
        footswitch: b?.footswitch ?? 0,
        color: catColor(b?.category),
        stranded: side,
        strandedWhy:
          side === "before"
            ? "Left of the split, so outside the parallel path. The pedal keeps it and it still plays — HX Edit just can't draw a path B this shape."
            : side === "after"
              ? "Right of the mixer, so outside the parallel path. The pedal keeps it and it still plays."
              : null,
      };
    });

    // Wires. Top spine runs from the input glyph across all top-row columns to the output glyph;
    // the parallel bracket drops at the split column and rises at the mixer column (both drawn in
    // the gap *before* their column, matching the routing).
    const topRight = colX(maxCol) + CW;
    const wires = [`M 20 ${midY(TOP_Y)} H ${topRight + 24}`];
    // The fixed input/output nodes (slots 0 and 9) — clickable to edit gate/threshold/decay and
    // level/pan in the param panel.
    const io = [];
    if (d.input_node) io.push({ slot: d.input_node.slot, x: 4, label: "IN" });
    if (d.output_node) io.push({ slot: d.output_node.slot, x: topRight + 8, label: "OUT" });
    // Serial preset + drag in flight: a dashed ghost of the would-be parallel path under the B row,
    // hinting that a drop there creates the split.
    const ghosts =
      showB && !split
        ? [`M ${colX(2) - (STEP - CW) / 2} ${midY(TOP_Y)} V ${midY(BOT_Y)} H ${topRight} V ${midY(TOP_Y)}`]
        : [];
    let nodes = [];
    let nodeDrops = [];
    // Shown while a split/mixer node drags. The valid gaps are a subset of the visible ones, and
    // with nothing to explain the gap you wanted the absence reads as a rendering bug — the tester
    // who hit it concluded the drop target was being covered by the row below (2026-08-02).
    let nodeHint = null;
    if (split) {
      const xSplit = gapX(d.split_pos);
      const xMixer = gapX(d.mixer_pos);
      const yT = midY(TOP_Y), yB = midY(BOT_Y);
      wires.push(`M ${xSplit} ${yT} V ${yB} H ${xMixer} V ${yT}`);
      // Seat the node glyphs in the vertical gap between the two rows (on the branch wire), where no
      // cell lives — so they never overlap blocks however tight the columns are.
      const yNode = (TOP_Y + CH + BOT_Y) / 2;
      nodes = [
        { kind: "split", x: xSplit, y: yNode, text: "⋔", slot: d.split_node?.slot },
        { kind: "mixer", x: xMixer, y: yNode, text: "⋉", slot: d.mixer_node?.slot },
      ];
      // While a node drags, offer the valid gap positions as drop zones — same range as the
      // backend, which is now only "split stays left of the mixer". The bracket does **not** have
      // to enclose the occupied B row: op 43 moves a loop block out past the mixer and the pedal
      // keeps and plays it, so refusing to *drag a node* into that arrangement was our rule, not
      // the device's, and it cost the tester three attempts in one evening. [2026-08-02]
      if (dragNode) {
        const [lo, hi] =
          dragNode === "split" ? [1, d.mixer_pos - 1] : [d.split_pos + 1, maxCol + 1];
        const cur = dragNode === "split" ? d.split_pos : d.mixer_pos;
        for (let p = lo; p <= hi; p++) {
          if (p !== cur) nodeDrops.push({ pos: p, x: gapX(p) });
        }
        // The only rule left is split-before-mixer, so say what a drop *does* rather than what is
        // forbidden. The two sides are not the same: a loop block left of the split has nothing
        // feeding it, one right of the mixer keeps playing (verified by ear), so only warn about the
        // side that costs you audio — and only when a drop could actually strand something.
        nodeHint =
          `Drop the ${dragNode} on a gap` +
          (dragNode === "split"
            ? " — it has to stay left of the mixer."
            : " — it has to stay right of the split.");
      }
    }

    return {
      items,
      wires,
      ghosts,
      nodes,
      nodeDrops,
      nodeHint,
      io,
      split,
      width: Math.max(colX(maxCol) + CW + 52, 560),
      height: showB ? BOT_Y + CH + 8 : TOP_Y + CH + 8,
    };
  });

  function onDrop(slot) {
    if (dragSrc != null && dragSrc !== slot) onplace?.(dragSrc, slot);
    dragSrc = null;
    dragOver = null;
  }
</script>

<div class="wrap">
  {#if view.nodeHint}<div class="nodehint">{view.nodeHint}</div>{/if}
  <div class="inner" style="width:{view.width}px; height:{view.height}px;">
    <svg class="wires" width={view.width} height={view.height}>
      {#each view.wires as d}<path class="wire" {d} />{/each}
      {#each view.ghosts as d}<path class="wire ghost" {d} />{/each}
    </svg>

    {#each view.io as n (n.slot)}
      <button
        class="ionode"
        class:sel={n.slot === selectedSlot}
        style="left:{n.x}px; top:{midY(TOP_Y) - 13}px;"
        title="Edit {n.label === 'IN' ? 'input (gate)' : 'output (level/pan)'} settings"
        onclick={() => onselect?.(n.slot)}
      >{n.label}</button>
    {/each}

    {#each view.nodes as n (n.kind)}
      <button
        class="node"
        class:sel={n.slot != null && n.slot === selectedSlot}
        draggable="true"
        title={n.kind === "split"
          ? "Split — click to edit its A/B balance, drag to move it"
          : "Mixer — click to edit the A/B levels, pans and polarity, drag to move it"}
        style="left:{n.x - 18}px; top:{n.y - 16}px;"
        ondragstart={(e) => {
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", n.kind);
          e.dataTransfer.setDragImage(e.currentTarget, 18, 16);
          // Deferred one tick — the drop zones appearing is a DOM change, and WebKit can abort a
          // drag whose DOM mutates synchronously inside dragstart (same as the block cells).
          setTimeout(() => (dragNode = n.kind), 0);
        }}
        ondragend={() => {
          dragNode = null;
          nodeOver = null;
        }}
        onclick={() => n.slot != null && onselect?.(n.slot)}
      >{n.text}</button>
    {/each}

    <!-- Valid gap positions for the dragged split/mixer node: slim full-height zones on the
         inter-column gaps the bracket wire can move to. -->
    {#each view.nodeDrops as d (d.pos)}
      <div
        class="gapdrop"
        class:over={nodeOver === d.pos}
        style="left:{d.x - 9}px; height:{view.height - 8}px;"
        role="button"
        tabindex="-1"
        ondragover={(e) => {
          e.preventDefault();
          nodeOver = d.pos;
        }}
        ondragleave={() => {
          if (nodeOver === d.pos) nodeOver = null;
        }}
        ondrop={(e) => {
          e.preventDefault();
          if (dragNode) onmovenode?.(dragNode, d.pos, dsp?.dsp ?? 0);
          dragNode = null;
          nodeOver = null;
        }}
      ></div>
    {/each}

    {#each view.items as c (c.slot)}
    {#if c.occupied}
      <button
        class="cell"
        class:sel={c.slot === selectedSlot}
        class:bypassed={c.bypassed}
        class:stranded={c.stranded != null}
        title={c.strandedWhy ?? c.name}
        class:insb={dragOver === c.slot && dragSide === "l" && dragSrc != null && dragSrc !== c.slot}
        class:insa={dragOver === c.slot && dragSide === "r" && dragSrc != null && dragSrc !== c.slot}
        draggable="true"
        style="left:{c.x}px; top:{c.y}px; width:{CW}px; height:{CH}px; --cat:{c.color};"
        ondragover={(e) => {
          // Occupied cells accept a dragged *block* too — dropping inserts it before/after this
          // block (by which half it's dropped on), shifting neighbors to make room.
          if (dragSrc == null || dragSrc === c.slot) return;
          e.preventDefault();
          const rect = e.currentTarget.getBoundingClientRect();
          dragOver = c.slot;
          dragSide = e.clientX - rect.left < rect.width / 2 ? "l" : "r";
        }}
        ondragleave={() => {
          if (dragOver === c.slot) {
            dragOver = null;
            dragSide = null;
          }
        }}
        ondrop={(e) => {
          e.preventDefault();
          if (dragSrc != null && dragSrc !== c.slot) oninsert?.(dragSrc, c.slot, dragSide === "l");
          dragSrc = null;
          dragOver = null;
          dragSide = null;
        }}
        ondragstart={(e) => {
          e.dataTransfer.effectAllowed = "move";
          // WebKitGTK won't fire `drop` without payload data, and defaults to an oversized drag
          // ghost — set both explicitly.
          e.dataTransfer.setData("text/plain", String(c.slot));
          e.dataTransfer.setDragImage(e.currentTarget, CW / 2, CH / 2);
          // Defer the state flip one tick: setting dragSrc reveals the B row (layout change), and
          // WebKit can abort a drag whose DOM mutates synchronously inside dragstart.
          setTimeout(() => (dragSrc = c.slot), 0);
        }}
        ondragend={() => {
          dragSrc = null;
          dragOver = null;
        }}
        onclick={() => onselect?.(c.slot)}
      >
        <ModelIcon
          symbolicId={c.symbolicId}
          category={c.iconCategory}
          name={c.modelName}
          size={26}
          dim={c.bypassed}
        />
        <span class="text">
          <span class="name">{c.name}</span>
          <span class="slot">slot {c.slot}</span>
        </span>
        {#if c.footswitch > 0}
          <span
            class="fs"
            title="Toggled by footswitch {c.footswitch} on the pedal"
          >FS{c.footswitch}</span>
        {/if}
        {#if c.stranded != null}<span class="unfed">outside path B</span>{/if}
      </button>
    {:else}
      <div
        class="drop"
        class:over={dragOver === c.slot}
        class:active={dragSrc != null}
        style="left:{c.x}px; top:{c.y}px; width:{CW}px; height:{CH}px;"
        role="button"
        tabindex="-1"
        title="Add a block here"
        onclick={() => onaddat?.(c.slot)}
        onkeydown={(e) => e.key === "Enter" && onaddat?.(c.slot)}
        ondragover={(e) => {
          e.preventDefault();
          dragOver = c.slot;
        }}
        ondragleave={() => {
          if (dragOver === c.slot) dragOver = null;
        }}
        ondrop={(e) => {
          e.preventDefault();
          onDrop(c.slot);
        }}
      ><span class="plus">＋</span></div>
    {/if}
    {/each}
  </div>
</div>

<style>
  .wrap {
    border: 1px solid #2a2e37;
    border-radius: 10px;
    background: #12141a;
    overflow-x: auto;
    max-width: 100%;
  }
  .inner {
    position: relative;
    transition: height 140ms ease;
  }
  /* Pinned left: .wrap scrolls horizontally, and a hint that scrolls off is no hint. */
  .nodehint {
    position: sticky;
    left: 0;
    padding: 6px 10px;
    border-bottom: 1px solid #2a2e37;
    font-size: 12px;
    color: #8b93a7;
  }
  .wires {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
  }
  .wire {
    stroke: #566072;
    stroke-width: 2;
    fill: none;
  }
  .wire.ghost {
    stroke: #3a4656;
    stroke-dasharray: 5 5;
  }
  .cell {
    position: absolute;
    display: flex;
    flex-direction: row;
    align-items: center;
    justify-content: flex-start;
    gap: 7px;
    font: inherit;
    background: #232833;
    /* Accent = the block's category color (--cat set inline per cell). */
    border: 1.5px solid var(--cat, #3a4150);
    border-radius: 8px;
    color: #e6e8ec;
    cursor: grab;
    padding: 0 7px;
  }
  .cell:active {
    cursor: grabbing;
  }
  .cell.sel {
    border-color: #f0c245;
    box-shadow: 0 0 0 1px #f0c245;
  }
  .cell.bypassed {
    background: #191b20;
    border-color: #3a3f49;
    border-style: dashed;
  }
  .cell.bypassed .name {
    color: #626a77;
  }
  /* A row-B block outside the split→mixer bracket — a layout HX Edit can't draw. Deliberately a
     muted grey and not amber: the block plays, so this is a note about the drawing, not a fault. */
  .cell.stranded {
    border-style: dashed;
  }
  .cell .unfed {
    position: absolute;
    top: -8px;
    right: -6px;
    padding: 0 4px;
    border-radius: 7px;
    background: #3a4049;
    color: #98a1ae;
    font-size: 9px;
    font-weight: 600;
    white-space: nowrap;
  }
  /* The footswitch this block's bypass sits on. Top-*left*, so it never collides with the
     stranded badge at top-right, and in the accent blue rather than the muted grey those use:
     this is a live binding on the pedal, not a note about our drawing. */
  .cell .fs {
    position: absolute;
    top: -8px;
    left: -6px;
    padding: 0 4px;
    border-radius: 7px;
    background: #24405e;
    color: #9fc4ee;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.02em;
    white-space: nowrap;
  }
  .cell.bypassed .fs {
    opacity: 0.55;
  }
  /* A dragged block hovering another block: an insertion bar on the half it will land on. */
  .cell.insb::before,
  .cell.insa::after {
    content: "";
    position: absolute;
    top: -4px;
    bottom: -4px;
    width: 4px;
    border-radius: 2px;
    background: #3f8ae0;
    box-shadow: 0 0 6px rgba(63, 138, 224, 0.8);
  }
  .cell.insb::before {
    left: -6px;
  }
  .cell.insa::after {
    right: -6px;
  }
  .cell .text {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
  }
  /* Two lines, then ellipsis: with the icon taking the left third, one line clipped almost every
     model name. */
  .cell .name {
    font-weight: 600;
    font-size: 11px;
    line-height: 1.2;
    text-align: left;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    max-width: 100%;
  }
  .cell .slot {
    font-size: 10px;
    color: #8891a0;
  }
  /* Empty grid slots are always faintly visible so the layout reads as a grid you drop into —
     and clickable: like HX Edit, clicking one opens the model picker to add a block there. */
  .drop {
    position: absolute;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1.5px dashed #2b303a;
    border-radius: 8px;
    cursor: pointer;
  }
  .drop .plus {
    color: #3a4150;
    font-size: 15px;
    opacity: 0;
    transition: opacity 80ms ease;
  }
  .drop:hover {
    border-color: #3a4150;
  }
  .drop:hover .plus {
    opacity: 1;
  }
  .drop.active {
    border-color: #3a4656;
    background: rgba(63, 138, 224, 0.06);
  }
  .drop.over {
    border-color: #3f8ae0;
    background: rgba(63, 138, 224, 0.2);
  }
  .node {
    position: absolute;
    width: 36px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    font: inherit;
    background: #2a2333;
    border: 1.5px solid #8a6bd9;
    border-radius: 8px;
    color: #e6e8ec;
    cursor: grab;
  }
  .node:active {
    cursor: grabbing;
  }
  /* Gap drop zones for a dragged split/mixer node: slim vertical slots on the wire gaps. */
  .gapdrop {
    position: absolute;
    top: 4px;
    width: 18px;
    border: 1.5px dashed #6b56a8;
    border-radius: 6px;
    background: rgba(138, 107, 217, 0.08);
  }
  .gapdrop.over {
    border-color: #8a6bd9;
    background: rgba(138, 107, 217, 0.3);
  }
  .node.sel {
    border-color: #f0c245;
    box-shadow: 0 0 0 1px #f0c245;
  }
  /* Fixed input/output nodes at the ends of the signal path. */
  .ionode {
    position: absolute;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    font: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.5px;
    padding: 0 7px;
    background: #1d2530;
    border: 1.5px solid #566072;
    border-radius: 13px;
    color: #9aa3b2;
    cursor: pointer;
  }
  .ionode:hover {
    border-color: #3f8ae0;
    color: #e6e8ec;
  }
  .ionode.sel {
    border-color: #f0c245;
    box-shadow: 0 0 0 1px #f0c245;
    color: #e6e8ec;
  }
</style>
