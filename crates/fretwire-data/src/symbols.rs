//! Parser for `Helix.sym` — the **device's authoritative per-model parameter ordering**.
//!
//! `Helix.sym` is a JSON array of `{ "symbol": "<deviceSymbolicID>", "parameters": [..] }`. Unlike
//! the host-side `.models` / `HelixModelDefs.bin` (one entry per model, e.g. `HD2_TremoloHarmonic`),
//! the device symbols carry a **`Mono`/`Stereo` suffix** (`HD2_TremoloHarmonicMono`,
//! `HD2_TremoloHarmonicStereo`) and the two variants have **different parameter orders and counts**.
//!
//! This matters because a preset's value vector is in **device order**, not `.models` order. The
//! `.models` list interleaves stereo-only params (e.g. a mono Harmonic Tremolo has no `Spread`),
//! so aligning a mono block's values against the `.models` order mislabels params past that point.
//! [`DeviceSymbols::resolve_order`] picks the variant whose length matches the observed vector and
//! returns the correct ordered names. Verified: Harmonic Tremolo (10 vals → Mono), 70s Chorus
//! (11 → Mono), Bucket Brigade (9 → Stereo). Reverbs carry one extra trailing value — the `Trails`
//! switch — so they're `symbol_len + 1` (e.g. `VIC_ReverbRotating`/"Dynamic Hall": 13 vs the
//! 12-param symbol); `resolve_order` accepts that and names the extra value `"Trails"`.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize)]
struct Entry {
    symbol: String,
    #[serde(default)]
    parameters: Vec<String>,
}

/// The device parameter-ordering table. `by_name` keys the param list by full device symbol
/// (with `Mono`/`Stereo`); `ordered` keeps `Helix.sym`'s **array order** so a block's numeric
/// model reference (`24 → 25`, an index into this array) resolves to its symbol + params.
#[derive(Debug, Clone)]
pub struct DeviceSymbols {
    by_name: HashMap<String, Vec<String>>,
    ordered: Vec<(String, Vec<String>)>,
}

/// The two channel-count variants a model can take on the device.
pub const VARIANTS: [&str; 2] = ["Mono", "Stereo"];

impl DeviceSymbols {
    /// Parse `Helix.sym`.
    pub fn parse(bytes: &[u8]) -> crate::Result<DeviceSymbols> {
        let entries: Vec<Entry> = serde_json::from_slice(bytes)?;
        let ordered: Vec<(String, Vec<String>)> = entries
            .into_iter()
            .map(|e| (e.symbol, e.parameters))
            .collect();
        let by_name = ordered.iter().cloned().collect();
        Ok(DeviceSymbols { by_name, ordered })
    }

    /// Number of device symbols.
    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    /// Resolve a block's numeric model reference (preset `24 → 25`) — an index into `Helix.sym`'s
    /// array — to its full device symbol (e.g. `"HD2_AmpUSPrincess"`) and ordered param names.
    /// This is the device's authoritative model identity; verified against a hand-built preset
    /// (591 → US Princess amp, 80 → Simple Delay) and the factory capture.
    pub fn by_index(&self, idx: usize) -> Option<(&str, &[String])> {
        self.ordered
            .get(idx)
            .map(|(s, p)| (s.as_str(), p.as_slice()))
    }

    /// Ordered parameter names for a full device symbol (e.g. `"HD2_TremoloHarmonicMono"`).
    pub fn params(&self, device_symbol: &str) -> Option<&[String]> {
        self.by_name.get(device_symbol).map(Vec::as_slice)
    }

    /// The inverse of [`Self::by_index`] — a full device symbol's position in `Helix.sym`, which is
    /// the number a preset stores at `24 → 25`. Needed when *writing* a preset, where the symbol is
    /// known and the index has to be produced.
    pub fn index_of(&self, device_symbol: &str) -> Option<usize> {
        self.ordered.iter().position(|(s, _)| s == device_symbol)
    }

    /// Which channel-count variants of a **host** symbolic id the device actually has, as the
    /// suffixes to append — `[""]` for a model with no variants at all (most amps and cabs), one
    /// entry where the device offers only one (reverbs are `Stereo`-only, IRs `Mono`-only), and
    /// both where there is a real choice.
    ///
    /// This is what makes a `tone` block's missing `@stereo` key readable: HX Edit writes the flag
    /// only when there is something to choose, so an absent flag means "the one that exists".
    pub fn variants_of(&self, host_symbol: &str) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = VARIANTS
            .iter()
            .copied()
            .filter(|v| self.by_name.contains_key(&format!("{host_symbol}{v}")))
            .collect();
        if out.is_empty() && self.by_name.contains_key(host_symbol) {
            out.push("");
        }
        out
    }

    /// Resolve the device parameter order for a **host** symbolic id (no suffix) given the observed
    /// param-vector length. Tries the `Mono` then `Stereo` variant and returns the one whose param
    /// count matches `count` as `(variant, ordered_names)`.
    ///
    /// Reverbs send **one extra trailing value** — the `Trails` switch — that the device symbol
    /// doesn't list, so the observed vector is `symbol_len + 1`. That case is accepted and the
    /// extra name is synthesized as `"Trails"`. Returns `None` if no variant matches.
    pub fn resolve_order(
        &self,
        host_symbol: &str,
        count: usize,
    ) -> Option<(&'static str, Vec<String>)> {
        // Prefer an exact length match across *both* variants (so a Mono `len+1` can't shadow the
        // correct Stereo exact match).
        for v in VARIANTS {
            if let Some(p) = self.by_name.get(&format!("{host_symbol}{v}"))
                && p.len() == count
            {
                return Some((v, p.clone()));
            }
        }
        // Reverb `@trails` +1: the last device value is the Trails on/off switch, not listed in
        // the symbol. Accept `symbol_len + 1` and name the extra value "Trails".
        for v in VARIANTS {
            if let Some(p) = self.by_name.get(&format!("{host_symbol}{v}"))
                && p.len() + 1 == count
            {
                let mut names = p.clone();
                names.push("Trails".to_string());
                return Some((v, names));
            }
        }
        None
    }
}
