//! The signal chain: the preset's blocks drawn as a horizontal path of clickable boxes connected
//! by wires (row A on top; the parallel row B below for split presets). Built from plain iced
//! widgets — `mouse_area`-wrapped containers — rather than a `canvas`, because the software
//! (tiny-skia) renderer doesn't stroke canvas paths reliably (wires/borders vanished). Clicking a
//! block selects it (drives the param panel in `main`); pressing and dragging it drops it into a
//! **gap** between blocks (the wires and the leading/trailing edges are the drop targets), which
//! reorders the chain — inserting, not replacing.

use crate::Message;
use fretwire_core::{EditorBlock, EditorPreset};
use iced::widget::{column, container, horizontal_rule, mouse_area, row, scrollable, text};
use iced::{Alignment, Background, Border, Element, Length, Theme};

const CHIP_W: f32 = 124.0;
const WIRE_W: f32 = 22.0;
const NODE_W: f32 = 92.0;

/// Drag state from the app: `(src_slot, target)` — the block being dragged and the insertion gap
/// under the cursor as `(row, pos)` (row 0 = series/A, 1 = parallel/B; `pos` = blocks before it).
/// `None` = no drag.
pub type Drag = Option<(i64, Option<(u8, usize)>)>;

/// Build the chain for a preset. Controllers (footswitch assignments, not DSP blocks) are omitted.
/// Wrapped in a horizontal scroll so a long chain pans instead of clipping. A surrounding
/// `mouse_area` catches a drag released off any target so a drop never gets stuck.
pub fn view(
    p: &EditorPreset,
    selected: Option<i64>,
    drag: Drag,
    moved: bool,
) -> Element<'static, Message> {
    let row_a = ordered_row(p, 0);
    let row_b = ordered_row(p, 1);
    let (src, gap) = (drag.map(|d| d.0), drag.and_then(|d| d.1));
    let dragging = src.is_some();
    // Show row B on a split preset (so the split/mixer nodes and the parallel branch are visible), or
    // once a drag has actually *moved* (so a serial preset gets a parallel drop target). Gating on
    // movement, not mere press, avoids a B-row flash on a click.
    let show_b = p.split || moved;

    // The split & mixer routing nodes, drawn as selectable (non-draggable) chips that **bracket the
    // parallel (B) row**: the Split chip is the branch-in point (before the B blocks), the Mixer the
    // branch-out (after them). Row A stays the common/main path. Clicking a chip selects it for
    // editing in the param panel.
    let split_chip = p.split.then(|| p.split_node.as_ref()).flatten().map(|sp| {
        node_chip(sp, "⋔ Split", split_type_label(sp), selected == Some(sp.slot))
    });
    let mixer_chip = p
        .split
        .then(|| p.mixer_node.as_ref())
        .flatten()
        .map(|mx| node_chip(mx, "⋉ Mixer", None, selected == Some(mx.slot)));

    // Gaps become active+wider only once a real drag is underway (`moved`), not on a mere press, so
    // a click doesn't change the row's width.
    let body: Element<'static, Message> = if p.split {
        // Top row: IN [common-before] ⋔Split [path A] ⋉Mixer [common-after] OUT — the split/mixer
        // chips placed at their signal-flow columns (split_pos/mixer_pos). Bottom row = path B,
        // indented so it sits under the parallel section (between the Split and Mixer chips).
        let sp = p.split_pos.unwrap_or(i64::MAX);
        let mp = p.mixer_pos.unwrap_or(i64::MAX);
        let n_before = row_a.iter().filter(|b| b.slot < sp).count();
        let top =
            split_top_row(&row_a, selected, src, gap, moved, sp, mp, split_chip, mixer_chip);
        // Approx width of IN + the common-before chips + the Split chip, to align path B under path A.
        let indent = 156.0 + 146.0 * n_before as f32;
        let b_lead: Vec<Element<'static, Message>> = vec![spacer(indent)];
        column![
            labeled_row("A", top),
            labeled_row("B", path_row(&row_b, 1, selected, src, gap, moved, false, b_lead, Vec::new())),
        ]
        .spacing(10)
        .into()
    } else if show_b {
        // Serial preset mid-drag: an empty B row appears as a parallel drop target.
        column![
            labeled_row("A", path_row(&row_a, 0, selected, src, gap, moved, true, Vec::new(), Vec::new())),
            labeled_row("B", path_row(&row_b, 1, selected, src, gap, moved, false, Vec::new(), Vec::new())),
        ]
        .spacing(10)
        .into()
    } else {
        path_row(&row_a, 0, selected, src, gap, moved, true, Vec::new(), Vec::new())
    };

    let scroller = scrollable(container(body).padding([8, 4]))
        .direction(scrollable::Direction::Horizontal(scrollable::Scrollbar::new()));

    // The surrounding mouse_area catches a drop off any target. While a drag is live it also reports
    // cursor movement, which promotes a press into a real drag (revealing row B) — so a plain click
    // (press+release, no movement) never flashes the parallel row.
    let mut area = mouse_area(scroller).on_release(Message::ChipReleased);
    if dragging {
        area = area.on_move(Message::DragMoved);
    }
    container(area).width(Length::Fill).into()
}

/// A single signal path: `IN ─[chip]─[chip]─ OUT` with a drop gap before every chip and after the
/// last (the connector wires). `gaps` enables the gaps as drop targets (main row, during a drag).
fn path_row(
    blocks: &[&EditorBlock],
    row_id: u8,
    selected: Option<i64>,
    src: Option<i64>,
    active_gap: Option<(u8, usize)>,
    gaps: bool,
    main: bool,
    leading: Vec<Element<'static, Message>>,
    trailing: Vec<Element<'static, Message>>,
) -> Element<'static, Message> {
    // If the dragged block is in THIS row, the gaps flanking its own position are no-ops, so don't
    // highlight them. (When it's in the other row, every gap here is a valid cross-row target.)
    let src_pos = src.and_then(|ss| blocks.iter().position(|b| b.slot == ss));
    let is_noop = |pos: usize| matches!(src_pos, Some(fp) if pos == fp || pos == fp + 1);
    let active = |pos: usize| active_gap == Some((row_id, pos)) && !is_noop(pos);

    let mut r = row![terminal(if main { "IN" } else { "↳" })].align_y(Alignment::Center).spacing(0);
    // Leading routing node (the Split chip on the parallel row) — the branch-in point.
    for node in leading {
        r = r.push(node);
    }
    for (i, b) in blocks.iter().enumerate() {
        r = r.push(gap(row_id, i, active(i), gaps));
        r = r.push(chip(b, row_id, i, selected == Some(b.slot), src, gaps));
    }
    let last = blocks.len();
    r = r.push(gap(row_id, last, active(last), gaps));
    // Trailing routing node (the Mixer chip on the parallel row) — the branch-out point.
    for node in trailing {
        r = r.push(node);
    }
    r = r.push(terminal(if main { "OUT" } else { "" }));
    r.into()
}

/// The top (A/common) row of a split preset: `IN [common-before] ⋔Split [path A] ⋉Mixer
/// [common-after] OUT`. The Split chip is inserted before the first block at/after `split_pos`, the
/// Mixer before the first block at/after `mixer_pos`; blocks between them are path A, blocks before
/// `split_pos` are common (pre-split), blocks at/after `mixer_pos` are common (post-mixer). If a
/// section is empty the chip simply lands at the boundary (e.g. both chips before OUT).
#[allow(clippy::too_many_arguments)]
fn split_top_row(
    blocks: &[&EditorBlock],
    selected: Option<i64>,
    src: Option<i64>,
    active_gap: Option<(u8, usize)>,
    gaps: bool,
    split_pos: i64,
    mixer_pos: i64,
    mut split_chip: Option<Element<'static, Message>>,
    mut mixer_chip: Option<Element<'static, Message>>,
) -> Element<'static, Message> {
    let src_pos = src.and_then(|ss| blocks.iter().position(|b| b.slot == ss));
    let is_noop = |pos: usize| matches!(src_pos, Some(fp) if pos == fp || pos == fp + 1);
    let active = |pos: usize| active_gap == Some((0, pos)) && !is_noop(pos);
    // Sentinel "row 2, pos 0" = the drop zone immediately before the Split chip (common-before end).
    let active_before_split = active_gap == Some((2, 0));

    let mut r = row![terminal("IN")].align_y(Alignment::Center).spacing(0);
    for (i, b) in blocks.iter().enumerate() {
        // Drop the Split/Mixer chip in as soon as the running slot reaches its column.
        if split_chip.is_some() && b.slot >= split_pos {
            r = r.push(gap(2, 0, active_before_split, gaps)); // drop-before-split target
            r = r.push(split_chip.take().unwrap());
        }
        if mixer_chip.is_some() && b.slot >= mixer_pos {
            r = r.push(gap(0, i, false, false));
            r = r.push(mixer_chip.take().unwrap());
        }
        r = r.push(gap(0, i, active(i), gaps));
        r = r.push(chip(b, 0, i, selected == Some(b.slot), src, gaps));
    }
    let last = blocks.len();
    // Any chip whose section had no blocks lands here.
    if let Some(c) = split_chip.take() {
        r = r.push(gap(2, 0, active_before_split, gaps)); // drop-before-split target
        r = r.push(c);
    }
    if let Some(c) = mixer_chip.take() {
        r = r.push(gap(0, last, false, false));
        r = r.push(c);
    }
    // The interactive end gap sits **after** the trailing nodes, so there's a drop target *past* the
    // Mixer (the common-after region) — dropping there lands the block after the mixer, matching where
    // it was dropped, instead of the pre-mixer gap silently placing it after.
    r = r.push(gap(0, last, active(last), gaps));
    r = r.push(terminal("OUT"));
    r.into()
}

/// A fixed-width invisible spacer (used to indent the parallel row under path A).
fn spacer(w: f32) -> Element<'static, Message> {
    container(text("")).width(Length::Fixed(w)).into()
}

/// Prefix a path row with a small A/B row label (for split presets).
fn labeled_row(tag: &str, path: Element<'static, Message>) -> Element<'static, Message> {
    row![
        container(text(tag.to_string()).size(13)).width(Length::Fixed(16.0)),
        path,
    ]
    .align_y(Alignment::Center)
    .spacing(4)
    .into()
}

/// How a chip should be coloured, in priority order.
#[derive(Clone, Copy)]
enum ChipState {
    Source,   // the block being dragged
    Selected, // the focused block (param panel)
    Bypassed, // an active-but-bypassed block
    Normal,   // a normal active block
}

/// One block box: name + a sub-line (variant / cab / slot), styled by state. Wrapped in a
/// `mouse_area` so a press starts a drag and entering it while dragging marks the gap *before* it as
/// the drop target; a release either drops (reorder) or — if nothing moved — selects (in `main`).
fn chip(
    b: &EditorBlock,
    row_id: u8,
    pos: usize,
    selected: bool,
    src: Option<i64>,
    gaps: bool,
) -> Element<'static, Message> {
    let bypassed = b.bypassed == Some(true);
    let variant = b.variant.map(|v| format!(" {v}")).unwrap_or_default();
    let sub = if b.paired_model_name.is_some() {
        format!("+ cab{variant}")
    } else if !variant.is_empty() {
        variant.trim().to_string()
    } else {
        format!("slot {}", b.slot)
    };

    let label = column![text(b.model_name.clone()).size(13), text(sub).size(10)]
        .spacing(2)
        .align_x(Alignment::Center)
        .width(Length::Fill);

    let state = if src == Some(b.slot) {
        ChipState::Source
    } else if selected {
        ChipState::Selected
    } else if bypassed {
        ChipState::Bypassed
    } else {
        ChipState::Normal
    };

    let chip = container(label)
        .width(Length::Fixed(CHIP_W))
        .padding([8, 6])
        .style(chip_style(state));

    let mut area = mouse_area(chip).on_press(Message::ChipPressed(b.slot)).on_release(Message::ChipReleased);
    // Hovering a chip during a drag = "insert before this block" (its gap index, in this row).
    if gaps {
        area = area.on_enter(Message::GapEntered(row_id, pos));
    }
    area.into()
}

/// A split/mixer routing node as a compact, selectable (non-draggable) chip. `subtitle` shows the
/// split type when known. Clicking selects it → the param panel edits its type/params.
fn node_chip(
    b: &EditorBlock,
    title: &str,
    subtitle: Option<String>,
    selected: bool,
) -> Element<'static, Message> {
    let mut label = column![text(title.to_string()).size(13)].spacing(2).align_x(Alignment::Center);
    if let Some(sub) = subtitle {
        label = label.push(text(sub).size(10));
    }
    let node = container(label.width(Length::Fill))
        .width(Length::Fixed(NODE_W))
        .padding([8, 6])
        .style(move |theme: &Theme| {
            let p = theme.extended_palette();
            let (bg, border) = if selected {
                (p.primary.base.color, p.primary.strong.color)
            } else {
                (p.background.strong.color, p.background.strong.text)
            };
            container::Style {
                background: Some(Background::Color(bg)),
                text_color: Some(if selected { p.primary.base.text } else { p.background.strong.text }),
                border: Border { color: border, width: 1.0, radius: 6.0.into() },
                ..container::Style::default()
            }
        });
    mouse_area(node).on_press(Message::SelectBlock(b.slot)).into()
}

/// The human split-type label (`Y`/`A/B`/…) for a split node, from its resolved symbol.
fn split_type_label(split: &EditorBlock) -> Option<String> {
    let sym = split.symbolic_id.as_deref()?;
    fretwire_core::editor::SPLIT_TYPES.iter().find(|(_, s, _)| *s == sym).map(|(_, _, label)| label.to_string())
}

/// The container style for a chip in a given [`ChipState`], resolved against the active theme.
fn chip_style(state: ChipState) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let p = theme.extended_palette();
        let (bg, fg, border, width) = match state {
            ChipState::Source => {
                (p.background.weak.color, p.background.weak.text, p.primary.base.color, 2.0)
            }
            ChipState::Selected => {
                (p.primary.base.color, p.primary.base.text, p.primary.strong.color, 1.0)
            }
            ChipState::Bypassed => {
                (p.secondary.base.color, p.secondary.base.text, p.secondary.strong.color, 1.0)
            }
            ChipState::Normal => {
                (p.success.base.color, p.success.base.text, p.success.strong.color, 1.0)
            }
        };
        container::Style {
            background: Some(Background::Color(bg)),
            text_color: Some(fg),
            border: Border { color: border, width, radius: 6.0.into() },
            ..container::Style::default()
        }
    }
}

/// A connector wire that doubles as a drop **gap** (insertion point `pos` in `row_id`). When
/// `active`, it shows a highlighted insertion bar; while a drag is live (`enabled`) it widens to an
/// easy-to-hit target and reports `GapEntered(row_id, pos)` on hover.
fn gap(row_id: u8, pos: usize, active: bool, enabled: bool) -> Element<'static, Message> {
    // Widen the hit area during a drag so drops (incl. an empty parallel row) are easy to land.
    let w = if enabled { 40.0 } else { WIRE_W };
    let bar: Element<'static, Message> = if active {
        // A bold accent marking where the block will land.
        container(text("").size(1))
            .width(Length::Fixed(w))
            .height(Length::Fixed(34.0))
            .style(|theme: &Theme| {
                let p = theme.extended_palette();
                container::Style {
                    background: Some(Background::Color(p.primary.strong.color)),
                    border: Border { radius: 3.0.into(), ..Border::default() },
                    ..container::Style::default()
                }
            })
            .into()
    } else {
        container(horizontal_rule(2)).width(Length::Fixed(w)).align_y(Alignment::Center).into()
    };
    if enabled {
        mouse_area(bar)
            .on_enter(Message::GapEntered(row_id, pos))
            .on_release(Message::ChipReleased)
            .into()
    } else {
        bar
    }
}

/// An IN/OUT terminal label.
fn terminal(s: &str) -> Element<'static, Message> {
    container(text(s.to_string()).size(12)).into()
}

/// Blocks on `row`, in slot order, excluding controller nodes.
fn ordered_row(p: &EditorPreset, row: u8) -> Vec<&EditorBlock> {
    let mut v: Vec<&EditorBlock> =
        p.blocks.iter().filter(|b| !b.is_controller && b.row == row).collect();
    v.sort_by_key(|b| b.slot);
    v
}
