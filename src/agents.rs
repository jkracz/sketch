//! What MCP agents are working on, so the canvas can show it without moving the camera.
//!
//! Modeled on Paper's agent indicator (behavior notes in `analysis/agent-activity.md`). Each
//! MCP session is one agent. Every tool call records the nodes it read or wrote; the
//! top-level frame around each fresh node gets a pulsing outline and a strip of falling dots
//! (drawn by `render::scene`), the top bar lists the busy agents and the page list marks the
//! pages they're on. A node goes quiet 40 s after its last touch, when the agent calls
//! `finish_working_on_nodes`, or when its session ends.

use crate::model::{Document, NodeId};
use std::collections::{BTreeMap, HashMap, HashSet};

/// One MCP session.
pub type AgentId = u64;

/// A node stays marked this long after the agent last touched it.
pub const EXPIRE_MS: f64 = 40_000.0;
/// Frame marks fade in and out over this long.
pub const FADE_MS: f64 = 400.0;

#[derive(Debug, Clone, Default)]
pub struct Agent {
    /// The client's name ("claude-code", "cursor"...), as the client reported it.
    pub client: Option<String>,
    /// (page id, node) → time of the last read or write, ms.
    touches: HashMap<(String, NodeId), f64>,
}

impl Agent {
    /// Name for the UI.
    pub fn label(&self) -> String {
        client_label(self.client.as_deref())
    }
}

/// A friendly name for an MCP client's `clientInfo.name`.
pub fn client_label(client: Option<&str>) -> String {
    let Some(c) = client
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != "unknown")
    else {
        return "Agent".into();
    };
    let k = c.to_ascii_lowercase();
    let known = [
        ("claude-code", "Claude Code"),
        ("claude code", "Claude Code"),
        ("claude-ai", "Claude"),
        ("claude", "Claude"),
        ("codex", "Codex"),
        ("cursor", "Cursor"),
        ("visual studio code", "VS Code"),
        ("vscode", "VS Code"),
        ("copilot", "Copilot"),
        ("windsurf", "Windsurf"),
        ("gemini", "Gemini"),
        ("opencode", "OpenCode"),
        ("t3", "T3 Code"),
    ];
    for (key, name) in known {
        if k.starts_with(key) {
            return name.into();
        }
    }
    c.into()
}

/// Fade state of one marked frame on the visible page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    /// When the mark appeared (ms); drives the fade-in and the animation phase.
    pub since: f64,
    /// When the frame went quiet, while it fades out.
    pub gone: Option<f64>,
}

impl Mark {
    /// Opacity from the fade in or out, 0..=1.
    pub fn alpha(&self, now: f64) -> f64 {
        match self.gone {
            Some(g) => 1.0 - ((now - g) / FADE_MS).clamp(0.0, 1.0),
            None => ((now - self.since) / FADE_MS).clamp(0.0, 1.0),
        }
    }
}

#[derive(Debug, Default)]
pub struct Activity {
    agents: BTreeMap<AgentId, Agent>,
    /// Marked top-level frames of `marks_page`.
    marks: BTreeMap<NodeId, Mark>,
    marks_page: String,
}

impl Activity {
    /// Note that `agent` touched `nodes` on page `page` at `now`.
    pub fn touch(
        &mut self,
        agent: AgentId,
        client: Option<&str>,
        page: &str,
        nodes: impl IntoIterator<Item = NodeId>,
        now: f64,
    ) {
        let a = self.agents.entry(agent).or_default();
        if client.is_some() {
            a.client = client.map(String::from);
        }
        for n in nodes {
            a.touches.insert((page.to_string(), n), now);
        }
    }

    /// Drop the agent's touches for which `keep` returns false (`finish_working_on_nodes`).
    pub fn retain(&mut self, agent: AgentId, mut keep: impl FnMut(&str, NodeId) -> bool) {
        if let Some(a) = self.agents.get_mut(&agent) {
            a.touches.retain(|(p, n), _| keep(p, *n));
        }
    }

    /// Forget everything the agent is working on.
    pub fn finish(&mut self, agent: AgentId) {
        if let Some(a) = self.agents.get_mut(&agent) {
            a.touches.clear();
        }
    }

    /// The agent's session ended.
    pub fn remove(&mut self, agent: AgentId) {
        self.agents.remove(&agent);
    }

    /// Agents with fresh touches, oldest session first.
    pub fn active(&self, now: f64) -> impl Iterator<Item = (AgentId, &Agent)> {
        self.agents
            .iter()
            .filter(move |(_, a)| a.touches.values().any(|t| now - t < EXPIRE_MS))
            .map(|(id, a)| (*id, a))
    }

    /// Fresh (page id, node) touches of one agent, latest first.
    pub fn touches_of(&self, agent: AgentId, now: f64) -> Vec<(&str, NodeId)> {
        let Some(a) = self.agents.get(&agent) else {
            return Vec::new();
        };
        let mut v: Vec<(&(String, NodeId), &f64)> = a
            .touches
            .iter()
            .filter(|(_, t)| now - **t < EXPIRE_MS)
            .collect();
        v.sort_by(|a, b| b.1.total_cmp(a.1).then(a.0.cmp(b.0)));
        v.into_iter().map(|((p, n), _)| (p.as_str(), *n)).collect()
    }

    /// Ids of pages some agent is working on.
    pub fn busy_pages(&self, now: f64) -> HashSet<&str> {
        let mut out = HashSet::new();
        for a in self.agents.values() {
            for ((p, _), t) in &a.touches {
                if now - t < EXPIRE_MS {
                    out.insert(p.as_str());
                }
            }
        }
        out
    }

    /// Top-level frames of `page` (whose content is `doc`) that agents are working on.
    pub fn busy_frames(&self, page: &str, doc: &Document, now: f64) -> HashSet<NodeId> {
        let mut out = HashSet::new();
        for a in self.agents.values() {
            for ((p, n), t) in &a.touches {
                if p == page
                    && now - t < EXPIRE_MS
                    && let Some(top) = top_level(doc, *n)
                {
                    out.insert(top);
                }
            }
        }
        out
    }

    /// Expire old touches and update the fades for the visible page. Call once per frame.
    pub fn tick(&mut self, now: f64, page: &str, doc: &Document) {
        for a in self.agents.values_mut() {
            a.touches.retain(|_, t| now - *t < EXPIRE_MS);
        }
        if self.marks_page != page {
            self.marks.clear();
            self.marks_page = page.to_string();
        }
        let live = self.busy_frames(page, doc, now);
        for id in &live {
            let m = self.marks.entry(*id).or_insert(Mark {
                since: now,
                gone: None,
            });
            if let Some(g) = m.gone.take() {
                // Came back while fading out: fade in from the current opacity.
                let shown = 1.0 - ((now - g) / FADE_MS).clamp(0.0, 1.0);
                m.since = now - shown * FADE_MS;
            }
        }
        for (id, m) in self.marks.iter_mut() {
            if !live.contains(id) && m.gone.is_none() {
                m.gone = Some(now);
            }
        }
        self.marks
            .retain(|id, m| doc.get(*id).is_some() && m.gone.is_none_or(|g| now - g < FADE_MS));
    }

    /// Marked frames of the visible page, as of the last [`Activity::tick`].
    pub fn marks(&self) -> impl Iterator<Item = (NodeId, Mark)> + '_ {
        self.marks.iter().map(|(id, m)| (*id, *m))
    }

    /// Something is on screen that changes by itself.
    pub fn animating(&self) -> bool {
        !self.marks.is_empty()
    }
}

/// The visible top-level layer holding `id` (itself when it's top-level).
pub fn top_level(doc: &Document, id: NodeId) -> Option<NodeId> {
    doc.get(id)?;
    let top = doc.ancestors(id).last().copied().unwrap_or(id);
    let n = doc.get(top)?;
    (n.visible && doc.root.contains(&top)).then_some(top)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Editor;
    use crate::geom::Rect;
    use crate::model::{Node, NodeKind};

    /// Two top-level frames, the first with a child.
    fn doc() -> (Document, NodeId, NodeId, NodeId) {
        let mut ed = Editor::default();
        let (f, c, g) = ed.edit("Add", |tx| {
            let frame = |tx: &mut crate::history::Tx, x| {
                let id = tx.alloc_id();
                let kind = NodeKind::Frame { clip: true };
                tx.insert(
                    Node::new(id, kind, Rect::new(x, 0.0, 200.0, 100.0)),
                    None,
                    None,
                )
            };
            let f = frame(tx, 0.0);
            let id = tx.alloc_id();
            let r = Rect::new(10.0, 10.0, 20.0, 20.0);
            let c = tx.insert(Node::new(id, NodeKind::Rect, r), Some(f), None);
            (f, c, frame(tx, 400.0))
        });
        (ed.doc, f, c, g)
    }

    #[test]
    fn a_touched_child_marks_its_top_level_frame() {
        let (d, f, c, g) = doc();
        let mut a = Activity::default();
        a.touch(1, Some("claude-code"), "p", [c], 0.0);
        assert_eq!(a.busy_frames("p", &d, 10.0), HashSet::from([f]));
        assert!(a.busy_frames("other", &d, 10.0).is_empty());
        a.tick(10.0, "p", &d);
        assert_eq!(a.marks().map(|(id, _)| id).collect::<Vec<_>>(), vec![f]);
        assert!(!a.marks().any(|(id, _)| id == g));
        assert_eq!(a.active(10.0).next().unwrap().1.label(), "Claude Code");
    }

    #[test]
    fn marks_fade_in_and_out() {
        let (d, f, _, _) = doc();
        let mut a = Activity::default();
        a.touch(1, None, "p", [f], 0.0);
        a.tick(0.0, "p", &d);
        let m = a.marks().next().unwrap().1;
        assert_eq!(m.alpha(0.0), 0.0);
        assert_eq!(m.alpha(FADE_MS), 1.0);
        a.finish(1);
        a.tick(1000.0, "p", &d);
        let m = a.marks().next().unwrap().1;
        assert_eq!(m.gone, Some(1000.0));
        assert!((m.alpha(1000.0 + FADE_MS / 2.0) - 0.5).abs() < 1e-9);
        a.tick(1000.0 + FADE_MS, "p", &d);
        assert!(!a.animating());
    }

    #[test]
    fn touches_expire() {
        let (d, f, _, _) = doc();
        let mut a = Activity::default();
        a.touch(1, None, "p", [f], 0.0);
        assert_eq!(a.busy_pages(EXPIRE_MS - 1.0), HashSet::from(["p"]));
        assert!(a.busy_pages(EXPIRE_MS).is_empty());
        a.tick(EXPIRE_MS, "p", &d);
        assert_eq!(a.active(EXPIRE_MS).count(), 0);
    }

    #[test]
    fn agents_are_separate() {
        let (d, f, _, g) = doc();
        let mut a = Activity::default();
        a.touch(1, Some("cursor"), "p", [f], 0.0);
        a.touch(2, Some("codex"), "p", [g], 0.0);
        a.finish(1);
        assert_eq!(a.busy_frames("p", &d, 1.0), HashSet::from([g]));
        let names: Vec<String> = a.active(1.0).map(|(_, a)| a.label()).collect();
        assert_eq!(names, ["Codex"]);
        a.remove(2);
        assert_eq!(a.active(1.0).count(), 0);
    }

    #[test]
    fn retain_drops_selected_nodes() {
        let (d, f, c, g) = doc();
        let mut a = Activity::default();
        a.touch(1, None, "p", [c, g], 0.0);
        a.retain(1, |_, n| !d.is_ancestor(f, n) && n != f);
        assert_eq!(a.busy_frames("p", &d, 1.0), HashSet::from([g]));
    }

    #[test]
    fn client_labels() {
        assert_eq!(client_label(None), "Agent");
        assert_eq!(client_label(Some("unknown")), "Agent");
        assert_eq!(client_label(Some("Cursor-vscode")), "Cursor");
        assert_eq!(client_label(Some("my-bot")), "my-bot");
    }
}
