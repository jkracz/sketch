use super::{page_id, tool};
use serde_json::{Value, json};

pub(super) fn definitions() -> Vec<Value> {
    let ids = |d: &str| json!({"type": "array", "items": {"type": "string"}, "description": d});
    vec![
        tool(
            "create_component",
            "Makes a main component from layers. One frame becomes the component in place; \
             anything else is wrapped in a new component frame. The layers must share a parent. \
             Place instances with <x-sketch-instance component=\"Name\"/> in write_html, on any \
             page; editing the main updates every instance. publish_design_system publishes the file's \
             components to other projects. Returns componentId, name and wrapped (whether a new \
             frame was made).",
            json!({
                "nodeIds": ids("The layers to make into one component. Make one call per component."),
                "name": {"type": "string", "description": "The component's name, e.g. \"Button/Primary\". Defaults to the frame's name, or \"Component N\" for a new frame."},
                "pageId": page_id(),
            }),
            &["nodeIds"],
            false,
        ),
        tool(
            "detach_instances",
            "Detaches instances into plain layers that keep their look and stop following the \
             main. Pass instance roots (get_node_info shows instanceOf); a layer inside an \
             instance is skipped, with the id of its instance. Returns detached and skipped ids.",
            json!({
                "nodeIds": ids("Instance ids."),
                "pageId": page_id(),
            }),
            &["nodeIds"],
            false,
        ),
        tool(
            "reset_overrides",
            "Resets overrides so the layers and everything inside them follow the main again. \
             Pass an instance to reset all of it, or a layer inside one to reset only that \
             layer. get_node_info lists a layer's overrides. Returns what was cleared on each \
             node.",
            json!({
                "nodeIds": ids("Instances, or layers inside instances."),
                "pageId": page_id(),
            }),
            &["nodeIds"],
            false,
        ),
    ]
}
