// Response maths for the global EQ plot, kept out of the component so it can be tested without a
// browser — the curve is the one part of that panel whose correctness isn't obvious by looking.
//
// **These shapes are not measured.** The pedal gives us frequency, Q, gain and two cut corners; it
// has never told us its filter topology. Textbook analog sections are the honest way to render
// known parameters through an unknown one, and the panel's caption says so.

export const F_MIN = 20;
export const F_MAX = 20000;

/**
 * Analog peaking section, in dB at `f`:
 *
 *   |H(jw)|^2 = ((w0^2-w^2)^2 + (A*w0*w/Q)^2) / ((w0^2-w^2)^2 + (w0*w/(A*Q))^2)
 *
 * Zero gain is exactly flat, which is why a band we can't read the gain of contributes nothing
 * rather than a guess.
 */
export function peakDb(f, f0, q, gainDb) {
  if (!gainDb || !f0 || !q) return 0;
  const A = Math.pow(10, gainDb / 40);
  const d = f0 * f0 - f * f;
  const num = d * d + Math.pow((A * f0 * f) / q, 2);
  const den = d * d + Math.pow((f0 * f) / (A * q), 2);
  return 10 * Math.log10(num / den);
}

/** Two-pole Butterworth high-pass (the "low cut"), in dB. `-3 dB` at the corner, 12 dB/oct below. */
export function hpDb(f, fc) {
  if (!fc || fc <= F_MIN) return 0;
  const r = Math.pow(f / fc, 4);
  return 10 * Math.log10(r / (1 + r));
}

/** Two-pole Butterworth low-pass (the "high cut"), in dB. */
export function lpDb(f, fc) {
  if (!fc || fc >= F_MAX) return 0;
  return 10 * Math.log10(1 / (1 + Math.pow(f / fc, 4)));
}

/**
 * The summed response at `f`.
 *
 * `bands` is `[{freq, q, gain}]` — a band with no `gain` is skipped entirely, which is how the two
 * whose gain ids were never identified are drawn: absent, not flat-by-choice. `lowCut`/`highCut`
 * are `null` when off.
 */
export function responseDb(f, { bands = [], lowCut = null, highCut = null } = {}) {
  let db = 0;
  for (const b of bands) {
    if (b == null || b.gain == null || b.freq == null) continue;
    db += peakDb(f, b.freq, b.q ?? 0.707, b.gain);
  }
  if (lowCut != null) db += hpDb(f, lowCut);
  if (highCut != null) db += lpDb(f, highCut);
  return db;
}

/** Log position 0..1 across the audible band, for a frequency slider. */
export const fPos = (f) =>
  Math.log10(Math.max(f, F_MIN) / F_MIN) / Math.log10(F_MAX / F_MIN);

/** The inverse of `fPos`. */
export const fFromPos = (p) => F_MIN * Math.pow(F_MAX / F_MIN, Number(p));
