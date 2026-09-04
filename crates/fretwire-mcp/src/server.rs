//! The tool surface. Three routers — read, write, save — composed at startup by what the
//! operator enabled, so an unlisted tool is not merely refused but absent.

use crate::offline::{self, Offline};
use crate::summary;
use fretwire_commands as c;
use fretwire_commands::AppState;
use fretwire_commands::dto::{ParamDto, PresetDto};
use fretwire_commands::events::{Event, EventSink};
use fretwire_core::Session;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

/// What the operator enabled on the command line. See `main.rs`.
#[derive(Clone, Copy, Debug)]
pub struct Gates {
    pub writes: bool,
    pub save: bool,
}

/// Events from the command layer (heartbeat pushes, export progress) have no listener here —
/// the assistant reads fresh state on every call — so they go to the log at debug level.
#[derive(Clone)]
pub struct LogSink;

impl EventSink for LogSink {
    fn emit(&self, event: Event) {
        tracing::debug!(event = event.name(), "{}", event.payload());
    }
}

type R = Result<String, String>;

#[derive(Clone)]
pub struct Fretwire {
    state: Arc<AppState>,
    offline: Arc<Offline>,
    gates: Gates,
    tool_router: ToolRouter<Fretwire>,
}

impl Fretwire {
    pub fn new(gates: Gates) -> Self {
        let mut tool_router = Self::read_tools();
        if gates.writes {
            tool_router += Self::write_tools();
        }
        if gates.save {
            tool_router += Self::save_tools();
        }
        Self {
            state: Arc::new(AppState::default()),
            offline: Arc::new(Offline::default()),
            gates,
            tool_router,
        }
    }

    /// The live session slot, for the heartbeat and the exit teardown.
    pub fn session(&self) -> Arc<Mutex<Option<Session>>> {
        self.state.session.clone()
    }

    async fn catalog_call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&fretwire_core::editor::Catalog) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let offline = self.offline.clone();
        tokio::task::spawn_blocking(move || f(offline.catalog()?))
            .await
            .map_err(|e| format!("task error: {e}"))?
    }

    async fn need_writes(&self) -> Result<(), String> {
        if self.gates.writes {
            Ok(())
        } else {
            Err("this server is read-only; restart fretwire-mcp with --allow-writes".into())
        }
    }
}

// ---------------------------------------------------------------- argument shapes

#[derive(Deserialize, JsonSchema)]
pub struct CatalogModelsArgs {
    /// Category id, from catalog_categories.
    pub category: i64,
    /// "Mono" or "Stereo" to list one variant; omit for both.
    pub variant: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BackupListArgs {
    /// Path to a fretwire export or device backup (.json, written by the editor or `fretwire backup-device`), or an HX Edit / POD Go Edit backup (.hxb / .pgb). `~/` is expanded.
    pub path: String,
}

#[derive(Deserialize, JsonSchema)]
pub struct ModelParamsArgs {
    /// Model index, as catalog_models lists it (what block_add and block_swap take).
    pub index: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct BackupDescribeArgs {
    /// Path to a fretwire export file. `~/` is expanded.
    pub path: String,
    /// Slot within its setlist, as backup_list shows it.
    pub index: i64,
    /// Setlist (bank) index. Defaults to 0, which is the only one on an HX Stomp.
    pub bank: Option<i64>,
    /// Include every block's parameter values. Defaults to true.
    pub params: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BackupDiffArgs {
    /// Export file holding preset A. `~/` is expanded.
    pub path_a: String,
    pub index_a: i64,
    pub bank_a: Option<i64>,
    /// Export file holding preset B. Defaults to path_a.
    pub path_b: Option<String>,
    pub index_b: i64,
    pub bank_b: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PresetReadArgs {
    /// Include every block's parameter values. Defaults to false; use block_params for one block's full detail.
    pub params: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SlotArgs {
    /// The block's slot number, as preset_read lists it.
    pub slot: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct PresetListArgs {
    /// Setlist (bank) index. Defaults to the one the pedal is in.
    pub bank: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BackupExportArgs {
    /// Where to write the export file on this machine (.json). `~/` is expanded.
    pub path: String,
    /// Setlist (bank) indices to export. Defaults to all of them.
    pub banks: Option<Vec<i64>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PresetGotoArgs {
    /// Slot within the setlist, as preset_list shows it.
    pub index: i64,
    /// Setlist (bank) index. Defaults to the one the pedal is in.
    pub bank: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BypassArgs {
    pub slot: i64,
    /// true to bypass the block, false to make it active.
    pub bypassed: bool,
}

#[derive(Deserialize, JsonSchema)]
pub struct ParamSetArgs {
    pub slot: i64,
    /// The parameter's name as block_params lists it (case-insensitive), or its [index].
    pub param: String,
    /// The new value in display units, as HX Edit shows it: a number ("6.5", "-18", "450"), an
    /// option's label for an enum ("421 Dynamic"), "on"/"off" for a switch. Prefix "raw:" to
    /// give the stored value instead.
    pub value: String,
    /// true to address the paired cab/IR of an amp+cab block instead of the amp.
    pub paired: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BlockAddArgs {
    /// The model's index, from catalog_models.
    pub model_index: i64,
    /// For an amp: the cab/IR model index to pair with it (catalog_models gives a suggested one
    /// in the Amp+Cab category). Omit for none.
    pub paired_index: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BlockSwapArgs {
    pub slot: i64,
    /// The new model's index, from catalog_models.
    pub model_index: i64,
    /// The cab/IR to pair, for an amp. Omit to keep the block's current pairing.
    pub paired_index: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SnapshotArgs {
    /// Snapshot number as the pedal shows it, starting at 1.
    pub snapshot: i64,
}

#[derive(Deserialize, JsonSchema)]
pub struct PresetSaveArgs {
    /// Slot to write, within the setlist. Overwrites whatever is there.
    pub slot: i64,
    /// Name to save under. Defaults to the preset's current name.
    pub name: Option<String>,
    /// Setlist (bank) index. Defaults to the one the pedal is in.
    pub bank: Option<i64>,
}

// ---------------------------------------------------------------- read tools

#[tool_router(router = read_tools, vis = "pub")]
impl Fretwire {
    /// Whether the Line 6 reference data (model and parameter names) is imported on this
    /// machine, and where. Without it, presets still decode but blocks and parameters show
    /// numbers instead of names.
    #[tool(annotations(read_only_hint = true))]
    async fn data_status(&self) -> R {
        let s = c::data_status();
        Ok(format!(
            "reference data {}: {} ({} file(s))",
            if s.present {
                "imported"
            } else {
                "NOT imported — run `fretwire import-data`"
            },
            s.dir,
            s.files
        ))
    }

    /// The model catalog's categories (Distortion, Amp, Cab, Delay, …) with the ids
    /// catalog_models takes. Needs no pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn catalog_categories(&self) -> R {
        self.catalog_call(|cat| {
            let mut lines: Vec<String> = cat
                .categories()
                .into_iter()
                .map(|(id, name)| format!("{id:>3}  {name}"))
                .collect();
            lines.push(format!(
                "{:>3}  Amp+Cab (every amp with its suggested cab, paired)",
                fretwire_core::editor::CATEGORY_AMP_CAB
            ));
            Ok(lines.join("\n"))
        })
        .await
    }

    /// The models in one category: index (what block_add / block_swap take), name, DSP cost.
    /// Needs no pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn catalog_models(&self, Parameters(a): Parameters<CatalogModelsArgs>) -> R {
        self.catalog_call(move |cat| {
            let models = cat.models_in_category(a.category, a.variant.as_deref());
            if models.is_empty() {
                return Err(format!("no models in category {}", a.category));
            }
            Ok(models
                .iter()
                .map(|m| {
                    let mut s = format!("{:>4}  {}", m.index, m.name);
                    if let Some(v) = m.variant {
                        s.push_str(&format!(" ({v})"));
                    }
                    if let Some(l) = m.dsp_load {
                        s.push_str(&format!(
                            "  {:.0}% DSP",
                            fretwire_core::editor::dsp_percent(l)
                        ));
                    }
                    if let Some(p) = m.default_paired_index {
                        s.push_str(&format!("  cab {p}"));
                    }
                    s
                })
                .collect::<Vec<_>>()
                .join("\n"))
        })
        .await
    }

    /// One model's parameters at their defaults — names, ranges, options, tempo-sync groups —
    /// before it is on the pedal: what a block of it would take. Needs no pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn model_params(&self, Parameters(a): Parameters<ModelParamsArgs>) -> R {
        self.catalog_call(move |cat| {
            let (name, variant, params) = cat
                .model_params(a.index)
                .ok_or_else(|| format!("no model at index {} — see catalog_models", a.index))?;
            let dtos: Vec<ParamDto> = params.iter().map(ParamDto::from).collect();
            let mut out = name;
            if let Some(v) = variant {
                out.push_str(&format!(" ({v})"));
            }
            out.push_str(&format!(
                " — index {}, {} parameters at their defaults:\n",
                a.index,
                dtos.len()
            ));
            out.push_str(&summary::param_lines(&dtos));
            Ok(out.trim_end().to_string())
        })
        .await
    }

    /// The presets in a fretwire export or device backup, with the setlist and slot each came
    /// from — or in an HX Edit / POD Go Edit backup (.hxb / .pgb), names only. Needs no pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn backup_list(&self, Parameters(a): Parameters<BackupListArgs>) -> R {
        tokio::task::spawn_blocking(move || {
            if let Some((device, items)) = offline::hxb_list(&a.path)? {
                return Ok(format!(
                    "{device} — HX Edit backup, {} preset(s) (names only; convert with \
                     `fretwire hxb-convert` to describe them)\n{}",
                    items.len(),
                    summary::preset_list(&items)
                ));
            }
            let b = offline::read_backup(&a.path)?;
            let items: Vec<_> = b
                .presets
                .iter()
                .map(|p| fretwire_commands::dto::PresetListItem {
                    label: None,
                    index: p.index,
                    bank: p.bank,
                    setlist: b
                        .setlists
                        .iter()
                        .find(|(bank, _)| *bank == p.bank)
                        .map(|(_, n)| n.clone()),
                    name: p.name.clone(),
                })
                .collect();
            // A device backup (format v3) carries the IR store and the global settings too;
            // say so, since these tools read only the presets out of it.
            let extra = if b.is_presets_only() {
                String::new()
            } else {
                format!(", {} IR(s), {} setting(s)", b.irs.len(), b.settings.len())
            };
            Ok(format!(
                "{} — {} preset(s){extra}\n{}",
                b.device,
                items.len(),
                summary::preset_list(&items)
            ))
        })
        .await
        .map_err(|e| format!("task error: {e}"))?
    }

    /// One preset from a fretwire export file: its blocks in signal order with their settings.
    /// Needs no pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn backup_describe(&self, Parameters(a): Parameters<BackupDescribeArgs>) -> R {
        self.catalog_call(move |cat| {
            let b = offline::read_backup(&a.path)?;
            let p = offline::backup_preset(cat, &b, a.bank.unwrap_or(0), a.index)?;
            Ok(summary::preset_summary(&p, a.params.unwrap_or(true)))
        })
        .await
    }

    /// What differs between two presets in export files (the same file or two): blocks added,
    /// removed or swapped, bypass changes, parameter values that moved. Needs no pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn backup_diff(&self, Parameters(a): Parameters<BackupDiffArgs>) -> R {
        self.catalog_call(move |cat| {
            let ba = offline::read_backup(&a.path_a)?;
            let pa = offline::backup_preset(cat, &ba, a.bank_a.unwrap_or(0), a.index_a)?;
            let pb = match &a.path_b {
                Some(path_b) if path_b.trim() != a.path_a.trim() => {
                    let bb = offline::read_backup(path_b)?;
                    offline::backup_preset(cat, &bb, a.bank_b.unwrap_or(0), a.index_b)?
                }
                _ => offline::backup_preset(cat, &ba, a.bank_b.unwrap_or(0), a.index_b)?,
            };
            Ok(summary::preset_diff(&pa, &pb))
        })
        .await
    }

    /// Whether an HX device is plugged in, and whether this server holds a session with it.
    #[tool(annotations(read_only_hint = true))]
    async fn device_status(&self) -> R {
        let found = c::detect().await?;
        let mut out = if found.is_empty() {
            "No HX device found on USB.".to_string()
        } else {
            found
                .iter()
                .map(|d| match &d.caveat {
                    Some(cv) => format!("{} present ({cv})", d.name),
                    None => format!("{} present", d.name),
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        if c::is_connected(&self.state) {
            let p = c::read_preset(&self.state).await?;
            out.push_str(&format!(
                "\nConnected. Current preset: \"{}\"{}",
                p.name.as_deref().unwrap_or("?"),
                if p.dirty { " (unsaved edits)" } else { "" }
            ));
        } else {
            out.push_str("\nNot connected — device_connect opens a session.");
        }
        Ok(out)
    }

    /// Open the editing session with the pedal (like HX Edit connecting) and read the current
    /// preset. Reads only; the pedal stays playable. Idempotent.
    #[tool(annotations(read_only_hint = true))]
    async fn device_connect(&self) -> R {
        let p = c::connect(&self.state).await?;
        Ok(summary::preset_summary(&p, false))
    }

    /// Close the session, returning the pedal to standalone. Unsaved edits stay in the pedal's
    /// edit buffer until it changes preset or powers off.
    #[tool(annotations(read_only_hint = true))]
    async fn device_disconnect(&self) -> R {
        c::disconnect(&self.state).await?;
        Ok("Disconnected.".into())
    }

    /// The preset currently loaded on the pedal: blocks in signal order, bypass state, snapshots,
    /// DSP headroom. Pass params=true for every parameter value.
    #[tool(annotations(read_only_hint = true))]
    async fn preset_read(&self, Parameters(a): Parameters<PresetReadArgs>) -> R {
        let p = c::read_preset(&self.state).await?;
        Ok(summary::preset_summary(&p, a.params.unwrap_or(false)))
    }

    /// One block of the current preset in full: every parameter with its value, range or
    /// options, and index — what param_set needs.
    #[tool(annotations(read_only_hint = true))]
    async fn block_params(&self, Parameters(a): Parameters<SlotArgs>) -> R {
        let p = c::read_preset(&self.state).await?;
        let b = find_block(&p, a.slot)?;
        Ok(summary::block_params(b))
    }

    /// The presets in a setlist on the pedal.
    #[tool(annotations(read_only_hint = true))]
    async fn preset_list(&self, Parameters(a): Parameters<PresetListArgs>) -> R {
        let items = c::list_presets(&self.state, a.bank).await?;
        Ok(summary::preset_list(&items))
    }

    /// The pedal's setlists (banks), in index order. One on an HX Stomp, eight on a Helix Floor.
    #[tool(annotations(read_only_hint = true))]
    async fn setlists(&self) -> R {
        let names = c::setlists(&self.state).await?;
        Ok(names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{i}  {n}"))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Export presets from the pedal to a fretwire export file on this machine — do this before
    /// a write session. Reads the pedal only (it steps through every preset, about a second each;
    /// audio follows along) and reloads the current preset afterwards, dropping unsaved edits.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn backup_export(&self, Parameters(a): Parameters<BackupExportArgs>) -> R {
        let banks = match a.banks {
            Some(b) if !b.is_empty() => b,
            _ => {
                let n = c::setlists(&self.state).await?.len() as i64;
                (0..n.max(1)).collect()
            }
        };
        let count = c::export_setlists(&self.state, LogSink, a.path.clone(), banks).await?;
        Ok(format!("Exported {count} preset(s) to {}", a.path))
    }
}

// ---------------------------------------------------------------- write tools (--allow-writes)

#[tool_router(router = write_tools, vis = "pub")]
impl Fretwire {
    /// Load a preset on the pedal. Unsaved edits to the current one are lost.
    #[tool(annotations(read_only_hint = false, destructive_hint = true))]
    async fn preset_goto(&self, Parameters(a): Parameters<PresetGotoArgs>) -> R {
        self.need_writes().await?;
        let bank = match a.bank {
            Some(b) => b,
            None => current_bank(&self.state).await?,
        };
        let p = c::goto_preset(&self.state, bank, a.index).await?;
        Ok(summary::preset_summary(&p, false))
    }

    /// Bypass or activate a block. An edit-buffer change: audible now, not saved.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn block_bypass(&self, Parameters(a): Parameters<BypassArgs>) -> R {
        self.need_writes().await?;
        let p = c::set_bypass(&self.state, a.slot, a.bypassed).await?;
        let b = find_block(&p, a.slot)?;
        Ok(format!("OK: {}", summary::block_line(b)))
    }

    /// Set one parameter of a block, in display units (see block_params for names, ranges and
    /// options). An edit-buffer change: audible now, not saved.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn param_set(&self, Parameters(a): Parameters<ParamSetArgs>) -> R {
        self.need_writes().await?;
        let before = c::read_preset(&self.state).await?;
        let block = find_block(&before, a.slot)?;
        let paired = a.paired.unwrap_or(false);
        let list = if paired {
            &block.paired_params
        } else {
            &block.params
        };
        if list.is_empty() {
            return Err(format!(
                "slot {} ({}) has no {}parameters",
                a.slot,
                block.model_name,
                if paired { "paired cab " } else { "" }
            ));
        }
        let p = find_param(list, &a.param)?;
        if !p.settable {
            return Err(format!("{} is read-only", p.name));
        }
        let idx = p.index as i64;
        let value = a.value.trim();

        // An enum takes a label (or a raw option number).
        if !p.enum_labels.is_empty() {
            let v = if let Some(pos) = p
                .enum_labels
                .iter()
                .position(|l| l.eq_ignore_ascii_case(value))
            {
                p.enum_base + pos as i64
            } else if let Some(n) = value
                .strip_prefix("raw:")
                .and_then(|r| r.trim().parse().ok())
            {
                n
            } else {
                return Err(format!(
                    "{} takes one of: {}",
                    p.name,
                    p.enum_labels.join(" | ")
                ));
            };
            let after = c::set_param_enum(&self.state, a.slot, paired, idx, v).await?;
            return Ok(report(&after, a.slot, paired, idx));
        }

        let raw: f64 = if let Some(b) = parse_switch(value) {
            if b { 1.0 } else { 0.0 }
        } else if let Some(r) = value.strip_prefix("raw:") {
            r.trim()
                .parse()
                .map_err(|_| format!("raw value {:?} is not a number", r.trim()))?
        } else {
            let (shown, unit) = leading_number(value)
                .ok_or_else(|| format!("value {value:?} is not a number for {}", p.name))?;
            let mut raw = summary::raw_from_display(p, shown, unit);
            if let (Some(lo), Some(hi)) = (p.min, p.max) {
                raw = raw.clamp(lo.min(hi), hi.max(lo));
            }
            raw
        };
        let after = if paired {
            c::set_paired_param(&self.state, a.slot, idx, raw as f32).await?
        } else {
            c::set_param(&self.state, a.slot, idx, raw as f32).await?
        };
        Ok(report(&after, a.slot, paired, idx))
    }

    /// Add a block (a model from catalog_models) at the end of the chain. An edit-buffer change.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn block_add(&self, Parameters(a): Parameters<BlockAddArgs>) -> R {
        self.need_writes().await?;
        let p = c::add_block(&self.state, a.model_index, a.paired_index.unwrap_or(-1)).await?;
        Ok(summary::preset_summary(&p, false))
    }

    /// Replace the model in a slot, keeping its position. An edit-buffer change.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn block_swap(&self, Parameters(a): Parameters<BlockSwapArgs>) -> R {
        self.need_writes().await?;
        let paired = match a.paired_index {
            Some(i) => i,
            None => {
                let cur = c::read_preset(&self.state).await?;
                find_block(&cur, a.slot)?.paired_index.unwrap_or(-1)
            }
        };
        let p = c::swap_model(&self.state, a.slot, a.model_index, paired).await?;
        let b = find_block(&p, a.slot)?;
        Ok(format!("OK: {}", summary::block_line(b)))
    }

    /// Remove a block from the preset. An edit-buffer change; undo brings it back.
    #[tool(annotations(read_only_hint = false, destructive_hint = true))]
    async fn block_delete(&self, Parameters(a): Parameters<SlotArgs>) -> R {
        self.need_writes().await?;
        let p = c::delete_block(&self.state, a.slot).await?;
        Ok(summary::preset_summary(&p, false))
    }

    /// Switch snapshot (1-based, as the pedal shows them).
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn snapshot_select(&self, Parameters(a): Parameters<SnapshotArgs>) -> R {
        self.need_writes().await?;
        let p = c::set_snapshot(&self.state, a.snapshot - 1).await?;
        Ok(summary::preset_summary(&p, false))
    }

    /// Undo the last edit-buffer change made through this session.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn undo(&self) -> R {
        self.need_writes().await?;
        let p = c::undo(&self.state).await?;
        Ok(format!(
            "Undone; {} step(s) left to undo.\n{}",
            p.undo_depth,
            summary::preset_summary(&p, false)
        ))
    }

    /// Redo the last undone change.
    #[tool(annotations(read_only_hint = false, destructive_hint = false))]
    async fn redo(&self) -> R {
        self.need_writes().await?;
        let p = c::redo(&self.state).await?;
        Ok(summary::preset_summary(&p, false))
    }

    /// Throw away every unsaved edit by reloading the preset from flash.
    #[tool(annotations(read_only_hint = false, destructive_hint = true))]
    async fn preset_revert(&self) -> R {
        self.need_writes().await?;
        let p = c::revert_preset(&self.state).await?;
        Ok(summary::preset_summary(&p, false))
    }
}

// ---------------------------------------------------------------- save tools (--allow-save)

#[tool_router(router = save_tools, vis = "pub")]
impl Fretwire {
    /// Write the edit buffer to a preset slot in flash — permanent, and it overwrites what was
    /// there. Export first (backup_export) if that slot matters.
    #[tool(annotations(read_only_hint = false, destructive_hint = true))]
    async fn preset_save(&self, Parameters(a): Parameters<PresetSaveArgs>) -> R {
        if !self.gates.save {
            return Err("saving is disabled; restart fretwire-mcp with --allow-save".into());
        }
        let cur = c::read_preset(&self.state).await?;
        let bank = match a.bank {
            Some(b) => b,
            None => cur.bank.unwrap_or(0),
        };
        let name = a
            .name
            .or(cur.name)
            .filter(|n| !n.trim().is_empty())
            .ok_or("the preset has no name; pass one")?;
        let p = c::save_preset(&self.state, bank, a.slot, name).await?;
        Ok(format!(
            "Saved to setlist {bank} slot {}.\n{}",
            a.slot,
            summary::preset_summary(&p, false)
        ))
    }
}

#[tool_handler(
    router = self.tool_router,
    name = "fretwire",
    instructions = "fretwire edits a Line 6 HX Stomp (and other HX devices) over USB. Tools that \
                    need no pedal read fretwire export files and the model catalog; the rest talk \
                    to the connected device. Parameters are set in display units, as HX Edit \
                    shows them. Nothing changes the pedal's flash unless a preset_save tool is \
                    listed and called; edit-buffer changes are audible immediately and are \
                    discarded by a preset change or power cycle. Before editing a preset the \
                    user cares about, export it (backup_export)."
)]
impl ServerHandler for Fretwire {}

// ---------------------------------------------------------------- helpers

fn find_block(p: &PresetDto, slot: i64) -> Result<&fretwire_commands::dto::BlockDto, String> {
    p.blocks
        .iter()
        .find(|b| b.slot == slot && !b.is_controller)
        .ok_or_else(|| {
            let have: Vec<String> = p
                .blocks
                .iter()
                .filter(|b| !b.is_controller)
                .map(|b| format!("{} ({})", b.slot, b.model_name))
                .collect();
            format!("no block in slot {slot}; slots in use: {}", have.join(", "))
        })
}

/// By `[index]`, bare index, or case-insensitive name.
fn find_param<'a>(list: &'a [ParamDto], key: &str) -> Result<&'a ParamDto, String> {
    let key = key.trim();
    let by_index = key
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse::<usize>()
        .ok()
        .and_then(|i| list.iter().find(|p| p.index == i));
    by_index
        .or_else(|| list.iter().find(|p| p.name.eq_ignore_ascii_case(key)))
        .or_else(|| {
            // A unique prefix / substring, for "thresh" → "Threshold".
            let hits: Vec<&ParamDto> = list
                .iter()
                .filter(|p| {
                    p.name
                        .to_ascii_lowercase()
                        .contains(&key.to_ascii_lowercase())
                })
                .collect();
            if hits.len() == 1 { Some(hits[0]) } else { None }
        })
        .ok_or_else(|| {
            format!(
                "no parameter {key:?}; this block has: {}",
                list.iter()
                    .map(|p| format!("[{}] {}", p.index, p.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
}

fn parse_switch(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" | "enabled" => Some(true),
        "off" | "false" | "no" | "disabled" => Some(false),
        _ => None,
    }
}

/// The number at the front of "6.5", "-18 dB", "450ms", "+3.0", and whatever unit follows it.
fn leading_number(v: &str) -> Option<(f64, &str)> {
    let end = v
        .char_indices()
        .take_while(|(i, ch)| {
            ch.is_ascii_digit() || *ch == '.' || (*i == 0 && matches!(ch, '-' | '+'))
        })
        .map(|(i, ch)| i + ch.len_utf8())
        .last()?;
    Some((v[..end].parse().ok()?, v[end..].trim()))
}

fn report(after: &PresetDto, slot: i64, paired: bool, idx: i64) -> String {
    let Ok(b) = find_block(after, slot) else {
        return summary::preset_summary(after, false);
    };
    let list = if paired { &b.paired_params } else { &b.params };
    match list.iter().find(|p| p.index as i64 == idx) {
        Some(p) => format!(
            "OK: {} {}{} = {}",
            b.model_name,
            if paired { "cab " } else { "" },
            p.name,
            summary::param_value(p)
        ),
        None => summary::preset_summary(after, false),
    }
}

async fn current_bank(state: &AppState) -> Result<i64, String> {
    Ok(c::read_preset(state).await?.bank.unwrap_or(0))
}
