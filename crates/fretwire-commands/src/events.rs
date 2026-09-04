//! The event surface — everything the backend pushes to the frontend without being asked.
//!
//! Three events, two producers (the heartbeat and the export sweep). Each transport implements
//! [`EventSink`] over its own delivery mechanism: Tauri emits on the `AppHandle`, a server would
//! fan out over a WebSocket. The wire name and JSON payload live *here*, in [`Event::name`] and
//! [`Event::payload`], so transports cannot drift apart on shape — the frontend's `listen()`
//! handlers (`App.svelte`) read exactly these.

use crate::dto::PushDto;

/// A backend-originated event, carrying its payload as typed data. [`Event::payload`] is the one
/// place the wire shape is decided.
pub enum Event {
    /// The heartbeat gave up on the device and dropped the session. Payload is the plain
    /// user-facing message string.
    DeviceLost(&'static str),
    /// Device-originated changes (footswitch bypass, panel snapshot/preset switch) drained by the
    /// heartbeat, so the GUI follows the hardware live.
    DevicePushes(Vec<PushDto>),
    /// Export-sweep progress, one per preset read (see `Session::export_setlists`).
    BackupProgress {
        done: usize,
        total: usize,
        /// `"presets"`, `"irs"` or `"settings"` — which part of the device the item belongs to.
        stage: &'static str,
        bank: i64,
        setlist: String,
        name: String,
    },
}

impl Event {
    /// The event name as the frontend subscribes to it.
    pub fn name(&self) -> &'static str {
        match self {
            Event::DeviceLost(_) => "device-lost",
            Event::DevicePushes(_) => "device-pushes",
            Event::BackupProgress { .. } => "backup-progress",
        }
    }

    /// The JSON payload the frontend's handler receives as `e.payload`.
    pub fn payload(&self) -> serde_json::Value {
        match self {
            Event::DeviceLost(msg) => serde_json::Value::from(*msg),
            Event::DevicePushes(dtos) => {
                serde_json::to_value(dtos).expect("PushDto serialization is infallible")
            }
            Event::BackupProgress {
                done,
                total,
                stage,
                bank,
                setlist,
                name,
            } => serde_json::json!({
                "done": done, "total": total, "stage": stage,
                "bank": bank, "setlist": setlist, "name": name,
            }),
        }
    }
}

/// A transport's delivery half. `emit` is synchronous and must not block meaningfully — the
/// heartbeat calls it from its own thread between beats, and the export sweep calls it while
/// holding the session lock.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: Event);
}
