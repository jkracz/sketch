//! Design systems in the editor: using, updating, removing and detaching one, each as one
//! undo step. The tokens live in `Editor::tokens` (see `TokenSet::attach`); the published
//! versions come from `crate::systems`.

use super::Editor;
use crate::systems::Published;
use std::collections::HashSet;

/// What updating a design system changed, for the status line and MCP results.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemUpdate {
    pub name: String,
    pub from: u32,
    pub to: u32,
    /// CSS names of tokens, and classes of text styles, the new version no longer has.
    pub removed: Vec<String>,
    /// Layers, on every page, that were bound to those tokens.
    pub unbound: usize,
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
            t.attach(p.link(), &p.tokens, &p.text_styles)
        })
    }

    /// Move a design system this file uses to `p`, when `p` is newer. None when the file
    /// doesn't use it or already has that version.
    pub fn update_system(&mut self, p: &Published) -> Option<SystemUpdate> {
        let from = self.tokens.system(&p.id)?.release;
        if p.release <= from {
            return None;
        }
        let label = format!("Update {} to v{}", p.name, p.release);
        let gone = self.edit_tokens(&label, None, |t| {
            t.update_system(p.link(), &p.tokens, &p.text_styles)
        });
        let ids: Vec<String> = gone.iter().map(|(id, _)| id.clone()).collect();
        Some(SystemUpdate {
            name: p.name.clone(),
            from,
            to: p.release,
            removed: gone.into_iter().map(|(_, name)| name).collect(),
            unbound: self.layers_bound_to(&ids),
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

    /// Layers on any page with a property bound to one of these tokens, or that use one of
    /// these text styles.
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
                    })
                    .count()
            })
            .sum()
    }
}
