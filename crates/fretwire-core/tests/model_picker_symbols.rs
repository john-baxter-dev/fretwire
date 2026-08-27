//! Regression: the model picker must look a symbol up **as its own table spells it**.
//!
//! HX Edit and POD Go Edit disagree on whether a `symbolicID` keeps its `Mono`/`Stereo` suffix.
//! HX Edit's `HelixModelDefs.bin` is keyed by the base (344 entries) with only 8 exceptions; POD Go
//! Edit's `PodGoModelDefs.bin` is keyed by the full suffixed symbol for all 180 of them and by the
//! base for none. `categories`/`models_in_category` used to look up the stripped base, so on a POD
//! Go every suffixed model vanished — the whole Wah and Reverb categories, and most of Delay — and
//! on an HX the eight DL4 legacy delays did too. [issue #15]
//!
//! Needs the (unshipped) Line 6 reference data (`have_bundled_data`, set by build.rs).
#![cfg(have_bundled_data)]

use fretwire_core::editor::Catalog;

const WAH: i64 = 11;
const REVERB: i64 = 10;
const DELAY: i64 = 9;

#[test]
fn the_hx_picker_lists_the_suffix_keyed_dl4_delays() {
    let cat = Catalog::load().expect("HX data");
    let delays = cat.models_in_category(DELAY, None);
    // `HD2_DL4AnalogDelayStereo` and friends are keyed by the full symbol even in HX Edit's table.
    assert!(
        delays.iter().any(|c| c.symbolic_id.contains("DL4")),
        "the DL4 legacy delays are missing from the HX delay list"
    );
}

#[test]
fn the_pod_go_picker_has_its_suffix_keyed_categories() {
    // Only meaningful where POD Go Edit's data has been imported too; skip otherwise.
    let Ok(cat) = Catalog::load_for_model("P34") else {
        return;
    };
    let ids: Vec<i64> = cat.categories().into_iter().map(|(id, _)| id).collect();
    for (id, name) in [(WAH, "Wah"), (REVERB, "Reverb")] {
        assert!(ids.contains(&id), "the POD Go has no {name} category");
        assert!(
            !cat.models_in_category(id, None).is_empty(),
            "the POD Go's {name} category is empty"
        );
    }
    // Every POD Go wah is `…Stereo`, so a base-keyed lookup would have found none of them.
    assert!(
        cat.models_in_category(WAH, None).len() > 5,
        "the POD Go wah list is suspiciously short"
    );
}
