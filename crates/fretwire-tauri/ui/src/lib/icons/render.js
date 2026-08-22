// Draws a model icon from its spec (see `spec.js`) as SVG markup, on a 32x32 grid.
//
// Kept as a plain string builder rather than Svelte markup for two reasons: the same code renders
// the contact sheet used to review the icon set, and a block cell that redraws on every drag frame
// is cheaper as one `{@html}` blob than ~20 reactive nodes.
//
// The vocabulary is deliberately small — an enclosure silhouette, a knob layout, a footswitch, a
// couple of faceplate marks. That is enough to tell a gold 3-knob overdrive from a big silver fuzz
// from a 4x12 cab at 22px, which is the whole job. Nothing here reproduces anyone's artwork: no
// logos, no lettering, no trade dress — just the shape-and-colour cue a player already carries in
// their head.

const N = (v) => Math.round(v * 100) / 100;

/** Relative luminance, to decide whether knobs/marks go dark-on-light or light-on-dark. */
function lum(hex) {
  const h = hex.replace("#", "");
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16) / 255);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}
const isLight = (hex) => lum(hex) > 0.45;

/** A darker/lighter sibling of `hex` — used for shadow edges and grille cloth. */
function shade(hex, amt) {
  const h = hex.replace("#", "");
  const p = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
  return (
    "#" +
    p
      .map((v) => Math.max(0, Math.min(255, Math.round(v + 255 * amt))).toString(16).padStart(2, "0"))
      .join("")
  );
}

const OUTLINE = "#0c0e11";

// ---- primitives ----------------------------------------------------------

const rect = (x, y, w, h, fill, rx = 1.5, extra = "") =>
  `<rect x="${N(x)}" y="${N(y)}" width="${N(w)}" height="${N(h)}" rx="${rx}" fill="${fill}"${extra}/>`;

const circle = (cx, cy, r, fill, extra = "") =>
  `<circle cx="${N(cx)}" cy="${N(cy)}" r="${N(r)}" fill="${fill}"${extra}/>`;

const line = (x1, y1, x2, y2, stroke, w = 1) =>
  `<path d="M${N(x1)} ${N(y1)}L${N(x2)} ${N(y2)}" stroke="${stroke}" stroke-width="${w}" stroke-linecap="round"/>`;

/**
 * One control knob: a disc with a pointer line at `angle` (degrees, 0 = up). The pointer is what
 * makes a knob read as a knob at 20px — a bare disc looks like a screw.
 */
function knob(cx, cy, r, body, angle = -30) {
  const dark = isLight(body);
  const cap = dark ? "#2a2d33" : "#d7dbe1";
  const point = dark ? "#e8ebef" : "#22252a";
  const a = ((angle - 90) * Math.PI) / 180;
  return (
    circle(cx, cy, r, cap) +
    line(cx, cy, cx + Math.cos(a) * r * 0.85, cy + Math.sin(a) * r * 0.85, point, r * 0.42)
  );
}

/**
 * `n` knobs laid out across the enclosure. `arc` lifts the middle of a 3-knob row (the Klon/Timmy
 * triangle); `grid` stacks them 2-up (Big Muff, multi-band); `row` is the plain Boss line.
 */
function knobRow(x0, x1, y, n, body, layout = "row") {
  if (n <= 0) return "";
  const r = n >= 5 ? 1.5 : n === 4 ? 1.75 : 2.15;
  const span = x1 - x0;
  const put = (i, count, yy, rr = r) => {
    const cx = count === 1 ? (x0 + x1) / 2 : x0 + (span * i) / (count - 1);
    return knob(cx, yy, rr, body, -40 + (i * 80) / Math.max(1, count - 1));
  };
  if (layout === "arc" && n === 3) {
    return knob(x0, y + 1.2, r, body, -55) + knob((x0 + x1) / 2, y - 1.4, r, body, 0) + knob(x1, y + 1.2, r, body, 55);
  }
  if (layout === "grid" && n >= 4) {
    const top = Math.ceil(n / 2);
    const bot = n - top;
    let s = "";
    for (let i = 0; i < top; i++) s += put(i, top, y - 2.1, 1.7);
    for (let i = 0; i < bot; i++) s += put(i, bot, y + 2.6, 1.7);
    return s;
  }
  let s = "";
  for (let i = 0; i < n; i++) s += put(i, n, y);
  return s;
}

/** Vertical slider bank — the graphic EQ tell. */
function sliders(x0, x1, y0, y1, n, body) {
  const dark = isLight(body);
  const track = dark ? "#3a3e45" : "#c4c9d1";
  const cap = dark ? "#1d1f24" : "#eef1f4";
  let s = "";
  for (let i = 0; i < n; i++) {
    const x = x0 + ((x1 - x0) * i) / (n - 1);
    // A shallow smile so the bank reads as a curve, not a picket fence.
    const t = (i / (n - 1)) * 2 - 1;
    const cy = (y0 + y1) / 2 + t * t * 2.6 - 1.3;
    s += line(x, y0, x, y1, track, 0.7) + rect(x - 0.9, cy - 0.55, 1.8, 1.1, cap, 0.4);
  }
  return s;
}

/** The stomp switch: a chrome button in a hex nut. */
function footswitch(cx, cy, r = 2.6) {
  return circle(cx, cy, r + 0.9, "#3c4149") + circle(cx, cy, r, "#b9c0c9") + circle(cx, cy - r * 0.3, r * 0.5, "#e3e8ee");
}

const led = (cx, cy, color) =>
  circle(cx, cy, 1.15, color) + circle(cx, cy, 2.1, color, ` opacity="0.28"`);

/** Grille cloth: fine diagonal weave, clipped to the given box by a pattern-free hatch. */
function cloth(x, y, w, h, base) {
  let s = rect(x, y, w, h, base, 1);
  const hatch = shade(base, isLight(base) ? -0.09 : 0.09);
  for (let i = -h; i < w; i += 2.2) {
    const x1 = Math.max(x, x + i);
    const y1 = y + Math.max(0, -i);
    const x2 = Math.min(x + w, x + i + h);
    const y2 = y1 + (x2 - x1);
    if (x2 > x1) s += line(x1, y1, x2, Math.min(y + h, y2), hatch, 0.6);
  }
  return s;
}

// ---- enclosure shapes ----------------------------------------------------
//
// Every shape returns the finished icon body. `s` is the resolved spec.

function drawStomp(s, geom) {
  const { x, w } = geom;
  const y = 3.5;
  const h = 25;
  const body = s.body;
  const top = shade(body, 0.1);
  let out = rect(x - 0.5, y - 0.5, w + 1, h + 1, OUTLINE, 2.4, ` opacity="0.55"`);
  out += rect(x, y, w, h, body, 2);
  out += rect(x, y, w, 2.4, top, 2); // lit top edge

  // Faceplate inset (the two-tone pedals: RAT, OCD, Klon).
  if (s.plate) out += rect(x + 1.6, y + 1.6, w - 3.2, h - 9.5, s.plate, 1);

  const kx0 = x + 3.2;
  const kx1 = x + w - 3.2;
  out += knobRow(kx0, kx1, y + 5.6, s.knobs ?? 3, s.plate ?? body, s.layout);

  if (s.mark === "window") out += rect(x + w / 2 - 3.4, y + 10.5, 6.8, 3.2, "#161a20", 0.6);
  if (s.mark === "stripe") out += rect(x, y + 11.5, w, 1.8, shade(body, isLight(body) ? -0.22 : 0.22), 0);
  if (s.mark === "bars")
    out += line(x + 3, y + 12, x + w - 3, y + 12, shade(body, isLight(body) ? -0.2 : 0.2), 1) +
      line(x + 3, y + 14, x + w - 3, y + 14, shade(body, isLight(body) ? -0.2 : 0.2), 1);

  out += led(x + w / 2, y + 2.9, s.led ?? "#ff4d4d");

  const sw = s.sw ?? 1;
  if (sw === 1) out += footswitch(x + w / 2, y + h - 4.4);
  else if (sw === 2) out += footswitch(x + w * 0.3, y + h - 4.4, 2.2) + footswitch(x + w * 0.7, y + h - 4.4, 2.2);
  else if (sw >= 3) {
    for (let i = 0; i < Math.min(sw, 4); i++)
      out += footswitch(x + 2.6 + ((w - 5.2) * i) / (Math.min(sw, 4) - 1), y + h - 4, 1.9);
  }
  return out;
}

function drawRound(s) {
  // Fuzz Face: a hemisphere on three feet, two knobs, switch low centre.
  const body = s.body;
  let out = circle(16, 16.5, 12.6, OUTLINE, ` opacity="0.55"`);
  out += circle(16, 16.5, 12.1, body);
  out += `<path d="M4.2 14.5A12.1 12.1 0 0 1 27.8 14.5Z" fill="${shade(body, 0.09)}"/>`;
  out += knob(11, 12.6, 2.3, body, -45) + knob(21, 12.6, 2.3, body, 45);
  out += led(16, 9.6, s.led ?? "#ff4d4d");
  out += footswitch(16, 23);
  return out;
}

function drawWedge(s) {
  // Sloped-top box (treble boosters, the wedge fuzzes) — top-mounted control, no faceplate.
  const body = s.body;
  let out = `<path d="M7 29.5L25 29.5L25 9.5L7 15.5Z" fill="${OUTLINE}" opacity="0.55"/>`;
  out += `<path d="M7.5 29L24.5 29L24.5 10L7.5 15.8Z" fill="${body}"/>`;
  out += `<path d="M7.5 15.8L24.5 10L24.5 12.4L7.5 18.1Z" fill="${shade(body, 0.12)}"/>`;
  out += knobRow(11, 21, 20, s.knobs ?? 1, body, "row");
  out += footswitch(16, 26, 2.2);
  return out;
}

function drawWah(s) {
  // Treadle in side profile: the one silhouette nobody mistakes for anything else. `teardrop`
  // rounds the heel, which is exactly what separates the teardrop wahs from the boxy ones.
  const body = s.body;
  if (s.teardrop) {
    let t = `<path d="M4 25.4Q4 20.2 9.4 17.4L27 9.6L28.4 12.6L9.6 21.8Q6.6 23.4 6.6 25.4Z" fill="${OUTLINE}" opacity="0.5"/>`;
    t += `<path d="M4.6 25L27.6 25L27.6 21.6L8 12.8Q4.6 16.4 4.6 21Z" fill="${shade(body, -0.12)}"/>`;
    t += `<path d="M8 12.8L27.6 21.6L27.6 18.8L9.4 10.4Q8.4 11.4 8 12.8Z" fill="${body}"/>`;
    t += rect(4.6, 24.8, 23, 2.6, "#575e69", 0.9);
    return t;
  }
  let out = `<path d="M3.5 25.5L28.5 25.5L28.5 21L6.5 11.5Z" fill="${OUTLINE}" opacity="0.5"/>`;
  out += `<path d="M4 25L28 25L28 21.4L7 12.2Z" fill="${shade(body, -0.12)}"/>`;
  out += `<path d="M7 12.2L28 21.4L28 18.6L8.4 9.8Z" fill="${body}"/>`; // treadle plate
  // Tread ribs.
  for (let i = 0; i < 5; i++) {
    const t = 0.12 + i * 0.19;
    out += line(8.4 + (19.6 * t) - 1.1, 10.4 + (8.8 * t), 8.4 + 19.6 * t, 12.6 + 8.8 * t, shade(body, 0.16), 0.9);
  }
  out += rect(4, 24.8, 24, 2.6, "#575e69", 0.9);
  return out;
}

/** Amp + cab: the head sitting on its stack, for the paired Amp+Cab blocks. */
function drawStack(s) {
  const body = s.body;
  const panel = s.panel ?? "#c9ced6";
  let out = rect(4.5, 12.5, 23, 16, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(5, 13, 22, 15, body, 1.6);
  out += cloth(6.4, 14.4, 19.2, 12.2, s.cloth ?? shade(body, isLight(body) ? -0.13 : 0.15));
  out += speakerGrid(6.4, 14.4, 19.2, 12.2, 2, 2, body);
  out += rect(3.5, 3.5, 25, 9, OUTLINE, 1.6, ` opacity="0.5"`);
  out += rect(4, 4, 24, 8, body, 1.4);
  out += rect(5.4, 5.2, 21.2, 3.4, panel, 0.7);
  out += knobRow(7.4, 24.6, 6.9, Math.min(s.knobs ?? 6, 6), panel, "row");
  return out;
}

function drawHead(s) {
  if (s.stack) return drawStack(s);
  // Amp head: cabinet, control panel, grille. The panel finish is the whole tell — tweed with a
  // brown panel, black with plexi gold, black with brushed silver.
  const body = s.body;
  const panel = s.panel ?? "#c9ced6";
  let out = rect(2, 6.5, 28, 19, OUTLINE, 2.2, ` opacity="0.5"`);
  out += rect(2.5, 7, 27, 18, body, 2);
  out += rect(5.6, 8.8, 20.8, 5.4, panel, 0.9);
  out += knobRow(7.6, 24.4, 11.5, s.knobs ?? 6, panel, "row");
  if (s.jump) out += line(7.6, 13.6, 10.6, 13.6, "#8b93a2", 0.9);
  out += cloth(4.2, 15.4, 23.6, 7.8, s.cloth ?? shade(body, isLight(body) ? -0.14 : 0.16));
  // Corner protectors — the detail that stops a head reading as a plain box.
  const cap = shade(body, isLight(body) ? -0.3 : 0.3);
  for (const [cx, cy] of [[3.9, 8.9], [28.1, 8.9], [3.9, 23.1], [28.1, 23.1]])
    out += rect(cx - 1.3, cy - 1.3, 2.6, 2.6, cap, 0.7);
  out += line(6.5, 25, 6.5, 26.6, "#15171b", 1.6) + line(25.5, 25, 25.5, 26.6, "#15171b", 1.6);
  out += led(28.2, 16, s.led ?? "#ff4d4d");
  return out;
}

function drawCombo(s) {
  const body = s.body;
  const panel = s.panel ?? "#c9ced6";
  let out = rect(4.5, 4.5, 23, 24, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(5, 5, 22, 23, body, 1.8);
  out += rect(6.4, 6.2, 19.2, 3.4, panel, 0.7);
  out += knobRow(8.4, 23.6, 7.9, Math.min(s.knobs ?? 4, 6), panel, "row");
  out += cloth(6.6, 10.8, 18.8, 15.4, s.cloth ?? shade(body, isLight(body) ? -0.14 : 0.16));
  if (s.speakers) {
    const [c, r] = s.speakers;
    out += speakerGrid(6.6, 10.8, 18.8, 15.4, c, r, body);
  }
  return out;
}

/** The driver array — `4x12` really does read as four circles in a square. */
function speakerGrid(x, y, w, h, cols, rows, body) {
  let out = "";
  const cw = w / cols;
  const ch = h / rows;
  const r = Math.min(cw, ch) / 2 - 0.55;
  for (let i = 0; i < cols; i++)
    for (let j = 0; j < rows; j++) {
      const cx = x + cw * (i + 0.5);
      const cy = y + ch * (j + 0.5);
      out += circle(cx, cy, r, shade(body, -0.16)) + circle(cx, cy, r * 0.62, shade(body, 0.06), ` opacity="0.5"`) + circle(cx, cy, r * 0.24, "#15171b");
    }
  return out;
}

function drawCab(s) {
  const body = s.body ?? "#3a3128";
  const [cols, rows] = s.speakers ?? [1, 1];
  // A mic'd cab gives up a slice of the right edge to the mic, so the two never look alike.
  const w = s.mic ? 19.5 : 24;
  let out = rect(3.5, 4.5, w + 1, 23, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(4, 5, w, 22, body, 1.8);
  const gw = w - 3.2;
  out += cloth(5.6, 6.6, gw, 18.8, s.cloth ?? shade(body, isLight(body) ? -0.13 : 0.15));
  out += speakerGrid(5.6, 6.6, gw, 18.8, cols, rows, body);
  if (s.mic) {
    // A dynamic mic on a boom, pointed at the grille — how the block is actually used.
    out += line(27.2, 17.5, 27.2, 27, "#454b55", 1.2) + rect(25.2, 26.6, 4, 1.3, "#454b55", 0.6);
    out += `<path d="M24.4 12.8A2.8 2.8 0 0 1 30 12.8L30 15.2A2.8 2.8 0 0 1 24.4 15.2Z" fill="#8d939c"/>`;
    out += `<path d="M24.4 12.8A2.8 2.8 0 0 1 30 12.8A2.8 2.8 0 0 1 24.4 12.8Z" fill="#c3c9d1"/>`;
    out += circle(27.2, 15.4, 1.3, "#5b626c") + line(27.2, 16.4, 27.2, 18, "#454b55", 1.2);
  }
  return out;
}

function drawRack(s) {
  // 1U studio box — the compressors, EQs and reverbs that were never pedals.
  const body = s.body;
  let out = rect(1.5, 9.5, 29, 13, OUTLINE, 1.6, ` opacity="0.5"`);
  out += rect(2, 10, 28, 12, body, 1.4);
  for (const cx of [3.6, 28.4]) out += circle(cx, 13, 0.75, "#9aa3b2") + circle(cx, 19, 0.75, "#9aa3b2");
  if (s.sliders) {
    out += sliders(6.5, 25.5, 12.4, 19.6, s.sliders, body);
    return out;
  }
  let kx0 = 6;
  if (s.vu) {
    out += rect(5.4, 12.4, 7.5, 7.2, "#e8e2cc", 0.8);
    out += `<path d="M6.6 18.4Q9.2 13.4 11.9 15.8" stroke="#2a2d33" stroke-width="0.7" fill="none"/>`;
    kx0 = 16;
  } else if (s.glyph) {
    out += rect(5, 12.2, 8.6, 7.6, "#14171c", 0.8);
    out += displayGlyph(s.glyph, 5, 12.2, 8.6, 7.6);
    kx0 = 16.5;
  }
  out += knobRow(kx0, 26.5, 16, s.knobs ?? 4, body, s.layout ?? "row");
  return out;
}

/**
 * The lit window on a rack unit. The algorithmic reverbs have no hardware to look like, so the
 * glyph is what separates a hall from a room from a shimmer — an arch, a box, a rising sparkle.
 */
function displayGlyph(kind, x, y, w, h) {
  const cx = x + w / 2;
  const cy = y + h / 2;
  const g = "#7fe0c8";
  const st = (d, sw = 0.8) => `<path d="${d}" stroke="${g}" stroke-width="${sw}" fill="none" stroke-linecap="round"/>`;
  switch (kind) {
    case "arch":
      return st(`M${x + 1.4} ${y + h - 1.4}L${x + 1.4} ${cy - 0.4}A${w / 2 - 1.4} ${w / 2 - 1.4} 0 0 1 ${x + w - 1.4} ${cy - 0.4}L${x + w - 1.4} ${y + h - 1.4}`);
    case "room":
      return st(`M${x + 1.6} ${y + h - 1.6}L${x + 1.6} ${y + 1.8}L${x + w - 1.6} ${y + 1.8}L${x + w - 1.6} ${y + h - 1.6}Z`);
    case "cave":
      return st(`M${x + 1.3} ${y + h - 1.4}L${x + 2.8} ${cy - 1}L${cx} ${y + 1.6}L${x + w - 2.8} ${cy - 0.4}L${x + w - 1.3} ${y + h - 1.4}`);
    case "plate":
      return st(`M${x + 1.5} ${y + 2.2}L${x + w - 1.5} ${y + 2.2}M${x + 1.5} ${cy}L${x + w - 1.5} ${cy}M${x + 1.5} ${y + h - 2.2}L${x + w - 1.5} ${y + h - 2.2}`, 0.75);
    case "shimmer":
      return st(`M${x + 1.5} ${y + h - 1.8}L${cx - 0.6} ${cy}L${x + w - 1.5} ${y + 1.8}`) +
        circle(x + w - 2.2, y + 2.2, 0.8, g);
    case "gate":
      return st(`M${x + 1.5} ${y + h - 1.8}L${x + 3} ${y + 1.8}L${x + w - 3.4} ${y + 1.8}L${x + w - 3.4} ${y + h - 1.8}`);
    case "echo":
      return st(`M${x + 2} ${y + 2}L${x + 2} ${y + h - 2}M${cx} ${y + 3.4}L${cx} ${y + h - 3.4}M${x + w - 2} ${y + 4.6}L${x + w - 2} ${y + h - 4.6}`, 0.9);
    case "particle":
      return [
        [2.2, 2.4],
        [5, 5],
        [3.4, 6.6],
        [6.6, 2.2],
        [7.2, 5.6],
      ]
        .map(([dx, dy]) => circle(x + dx, y + dy, 0.62, g))
        .join("");
    case "wave":
    default:
      return st(`M${x + 1.4} ${cy}Q${x + w * 0.3} ${y + 1.6} ${cx} ${cy}T${x + w - 1.4} ${cy}`);
  }
}

function drawReel(s) {
  // Tape echo: two reels behind a lid. Reads instantly and separates tape from digital delay.
  const body = s.body;
  let out = rect(2.5, 7.5, 27, 17, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(3, 8, 26, 16, body, 1.8);
  for (const cx of [10, 22]) {
    out += circle(cx, 14.5, 4.6, shade(body, -0.18)) + circle(cx, 14.5, 4.1, "#2b2f36");
    out += circle(cx, 14.5, 1.5, shade(body, 0.2)) + circle(cx, 14.5, 0.6, "#12141a");
  }
  out += line(10, 19.6, 22, 19.6, "#1a1d22", 0.9); // tape path
  out += knobRow(7, 25, 21.6, s.knobs ?? 3, body, "row");
  return out;
}

function drawMic(s) {
  const body = s.body ?? "#8d939c";
  let out = circle(16, 11, 6.4, OUTLINE, ` opacity="0.5"`);
  out += circle(16, 11, 6, body);
  out += cloth(11.4, 6.6, 9.2, 8.8, shade(body, -0.14));
  out += circle(16, 11, 5.9, "none", ` stroke="${shade(body, 0.18)}" stroke-width="0.8"`);
  out += rect(14.4, 16.4, 3.2, 6.4, shade(body, -0.1), 0.8);
  out += rect(11.6, 22.6, 8.8, 2, "#2c3038", 0.8);
  return out;
}

function drawUtil(s) {
  // Jacks and a signal arrow: sends, returns, loops, volume, the routing utilities.
  const body = s.body ?? "#4a515c";
  let out = rect(3.5, 8.5, 25, 15, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(4, 9, 24, 14, body, 1.8);
  const jacks = s.jacks ?? 2;
  for (let i = 0; i < jacks; i++) {
    const cx = 9 + i * (14 / Math.max(1, jacks - 1 || 1));
    out += circle(cx, 16, 3.1, shade(body, -0.22)) + circle(cx, 16, 1.9, "#15171b") + circle(cx, 16, 0.8, shade(body, 0.25));
  }
  if (s.arrow)
    out += `<path d="M19.5 16L25 16M22.8 13.4L25.4 16L22.8 18.6" stroke="#dfe4ea" stroke-width="1.1" fill="none" stroke-linecap="round" stroke-linejoin="round"/>`;
  return out;
}

function drawPedalboard(s) {
  // Volume/expression pedal — the same treadle, but smooth-topped and with the travel arc drawn
  // above it, so it never reads as a wah.
  const body = s.body ?? "#4a515c";
  let out = `<path d="M3.5 25.4L28.5 25.4L28.5 20.8L6.5 11Z" fill="${OUTLINE}" opacity="0.5"/>`;
  out += `<path d="M4 25L28 25L28 21.4L7 12.2Z" fill="${shade(body, -0.14)}"/>`;
  out += `<path d="M7 12.2L28 21.4L28 18.4L8.6 9.6Z" fill="${body}"/>`;
  out += `<path d="M9.6 12.4L26.6 20.2" stroke="${shade(body, 0.2)}" stroke-width="0.8"/>`;
  out += rect(4, 24.8, 24, 2.6, "#575e69", 0.9);
  out += `<path d="M8 8.4Q14 3.6 20.4 5.6" stroke="#9aa3b2" stroke-width="1" fill="none" stroke-linecap="round"/>`;
  out += `<path d="M18.2 3.6L21 5.8L18 7.4" stroke="#9aa3b2" stroke-width="1" fill="none" stroke-linecap="round" stroke-linejoin="round"/>`;
  return out;
}

function drawLooper(s) {
  const body = s.body ?? "#5a6270";
  let out = rect(2.5, 6.5, 27, 19, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(3, 7, 26, 18, body, 1.8);
  out += circle(16, 13.5, 4.6, shade(body, -0.2)) + circle(16, 13.5, 1.2, shade(body, 0.22));
  out += `<path d="M11.8 13.5A4.2 4.2 0 0 1 20.2 13.5" stroke="#e6e8ec" stroke-width="1" fill="none"/>`;
  const n = s.sw ?? 2;
  for (let i = 0; i < n; i++) out += footswitch(6.5 + ((19) * i) / Math.max(1, n - 1), 21, 1.9);
  return out;
}


function drawRotary(s) {
  // Leslie: a tall cabinet with louvered slots over the rotor and the horn.
  const body = s.body ?? "#3c2f26";
  let out = rect(6.5, 2.5, 19, 27, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(7, 3, 18, 26, body, 1.8);
  const slot = shade(body, -0.2);
  const lit = shade(body, 0.12);
  // Upper horn louvers and lower rotor louvers, angled opposite ways — the giveaway.
  for (let i = 0; i < 4; i++) {
    out += `<path d="M8.8 ${N(6.4 + i * 2)}L23.2 ${N(5.2 + i * 2)}" stroke="${slot}" stroke-width="1.2" stroke-linecap="round"/>`;
    out += `<path d="M8.8 ${N(17.8 + i * 2)}L23.2 ${N(19 + i * 2)}" stroke="${slot}" stroke-width="1.2" stroke-linecap="round"/>`;
  }
  out += rect(7.6, 14.4, 16.8, 2.4, lit, 0.6);
  return out;
}

function drawSpring(s) {
  // A reverb tank: the pan, with one (or two) springs stretched down its length.
  const body = s.body ?? "#383d45";
  let out = rect(2.5, 8.5, 27, 15, OUTLINE, 2, ` opacity="0.5"`);
  out += rect(3, 9, 26, 14, body, 1.8);
  const wire = shade(body, isLight(body) ? -0.3 : 0.34);
  const tanks = s.tanks ?? 1;
  const ys = tanks === 2 ? [13, 19] : [16];
  for (const y of ys) {
    let d = `M5.5 ${y}`;
    // A coil drawn as alternating half-arcs — reads as a spring even at 20px.
    for (let x = 5.5; x < 26; x += 2.6) d += `A1.3 ${tanks === 2 ? 1.9 : 2.6} 0 0 1 ${N(x + 2.6)} ${y}`;
    out += `<path d="${d}" stroke="${wire}" stroke-width="1" fill="none"/>`;
    out += circle(5.2, y, 1.1, wire) + circle(26.8, y, 1.1, wire);
  }
  return out;
}

const SHAPES = {
  stomp: (s) => drawStomp(s, { x: 8, w: 16 }),
  stompWide: (s) => drawStomp(s, { x: 4.5, w: 23 }),
  stompNarrow: (s) => drawStomp(s, { x: 10, w: 12 }),
  round: drawRound,
  wedge: drawWedge,
  wah: drawWah,
  head: drawHead,
  combo: drawCombo,
  cab: drawCab,
  rack: drawRack,
  reel: drawReel,
  mic: drawMic,
  util: drawUtil,
  pedalboard: drawPedalboard,
  looper: drawLooper,
  rotary: drawRotary,
  spring: drawSpring,
};

/** Inner SVG markup for a spec, on the 32x32 grid. */
export function iconBody(spec) {
  const draw = SHAPES[spec.shape] ?? SHAPES.stomp;
  return draw(spec);
}

/** A complete standalone `<svg>` — used by the contact sheet and anywhere outside Svelte. */
export function iconSvg(spec, size = 24) {
  return `<svg viewBox="0 0 32 32" width="${size}" height="${size}" xmlns="http://www.w3.org/2000/svg">${iconBody(spec)}</svg>`;
}

export { shade, isLight };
