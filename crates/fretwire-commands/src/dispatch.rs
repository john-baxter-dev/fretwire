//! JSON dispatch — command name + JSON args in, JSON out. This is what a network transport
//! (`fretwire-serve`, and one day MCP) calls instead of Tauri's generated handler; it lives here
//! rather than in the server crate so the offline suite covers it on a clean clone with no
//! hardware.
//!
//! **Argument names are the frontend's**: the UI sends camelCase (`paramIndex`, `modelIndex` —
//! Tauri's macro converts them to the snake_case Rust parameters), so [`Args`] looks names up
//! camelCase-first with a snake_case fallback, matching Tauri v2's tolerance.
//!
//! Three lists must agree — the match below, [`COMMAND_NAMES`], and `generate_handler!` in
//! `fretwire-tauri/src/main.rs`. The Tauri list fails to compile when a wrapper is missing; the
//! two here are pinned to each other by `every_name_dispatches`, and a command missing from the
//! match fails loudly at runtime ("unknown command") rather than silently.

use crate::AppState;
use crate::events::EventSink;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Every command the dispatcher answers — the serve-side registry, one name per match arm.
pub const COMMAND_NAMES: &[&str] = &[
    "data_status",
    "update_status",
    "update_check",
    "update_pref",
    "device_numbering",
    "settings_read",
    "settings_write",
    "ir_list",
    "ir_scan",
    "ir_export",
    "ir_upload",
    "ir_export_inline",
    "ir_upload_inline",
    "ir_delete",
    "ir_rename",
    "import_data",
    "detect",
    "is_connected",
    "connect",
    "disconnect",
    "read_preset",
    "undo",
    "redo",
    "history_jump",
    "preview_param",
    "preview_paired_param",
    "set_bypass",
    "set_param",
    "set_paired_param",
    "set_param_enum",
    "swap_model",
    "add_block",
    "add_block_at",
    "delete_block",
    "clear_preset",
    "reorder_block",
    "move_block_to_row",
    "move_before_split",
    "place_block",
    "insert_block",
    "set_node_pos",
    "set_split_type",
    "assign_bypass",
    "unassign_bypass",
    "set_switch_label",
    "set_switch_color",
    "set_switch_momentary",
    "revert_preset",
    "assign_param",
    "set_assign_travel",
    "set_snapshot",
    "goto_preset",
    "save_preset",
    "rename_preset",
    "rename_snapshot",
    "list_presets",
    "setlists",
    "cross_setlist_write_allowed",
    "export_setlists",
    "cancel_export",
    "backup_show",
    "restore_preset",
    "export_setlists_inline",
    "backup_show_inline",
    "restore_preset_inline",
    "backup_device",
    "backup_device_inline",
    "backup_info",
    "backup_info_inline",
    "restore_device",
    "restore_device_inline",
    "split_types",
    "categories",
    "models_in_category",
    "copy_preset",
    "paste_preset",
    "clipboard_preset",
    "copy_block",
    "paste_block",
    "clipboard_block",
];

/// The JSON argument bag, looked up by the frontend's (camelCase) names.
struct Args(Value);

impl Args {
    /// A required argument. Missing or mistyped is an error naming the argument, so the message
    /// points at the actual call site mistake rather than a generic parse failure.
    fn req<T: DeserializeOwned>(&self, name: &str) -> Result<T, String> {
        let v = self
            .get(name)
            .ok_or_else(|| format!("missing argument `{name}`"))?;
        serde_json::from_value(v.clone()).map_err(|e| format!("argument `{name}`: {e}"))
    }

    /// An optional argument: absent or `null` is `None`.
    fn opt<T: DeserializeOwned>(&self, name: &str) -> Result<Option<T>, String> {
        match self.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone())
                .map(Some)
                .map_err(|e| format!("argument `{name}`: {e}")),
        }
    }

    /// camelCase first (what the UI sends), snake_case as the fallback Tauri also accepts.
    fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name).or_else(|| {
            let snake = camel_to_snake(name);
            (snake != name).then(|| self.0.get(&snake)).flatten()
        })
    }
}

fn camel_to_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Serialize a command's result to the wire value. `()` serializes to `null`, matching what a
/// Tauri `invoke` resolves with for a unit command.
fn ok<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("serializing the result: {e}"))
}

/// Run one command by name. Errors cross as plain strings, exactly as Tauri rejects an `invoke`.
/// The sink is consumed by the one command that emits (`export_setlists`); every other arm
/// ignores it.
pub async fn dispatch(
    state: &AppState,
    sink: impl EventSink,
    command: &str,
    args: Value,
) -> Result<Value, String> {
    use crate as c;
    let a = Args(args);
    match command {
        // ---- reference data / connection ----
        "data_status" => ok(c::data_status()),
        "import_data" => ok(c::import_data(a.req("source")?).await?),
        // ---- update check ----
        "update_status" => ok(c::update_status()),
        "update_check" => ok(c::update_check(a.opt("force")?.unwrap_or(false)).await?),
        "update_pref" => ok(c::update_pref(a.req("enabled")?).await?),
        "detect" => ok(c::detect().await?),
        "is_connected" => ok(c::is_connected(state)),
        "connect" => ok(c::connect(state).await?),
        "disconnect" => ok(c::disconnect(state).await?),
        "read_preset" => ok(c::read_preset(state).await?),
        // ---- undo / redo ----
        "undo" => ok(c::undo(state).await?),
        "redo" => ok(c::redo(state).await?),
        "history_jump" => ok(c::history_jump(state, a.req("index")?).await?),
        // ---- block edits ----
        "set_bypass" => ok(c::set_bypass(state, a.req("slot")?, a.req("bypassed")?).await?),
        "set_param" => {
            ok(c::set_param(state, a.req("slot")?, a.req("paramIndex")?, a.req("value")?).await?)
        }
        "preview_param" => {
            ok(
                c::preview_param(state, a.req("slot")?, a.req("paramIndex")?, a.req("value")?)
                    .await?,
            )
        }
        "preview_paired_param" => ok(c::preview_paired_param(
            state,
            a.req("slot")?,
            a.req("paramIndex")?,
            a.req("value")?,
        )
        .await?),
        "set_paired_param" => {
            ok(
                c::set_paired_param(state, a.req("slot")?, a.req("paramIndex")?, a.req("value")?)
                    .await?,
            )
        }
        "set_param_enum" => ok(c::set_param_enum(
            state,
            a.req("slot")?,
            a.req("paired")?,
            a.req("paramIndex")?,
            a.req("value")?,
        )
        .await?),
        "swap_model" => ok(c::swap_model(
            state,
            a.req("slot")?,
            a.req("modelIndex")?,
            a.req("pairedIndex")?,
        )
        .await?),
        "add_block" => ok(c::add_block(state, a.req("modelIndex")?, a.req("pairedIndex")?).await?),
        "add_block_at" => ok(c::add_block_at(
            state,
            a.req("slot")?,
            a.req("modelIndex")?,
            a.req("pairedIndex")?,
        )
        .await?),
        "delete_block" => ok(c::delete_block(state, a.req("slot")?).await?),
        "clear_preset" => ok(c::clear_preset(state).await?),
        "revert_preset" => ok(c::revert_preset(state).await?),
        "reorder_block" => ok(c::reorder_block(state, a.req("srcSlot")?, a.req("gap")?).await?),
        // ---- routing ----
        "move_block_to_row" => {
            ok(
                c::move_block_to_row(state, a.req("srcSlot")?, a.req("parallel")?, a.req("pos")?)
                    .await?,
            )
        }
        "move_before_split" => ok(c::move_before_split(state, a.req("srcSlot")?).await?),
        "place_block" => ok(c::place_block(state, a.req("srcSlot")?, a.req("dstSlot")?).await?),
        "insert_block" => ok(c::insert_block(
            state,
            a.req("srcSlot")?,
            a.req("dstSlot")?,
            a.req("before")?,
        )
        .await?),
        "set_node_pos" => {
            ok(c::set_node_pos(state, a.req("node")?, a.req("pos")?, a.req("dsp")?).await?)
        }
        "set_split_type" => {
            ok(c::set_split_type(state, a.req("splitSlot")?, a.req("modelIndex")?).await?)
        }
        // ---- controller assignments ----
        "assign_bypass" => ok(c::assign_bypass(state, a.req("slot")?, a.req("switch")?).await?),
        "unassign_bypass" => ok(c::unassign_bypass(state, a.req("slot")?, a.req("switch")?).await?),
        "set_switch_label" => {
            ok(c::set_switch_label(state, a.req("switch")?, a.opt("label")?).await?)
        }
        "set_switch_momentary" => {
            ok(c::set_switch_momentary(state, a.req("switch")?, a.req("momentary")?).await?)
        }
        "set_switch_color" => {
            ok(c::set_switch_color(state, a.req("switch")?, a.opt("color")?).await?)
        }
        "assign_param" => ok(c::assign_param(
            state,
            a.req("slot")?,
            a.req("paramIndex")?,
            a.req("source")?,
            a.req("paired")?,
        )
        .await?),
        "set_assign_travel" => ok(c::set_assign_travel(
            state,
            a.req("slot")?,
            a.req("paramIndex")?,
            a.req("max")?,
            a.req("value")?,
            a.req("paired")?,
        )
        .await?),
        // ---- snapshots / navigation / persistence ----
        "set_snapshot" => ok(c::set_snapshot(state, a.req("index")?).await?),
        "goto_preset" => ok(c::goto_preset(state, a.req("bank")?, a.req("preset")?).await?),
        "save_preset" => {
            ok(c::save_preset(state, a.req("bank")?, a.req("slot")?, a.req("name")?).await?)
        }
        "rename_preset" => {
            ok(c::rename_preset(state, a.req("bank")?, a.req("slot")?, a.req("name")?).await?)
        }
        "rename_snapshot" => ok(c::rename_snapshot(state, a.req("index")?, a.req("name")?).await?),
        "list_presets" => ok(c::list_presets(state, a.opt("bank")?).await?),
        "setlists" => ok(c::setlists(state).await?),
        "cross_setlist_write_allowed" => ok(c::cross_setlist_write_allowed().await),
        // ---- backup / restore ----
        "export_setlists" => {
            ok(c::export_setlists(state, sink, a.req("path")?, a.req("banks")?).await?)
        }
        "cancel_export" => ok(c::cancel_export(state).await?),
        "backup_show" => ok(c::backup_show(a.req("path")?).await?),
        "restore_preset" => ok(c::restore_preset(
            state,
            a.req("path")?,
            a.req("index")?,
            a.req("slot")?,
            a.req("bank")?,
        )
        .await?),
        // The same three with the file inline — for a frontend whose disk the daemon can't see.
        "export_setlists_inline" => {
            ok(c::export_setlists_inline(state, sink, a.req("banks")?).await?)
        }
        "backup_show_inline" => ok(c::backup_show_inline(a.req("json")?).await?),
        "backup_device" => ok(c::backup_device(
            state,
            sink,
            a.req("path")?,
            a.req("banks")?,
            a.req("irs")?,
            a.req("settings")?,
        )
        .await?),
        "backup_device_inline" => ok(c::backup_device_inline(
            state,
            sink,
            a.req("banks")?,
            a.req("irs")?,
            a.req("settings")?,
        )
        .await?),
        "backup_info" => ok(c::backup_info(a.req("path")?).await?),
        "backup_info_inline" => ok(c::backup_info_inline(a.req("json")?).await?),
        "restore_device" => ok(c::restore_device(
            state,
            sink,
            a.req("path")?,
            a.req("presets")?,
            a.req("irs")?,
            a.req("settings")?,
        )
        .await?),
        "restore_device_inline" => ok(c::restore_device_inline(
            state,
            sink,
            a.req("json")?,
            a.req("presets")?,
            a.req("irs")?,
            a.req("settings")?,
        )
        .await?),
        "restore_preset_inline" => ok(c::restore_preset_inline(
            state,
            a.req("json")?,
            a.req("index")?,
            a.req("slot")?,
            a.req("bank")?,
        )
        .await?),
        // ---- clipboards ----
        "copy_preset" => ok(c::copy_preset(state).await?),
        "paste_preset" => ok(c::paste_preset(state).await?),
        "clipboard_preset" => ok(c::clipboard_preset(state)),
        "copy_block" => ok(c::copy_block(state, a.req("slot")?).await?),
        "paste_block" => ok(c::paste_block(state, a.req("slot")?).await?),
        "clipboard_block" => ok(c::clipboard_block(state)),
        // ---- catalog ----
        "split_types" => ok(c::split_types()),
        "categories" => ok(c::categories(state).await?),
        "models_in_category" => {
            ok(c::models_in_category(state, a.req("category")?, a.opt("variant")?).await?)
        }
        "device_numbering" => ok(c::device_numbering(state).await?),
        // ---- device settings ----
        "settings_read" => ok(c::settings_read(state, a.req("all")?).await?),
        "settings_write" => ok(c::settings_write(state, a.req("id")?, a.req("value")?).await?),
        // ---- user IR slots ----
        "ir_list" => ok(c::ir_list(state).await?),
        "ir_scan" => ok(c::ir_scan(state).await?),
        "ir_export" => ok(c::ir_export(state, a.req("slot")?, a.req("path")?).await?),
        "ir_upload" => ok(c::ir_upload(
            state,
            a.req("slot")?,
            a.req("path")?,
            a.opt("name")?,
            a.req("overwrite")?,
            a.req("force")?,
        )
        .await?),
        "ir_delete" => ok(c::ir_delete(state, a.req("slot")?).await?),
        "ir_rename" => ok(c::ir_rename(state, a.req("slot")?, a.req("name")?).await?),
        "ir_export_inline" => ok(c::ir_export_inline(state, a.req("slot")?).await?),
        "ir_upload_inline" => ok(c::ir_upload_inline(
            state,
            a.req("slot")?,
            a.req("wavBase64")?,
            a.req("name")?,
            a.req("overwrite")?,
            a.req("force")?,
        )
        .await?),
        _ => Err(format!("unknown command: {command}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct NoopSink;
    impl EventSink for NoopSink {
        fn emit(&self, _event: crate::events::Event) {}
    }

    async fn call(command: &str, args: Value) -> Result<Value, String> {
        let state = AppState::default();
        dispatch(&state, NoopSink, command, args).await
    }

    /// Every registered name reaches a real match arm. Called with empty args, a command fails on
    /// a missing argument or a missing session — anything but "unknown command", which means the
    /// match and [`COMMAND_NAMES`] have drifted apart.
    ///
    /// `connect` is the one exception: with a pedal plugged in it would actually claim the USB
    /// interface (possibly out from under a running editor), so it is skipped here and its arm is
    /// pinned by `connect_is_matched_without_running`.
    #[tokio::test]
    async fn every_name_dispatches() {
        assert_eq!(COMMAND_NAMES.len(), 80, "the surface was 80 commands");
        for name in COMMAND_NAMES {
            if *name == "connect" {
                continue;
            }
            let res = call(name, serde_json::json!({})).await;
            if let Err(e) = &res {
                assert!(
                    !e.starts_with("unknown command"),
                    "`{name}` is in COMMAND_NAMES but not in the dispatch match"
                );
            }
        }
    }

    /// `connect`'s arm exists — proven by the *absence* of the unknown-command error when the
    /// name is checked against the list the sweep above pins to the match.
    #[test]
    fn connect_is_matched_without_running() {
        assert!(COMMAND_NAMES.contains(&"connect"));
    }

    #[tokio::test]
    async fn unknown_commands_fail_loudly() {
        let e = call("frobnicate", serde_json::json!({})).await.unwrap_err();
        assert_eq!(e, "unknown command: frobnicate");
    }

    /// The frontend's camelCase names resolve, and the snake_case forms Tauri also accepts do
    /// too. "not connected" (not "missing argument") proves the args parsed.
    #[tokio::test]
    async fn camel_and_snake_case_args_both_resolve() {
        for args in [
            serde_json::json!({"slot": 1, "paramIndex": 2, "value": 0.5}),
            serde_json::json!({"slot": 1, "param_index": 2, "value": 0.5}),
        ] {
            let e = call("set_param", args).await.unwrap_err();
            assert!(e.contains("not connected"), "got: {e}");
        }
    }

    #[tokio::test]
    async fn a_missing_argument_names_itself() {
        let e = call("set_param", serde_json::json!({"slot": 1, "value": 0.5}))
            .await
            .unwrap_err();
        assert_eq!(e, "missing argument `paramIndex`");
    }

    #[tokio::test]
    async fn a_mistyped_argument_names_itself() {
        let e = call(
            "set_bypass",
            serde_json::json!({"slot": "one", "bypassed": true}),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("argument `slot`:"), "got: {e}");
    }

    /// Offline round-trip through JSON: the same shape Tauri hands the frontend.
    #[tokio::test]
    async fn data_status_round_trips() {
        let v = call("data_status", serde_json::json!({})).await.unwrap();
        assert!(v.get("present").is_some(), "got: {v}");
    }

    /// `()` results resolve as `null`, matching a Tauri unit command.
    #[tokio::test]
    async fn unit_results_are_null() {
        let v = call("cancel_export", serde_json::json!({})).await.unwrap();
        assert_eq!(v, Value::Null);
    }

    #[tokio::test]
    async fn backup_show_reports_the_file_error() {
        let e = call(
            "backup_show",
            serde_json::json!({"path": "/nonexistent/fretwire-test.json"}),
        )
        .await
        .unwrap_err();
        assert!(e.contains("reading"), "got: {e}");
    }

    /// The inline variants parse what they are handed, without a device or a disk.
    #[tokio::test]
    async fn backup_show_inline_lists_the_file() {
        let backup = fretwire_core::backup::Backup {
            device: "HX Stomp".into(),
            setlists: vec![(0, "FACTORY 1".into())],
            presets: vec![fretwire_core::backup::BackupPreset {
                bank: 0,
                index: 3,
                name: "Clean".into(),
                raw: vec![1, 2, 3],
            }],
            ..Default::default()
        };
        let v = call(
            "backup_show_inline",
            serde_json::json!({"json": backup.to_json()}),
        )
        .await
        .unwrap();
        assert_eq!(v[0]["index"], 3);
        assert_eq!(v[0]["name"], "Clean");
        assert_eq!(v[0]["setlist"], "FACTORY 1");

        let e = call(
            "backup_show_inline",
            serde_json::json!({"json": "not json"}),
        )
        .await
        .unwrap_err();
        assert!(!e.contains("not connected"), "got: {e}");
    }

    #[tokio::test]
    async fn ir_upload_inline_rejects_bad_base64_before_the_device() {
        let e = call(
            "ir_upload_inline",
            serde_json::json!({"slot": 0, "wavBase64": "@@@", "name": "x", "overwrite": false, "force": false}),
        )
        .await
        .unwrap_err();
        assert!(e.starts_with("wav_base64:"), "got: {e}");
    }

    /// Optional args accept absent and null alike.
    #[tokio::test]
    async fn optional_args_default() {
        for args in [serde_json::json!({}), serde_json::json!({"bank": null})] {
            let e = call("list_presets", args).await.unwrap_err();
            assert!(e.contains("not connected"), "got: {e}");
        }
    }
}
