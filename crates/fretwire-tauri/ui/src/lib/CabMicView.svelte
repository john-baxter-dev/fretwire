<script>
  // The cab's mic placement, drawn: which microphone, how far off the grille, where across the
  // cone, and at what angle. HX Edit shows the same four numbers as a picture, and it is the one
  // place in the editor where a picture genuinely beats the sliders — "2.00 inches, position 4.2"
  // means nothing until you can see that it is a 57 just off the dust cap.
  //
  // It is a *view* of the same parameters the grid below edits, not a second source of truth: it
  // reads them from the same `params` array, writes through the same `onFloat`/`onPreview`
  // callbacks, and the device's re-read is what settles the value. Dragging the mic sets Distance
  // (horizontally) and Position (vertically), and the arrow keys do the same one step at a time.
  // Angle is left to the segmented buttons the grid already gives it — it has two stops, so a
  // second control for it would be clutter; the drawing shows the tilt without offering to set it.
  //
  // No artwork is read or shipped — the mic is generated from `icons/mics.js`, same rule as the
  // model icons. See `docs/icons.md`.
  import { micSpec, micLength } from "./icons/mics.js";

  let {
    params = [],
    paired = false,
    slot,
    onFloat,
    onPreview,
    // ParamPanel owns the format rules (they come off the reference data per param), so borrow its
    // formatter rather than teaching this component the same recipe a second time.
    fmtVal,
  } = $props();

  const byName = (n) => params.find((p) => p.name === n);
  const micP = $derived(byName("Mic"));
  const posP = $derived(byName("Position"));
  const distP = $derived(byName("Distance"));
  const angP = $derived(byName("Angle"));

  // ---- geometry -------------------------------------------------------------------------------
  // A side-on cross-section: the cone opens to the left, the mic stands off it, and the vertical
  // axis doubles as the radial scale the Position parameter runs along.
  const W = 336;
  // Tall enough that a mic angled 45° out at the rim still fits: the tilt turns about the
  // capsule, so the body swings up by most of its own length on top of the position offset.
  const H = 180;
  const CY = 90; // the speaker's axis
  const BAFFLE_X = 232; // front face of the baffle — where Distance is measured from
  const BAFFLE_T = 6; // board thickness, so the front plane reads as a plane
  const CAB_BACK = 280;
  const APEX_X = 266; // where the cone meets the voice coil
  const RIM_R = 50; // cone radius at the mouth
  const APEX_R = 8;
  const MIC_NEAR = 214; // nose position at the minimum distance
  const MIC_FAR = 92; // ...and at the maximum
  const RULE_X = 292; // the radial scale, in the gutter beside the cabinet

  const lerp = (a, b, t) => a + (b - a) * t;
  const clamp01 = (t) => Math.min(1, Math.max(0, t));
  const norm = (p, v) => clamp01((v - (p?.min ?? 0)) / ((p?.max ?? 1) - (p?.min ?? 0) || 1));
  const denorm = (p, t) => lerp(p?.min ?? 0, p?.max ?? 1, clamp01(t));

  /** Where the cone wall sits at radius `r` — the profile everything else is pinned to. */
  const coneX = (r) =>
    lerp(APEX_X, BAFFLE_X, clamp01((r - APEX_R) / (RIM_R - APEX_R)));

  // The dust cap's radius is taken from **Position's own default**, because that default is the
  // classic "just off the cap" placement — 0.23 of the way out on every stock cab. Deriving it
  // means the "Cap edge" tick lands where the reference data says the neutral position is, instead
  // of at a radius this drawing made up. Cabs with no Position (the legacy family) never show it.
  const capR = $derived(RIM_R * clamp01(posP?.default ?? 0.23));

  // ---- live values ----------------------------------------------------------------------------
  // Held locally only while a gesture is in flight; otherwise the device's value is the truth.
  let drag = $state(null); // {dist, pos} in stored units

  const distVal = $derived(drag?.dist ?? distP?.value ?? 1);
  const posVal = $derived(drag?.pos ?? posP?.value ?? 0);
  const angVal = $derived(angP?.value ?? 0);

  const micX = $derived(lerp(MIC_NEAR, MIC_FAR, norm(distP, distVal)));
  // Position runs from the axis out to the rim. Drawn upward: "further out" reads as "up the cone".
  const micR = $derived(posP ? clamp01(posVal / (posP.max || 1)) * (RIM_R - 4) : 0);
  const micY = $derived(CY - micR);

  const spec = $derived(
    micSpec(micP ? micP.enum_labels?.[Math.round(micP.value) - (micP.enum_base ?? 0)] : null),
  );

  // Angle tilts the mic about its capsule, so the capsule — and therefore the spot on the cone the
  // mic sits over — does not move. That is why the aim ray is drawn straight: it marks the radius
  // Position put the mic on, which is a fact about the placement. The tilt is shown as an arc at
  // the capsule instead of a long ray, because projecting a 45° ray across 12 inches lands it
  // several cone-widths off the speaker and reads as a much wilder placement than it is.
  const ARC_R = 22;
  const arc = $derived.by(() => {
    const t = (angVal * Math.PI) / 180;
    return {
      x: micX + ARC_R * Math.cos(t),
      y: micY + ARC_R * Math.sin(t),
      lx: micX + (ARC_R + 17) * Math.cos(t / 2),
      ly: micY + (ARC_R + 17) * Math.sin(t / 2),
    };
  });

  // A short distance leaves no room to centre the label between the ticks, so it steps outside.
  const dimNarrow = $derived(BAFFLE_X - micX < 46);

  // ---- dragging -------------------------------------------------------------------------------
  let svgEl = $state();
  let gesture = null;

  /** Client pixels → viewBox units, so a drag tracks the pointer at any panel width. */
  function toUser(e) {
    const r = svgEl.getBoundingClientRect();
    return { x: ((e.clientX - r.left) / r.width) * W, y: ((e.clientY - r.top) / r.height) * H };
  }

  function down(e) {
    if (!distP?.settable && !posP?.settable) return;
    e.preventDefault();
    const at = toUser(e);
    gesture = { at, dist: distVal, pos: posVal };
    drag = { dist: distVal, pos: posVal };
    e.currentTarget.setPointerCapture(e.pointerId);
  }

  function move(e) {
    if (!gesture) return;
    const at = toUser(e);
    const next = { ...drag };
    if (distP?.settable) {
      // Rightward is closer, so the travel is inverted against the x axis.
      const t = norm(distP, gesture.dist) + (gesture.at.x - at.x) / (MIC_NEAR - MIC_FAR);
      next.dist = denorm(distP, t);
    }
    if (posP?.settable) {
      const t = clamp01(gesture.pos / (posP.max || 1)) + (gesture.at.y - at.y) / (RIM_R - 4);
      next.pos = denorm(posP, clamp01(t));
    }
    // Only stream what actually moved — each preview is a USB write.
    if (next.dist !== drag.dist) onPreview?.(slot, paired, distP.index, next.dist);
    if (next.pos !== drag.pos) onPreview?.(slot, paired, posP.index, next.pos);
    drag = next;
  }

  async function up(e) {
    if (!gesture) return;
    const { dist, pos } = drag;
    const start = gesture;
    gesture = null;
    e.currentTarget.releasePointerCapture?.(e.pointerId);
    // One commit per parameter that actually changed: two history entries is honest, since the
    // gesture really did move two independent values.
    const sends = [];
    if (distP?.settable && dist !== start.dist) sends.push(onFloat(slot, paired, distP.index, dist));
    if (posP?.settable && pos !== start.pos) sends.push(onFloat(slot, paired, posP.index, pos));
    try {
      await Promise.all(sends);
    } finally {
      drag = null; // back to whatever the device says
    }
  }

  /**
   * The drag's keyboard equivalent: left/right walk the mic off and onto the grille, up/down move
   * it out toward the rim and back to the cap. One step per press, straight to a commit — there is
   * no gesture to stream, so previewing would only add a write.
   */
  function key(e) {
    const moves = {
      ArrowLeft: [distP, +1],
      ArrowRight: [distP, -1],
      ArrowUp: [posP, +1],
      ArrowDown: [posP, -1],
    };
    const [p, dir] = moves[e.key] ?? [];
    if (!p?.settable) return;
    e.preventDefault();
    const step = p.step ?? ((p.max ?? 1) - (p.min ?? 0)) / 20;
    const cur = p === distP ? distVal : posVal;
    const v = Math.min(p.max ?? 1, Math.max(p.min ?? 0, cur + dir * step));
    if (v !== cur) onFloat(slot, paired, p.index, v);
  }

  const micName = $derived(spec?.label ?? "Mic");
  const readout = $derived(
    [
      distP && `${fmtVal(distP, distVal)} from the grille`,
      posP && `position ${fmtVal(posP, posVal)}`,
      angP && `${fmtVal(angP, angVal)}`,
    ]
      .filter(Boolean)
      .join(" · "),
  );
</script>

{#if distP}
  <div class="cabmic">
    <svg
      bind:this={svgEl}
      viewBox="0 0 {W} {H}"
      role="img"
      aria-label="{micName}, {readout}"
    >
      <!-- The cabinet in section: box, then the baffle board with the driver's hole cut in it,
           then the cone falling back to the voice coil and the motor behind it. Drawn in that
           order because it is the baffle's front face that Distance is measured from, and a plane
           you can see is worth more here than one you have to infer. -->
      <rect class="box" x={BAFFLE_X} y="6" width={CAB_BACK - BAFFLE_X} height={H - 12} rx="3" />
      <rect class="baffle" x={BAFFLE_X} y="6" width={BAFFLE_T} height={CY - RIM_R - 6} />
      <rect class="baffle" x={BAFFLE_X} y={CY + RIM_R} width={BAFFLE_T} height={H - 6 - CY - RIM_R} />
      <rect class="motor" x={APEX_X} y={CY - 15} width={CAB_BACK - APEX_X - 4} height="30" rx="2" />
      <!-- cone section: mouth at the baffle, apex at the voice coil -->
      <path
        class="cone"
        d="M {BAFFLE_X},{CY - RIM_R} L {APEX_X},{CY - APEX_R} L {APEX_X},{CY + APEX_R} L {BAFFLE_X},{CY + RIM_R} Z"
      />
      <!-- surround: the roll at the rim, which is where "Edge" is -->
      <path class="surround" d="M {BAFFLE_X},{CY - RIM_R} a 5,5 0 0 0 0,-9" />
      <path class="surround" d="M {BAFFLE_X},{CY + RIM_R} a 5,5 0 0 1 0,9" />
      <!-- dust cap, bulging toward the mic -->
      {#if posP}
        <path
          class="cap"
          d="M {coneX(capR)},{CY - capR} Q {coneX(capR) - 14},{CY} {coneX(capR)},{CY + capR} Z"
        />
      {/if}

      <!-- the radial scale: what Position is measured against -->
      <g class="scale">
        <line x1={RULE_X} y1={CY - RIM_R} x2={RULE_X} y2={CY} />
        {#each [{ r: RIM_R, t: "Edge" }, ...(posP ? [{ r: capR, t: "Cap edge" }] : []), { r: 0, t: "Center" }] as m}
          <line class="lead" x1={coneX(m.r)} y1={CY - m.r} x2={RULE_X} y2={CY - m.r} />
          <line class="tick" x1={RULE_X - 4} y1={CY - m.r} x2={RULE_X + 4} y2={CY - m.r} />
          <text x={RULE_X + 8} y={CY - m.r + 3}>{m.t}</text>
        {/each}
      </g>

      <!-- the radius the mic sits on, and where that meets the cone -->
      <line class="aim" x1={micX} y1={micY} x2={BAFFLE_X} y2={micY} />
      <circle class="aimdot" cx={coneX(micR)} cy={micY} r="2.5" />
      <!-- tilt, as the angle between the mic's axis and the speaker's -->
      {#if angVal > 0.5}
        <g class="arc">
          <line x1={micX} y1={micY} x2={arc.x} y2={arc.y} />
          <path d="M {micX + ARC_R},{micY} A {ARC_R},{ARC_R} 0 0 1 {arc.x},{arc.y}" />
          <text x={arc.lx} y={arc.ly + 3} text-anchor="middle">{fmtVal(angP, angVal)}</text>
        </g>
      {/if}

      <!-- distance dimension -->
      <g class="dim">
        <line x1={micX} y1={H - 20} x2={BAFFLE_X} y2={H - 20} />
        <line class="tick" x1={micX} y1={H - 25} x2={micX} y2={H - 15} />
        <line class="tick" x1={BAFFLE_X} y1={H - 25} x2={BAFFLE_X} y2={H - 15} />
        <text
          x={dimNarrow ? micX - 6 : (micX + BAFFLE_X) / 2}
          y={dimNarrow ? H - 17 : H - 24}
          text-anchor={dimNarrow ? "end" : "middle"}>{fmtVal(distP, distVal)}</text>
      </g>

      <!-- the mic itself: drawn pointing +x from the capsule, then tilted about it -->
      {#if spec}
        <g
          class="mic"
          class:grabbable={distP?.settable || posP?.settable}
          transform="translate({micX},{micY}) rotate({angVal})"
          role="button"
          tabindex="0"
          aria-label="{micName} placement — {readout}"
          onpointerdown={down}
          onpointermove={move}
          onpointerup={up}
          onpointercancel={up}
          onkeydown={key}
        >
          <title>{micName} — drag to move it across the cone and off the grille</title>
          <!-- grab target: a little wider than the silhouette, so thin mics stay easy to catch -->
          <rect
            class="hit"
            x={-micLength(spec)}
            y={-spec.headR - 5}
            width={micLength(spec) + 6}
            height={spec.headR * 2 + 10}
          />
          <!-- body -->
          <rect
            class="body"
            x={-spec.headLen - spec.bodyLen}
            y={-spec.bodyR}
            width={spec.bodyLen}
            height={spec.bodyR * 2}
            rx={spec.bodyR * 0.5}
            fill={spec.body}
          />
          <!-- head -->
          {#if spec.head === "ball"}
            <rect
              x={-spec.headLen}
              y={-spec.headR}
              width={spec.headLen}
              height={spec.headR * 2}
              rx={spec.headR}
              fill={spec.accent}
            />
          {:else if spec.head === "barrel"}
            <rect
              x={-spec.headLen}
              y={-spec.headR}
              width={spec.headLen}
              height={spec.headR * 2}
              rx="2"
              fill={spec.accent}
            />
          {:else if spec.head === "capsule"}
            <rect
              x={-spec.headLen}
              y={-spec.headR}
              width={spec.headLen}
              height={spec.headR * 2}
              rx={spec.headLen * 0.45}
              fill={spec.accent}
            />
          {:else if spec.head === "slab"}
            <rect
              x={-spec.headLen}
              y={-spec.headR}
              width={spec.headLen * 0.55}
              height={spec.headR * 2}
              rx="2.5"
              fill={spec.accent}
            />
          {:else}
            <!-- bottle: a sphere on a shoulder -->
            <circle cx={-spec.headR * 0.9} cy="0" r={spec.headR} fill={spec.accent} />
          {/if}
          <!-- collar, and the cable stub that says which end is the back -->
          <rect
            class="collar"
            x={-spec.headLen - 3}
            y={-spec.bodyR - 1}
            width="3.5"
            height={spec.bodyR * 2 + 2}
          />
          <line
            class="cable"
            x1={-spec.headLen - spec.bodyLen}
            y1="0"
            x2={-micLength(spec) - 6}
            y2="0"
          />
        </g>

      {/if}
    </svg>
    <div class="caption">{micName}{readout ? ` — ${readout}` : ""}</div>
  </div>
{/if}

<style>
  /* Sized as a flex item, because the panel puts it beside the param grid: it takes ~340px when
     that fits and wraps under the params when it doesn't. The basis is what the drawing needs to
     stay readable — much under 280px and the scale labels start colliding. */
  .cabmic {
    flex: 0 1 340px;
    min-width: 280px;
    /* Centres itself only once it has wrapped onto its own row: while it shares the row the param
       grid's `flex-grow` has already eaten the free space, so there is none for the auto margins
       to take and it stays snug against the params. */
    margin: 2px auto 4px;
  }
  svg {
    display: block;
    /* Fills whatever the flex basis settles on — it is an illustration, not a canvas, and the
       column it sits in is already the cap on how big it gets. */
    width: 100%;
    height: auto;
    touch-action: none;
    user-select: none;
  }
  .box {
    fill: #1f242b;
    stroke: #3a4150;
  }
  .baffle,
  .motor {
    fill: #3a4150;
    stroke: #4a5262;
  }
  .cone {
    fill: #30363f;
    stroke: #3a4150;
    stroke-width: 1;
  }
  .surround {
    fill: none;
    stroke: #3a4150;
    stroke-width: 2.5;
    stroke-linecap: round;
  }
  .cap {
    fill: #4a5262;
    stroke: #5a6474;
  }
  .scale line {
    stroke: #3a4150;
  }
  .scale .lead {
    stroke-dasharray: 2 3;
    opacity: 0.55;
  }
  .scale text {
    fill: #9aa3b2;
    font-size: 7.5px;
  }
  .aim {
    stroke: #3f8ae0;
    stroke-width: 1;
    stroke-dasharray: 3 3;
    opacity: 0.8;
  }
  .aimdot {
    fill: #3f8ae0;
    opacity: 0.8;
  }
  .dim line {
    stroke: #9aa3b2;
    opacity: 0.7;
  }
  .dim text {
    fill: #c3c9d4;
    font-size: 9px;
  }
  .mic .hit {
    fill: transparent;
  }
  .mic.grabbable {
    cursor: grab;
  }
  .mic .collar {
    fill: #3a4150;
  }
  .mic .cable {
    stroke: #3a4150;
    stroke-width: 2;
    stroke-linecap: round;
  }
  .arc line,
  .arc path {
    fill: none;
    stroke: #3f8ae0;
    stroke-width: 1;
    opacity: 0.9;
  }
  .arc text {
    fill: #9aa3b2;
    font-size: 8px;
  }
  .caption {
    font-size: 11px;
    color: #9aa3b2;
    text-align: center;
  }
</style>
