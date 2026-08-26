//! Parser for `HelixModelDefs.bin` — a MessagePack **array of all model definitions** (681), in
//! the device's canonical order. This is the authoritative model table: each entry's
//! `symbolicID` is **globally unique** (681/681), making it the canonical model identifier.
//!
//! Resolving a preset block → model: the preset does **not** carry a numeric model id that
//! indexes this array (see `docs/preset-format.md`) — slot `24 → 25` is a runtime DSP handle
//! (non-monotonic, not in this table) and path `11 → 6` encodes only the *category*. So we
//! resolve by **display name** ([`ids_by_name`](ModelDefs::ids_by_name)). Names are not unique
//! (164 collide: cab mic/pan variants, amp vs preamp, legacy delays); [`resolve`](ModelDefs::resolve)
//! narrows with the **param count** (which the value vector gives us — preferred over category) and,
//! when still tied, the category. `(name, param_count)` alone resolves 150/164 collisions; only 11
//! amp/preamp pairs with equal param counts also need category (the undecoded `11 → 6`); 3 are
//! unresolvable (the `2x12 Match H30`/`G25` data defect + per-device I/O). See
//! `tools/analyze-name-collisions.js`.

use rmpv::Value;

/// The full model table, indexed by numeric model id.
#[derive(Debug, Clone)]
pub struct ModelDefs(Vec<Value>);

impl ModelDefs {
    /// Parse the MessagePack array. Accepts a leading envelope by locating the array root.
    pub fn parse(bytes: &[u8]) -> crate::Result<ModelDefs> {
        let mut cur = bytes;
        let value = rmpv::decode::read_value(&mut cur)
            .map_err(|e| crate::Error::Stream(format!("model defs msgpack: {e}")))?;
        match value {
            Value::Array(a) => Ok(ModelDefs(a)),
            other => Err(crate::Error::Stream(format!(
                "expected array, got {other:?}"
            ))),
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn field(&self, id: usize, key: &str) -> Option<&str> {
        // Keys and string values in this blob carry a trailing NUL — trim on both sides.
        self.raw(id, key)
            .and_then(|v| v.as_str())
            .map(|s| s.trim_end_matches('\0'))
    }

    /// Raw value of a (NUL-padded) string key within an entry's map.
    fn raw(&self, id: usize, key: &str) -> Option<&Value> {
        if let Value::Map(entries) = self.0.get(id)? {
            for (k, v) in entries {
                if k.as_str().map(|s| s.trim_end_matches('\0')) == Some(key) {
                    return Some(v);
                }
            }
        }
        None
    }

    /// `symbolicID` of the model at numeric id `id` (e.g. `"HD2_DelayBucketBrigade"`).
    pub fn symbolic_id(&self, id: usize) -> Option<&str> {
        self.field(id, "symbolicID")
    }

    /// Numeric id of the model whose `symbolicID` equals `sym`. `symbolicID`s are unique across
    /// the table (681/681), so this is the canonical reverse lookup — used to turn a device
    /// block's symbol into a display name + category, independent of the signal path.
    ///
    /// **The two vendors disagree on whether the `Mono`/`Stereo` suffix belongs here**, so the
    /// caller cannot know which form to ask for. `HelixModelDefs.bin` strips it
    /// (`HD2_DistScream808`) while `PodGoModelDefs.bin` keeps it (`HD2_DistScream808Mono`), and
    /// each file's own symbol table uses the suffixed form. So we try the symbol as given first,
    /// then its stripped base. Trying both is strictly better on *both* devices — matching
    /// 748/833 Helix symbols against 740 for stripping alone, and 537/627 POD Go symbols against
    /// 372. [2026-08-25, issue #15]
    pub fn id_by_symbolic_id(&self, sym: &str) -> Option<usize> {
        self.find_exact(sym).or_else(|| {
            let base = sym
                .strip_suffix("Mono")
                .or_else(|| sym.strip_suffix("Stereo"))?;
            self.find_exact(base)
        })
    }

    fn find_exact(&self, sym: &str) -> Option<usize> {
        (0..self.len()).find(|&i| self.symbolic_id(i) == Some(sym))
    }

    /// Display `name` of the model at numeric id `id` (e.g. `"Bucket Brigade"`).
    pub fn name(&self, id: usize) -> Option<&str> {
        self.field(id, "name")
    }

    /// Category id of the model at `id` (cross-references `HX_ModelCatalog.json`). The preset's
    /// path key `11 → 6` encodes this category (not the model itself — same-category models
    /// share a `11 → 6` value), so once that encoding is decoded it disambiguates the name
    /// collisions below.
    pub fn category(&self, id: usize) -> Option<i64> {
        self.raw(id, "category").and_then(Value::as_i64)
    }

    /// Number of parameters the model declares.
    pub fn param_count(&self, id: usize) -> Option<usize> {
        match self.raw(id, "params") {
            Some(Value::Array(a)) => Some(a.len()),
            _ => None,
        }
    }

    /// All model ids whose display `name` matches. Display names are **not unique** in the table
    /// (163 collide: cab mic/pan variants, amp vs preamp, legacy vs modern delays, per-device
    /// I/O), so this can return several. Use [`resolve`](Self::resolve) to narrow with a category.
    pub fn ids_by_name(&self, name: &str) -> Vec<usize> {
        (0..self.len())
            .filter(|&i| self.name(i) == Some(name))
            .collect()
    }

    /// Resolve a block to a single unambiguous model id from its display name plus, optionally,
    /// its category (preset path key `11 → 6`, once decoded).
    ///
    /// Returns `Ok(id)` when exactly one model matches. Returns `Err(candidates)` listing all
    /// matches when the name (and category, if given) are still ambiguous. With name **and**
    /// category this is unique for every real model except the `2x12 Match H30` / `Match G25`
    /// cab pair (a duplicate-name defect in Line 6's own data) and the per-device I/O blocks
    /// (which are disambiguated by the device family, not modeled here).
    pub fn resolve(
        &self,
        name: &str,
        category: Option<i64>,
    ) -> std::result::Result<usize, Vec<usize>> {
        let mut candidates = self.ids_by_name(name);
        if let Some(cat) = category {
            let filtered: Vec<usize> = candidates
                .iter()
                .copied()
                .filter(|&i| self.category(i) == Some(cat))
                .collect();
            // Only narrow if the category actually matched something; otherwise keep the name
            // candidates so a caller still sees the real options.
            if !filtered.is_empty() {
                candidates = filtered;
            }
        }
        match candidates.as_slice() {
            [one] => Ok(*one),
            _ => Err(candidates),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-entry table keyed by `symbolic_id`. Line 6 NUL-terminates its strings and the parser
    /// trims them, so the fixture does the same.
    fn defs_keyed_by(symbolic_id: &str) -> ModelDefs {
        let entry = Value::Map(vec![
            (
                Value::from("symbolicID\0"),
                Value::from(format!("{symbolic_id}\0")),
            ),
            (Value::from("name\0"), Value::from("Scream 808\0")),
        ]);
        let mut buf = Vec::new();
        rmpv::encode::write_value(&mut buf, &Value::Array(vec![entry])).unwrap();
        ModelDefs::parse(&buf).unwrap()
    }

    #[test]
    fn a_suffix_keeping_table_matches_the_symbol_as_given() {
        // POD Go Edit's spelling — `PodGoModelDefs.bin` keys on the suffixed symbol, exactly as
        // `PodGo.sym` spells it.
        let defs = defs_keyed_by("HD2_DistScream808Mono");
        assert_eq!(defs.id_by_symbolic_id("HD2_DistScream808Mono"), Some(0));
    }

    #[test]
    fn a_suffix_stripping_table_still_matches_a_suffixed_symbol() {
        // HX Edit's spelling — `HelixModelDefs.bin` drops the suffix that `Helix.sym` carries, so
        // the lookup has to fall back to the base.
        let defs = defs_keyed_by("HD2_DistScream808");
        assert_eq!(defs.id_by_symbolic_id("HD2_DistScream808Stereo"), Some(0));
        assert_eq!(defs.id_by_symbolic_id("HD2_DistScream808"), Some(0));
    }

    #[test]
    fn the_fallback_does_not_match_a_different_model() {
        // Trying two spellings must not turn a miss into a wrong hit.
        let defs = defs_keyed_by("HD2_DistScream808");
        assert_eq!(defs.id_by_symbolic_id("HD2_DistMinotaurMono"), None);
    }
}
