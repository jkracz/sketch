//! Design systems in the editor: using, updating, removing and detaching one, each as one
//! undo step. The tokens live in `Editor::tokens` (see `TokenSet::attach`); the published
//! versions come from `crate::systems`.

use super::Editor;
use crate::model::NodeId;
use crate::systems::Published;
use std::collections::HashSet;

/// What updating a design system changed, for the status line and MCP results.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemUpdate {
    pub name: String,
    pub from: u32,
    pub to: u32,
    /// CSS names of tokens and effect styles, and classes of text styles, the new version no
    /// longer has.
    pub removed: Vec<String>,
    /// Layers, on every page, that were bound to those tokens.
    pub unbound: usize,
    /// Names of components the new version no longer has.
    pub removed_components: Vec<String>,
    /// Instances of those components, on every page. They act as plain frames now.
    pub orphaned: usize,
}

impl SystemUpdate {
    pub fn message(&self) -> String {
        let mut m = format!("Updated {} to v{}.", self.name, self.to);
        if !self.removed.is_empty() {
            m.push_str(&format!(
                " Removed {}; {} lost {} binding.",
                crate::systems::name_list(&self.removed, 5),
                plural(self.unbound, "layer"),
                if self.unbound == 1 { "its" } else { "their" }
            ));
        }
        if !self.removed_components.is_empty() {
            let names: Vec<String> = (self.removed_components.iter())
                .map(|n| format!("{n} component"))
                .collect();
            m.push_str(&format!(
                " Removed {}; {} {} now.",
                crate::systems::name_list(&names, 5),
                plural(self.orphaned, "instance"),
                if self.orphaned == 1 {
                    "is a plain frame"
                } else {
                    "are plain frames"
                }
            ));
        }
        m
    }
}

pub fn plural(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

impl Editor {
    /// Start using a published design system.
    pub fn use_system(&mut self, p: &Published) -> Result<(), String> {
        if self.published.as_ref().is_some_and(|l| l.id == p.id) {
            return Err(format!(
                "This file publishes {}, so it can't also use it.",
                p.name
            ));
        }
        let label = format!("Use {} design system", p.name);
        self.edit_tokens(&label, None, |t| {
            t.attach(p.link(), &p.tokens, &p.text_styles, &p.effect_styles)?;
            t.replace_components(&p.id, p.linked_components());
            Ok::<_, String>(())
        })?;
        // Instances left from an earlier use link up again.
        self.sync_all_instances();
        Ok(())
    }

    /// Move a design system this file uses to `p`, when `p` is newer. None when the file
    /// doesn't use it or already has that version.
    pub fn update_system(&mut self, p: &Published) -> Option<SystemUpdate> {
        let from = self.tokens.system(&p.id)?.release;
        if p.release <= from {
            return None;
        }
        let label = format!("Update {} to v{}", p.name, p.release);
        let (gone, gone_components) = self.edit_tokens_then(
            &label,
            None,
            |t| {
                let gone = t.update_system(p.link(), &p.tokens, &p.text_styles, &p.effect_styles);
                (gone, t.replace_components(&p.id, p.linked_components()))
            },
            // This page's instances in the same step, so undo brings back layers the update
            // removed with their overrides.
            |ed| {
                let roots = ed.linked_roots(&ed.doc);
                ed.sync_instances(&roots);
            },
        );
        // Other pages now, so the update is saved whole and not when each page is next shown.
        self.sync_all_instances();
        let ids: Vec<String> = gone.iter().map(|(id, _)| id.clone()).collect();
        let comp_ids: Vec<&str> = gone_components.iter().map(|c| c.id.as_str()).collect();
        Some(SystemUpdate {
            name: p.name.clone(),
            from,
            to: p.release,
            removed: gone.into_iter().map(|(_, name)| name).collect(),
            unbound: self.layers_bound_to(&ids),
            orphaned: self.instances_of(&comp_ids),
            removed_components: gone_components.into_iter().map(|c| c.name).collect(),
        })
    }

    /// Stop using a design system. Layers bound to its tokens keep their values. Returns how
    /// many layers were bound.
    pub fn remove_system(&mut self, id: &str) -> Result<usize, String> {
        let name = self.system_name(id)?;
        let label = format!("Remove {name} design system");
        let ids = self.edit_tokens(&label, None, |t| t.remove_system(id));
        Ok(self.layers_bound_to(&ids))
    }

    /// Make a design system's tokens local, keeping their names and bindings.
    pub fn detach_system(&mut self, id: &str) -> Result<(), String> {
        let name = self.system_name(id)?;
        let label = format!("Detach {name} design system");
        self.edit_tokens(&label, None, |t| t.detach_system(id));
        Ok(())
    }

    fn system_name(&self, id: &str) -> Result<String, String> {
        self.tokens
            .system(id)
            .map(|s| s.name.clone())
            .ok_or_else(|| "This file doesn't use that design system.".to_string())
    }

    /// Instance roots on any page linked to these design system components.
    fn instances_of(&self, ids: &[&str]) -> usize {
        (0..self.pages.len())
            .map(|i| {
                let doc = self.page_doc(i);
                let comp = |id: Option<NodeId>| {
                    (id.and_then(|id| doc.get(id)?.instance.as_ref()?.component.as_deref()))
                        .filter(|c| ids.contains(c))
                };
                (doc.nodes.values())
                    .filter(|n| comp(Some(n.id)).is_some() && comp(n.parent) != comp(Some(n.id)))
                    .count()
            })
            .sum()
    }

    /// Layers on any page using these token or style ids.
    fn layers_bound_to(&self, ids: &[String]) -> usize {
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        (0..self.pages.len())
            .map(|i| {
                self.page_doc(i)
                    .nodes
                    .values()
                    .filter(|n| {
                        n.tokens.values().any(|t| ids.contains(t.as_str()))
                            || n.text_style_ref
                                .as_ref()
                                .is_some_and(|r| ids.contains(r.id.as_str()))
                            || n.effect_style
                                .as_ref()
                                .is_some_and(|s| ids.contains(s.as_str()))
                    })
                    .count()
            })
            .sum()
    }
}
