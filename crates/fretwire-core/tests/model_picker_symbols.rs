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

/// The same suffix lesson, for the DSP loads: POD Go Edit's `.models` key each entry by the full
/// device symbol, so a base-keyed lookup left every suffixed model with no load and the "DSP free"
/// figure summing only the rest. [issue #15, 2026-09-16]
#[test]
fn the_pod_go_picker_prices_its_suffix_keyed_models() {
    let Ok(cat) = Catalog::load_for_model("P34") else {
        return;
    };
    let delays = cat.models_in_category(DELAY, None);
    let adriatic = delays
        .iter()
        .find(|c| c.name == "Adriatic Delay")
        .expect("an Adriatic Delay on the POD Go");
    // The choice carries the base symbol and the variant; the load is keyed by their join.
    assert_eq!(adriatic.symbolic_id, "HD2_DelayAdriaticDelay");
    assert_eq!(adriatic.variant, Some("Stereo"));
    assert_eq!(
        adriatic.dsp_load,
        Some(12.5),
        "keyed by the full symbol in delay.models"
    );
    let priced = delays.iter().filter(|c| c.dsp_load.is_some()).count();
    assert!(
        priced * 10 >= delays.len() * 9,
        "only {priced} of {} POD Go delays carry a load",
        delays.len()
    );
}

/// The HX side of the same fix: the eight DL4 legacy delays are keyed by their full symbol in HX
/// Edit's `delay.models` too, so they listed (since 2026-08-26) but showed no load.
#[test]
fn the_hx_picker_prices_the_dl4_delays() {
    let cat = Catalog::load().expect("HX data");
    let tape = cat
        .models_in_category(DELAY, None)
        .into_iter()
        .find(|c| c.name == "Tape Echo" && c.symbolic_id.contains("DL4"))
        .expect("the DL4 Tape Echo");
    assert_eq!(tape.dsp_load, Some(11.0));
}

/// The POD Go has no amp+cab block: its amp and cab/IR are two slots, paired by the pedal's own
/// Link Amp/Cab setting, and a paired op 40 is refused with `-3`. So the synthetic Amp+Cab list
/// is an HX-only category. [issue #15, 2026-09-16]
#[test]
fn the_pod_go_picker_has_no_amp_cab_category() {
    let Ok(cat) = Catalog::load_for_model("P34") else {
        return;
    };
    assert!(cat.pod_go());
    assert!(
        !cat.categories()
            .iter()
            .any(|(id, _)| *id == fretwire_core::editor::CATEGORY_AMP_CAB),
        "Amp+Cab offered on a POD Go"
    );
    assert!(
        cat.models_in_category(fretwire_core::editor::CATEGORY_AMP_CAB, None)
            .is_empty()
    );
    // The amps themselves are still there to pick from.
    assert!(cat.categories().iter().any(|(id, _)| *id == 1));
    let hx = Catalog::load().expect("HX data");
    assert!(!hx.pod_go());
    assert!(
        hx.categories()
            .iter()
            .any(|(id, _)| *id == fretwire_core::editor::CATEGORY_AMP_CAB),
        "the HX still gets its Amp+Cab list"
    );
}
