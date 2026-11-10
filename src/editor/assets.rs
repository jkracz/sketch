//! Dropping a design system component from the assets panel onto the canvas.

use super::Editor;
use super::structure::{Source, instantiate};
use crate::geom::{Rect, V2};
use crate::model::NodeId;

impl Editor {
    /// The frame a layer dropped at world point `p` goes into: the topmost one there, outside
    /// instances, whose layers follow their main.
    pub fn drop_frame_at(&self, p: V2) -> Option<NodeId> {
        let instances: Vec<NodeId> = (self.doc.nodes.values())
            .filter(|n| n.instance.is_some())
            .map(|n| n.id)
            .collect();
        self.frame_at(p, &instances)
    }

    /// Place an instance of design system component `id` centered at world point `at`, inside
    /// the frame there (at the drop point in an auto-layout frame's flow), and select it.
    /// One undo step.
    pub fn drop_component(&mut self, id: &str, at: V2) -> Option<NodeId> {
        let c = self.tokens.component(id)?.clone();
        let main = c.doc.get(c.main()?)?;
        let (w, h) = (main.rect.w, main.rect.h);
        let parent = self.drop_frame_at(at);
        let o = self.doc.parent_origin(parent);
        let rect = Rect::new(
            (at.x - w * 0.5).round() - o.x,
            (at.y - h * 0.5).round() - o.y,
            w,
            h,
        );
        let flow = parent.filter(|p| self.doc.get(*p).is_some_and(|f| f.layout.is_some()));
        let src = Source {
            doc: Some(&c.doc),
            page: None,
            component: Some(&c.id),
        };
        let label = format!("Insert {}", c.name);
        let id = self.edit(&label, |tx| {
            let id = instantiate(tx, src, main.id, parent, None, Some(rect));
            // The main's place in its own parent doesn't carry over.
            let lc = &mut tx.node_mut(id).unwrap().layout_child;
            lc.absolute = false;
            lc.constraints = Default::default();
            if let Some(f) = flow {
                let i = crate::layout::drop_index(tx.doc(), f, id, at - o);
                tx.move_to_index(id, i);
            }
            id
        });
        self.selection = vec![id];
        Some(id)
    }
}

/// A design system copy of a blue 100×40 "Button" frame holding a 20×20 square, for tests.
#[cfg(test)]
pub(crate) fn test_button(ed: &mut Editor) -> String {
    use crate::model::{LinkedComponent, Node, NodeKind};
    let mut main = Node::new(
        NodeId(1),
        NodeKind::Frame { clip: true },
        Rect::new(500.0, 500.0, 100.0, 40.0),
    );
    main.name = "Button".into();
    main.style
        .set_fill(Some(crate::model::Color::rgb(0, 0, 255)));
    main.component = true;
    // As in a source file where the main sits in an auto-layout frame, out of the flow.
    main.layout_child.absolute = true;
    main.children = vec![NodeId(2)];
    let mut dot = Node::new(NodeId(2), NodeKind::Rect, Rect::new(10.0, 10.0, 20.0, 20.0));
    dot.parent = Some(NodeId(1));
    let c = LinkedComponent::new(
        "brand/button".into(),
        "brand",
        "Button".into(),
        vec![main, dot],
    );
    ed.tokens = std::mem::take(&mut ed.tokens).with_components(vec![c]);
    "brand/button".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::v2;
    use crate::model::{AutoLayout, Node, NodeKind};

    fn frame(ed: &mut Editor, r: Rect) -> NodeId {
        ed.add_node(
            Node::new(NodeId(0), NodeKind::Frame { clip: true }, r),
            None,
        )
    }

    #[test]
    fn a_drop_on_empty_canvas_centers_a_linked_instance_in_one_undo_step() {
        let mut ed = Editor::default();
        let c = test_button(&mut ed);
        let id = ed.drop_component(&c, v2(200.0, 100.0)).unwrap();
        let n = ed.doc.get(id).unwrap();
        assert_eq!(n.parent, None);
        assert_eq!(
            ed.doc.world_rect(id),
            Some(Rect::new(150.0, 80.0, 100.0, 40.0))
        );
        assert_eq!(
            n.instance.as_ref().unwrap().component.as_deref(),
            Some("brand/button")
        );
        assert_eq!(n.children.len(), 1);
        assert_eq!(ed.selection, [id]);
        ed.undo();
        assert!(ed.doc.get(id).is_none());
        assert!(ed.doc.nodes.is_empty());
    }

    #[test]
    fn a_drop_on_a_frame_goes_inside_it() {
        let mut ed = Editor::default();
        let c = test_button(&mut ed);
        let f = frame(&mut ed, Rect::new(100.0, 100.0, 400.0, 300.0));
        let id = ed.drop_component(&c, v2(300.0, 200.0)).unwrap();
        assert_eq!(ed.doc.get(id).unwrap().parent, Some(f));
        assert_eq!(
            ed.doc.world_rect(id),
            Some(Rect::new(250.0, 180.0, 100.0, 40.0))
        );
    }

    #[test]
    fn a_drop_on_an_auto_layout_frame_joins_its_flow_at_the_pointer() {
        let mut ed = Editor::default();
        let c = test_button(&mut ed);
        let f = frame(&mut ed, Rect::new(0.0, 0.0, 400.0, 60.0));
        let a = ed.add_node(
            Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 40.0, 40.0)),
            Some(f),
        );
        let b = ed.add_node(
            Node::new(NodeId(0), NodeKind::Rect, Rect::new(0.0, 0.0, 40.0, 40.0)),
            Some(f),
        );
        ed.edit("layout", |tx| {
            tx.node_mut(f).unwrap().layout = Some(AutoLayout::default())
        });
        // a spans x 10..50 and b 60..100: a drop at x 55 goes between them.
        let id = ed.drop_component(&c, v2(55.0, 30.0)).unwrap();
        assert_eq!(ed.doc.children(Some(f)), [a, id, b]);
        assert_eq!(ed.doc.world_rect(id).unwrap().x, 60.0);
        // It stays in the flow when it syncs from the main, which isn't.
        ed.sync_page_instances(ed.current_page());
        assert!(!ed.doc.get(id).unwrap().layout_child.absolute);
        assert_eq!(ed.doc.world_rect(id).unwrap().x, 60.0);
    }

    #[test]
    fn a_drop_on_an_instance_goes_beside_it_not_into_its_layers() {
        let mut ed = Editor::default();
        let c = test_button(&mut ed);
        let f = frame(&mut ed, Rect::new(0.0, 0.0, 600.0, 400.0));
        let first = ed.drop_component(&c, v2(300.0, 200.0)).unwrap();
        assert_eq!(ed.doc.get(first).unwrap().parent, Some(f));
        let second = ed.drop_component(&c, v2(300.0, 200.0)).unwrap();
        assert_eq!(ed.doc.get(second).unwrap().parent, Some(f));
        assert_eq!(ed.doc.get(first).unwrap().children.len(), 1);
    }
}
