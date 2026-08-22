<script>
  // The pedal's global EQ, drawn rather than tabulated. Five numbers in a table tell you nothing
  // about the shape they make; a curve tells you at a glance, and this EQ sits across the whole
  // instrument, so being able to see it matters more here than for any per-preset control.
  //
  // **The curve is indicative, not measured.** We know the *parameters* — frequency, Q, gain, and
  // the two cut corners — because they were read off the device. We have never observed the filter
  // *shapes*, so the bands are drawn as textbook analog peaking sections and the cuts as 12 dB/oct
  // Butterworth corners. That is the honest way to render known parameters through an unknown
  // topology, and it is what the caption says.

  import { F_MIN, F_MAX, responseDb, fPos, fFromPos } from "./eqcurve.js";

  let { settings = [], busy = false, onWrite } = $props();

  const at = (id) => settings.find((s) => s.id === id);
  const val = (id, dflt = 0) => {
    const s = at(id);
    return s && typeof s.value === "number" ? s.value : dflt;
  };
  const known = (id) => at(id) != null;

  // Ids, from docs/protocol.md. Mid and high gain/Q are the two the sweep never pinned down; the
  // band renders with the gain it has and says so when it has none.
  const BANDS = [
    { name: "Low", freq: 190, q: 191, gain: 192 },
    { name: "Mid", freq: 193, q: 194, gain: 195 },
    { name: "High", freq: 196, q: 197, gain: 198 },
  ];
  const LOW_CUT = 199;
  const HIGH_CUT = 200;

  const available = $derived(settings.some((s) => s.id >= 190 && s.id <= 200));

  // --- geometry ---------------------------------------------------------------
  const W = 560, H = 190, PAD_L = 34, PAD_R = 10, PAD_T = 12, PAD_B = 22;
  const DB = 15;
  const plotW = W - PAD_L - PAD_R;
  const plotH = H - PAD_T - PAD_B;

  const x = (f) =>
    PAD_L + (Math.log10(Math.max(f, F_MIN) / F_MIN) / Math.log10(F_MAX / F_MIN)) * plotW;
  const y = (db) => PAD_T + ((DB - db) / (2 * DB)) * plotH;

  const GRID_F = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
  const LABEL_F = new Set([100, 1000, 10000]);
  const hz = (f) => (f >= 1000 ? `${f / 1000}k` : String(f));

  // --- response (see eqcurve.js) ---

  const isOff = (id) => {
    const s = at(id);
    return !s || s.off == null ? false : Math.abs(Number(s.value) - s.off) < 0.05;
  };

  const curve = $derived.by(() => {
    if (!available) return "";
    const shape = {
      bands: BANDS.filter((b) => known(b.freq) && known(b.gain)).map((b) => ({
        freq: val(b.freq, 1000),
        q: val(b.q, 0.707) || 0.707,
        gain: val(b.gain, 0),
      })),
      lowCut: known(LOW_CUT) && !isOff(LOW_CUT) ? val(LOW_CUT, F_MIN) : null,
      highCut: known(HIGH_CUT) && !isOff(HIGH_CUT) ? val(HIGH_CUT, F_MAX) : null,
    };
    const pts = [];
    for (let i = 0; i <= 220; i++) {
      const f = F_MIN * Math.pow(F_MAX / F_MIN, i / 220);
      const db = Math.max(-DB, Math.min(DB, responseDb(f, shape)));
      pts.push(`${x(f).toFixed(1)},${y(db).toFixed(1)}`);
    }
    return "M" + pts.join("L");
  });

  // Bands whose gain we have never identified: show the frequency as a marker, and say why there is
  // no handle, rather than drawing a flat band that looks deliberately set to zero.
  const unpinned = $derived(BANDS.filter((b) => known(b.freq) && !known(b.gain)));

  // --- controls ---------------------------------------------------------------
  // Frequency sliders are log-positioned (see eqcurve.js); the pedal's own range for each is
  // unknown, so these span the audible band and the readout stays authoritative.

  // `change`, never `input`: a range fires continuously while dragged, and every one of these is a
  // write to the pedal. One write per gesture.
  const write = (id, v) => {
    if (busy || !known(id)) return;
    const n = Number(v);
    if (Number.isFinite(n)) onWrite?.(id, n);
  };

  const fmt = (v, dp = 1) =>
    Number.isInteger(v) ? String(v) : Number(v).toFixed(dp).replace(/\.?0+$/, "");
</script>

{#if !available}
  <div class="eq-empty">This device did not answer the global EQ ids.</div>
{:else}
  <div class="eq">
    <svg viewBox={`0 0 ${W} ${H}`} class="plot" role="img" aria-label="Global EQ response">
      <defs>
        <linearGradient id="eqfill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stop-color="#5b8def" stop-opacity="0.28" />
          <stop offset="100%" stop-color="#5b8def" stop-opacity="0" />
        </linearGradient>
      </defs>

      <rect x={PAD_L} y={PAD_T} width={plotW} height={plotH} class="bg" />

      {#each GRID_F as f (f)}
        <line x1={x(f)} y1={PAD_T} x2={x(f)} y2={PAD_T + plotH} class="grid" />
        {#if LABEL_F.has(f)}
          <text x={x(f)} y={H - 7} class="tick" text-anchor="middle">{hz(f)}</text>
        {/if}
      {/each}
      {#each [-12, -6, 6, 12] as db (db)}
        <line x1={PAD_L} y1={y(db)} x2={PAD_L + plotW} y2={y(db)} class="grid" />
        <text x={PAD_L - 6} y={y(db) + 3} class="tick" text-anchor="end">{db > 0 ? "+" : ""}{db}</text>
      {/each}
      <line x1={PAD_L} y1={y(0)} x2={PAD_L + plotW} y2={y(0)} class="zero" />
      <text x={PAD_L - 6} y={y(0) + 3} class="tick" text-anchor="end">0</text>

      <path d={`${curve}L${PAD_L + plotW},${y(-DB)}L${PAD_L},${y(-DB)}Z`} class="fill" />
      <path d={curve} class="curve" />

      {#each BANDS as b (b.name)}
        {#if known(b.freq) && known(b.gain)}
          <circle cx={x(val(b.freq, 1000))} cy={y(val(b.gain, 0))} r="5" class="handle" />
          <text x={x(val(b.freq, 1000))} y={y(val(b.gain, 0)) - 10} class="hlabel" text-anchor="middle">
            {b.name}
          </text>
        {/if}
      {/each}
      {#each unpinned as b (b.name)}
        <line x1={x(val(b.freq, 1000))} y1={PAD_T} x2={x(val(b.freq, 1000))} y2={PAD_T + plotH} class="unpinned" />
        <text x={x(val(b.freq, 1000))} y={PAD_T + 11} class="hlabel dim" text-anchor="middle">{b.name}</text>
      {/each}
      {#each [LOW_CUT, HIGH_CUT] as id (id)}
        {#if known(id) && !isOff(id)}
          <line x1={x(val(id, F_MIN))} y1={PAD_T} x2={x(val(id, F_MIN))} y2={PAD_T + plotH} class="cut" />
        {/if}
      {/each}
    </svg>

    <div class="caption">
      Indicative shape: the parameters are read from the pedal, the filter topology is not something
      we have observed.
    </div>

    <div class="rows">
      {#each BANDS as b (b.name)}
        {#if known(b.freq)}
          <div class="band">
            <div class="bname">{b.name}</div>
            <label class="ctl">
              <span>Freq</span>
              <input
                type="range" min="0" max="1" step="0.001" disabled={busy}
                value={fPos(val(b.freq, 1000))}
                onchange={(e) => write(b.freq, Math.round(fFromPos(e.currentTarget.value)))}
              />
              <output>{fmt(val(b.freq, 1000), 0)} Hz</output>
            </label>
            {#if known(b.gain)}
              <label class="ctl">
                <span>Gain</span>
                <input
                  type="range" min="-12" max="12" step="0.1" disabled={busy}
                  value={val(b.gain, 0)}
                  onchange={(e) => write(b.gain, e.currentTarget.value)}
                />
                <output>{fmt(val(b.gain, 0))} dB</output>
              </label>
              <label class="ctl">
                <span>Q</span>
                <input
                  type="range" min="0.1" max="10" step="0.01" disabled={busy}
                  value={val(b.q, 0.707)}
                  onchange={(e) => write(b.q, e.currentTarget.value)}
                />
                <output>{fmt(val(b.q, 0.707), 3)}</output>
              </label>
            {:else}
              <div class="pending">
                Gain and Q not identified — likely ids {b.q}/{b.gain}. Two turns of this band's
                knobs with <code>settings-diff</code> running would settle it.
              </div>
            {/if}
          </div>
        {/if}
      {/each}

      <div class="band cuts">
        <div class="bname">Cuts</div>
        {#each [{ id: LOW_CUT, label: "Low cut" }, { id: HIGH_CUT, label: "High cut" }] as c (c.id)}
          {#if known(c.id)}
            <label class="ctl">
              <span>{c.label}</span>
              <input
                type="range" min="0" max="1" step="0.001" disabled={busy}
                value={fPos(val(c.id, F_MIN))}
                onchange={(e) => write(c.id, Math.round(fFromPos(e.currentTarget.value)))}
              />
              <output class:off={isOff(c.id)}>
                {isOff(c.id) ? "Off" : `${fmt(val(c.id, F_MIN), 0)} Hz`}
              </output>
            </label>
          {/if}
        {/each}
        <div class="pending">
          The cuts have no separate enable: they turn off by parking at their end of the range.
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .eq { padding: 4px 0 8px; }
  .plot { width: 100%; height: auto; display: block; }
  .bg { fill: #12151a; stroke: #2b303a; }
  .grid { stroke: #232830; stroke-width: 1; }
  .zero { stroke: #3a4150; stroke-width: 1; }
  .tick { fill: #6b7280; font-size: 9px; }
  .curve { fill: none; stroke: #5b8def; stroke-width: 2; stroke-linejoin: round; }
  .fill { fill: url(#eqfill); stroke: none; }
  .handle { fill: #5b8def; stroke: #cddcff; stroke-width: 1.5; }
  .hlabel { fill: #c9d1d9; font-size: 9px; }
  .hlabel.dim { fill: #6b7280; }
  .unpinned { stroke: #6b7280; stroke-width: 1; stroke-dasharray: 3 3; }
  .cut { stroke: #b0793f; stroke-width: 1; stroke-dasharray: 2 3; }
  .caption { color: #6b7280; font-size: 11px; margin: 6px 2px 10px; }
  .rows { display: flex; flex-direction: column; gap: 10px; }
  .band {
    border: 1px solid #262b34;
    border-radius: 6px;
    padding: 8px 10px;
    background: #171a20;
  }
  .bname {
    font-size: 11px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: #8b93a3;
    margin-bottom: 5px;
  }
  .ctl {
    display: grid;
    grid-template-columns: 58px 1fr 78px;
    align-items: center;
    gap: 8px;
    font-size: 12px;
    padding: 2px 0;
  }
  .ctl span { color: #8b93a3; }
  .ctl output {
    text-align: right;
    color: #e6e9ef;
    font-variant-numeric: tabular-nums;
  }
  .ctl output.off { color: #8b93a3; }
  input[type="range"] { width: 100%; accent-color: #5b8def; }
  input[type="range"]:disabled { opacity: 0.5; }
  .pending { color: #6b7280; font-size: 11px; margin-top: 4px; }
  code { color: #c9d1d9; }
  .eq-empty { padding: 24px 0; text-align: center; color: #8b93a3; }
</style>
