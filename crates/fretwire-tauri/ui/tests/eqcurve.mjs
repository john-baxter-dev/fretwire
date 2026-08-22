// The global EQ curve's maths. Pure functions, so they get checked against textbook behaviour
// rather than by looking at the plot — a curve that is subtly wrong looks fine.
//
//   npm test        (from crates/fretwire-tauri/ui)

import { F_MIN, F_MAX, peakDb, hpDb, lpDb, responseDb, fPos, fFromPos } from "../src/lib/eqcurve.js";
let pass = 0, fail = 0;
const ok = (c, m) => { c ? pass++ : (fail++, console.error("FAIL:", m)); };
const near = (a, b, tol, m) => ok(Math.abs(a - b) <= tol, `${m} — got ${a}, wanted ${b}±${tol}`);

// --- peaking band ---
near(peakDb(1000, 1000, 1, 6), 6, 0.001, "a peak hits its gain exactly at centre");
near(peakDb(1000, 1000, 1, -6), -6, 0.001, "a cut hits its gain at centre");
near(peakDb(20, 1000, 1, 12), 0, 0.05, "a peak is flat three decades below centre");
near(peakDb(20000, 1000, 1, 12), 0, 0.05, "…and above it");
ok(peakDb(1000, 1000, 1, 0) === 0, "zero gain is exactly flat, not nearly");
ok(peakDb(500, 1000, 1, 0) === 0, "…at every frequency");
// Symmetry: a boost and the matching cut mirror through 0 dB.
near(peakDb(700, 1000, 2, 9), -peakDb(700, 1000, 2, -9), 1e-9, "boost and cut mirror");
// Higher Q is narrower: further from centre, the wide band must still be doing more.
ok(
  Math.abs(peakDb(500, 1000, 0.5, 9)) > Math.abs(peakDb(500, 1000, 4, 9)),
  "a low-Q band is wider than a high-Q one",
);

// --- cuts ---
near(hpDb(100, 100), -3.01, 0.02, "the high-pass is -3 dB at its corner");
near(lpDb(1000, 1000), -3.01, 0.02, "the low-pass is -3 dB at its corner");
near(hpDb(50, 100) - hpDb(25, 100), 12, 0.6, "the high-pass falls ~12 dB/oct below the corner");
near(lpDb(4000, 1000) - lpDb(2000, 1000), -12, 0.6, "the low-pass falls ~12 dB/oct above it");
ok(hpDb(1000, 100) > -0.1, "well above its corner the high-pass is out of the way");
ok(hpDb(500, F_MIN) === 0, "a high-pass parked at the bottom of the range is off");
ok(lpDb(500, F_MAX) === 0, "a low-pass parked at the top is off");

// --- summation ---
const flat = responseDb(1000, {});
ok(flat === 0, "no bands and no cuts is a flat line");
near(
  responseDb(1000, { bands: [{ freq: 1000, q: 1, gain: 4 }, { freq: 1000, q: 1, gain: 3 }] }),
  7, 0.001,
  "overlapping bands sum in dB",
);
// The point of the whole exercise: a band whose gain id was never identified contributes nothing,
// rather than being drawn as a deliberate 0 dB.
near(
  responseDb(1000, { bands: [{ freq: 1000, q: 1, gain: null }, { freq: 1000, q: 1, gain: 5 }] }),
  5, 0.001,
  "a band with an unknown gain is skipped, not zeroed",
);
near(
  responseDb(100, { bands: [{ freq: 100, q: 1, gain: 6 }], lowCut: 100 }),
  6 - 3.01, 0.02,
  "a cut and a band at the same frequency add",
);
ok(responseDb(1000, { lowCut: null, highCut: null }) === 0, "null cuts are off, not 0 Hz");

// --- slider mapping ---
near(fPos(F_MIN), 0, 1e-9, "the bottom of the range maps to 0");
near(fPos(F_MAX), 1, 1e-9, "the top maps to 1");
for (const f of [20, 63, 440, 1000, 5000, 20000]) {
  near(fFromPos(fPos(f)), f, f * 1e-9, `${f} Hz round-trips through the slider mapping`);
}
// Log, not linear: the midpoint of the slider is the geometric mean, ~632 Hz, not 10 kHz.
near(fFromPos(0.5), Math.sqrt(F_MIN * F_MAX), 0.001, "the slider is logarithmic");
near(fPos(10), 0, 1e-9, "sub-audible input clamps rather than going negative");

console.log(`${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
