//! Top-bar chips for the MCP agents that are working, and the busy-page dot.
//!
//! A chip names the agent and pulses while it works. Hovering lists the frames it's on;
//! clicking brings them into view (switching page if needed). Agents never move the view
//! themselves, so this is how the user goes to look.

use crate::agents::AgentId;
use crate::editor::Editor;
use crate::render::scene::{AGENT, agent_pulse};
use egui::{Color32, Ui};
use std::time::Duration;

/// Agent color at `alpha` (0..=1).
pub fn agent_color(alpha: f64) -> Color32 {
    let c = super::c32(AGENT);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (alpha.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Draw a chip per working agent (call inside a right-to-left layout).
pub fn chips(ui: &mut Ui, ed: &mut Editor) {
    let now = ed.now_ms;
    let agents: Vec<(AgentId, String)> = ed
        .agents
        .active(now)
        .map(|(id, a)| (id, a.label()))
        .collect();
    if agents.is_empty() {
        return;
    }
    ui.ctx().request_repaint_after(Duration::from_millis(33));
    let p = crate::theme::pal(ui);
    for (id, label) in agents.into_iter().rev() {
        let font = egui::FontId::proportional(crate::theme::SECONDARY);
        let g = ui.painter().layout_no_wrap(label.clone(), font, p.text);
        let size = egui::vec2(g.size().x + 26.0, 22.0);
        let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
        let fill = if resp.hovered() { p.hover } else { p.control };
        ui.painter().rect_filled(rect, 11.0, fill);
        let dot = egui::pos2(rect.left() + 11.0, rect.center().y);
        let pulse = agent_pulse(now);
        ui.painter()
            .circle_filled(dot, 6.0, agent_color(0.25 * pulse));
        ui.painter().circle_filled(dot, 3.5, agent_color(1.0));
        ui.painter().galley(
            egui::pos2(rect.left() + 20.0, rect.center().y - g.size().y / 2.0),
            g,
            Color32::PLACEHOLDER,
        );
        let where_ = working_on(ed, id);
        let resp = resp.on_hover_text(format!("{label} is working on {where_}. Click to show."));
        if resp.clicked() {
            show(ed, id);
        }
        ui.add_space(4.0);
    }
}

/// "Hero, Footer" on this page, or the other page's name.
fn working_on(ed: &Editor, agent: AgentId) -> String {
    let now = ed.now_ms;
    let page = ed.active_page_id().to_string();
    let mut names: Vec<String> = Vec::new();
    let mut other: Option<String> = None;
    for (p, n) in ed.agents.touches_of(agent, now) {
        if p == page {
            if let Some(top) = crate::agents::top_level(&ed.doc, n) {
                let name = ed.doc.get(top).map(|n| n.name.clone()).unwrap_or_default();
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        } else if other.is_none() {
            other = ed.pages.iter().find(|s| s.id == p).map(|s| s.name.clone());
        }
    }
    match (names.len(), other) {
        (0, Some(o)) => format!("page {o}"),
        (0, None) => "this file".into(),
        (1..=3, _) => names.join(", "),
        (n, _) => format!("{} and {} more", names[..2].join(", "), n - 2),
    }
}

/// Bring the frames `agent` is working on into view, switching to its page if it's working
/// elsewhere.
pub fn show(ed: &mut Editor, agent: AgentId) {
    let now = ed.now_ms;
    let touches: Vec<(String, crate::model::NodeId)> = ed
        .agents
        .touches_of(agent, now)
        .into_iter()
        .map(|(p, n)| (p.to_string(), n))
        .collect();
    let here = ed.active_page_id().to_string();
    let page = if touches.iter().any(|(p, _)| *p == here) {
        here
    } else if let Some((p, _)) = touches.first() {
        p.clone()
    } else {
        return;
    };
    if let Some(i) = ed.pages.iter().position(|s| s.id == page) {
        ed.switch_page(i);
    }
    let rect = touches
        .iter()
        .filter(|(p, _)| *p == page)
        .filter_map(|(_, n)| crate::agents::top_level(&ed.doc, *n))
        .filter_map(|id| ed.doc.world_rect(id))
        .reduce(|a, b| a.union(&b));
    if let Some(r) = rect {
        ed.camera.fit(&r, ed.viewport, 64.0, 1.0);
        ed.pending_fit = false;
        ed.revision += 1;
    }
}

/// A pulsing dot at the right end of a page row whose page an agent is working on.
pub fn page_dot(ui: &Ui, row: egui::Rect, now: f64) {
    ui.ctx().request_repaint_after(Duration::from_millis(33));
    let c = egui::pos2(row.right() - 12.0, row.center().y);
    ui.painter()
        .circle_filled(c, 5.0, agent_color(0.25 * agent_pulse(now)));
    ui.painter().circle_filled(c, 3.0, agent_color(1.0));
}
