//! Tool catalogue: names, descriptions and input schemas.
//!
//! Names and argument shapes follow Paper's MCP server where Sketch has an equivalent
//! (`analysis/paper-mcp.md` §2), so agent prompts written for Paper carry over. Sketch
//! additions: `create_shape` (whiteboard shapes and arrows), `select_nodes`, `undo`, `redo`.

use crate::model::TokenKind;
use serde_json::{Map, Value, json};

mod components;

pub const INSTRUCTIONS: &str = "\
Sketch is a local design and whiteboard app with an infinite canvas. These tools read and \
edit the project open in the running app; every change is one undo step and shows up live.

- Start with get_basic_info (pages, artboards, sizes) and get_selection (what the user is \
looking at). Use list_resources / open_file / create_file to pick a project.
- Projects can sit in folders, one level deep. list_resources returns every folder and each \
project's location. To put a project in a folder the user names, pass the folder's id as \
parentId to create_file or move_resources. If it isn't listed, call create_folder first, which \
returns the existing folder when the name is taken. parentId \"root\" is the top level. Two \
folders can hold projects with the same name, so prefer ids or Folder/Project keys.
- duplicate_file copies a project. trash_resources moves projects and folders to the trash; \
list_trash lists them and restore_resources brings them back. Only the user can delete forever.
- Pages: create_page, rename_pages, duplicate_page, arrange_pages (reorder), delete_pages. \
move_to_page moves or copies layers to another page.
- For details read get_guide({ topic: \"sketch-rs-mcp-instructions\" }) once per session.
- pageId defaults to the page the user is viewing; pass it to work on another page without \
switching the user's view. Node ids are opaque strings; never show them to the user.
- Build UI with create_artboard, then write_html into it, one visual group per call (a header, \
a row, a card) so the user sees progress. Inline styles only; flexbox (display:flex, gap, \
padding, align-items, justify-content) is the layout model; no margin, grid or tables.
- Text defaults to Inter. Use px for font sizes. Find Google Fonts with search_fonts, by name \
or category, then set font-family and font-weight.
- Edit with update_styles, set_text_content, rename_nodes, duplicate_nodes, move_nodes, \
delete_nodes. Whiteboard shapes, sticky notes and arrows: create_shape; change an arrow later \
with update_arrows. Pencil strokes: create_stroke. Hand-drawn style: update_styles with \
{\"sketch\": true, \"roughness\": 1, \"fillStyle\": \"hachure\"}.
- Vectors: boolean_nodes (union, subtract, intersect, exclude), flatten_nodes, and \
set_vector_points to edit a path's points. Bold or color part of a text with \
set_text_range_styles.
- Design tokens are project-wide CSS variables: get_tokens, create_tokens, set_tokens. Use \
var(--token) in update_styles and write_html styles to bind a property to a token, so later \
token changes reach it.
- Text styles name a set of text settings, like Heading/L. get_tokens lists them as classes. \
Apply one with class=\"heading-l\" in write_html or {\"textStyle\": \"heading-l\"} in \
update_styles; create_text_styles and set_text_styles manage them.
- Effect styles name a list of shadows and blurs, like Elevation/2. get_tokens lists them as \
CSS variables. Apply one with box-shadow: var(--elevation-2) in write_html or \
{\"effectStyle\": \"elevation-2\"} in update_styles.
- Components: create_component makes a main from layers; place instances of it with \
<x-sketch-instance component=\"Name\"/> in write_html. Edits to an instance's layers are \
overrides; reset_overrides undoes them and detach_instances turns instances into plain layers. \
get_basic_info lists the file's components; get_node_info shows an instance's main.
- Design systems are token sets and components other projects publish. list_design_systems \
gives a short summary of each; get_design_system loads one system's usage notes, tokens and \
component names (filter by types or namePattern to load less); use_design_system lets the open \
file bind them as var(--prefix-name), its text styles as prefix-name classes and its effect \
styles as var(--prefix-name), and place its components with \
<x-sketch-instance component=\"Name\"/> in write_html. publish_design_system publishes the \
open file's own tokens, styles and main components.
- export writes PNG, SVG or PDF files; the key \"root\" exports the whole page and \
transparent: true drops the white background.
- Check your work with get_screenshot. Artboard height is a starting point: when content \
clips set height: \"fit-content\" with update_styles.
- Call finish_working_on_nodes when done.";

fn file_id() -> Value {
    json!({"type": "string", "description": "The project to operate on (id, name, Folder/Project location or path from list_resources). When omitted the project open in the app is used."})
}

pub(super) fn page_id() -> Value {
    json!({"type": "string", "description": "The page to operate on. When omitted the page the user is viewing is used. The user's page can change between calls; prefer explicit page ids."})
}

fn token_type() -> Value {
    let kinds: Vec<&str> = TokenKind::ALL.iter().map(|k| k.key()).collect();
    json!({"type": "string", "enum": kinds, "description": "Design token type."})
}

/// The `types` filter of the listing tools: token types, plus `textStyle` and `effectStyle`.
fn types_filter() -> Value {
    let mut kinds: Vec<&str> = TokenKind::ALL.iter().map(|k| k.key()).collect();
    kinds.extend(["textStyle", "effectStyle"]);
    json!({"type": "array", "items": {"type": "string", "enum": kinds}, "description": "Only these token types; \"textStyle\" for text styles and \"effectStyle\" for effect styles. Omit for all."})
}

fn design_system_types() -> Value {
    let mut v = types_filter();
    v["items"]["enum"]
        .as_array_mut()
        .expect("types_filter lists an enum")
        .push(json!(crate::mcp::exec::COMPONENT_TYPE));
    v["description"] = json!(
        "Only these token types; \"textStyle\" for text styles, \"effectStyle\" for effect styles and \"component\" for components. Omit for all."
    );
    v
}

fn style_props(extra: &[(&str, Value)]) -> Value {
    let mut m = style_fields();
    for (k, v) in extra {
        m.insert((*k).into(), v.clone());
    }
    Value::Object(m)
}

fn style_fields() -> Map<String, Value> {
    let field =
        |d: &str| json!({"anyOf": [{"type": "string"}, {"type": "number"}], "description": d});
    let mut m = Map::new();
    m.insert(
        "fontFamily".into(),
        field("Family name or var(--font-family-token). Default Inter."),
    );
    m.insert(
        "fontSize".into(),
        field("px (\"32px\") or var(--font-size-token). Default 16px."),
    );
    m.insert(
        "fontWeight".into(),
        field("1 to 1000 or var(--font-weight-token). Default 400."),
    );
    m.insert(
        "fontStyle".into(),
        json!({"type": "string", "enum": ["normal", "italic"]}),
    );
    m.insert("lineHeight".into(), field("A multiple or percent (1.2, \"120%\"), \"normal\", or var(--line-height-token). Default normal."));
    m.insert(
        "letterSpacing".into(),
        field("px or em (\"-0.02em\") or var(--letter-spacing-token). Default 0."),
    );
    m.insert(
        "description".into(),
        json!({"type": "string", "maxLength": 1024, "description": "What the style is for."}),
    );
    m
}

fn mode_values() -> Value {
    json!({
        "type": "object",
        "additionalProperties": {"anyOf": [{"type": "string"}, {"type": "number"}, {"type": "null"}]},
        "description": "Values for the file's other modes by mode name, e.g. {\"Dark\": \"#111111\"}. \
                        A mode left out uses `value`. Aliases resolve in the same mode.",
    })
}

fn token_value() -> Value {
    json!({
        "anyOf": [{"type": "string"}, {"type": "number"}],
        "description": "A CSS color for color tokens; px for sizes (\"8px\"); px or em for \
                        letterSpacing (\"-0.025em\" follows the text's font size); a number for \
                        fontWeight; a multiple or percent for lineHeight (1.5, \"150%\"); 0 to 1 \
                        or a percent for opacity. `var(--other-token)` aliases another token.",
    })
}

pub(super) fn styles_schema(desc: &str) -> Value {
    json!({
        "type": "object",
        "propertyNames": {"type": "string"},
        "additionalProperties": {"anyOf": [{"type": "string"}, {"type": "number"}, {"type": "boolean"}]},
        "description": desc,
    })
}

pub const FONT_CATEGORIES: [&str; 5] =
    ["sans-serif", "serif", "display", "handwriting", "monospace"];
pub const MAX_FONT_RESULTS: usize = 100;
/// The `parentId` for projects in no folder.
pub const TOP_LEVEL: &str = "root";

pub(super) fn tool(
    name: &str,
    desc: &str,
    props: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    let mut props = props;
    if let Some(m) = props.as_object_mut()
        && !m.contains_key("fileId")
        && !matches!(
            name,
            "list_resources"
                | "create_file"
                | "create_folder"
                | "move_resources"
                | "rename_resource"
                | "get_font_family_info"
                | "search_fonts"
                | "get_guide"
                | "open_file"
                | "list_design_systems"
                | "get_design_system"
        )
    {
        m.insert("fileId".into(), file_id());
    }
    let ann = if read_only {
        json!({"readOnlyHint": true})
    } else if name == "delete_nodes" {
        json!({"destructiveHint": true, "readOnlyHint": false})
    } else {
        json!({"destructiveHint": false, "readOnlyHint": false})
    };
    json!({
        "name": name,
        "description": desc,
        "inputSchema": {
            "type": "object",
            "properties": props,
            "required": required,
        },
        "annotations": ann,
    })
}

/// Every tool, as MCP `Tool` JSON objects.
pub fn definitions() -> Vec<Value> {
    let mut v = core_definitions();
    v.extend(super::pages::definitions());
    v.extend(super::projects::definitions());
    v.extend(super::vectors::definitions());
    v
}

fn core_definitions() -> Vec<Value> {
    let node_id = |d: &str| json!({"type": "string", "description": d});
    let ids = |d: &str| json!({"type": "array", "items": {"type": "string"}, "description": d});
    let mut all = vec![
        // ------------------------------------------------------------ projects and pages
        tool(
            "list_resources",
            "Lists the user's folders, then their Sketch projects, most recently modified first. \
             Each item has type \"folder\" or \"file\". Folders have id, name, path and fileCount; \
             all of them are always listed. Files have id, name, location (\"Folder/Project\", or \
             just the name at the top level), path, page count, updatedAt, `open` for the project \
             open in the app, and parentId and folder when they're in a folder. Pass parentId to \
             list only the projects in one folder.",
            json!({
                "limit": {"type": "integer", "description": "Maximum projects to return. Defaults to 50."},
                "parentId": {"type": "string", "description": "Folder id or name. Only list projects directly inside it. \"root\" lists projects in no folder."},
            }),
            &[],
            true,
        ),
        tool(
            "open_file",
            "Opens a project in the app (saving the current one) and returns a get_basic_info \
             response. Optional pageId switches the user to that page.",
            json!({
                "fileId": {"type": "string", "minLength": 1, "description": "Project id, name, Folder/Project location or path (see list_resources)."},
                "pageId": {"type": "string", "description": "Optional page to show."},
            }),
            &["fileId"],
            false,
        ),
        tool(
            "create_file",
            "Creates a new empty project and returns its id. Pass parentId to create it inside a \
             folder; a folder that doesn't exist is an error listing the ones that do. Call \
             open_file with the id to work in it.",
            json!({
                "name": {"type": "string", "description": "Optional project name."},
                "parentId": {"type": "string", "description": "Optional folder id or name. Defaults to the top level."},
            }),
            &[],
            false,
        ),
        tool(
            "create_folder",
            "Creates a folder for organizing projects and returns its id, or returns the existing \
             folder with that name (ignoring case), with `created` false. Folders hold projects \
             and can't be nested.",
            json!({"name": {"type": "string", "description": "Folder name. Defaults to \"New Folder\"."}}),
            &[],
            false,
        ),
        tool(
            "move_resources",
            "Moves projects into a folder, or to the top level with parentId \"root\". The project \
             open in the app can be moved and stays open. Every id is checked first: if any can't \
             be found, nothing moves. A project whose name is taken in the folder gets the next \
             free name. Returns `moved` and `unchanged` (already there) with each project's \
             current id, name and path.",
            json!({
                "resourceIds": {"type": "array", "minItems": 1, "items": {"type": "string"}, "description": "Project ids, names or Folder/Project locations."},
                "parentId": {"type": "string", "minLength": 1, "description": "Folder id or name, or \"root\" for the top level."},
            }),
            &["resourceIds", "parentId"],
            false,
        ),
        tool(
            "rename_resource",
            "Renames a project or folder and returns the stored name, which gets the next free \
             name if the one asked for is taken. It keeps its id and contents. The project open \
             in the app can be renamed and stays open.",
            json!({
                "resourceId": {"type": "string", "minLength": 1, "description": "Project or folder id, or name."},
                "name": {"type": "string", "minLength": 1, "description": "The new name."},
            }),
            &["resourceId", "name"],
            false,
        ),
        tool(
            "create_page",
            "Creates a new page at the end of the page list and returns its id, without switching \
             the user's page. Use the returned pageId in later calls.",
            json!({"name": {"type": "string", "description": "Optional page name. Defaults to \"Page N\"."}}),
            &[],
            false,
        ),
        tool(
            "rename_pages",
            "Rename one or more pages. Does not switch the user's page.",
            json!({"updates": {"type": "array", "description": "Page renames.", "items": {
                "type": "object",
                "properties": {"pageId": {"type": "string"}, "name": {"type": "string"}},
                "required": ["pageId", "name"],
            }}}),
            &["updates"],
            false,
        ),
        // ------------------------------------------------------------ reading
        tool(
            "get_basic_info",
            "Essential context about the open project: project name, the page, node count, \
             artboards (top-level frames) with ids, sizes and world positions, other top-level \
             layers, fonts, `pages` with the active one marked isActive, and `components`: the \
             file's main components on every page, with ids, names and sizes.\n\
             - worldX/worldY are world positions; x/y are relative to the parent.\n\
             - Pass pageId to inspect another page without switching the user's view.",
            json!({"pageId": page_id()}),
            &[],
            true,
        ),
        tool(
            "get_selection",
            "The selected layers on the page the user is viewing: ids, names, types, sizes, \
             positions and the artboard each belongs to.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "get_node_info",
            "Details of one node: type, name, size, visibility, lock state, parent, children ids, \
             text content, x/y (relative to parent) and worldX/worldY. Errors if it doesn't exist.\n\
             - A main component has component: true and instanceCount.\n\
             - An instance has instanceOf (its component's name) and, for the file's own \
             components, mainComponentId. A layer inside one has instanceRootId. overrides lists \
             what was changed on that layer.",
            json!({"nodeId": node_id("The node to inspect.")}),
            &["nodeId"],
            true,
        ),
        tool(
            "get_children",
            "Direct children of a node (or of the page root when nodeId is \"root\"), with ids, \
             names, types, child counts and positions.",
            json!({"nodeId": node_id("The parent node, or \"root\"."), "pageId": page_id()}),
            &["nodeId"],
            true,
        ),
        tool(
            "get_tree_summary",
            "Compact indented text tree of a subtree: type, name, id and size per node. Much \
             cheaper than get_jsx for orientation. nodeId \"root\" summarizes the whole page.",
            json!({
                "nodeId": node_id("Root of the summary, or \"root\"."),
                "depth": {"type": "number", "description": "Maximum depth (default 3, max 10)."},
                "pageId": page_id(),
            }),
            &["nodeId"],
            true,
        ),
        tool(
            "get_screenshot",
            "Render a node (or the whole page when nodeId is omitted or \"root\") to a PNG and \
             return it as an image. Scale 1 (default) is enough for layout checks; use 2 to read \
             small text. Large images are scaled down to fit API limits.",
            json!({
                "nodeId": node_id("The node to capture; omit for the whole page."),
                "scale": {"type": "number", "description": "Render scale: 1 (default) or 2."},
                "pageId": page_id(),
            }),
            &[],
            true,
        ),
        tool(
            "get_jsx",
            "JSX for a node and its descendants with inline styles (frames become flex divs, text \
             becomes spans, vectors become inline SVG). Use it to export a design to code.",
            json!({
                "nodeId": node_id("The node to convert."),
                "format": {"type": "string", "enum": ["inline-styles", "tailwind"], "description": "Only inline styles are produced; \"tailwind\" is accepted for compatibility."},
            }),
            &["nodeId"],
            true,
        ),
        tool(
            "get_computed_styles",
            "CSS styles (camelCase) of one or more nodes, as {nodeId: styles}.",
            json!({"nodeIds": ids("Nodes to read.")}),
            &["nodeIds"],
            true,
        ),
        tool(
            "get_fill_image",
            "The image used by a node's image fill, as image content.",
            json!({"nodeId": node_id("A node with an image fill.")}),
            &["nodeId"],
            true,
        ),
        tool(
            "find_nodes",
            "Find nodes by text content and/or computed style. Searches the whole project by \
             default; pass pageId for one page or nodeId for a subtree. `textValue` matches text \
             case-insensitively with * wildcards anchored to the whole value. `filters` are \
             {styleName, styleValue} pairs (AND), values may use * wildcards; colors match by value.",
            json!({
                "pageId": page_id(),
                "nodeId": node_id("Optional subtree root; takes priority over pageId."),
                "textValue": {"type": "string"},
                "filters": {"type": "array", "items": {"type": "object", "properties": {
                    "styleName": {"type": "string"}, "styleValue": {"type": "string"}}}},
            }),
            &[],
            true,
        ),
        tool(
            "get_font_family_info",
            "Whether font families are available and their weights/styles. Inter is bundled and \
             any Google Fonts family downloads the first time text uses it, showing Inter until \
             it arrives. Other families stay in the document and render in Inter.",
            json!({"familyNames": {"type": "array", "items": {"type": "string"}}}),
            &["familyNames"],
            true,
        ),
        tool(
            "search_fonts",
            "Find Google Fonts families to use as font-family, most popular first. Returns each \
             family's weights and italics, whether it's loaded, and how many families match. \
             Families download the first time text uses them.",
            json!({
                "query": {"type": "string", "description": "Part of a family name. Omit to browse."},
                "category": {
                    "type": "string",
                    "enum": FONT_CATEGORIES,
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_FONT_RESULTS,
                    "description": "Most families to return. Defaults to 20.",
                },
            }),
            &[],
            true,
        ),
        tool(
            "get_guide",
            "Read a guide. Topics: \"sketch-rs-mcp-instructions\" (read first), \"html\" (what \
             write_html supports), \"whiteboard\" (shapes, arrows, stickies, hand-drawn style).",
            json!({"topic": {"type": "string"}}),
            &["topic"],
            true,
        ),
        tool(
            "export",
            "Export nodes to files. `nodes` maps node ids to export settings \
             ([{format: png|svg|pdf, scale: \"2x\" | \"512w\" | \"512h\" | \"720p\", \
             transparent?}]; [] = 2x PNG). The key \"root\" exports the whole page. Exports have a \
             white background unless transparent is true. Files go to `directory` (default: the \
             Downloads folder). Returns the paths written.",
            json!({
                "nodes": {"type": "object", "additionalProperties": {"type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "format": {"type": "string", "enum": ["png", "svg", "pdf"]},
                        "scale": {"type": "string"},
                        "transparent": {"type": "boolean", "description": "Leave the background transparent instead of white. Default false."},
                    },
                    "required": ["format"],
                }}},
                "directory": {"type": "string", "description": "Output folder (absolute path)."},
                "pageId": page_id(),
            }),
            &["nodes"],
            false,
        ),
        tool(
            "export_combined_pdf",
            "Export several nodes into one PDF, one page per node, ordered top-to-bottom then \
             left-to-right. Returns the file path.",
            json!({
                "nodeIds": ids("Nodes to put on the pages."),
                "path": {"type": "string", "description": "Output file (default: Downloads/<project>.pdf)."},
                "transparent": {"type": "boolean", "description": "Leave page backgrounds transparent instead of white. Default false."},
            }),
            &["nodeIds"],
            false,
        ),
        // ------------------------------------------------------------ writing
        tool(
            "create_artboard",
            "Creates an artboard (top-level frame) and returns its id; then add content with \
             write_html({mode: \"insert-children\"}). `styles` are camelCase CSS; width and height \
             are required (e.g. \"1440px\"). Artboards default to display:flex, \
             flexDirection:column and a white background, and are placed in an empty spot unless \
             left/top are given. Default sizes: desktop 1440×900, tablet 768×1024, mobile 390×844.",
            json!({
                "name": {"type": "string", "description": "Layer name."},
                "styles": {
                    "type": "object",
                    "properties": {"width": {"type": "string"}, "height": {"type": "string"}},
                    "required": ["width", "height"],
                    "additionalProperties": {"anyOf": [{"type": "string"}, {"type": "number"}]},
                    "description": "CSS styles, e.g. {\"width\": \"390px\", \"height\": \"844px\", \"backgroundColor\": \"#f5f5f5\", \"padding\": \"24px\", \"gap\": \"16px\"}",
                },
                "pageId": page_id(),
            }),
            &["name", "styles"],
            false,
        ),
        tool(
            "write_html",
            "Parse HTML with inline styles into layers. Write incrementally: each call should add \
             one visual group (a header, one list row, a card) so the user sees progress.\n\
             - mode \"insert-children\" appends the HTML inside targetNodeId (\"root\" = the page); \
             \"replace\" replaces targetNodeId with it.\n\
             - Inline styles only. Flexbox (display:flex, flex-direction, gap, padding, \
             align-items, justify-content, flex-grow, flex-wrap) is the layout model; block \
             elements stack vertically. position:absolute with left/top is supported. No margin, \
             grid or tables.\n\
             - Text: font-family, font-size, font-weight (1 to 1000), font-style, color, \
             line-height, letter-spacing, text-align, text-decoration; <b>/<i>/<u>/<s>/\
             <span style> make rich text. Google Fonts families download on first use and show \
             Inter until they arrive.\n\
             - Boxes: background(-color/-image: colors, linear/radial gradients, url()), border, \
             border-radius, opacity, overflow:hidden, mix-blend-mode, width/height (px, %, \
             fit-content).\n\
             - <img src> takes paper-asset:///abs/path, file:// URLs, absolute paths or data: URIs. \
             <svg> markup is imported as vector layers.\n\
             - layer-name=\"...\" names layers; <x-sketch-clone node-id=\"ID\" style=\"...\"/> \
             clones an existing node.\n\
             - <x-sketch-instance component=\"Button\" style=\"...\"/> places an instance of \
             one of the file's own components (by name or main id, see get_basic_info) or of a \
             component from a design system the file uses (see get_design_system), at its own \
             size unless style sets one. The file's own component wins a name clash; \
             \"System/Name\" picks a system's. The result lists each instance's text layers \
             for set_text_content.\n\
             - class=\"heading-l\" applies that text style (see get_tokens); inline styles \
             after it override single fields. Text inside the element inherits it.\n\
             - box-shadow: var(--elevation-2) applies that effect style (see get_tokens), with \
             all its shadows and blurs. On text, use text-shadow.\n\
             - data-theme=\"dark\" on a frame picks a token mode for it and its children \
             (see get_tokens); var() takes each token's value in that mode.\n\
             Returns the ids of the created top-level layers and the styles that were ignored.",
            json!({
                "html": {"type": "string", "description": "HTML to parse into layers."},
                "targetNodeId": {"type": "string", "description": "Target node id, or \"root\" for the page."},
                "mode": {"type": "string", "enum": ["insert-children", "replace"]},
                "pageId": page_id(),
            }),
            &["html", "targetNodeId", "mode"],
            false,
        ),
        tool(
            "create_shape",
            "Create a whiteboard element: rectangle, ellipse, diamond, triangle, polygon, star, \
             line, arrow, sticky (sticky note with text) or text. Coordinates are world units, or \
             relative to parentId when given. Lines and arrows take `points` ([[x,y],...], at \
             least two) and arrows may bind to layers with startNodeId/endNodeId (the arrow then \
             follows them). `styles` takes the same CSS as update_styles plus the hand-drawn keys \
             sketch, roughness (0 architect, 1 artist, 2 cartoonist), fillStyle (hachure, \
             cross-hatch, solid, zigzag, dots), strokeStyle (solid, dashed, dotted). Arrows also \
             take route (straight, curved, elbow) and startHead/endHead (none, arrow, triangle, \
             dot, bar, diamond).",
            json!({
                "type": {"type": "string", "enum": ["rectangle", "ellipse", "diamond", "triangle", "polygon", "star", "line", "arrow", "sticky", "text"]},
                "x": {"type": "number"}, "y": {"type": "number"},
                "width": {"type": "number"}, "height": {"type": "number"},
                "points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}},
                "text": {"type": "string", "description": "Text for sticky and text."},
                "name": {"type": "string"},
                "parentId": {"type": "string", "description": "Frame to place the shape in."},
                "startNodeId": {"type": "string"}, "endNodeId": {"type": "string"},
                "route": {"type": "string", "enum": ["straight", "curved", "elbow"]},
                "startHead": {"type": "string"}, "endHead": {"type": "string"},
                "sides": {"type": "integer", "description": "Polygon sides / star points."},
                "styles": styles_schema("CSS and hand-drawn styles."),
                "pageId": page_id(),
            }),
            &["type"],
            false,
        ),
        tool(
            "update_styles",
            "Update styles on one or more nodes (camelCase CSS, like React.CSSProperties). Setting \
             left/top on an artboard moves it. textStyle: \"heading-l\" applies a text style to \
             text layers; keys after it override single fields. effectStyle: \"elevation-2\" gives a \
             layer an effect style's shadows and blurs, as does boxShadow: \"var(--elevation-2)\". It \
             wins over shadow and blur values in the same update; setting one by value without it \
             detaches the style. mode: \"Dark\" makes frames and \
             their children show tokens in that mode (\"inherit\" clears it). Hand-drawn keys: sketch, \
             roughness, fillStyle, strokeStyle, seed. Styles with no effect are returned under \
             ignoredStyles.",
            json!({"updates": {"type": "array", "description": "Each item applies styles to nodeIds.", "items": {
                "type": "object",
                "properties": {
                    "nodeIds": {"type": "array", "items": {"type": "string"}},
                    "styles": styles_schema("CSS properties."),
                },
                "required": ["nodeIds", "styles"],
            }}}),
            &["updates"],
            false,
        ),
        tool(
            "set_text_content",
            "Set the text of one or more Text or Sticky nodes (batch).",
            json!({"updates": {"type": "array", "items": {
                "type": "object",
                "properties": {"nodeId": {"type": "string"}, "textContent": {"type": "string"}},
                "required": ["nodeId", "textContent"],
            }}}),
            &["updates"],
            false,
        ),
        tool(
            "rename_nodes",
            "Rename layers (batch). Names are truncated to 50 characters.",
            json!({"updates": {"type": "array", "items": {
                "type": "object",
                "properties": {"nodeId": {"type": "string"}, "name": {"type": "string"}},
                "required": ["nodeId", "name"],
            }}}),
            &["updates"],
            false,
        ),
        tool(
            "delete_nodes",
            "Delete nodes and their descendants. Verify parents with get_node_info first when unsure.",
            json!({"nodeIds": ids("Nodes to delete.")}),
            &["nodeIds"],
            false,
        ),
        tool(
            "duplicate_nodes",
            "Deep-clone nodes. Each copy goes under parentId when given, else next to its source; \
             duplicated artboards are placed in an empty spot. Returns new ids and a \
             descendantIdMap from every original descendant id to its copy.",
            json!({"nodes": {"type": "array", "items": {
                "type": "object",
                "properties": {"id": {"type": "string"}, "parentId": {"type": "string"}},
                "required": ["id"],
            }}}),
            &["nodes"],
            false,
        ),
        tool(
            "move_nodes",
            "Move nodes keeping their ids. Each move is {nodeId, before: siblingId}, {nodeId, \
             after: siblingId} or {nodeId, parentId, index?} (parentId \"root\" = the page root). \
             Moves apply in order. In auto-layout parents this changes the visual order; in free \
             parents it changes stacking and keeps the world position.",
            json!({"moves": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "nodeId": {"type": "string"}, "before": {"type": "string"},
                    "after": {"type": "string"}, "parentId": {"type": "string"},
                    "index": {"type": "integer", "minimum": 0},
                },
                "required": ["nodeId"],
            }}}),
            &["moves"],
            false,
        ),
        tool(
            "select_nodes",
            "Select nodes on the page the user is viewing so they can see what you mean; \
             zoom: true also frames them in the viewport. An empty list clears the selection.",
            json!({"nodeIds": ids("Nodes to select."), "zoom": {"type": "boolean"}}),
            &["nodeIds"],
            false,
        ),
        tool(
            "undo",
            "Undo the last change on a page (the user's or an agent's).",
            json!({"pageId": page_id()}),
            &[],
            false,
        ),
        tool(
            "redo",
            "Redo the last undone change on a page.",
            json!({"pageId": page_id()}),
            &[],
            false,
        ),
        // ------------------------------------------------------------------ design tokens
        tool(
            "get_tokens",
            "Lists the project's design tokens (colors, spacing, typography...), grouped by \
             type, and its text styles (`class` and one line of CSS) and effect styles (`var` \
             and one line of CSS) in short form. Tokens and styles belong to the project, so \
             every page shares them. By default only the \
             file's own, plus one line per design system the file uses; pass `system` to list \
             a design system's. Tokens can have a value per mode (`modes` lists the file's, \
             the first is the default); a token's `modes` holds only values that differ. CSS \
             puts other modes in `[data-theme=\"dark\"]` blocks, and writes each effect style \
             as a variable (`--elevation-2: 0px 1px 2px …`, plus `--elevation-2-blur` and \
             `--elevation-2-backdrop-blur` when it has blurs).",
            json!({
                "system": {"type": "string", "description": "A design system the file uses (id, name or prefix) to list its tokens instead, or \"all\" for every token."},
                "format": {"type": "string", "enum": ["json", "css", "tailwind"], "description": "\"json\" (default) returns structured tokens, \"css\" a `:root { ... }` stylesheet, \"tailwind\" a Tailwind v4 `@theme { ... }` block."},
                "namePattern": {"type": "string", "description": "Glob matched against the full CSS variable name, ignoring case. `*` is the wildcard: \"--color-*\", \"*-500\", \"*brand*\". Text styles match as \".class\"."},
                "types": types_filter(),
            }),
            &[],
            true,
        ),
        tool(
            "create_tokens",
            "Creates design tokens. Each entry needs `type`, `name` and `value`; use \
             `var(--other-token)` as the value to alias a token of the same type. Returns one \
             result per entry, in order, with errors reported per entry. Creating a token that \
             already exists with the same type, name and value returns it. `value` is the first \
             mode's; `modes` gives others, like {\"Dark\": \"#111111\"}. Define color tokens \
             semantically first, neutrals before accents; order other types smallest first. \
             Reuse tokens before adding new ones.",
            json!({"tokens": {"type": "array", "minItems": 1, "description": "Tokens to create.", "items": {
                "type": "object",
                "properties": {
                    "type": token_type(),
                    "name": {"type": "string", "description": "CSS custom property name, e.g. \"--color-primary\"."},
                    "value": token_value(),
                    "modes": mode_values(),
                    "description": {"type": "string", "maxLength": 1024, "description": "What the token is for and how to use it."},
                },
                "required": ["type", "name", "value"],
            }}}),
            &["tokens"],
            false,
        ),
        tool(
            "set_tokens",
            "Updates or deletes design tokens by CSS variable name, applied in order. Set \
             `newName` to rename, `value` to change the value (`var(--other)` aliases), \
             `modes` to change other modes' values (null makes a mode use `value`), \
             `delete: true` to delete. Changing a value updates every layer bound to the token, \
             on all pages. Deleting keeps those layers' current values. Results are per entry.",
            json!({"tokens": {"type": "array", "minItems": 1, "description": "Changes to apply.", "items": {
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "The token's CSS variable name, e.g. \"--color-primary\"."},
                    "newName": {"type": "string", "description": "New CSS variable name."},
                    "value": token_value(),
                    "modes": mode_values(),
                    "description": {"type": "string", "maxLength": 1024, "description": "New description. An empty string clears it."},
                    "delete": {"type": "boolean", "description": "Delete the token."},
                },
                "required": ["name"],
            }}}),
            &["tokens"],
            false,
        ),
        tool(
            "create_text_styles",
            "Creates text styles: named sets of font family, size, weight, style, line height \
             and letter spacing, like \"Heading/L\". Each field is a value or var(--token) of \
             the matching type, so the style follows the token. Missing fields take the \
             defaults. The name becomes a CSS class (\"Heading/L\" is heading-l). Apply a \
             style with class=\"heading-l\" in write_html or {textStyle: \"heading-l\"} in \
             update_styles. Results are per entry.",
            json!({"styles": {"type": "array", "minItems": 1, "description": "Styles to create.", "items": {
                "type": "object",
                "properties": style_props(&[
                    ("name", json!({"type": "string", "description": "Style name, e.g. \"Heading/L\"."})),
                ]),
                "required": ["name"],
            }}}),
            &["styles"],
            false,
        ),
        tool(
            "set_text_styles",
            "Updates or deletes text styles by class or name, applied in order. Set `newName` \
             to rename, any field to change it, `delete: true` to delete. Setting a field drops \
             the per-mode values get_tokens lists under `modes`; a var() value still follows \
             its token in each mode. Changing a style updates every layer that uses it, on all pages, except \
             fields a layer overrides. Deleting keeps those layers' current values.",
            json!({"styles": {"type": "array", "minItems": 1, "description": "Changes to apply.", "items": {
                "type": "object",
                "properties": style_props(&[
                    ("name", json!({"type": "string", "description": "The style's class (\"heading-l\") or name."})),
                    ("newName", json!({"type": "string"})),
                    ("delete", json!({"type": "boolean"})),
                ]),
                "required": ["name"],
            }}}),
            &["styles"],
            false,
        ),
        // ----------------------------------------------------------------- design systems
        tool(
            "list_design_systems",
            "Lists published design systems: tokens and components other projects can use. \
             Returns a short summary of each (name, CSS prefix, version, description, token \
             counts by type, component count), not the tokens. Load one with get_design_system.",
            json!({}),
            &[],
            true,
        ),
        tool(
            "get_design_system",
            "Loads one design system: its usage notes, tokens (one compact line each) and \
             components (name and size). Pass `types` or `namePattern` to load only what the \
             task needs, and `component` to load one component's layers. Works without an open \
             project.",
            json!({
                "id": {"type": "string", "description": "The system's id, name or prefix, from list_design_systems."},
                "types": design_system_types(),
                "namePattern": {"type": "string", "description": "Glob matched against the full CSS variable name, ignoring case, e.g. \"--brand-color-*\". Text styles match as \".class\", components by name (\"Button*\")."},
                "component": {"type": "string", "description": "A component's name: returns its layers as JSX instead of the system."},
                "format": {"type": "string", "enum": ["text", "json", "css", "tailwind"], "description": "\"text\" (default): notes and `--name: value;` lines. \"json\": structured. \"css\" or \"tailwind\": a stylesheet without notes or components."},
            }),
            &["id"],
            true,
        ),
        tool(
            "use_design_system",
            "Lets the open file use a design system, as one undo step. Its tokens become \
             read-only CSS variables named with the system's prefix (--brand-primary), so \
             var() in styles binds to them. The file gets each new version when it opens.",
            json!({"id": {"type": "string", "description": "The system's id, name or prefix."}}),
            &["id"],
            false,
        ),
        tool(
            "remove_design_system",
            "Stops the open file using a design system, as one undo step. Layers bound to its \
             tokens keep their values. With `detach: true` its tokens become local tokens \
             instead, keeping their names and bindings.",
            json!({
                "id": {"type": "string", "description": "The system's id, name or prefix."},
                "detach": {"type": "boolean", "description": "Keep the tokens as local tokens."},
            }),
            &["id"],
            false,
        ),
        tool(
            "publish_design_system",
            "Publishes the open file's own tokens, text styles and effect styles as a design system other \
             files can use, or publishes a new version. Files that use it update when they \
             open. Omitted fields keep their last values. The prefix is fixed after the first \
             publish. Not an undo step: publish again to roll back. Returns the changed token \
             and style names.",
            json!({
                "name": {"type": "string", "description": "Defaults to the project name."},
                "prefix": {"type": "string", "description": "CSS prefix for the tokens in other files: lowercase letters, digits and dashes, at most 16 characters. Defaults to the name."},
                "description": {"type": "string", "maxLength": 280, "description": "One line on what the system is for. An empty string clears it."},
                "notes": {"type": "string", "maxLength": 10000, "description": "Usage notes in Markdown, for people and agents: which tokens to use where. An empty string clears them."},
            }),
            &[],
            false,
        ),
        tool(
            "finish_working_on_nodes",
            "Call when done creating or editing. Saves the project.",
            json!({"nodeIds": ids("Optional nodes you worked on.")}),
            &[],
            false,
        ),
    ];
    all.extend(components::definitions());
    all
}

pub const GUIDE_INSTRUCTIONS: &str = "\
# Designing in Sketch over MCP

1. Orient: get_basic_info → pages, artboards, sizes, folder; get_selection → what the user
   points at.
2. Plan a brief before a new design: palette (3–5 colors), type scale (e.g. 12/14/16/20/28/40),
   spacing scale (4/8/12/16/24/32), corner radius.
3. Create an artboard sized for the device (create_artboard). Artboards are flex columns.
4. Fill it with write_html, one visual group per call, into the artboard id. Keep the returned
   ids: later calls can target them (insert-children) or replace them.
5. Iterate with update_styles / set_text_content / duplicate_nodes instead of rewriting.
6. Review with get_screenshot (scale 1; 2 for small text). Fix clipping with
   update_styles({height: \"fit-content\"}) on the artboard.
7. finish_working_on_nodes at the end (also saves the project).

Layout model: every container is an auto-layout frame (flexbox). display:flex with
flex-direction row/column, gap, padding, justify-content (start, center, end, space-between),
align-items (start, center, end; stretch is the default and makes children fill the cross axis
when the container has a definite size). flex-grow:1 fills the main axis; width:100% fills too.
height/width fit-content hug the content. position:absolute + left/top places freely.

Folders: projects sit at the top level or in one folder, and folders don't nest. list_resources
shows all folders plus each project's `location`, so answer \"where is X?\" from that.
- \"Make this in Acme\": find Acme in list_resources, then create_file with parentId.
- \"Put X in Acme\": move_resources with X's id and Acme's id as parentId.
- \"Take X out of its folder\": move_resources with parentId \"root\".
- Folder not listed: create_folder if the user asked for a new one. If it looks like a typo of
  a listed folder, use that folder or ask.
Both move_resources and rename_resource work on the open project. Report the returned name,
which can differ from the old one after a name clash.
- \"Delete X\": trash_resources moves projects and folders to the trash, where the user can
  still get them back. Say that it's in the trash. Deleting forever and emptying the trash
  are left to the user. Trashing the open project closes it and the app shows the home
  screen, so only do that when the user asked.
- \"Bring X back\": list_trash, then restore_resources with its id. A folder's id or name
  restores every project trashed from it.
- \"Make a copy of X\": duplicate_file, then open_file with the returned fileId.

Pages: each page is its own canvas with its own undo history. create_page, duplicate_page,
rename_pages and arrange_pages never switch the user's page. delete_pages can't be undone, so
delete only pages the user asked to remove. move_to_page moves layers to another page (copy:
true copies them), keeping their place on the canvas. Their ids change; use the returned
idMap. Moving main components keeps their instances linked.

Whiteboard: create_shape makes rectangles, ellipses, diamonds, stickies, text and arrows that
bind to shapes (they re-route when shapes move). update_arrows rebinds, reroutes or changes the
heads of an existing arrow. create_stroke draws a pencil stroke through points. Turn on the
hand-drawn look with {sketch: true, roughness: 1, fillStyle: \"hachure\"}. More in
get_guide({topic: \"whiteboard\"}).

Vectors: boolean_nodes combines layers into one path (destructive; curves become segments).
flatten_nodes turns shapes and text into paths. get_node_info returns a path's `vector`
(subpaths of world-space points with optional in/out bezier handles); edit it and send it back
with set_vector_points, or pass an SVG `d` string.

Rich text: set_text_range_styles styles part of a text layer, picked by `match` (the words) or
start/end character offsets: fontWeight, fontStyle, fontFamily, fontSize, color,
textDecoration. reset: true returns the range to the layer's style. get_node_info lists styled
ranges as `textRanges`. write_html can also write runs with <b>, <i> and <span style>.

Design tokens: get_tokens first, and reuse what's there. create_tokens adds project-wide CSS
variables; set_tokens renames, changes or deletes them. A style whose whole value is
var(--token) in create_artboard, write_html, update_styles or create_shape binds that property,
so changing the token updates it on every page. Bindable: background and text color, border
color, border width (borderWidth tokens), border radius, opacity, gap, padding, font size,
weight, line height and letter spacing.
Inside a longer value (1px solid var(--line)) or on an inline span, the token's value is used
without a binding. Setting a plain value on a bound property detaches it. get_computed_styles
and get_jsx show bound properties as var(--token). Letter spacing in em, plain or from a token,
follows the text's font size.

Text styles: a style names a set of text settings (family, size, weight, italic, line height,
letter spacing), each a value or var(--token). get_tokens lists them as a class and one line of
CSS. Pick the style for each text role instead of setting five properties: class=\"heading-l\"
in write_html, or {textStyle: \"heading-l\"} in update_styles. A property set after the style
overrides that field on the layer; get_computed_styles shows textStyle and
textStyleOverrides. create_text_styles adds styles and set_text_styles edits or deletes them;
editing a style updates every layer that uses it.

Effect styles: a style names a list of shadows and blurs, such as Elevation/2. A shadow's color
is a value or var(--token). get_tokens lists them as a variable and one line of CSS. Give a
layer the whole list with box-shadow: var(--elevation-2) in write_html (text-shadow on text), or
{effectStyle: \"elevation-2\"} in update_styles. A style with blurs also writes
--elevation-2-blur for filter and --elevation-2-backdrop-blur for backdrop-filter; any of the
three applies the style. get_computed_styles shows effectStyle and the variables, and get_jsx
writes the variables, or effectStyle where no variable fits (text whose style has spread or
inset shadows); write_html takes that as effect-style. Vector shapes export as inline SVG with
their shadows as filter values, without the style. effectStyle wins over
shadow and blur values set beside it. Setting a shadow or blur by value without it detaches the
style and keeps the rest.
Editing a style in the app updates every layer that uses it.

Design systems: a project can publish its tokens for other projects to use. Load only what the
task needs: list_design_systems gives one summary per system; get_design_system returns one
system's usage notes and tokens, and takes types and namePattern to narrow them. Follow the
usage notes. use_design_system adds a system to the open file: its tokens are read-only and
named with its prefix (--brand-primary), and var() binds to them like any token. Its text
styles are read-only classes with the same prefix (brand-heading-l), and its effect styles
read-only variables (--brand-elevation-2). To customize a token, alias
it from a local token (create_tokens with value var(--brand-primary)). get_tokens lists the
file's own tokens and one line per system; pass system to see a system's tokens.
publish_design_system publishes the open file's own tokens, styles and main components; write
notes that say which token, style or component to use where. unpublish_design_system takes it
off the list; files that use it keep their copy, and publishing again brings it back.

Components: build one as a frame with write_html, then create_component({nodeIds: [frame],
name: \"Button/Primary\"}). Several layers, or a non-frame, are wrapped in a new frame. Place
instances with <x-sketch-instance component=\"Button/Primary\"/> in write_html, on any page
and inside any flex layout; pass the main's id instead of the name when two share it.
get_basic_info lists the file's components. Edit the main (update_styles, write_html,
set_text_content) to change every instance. update_styles, set_text_content and rename_nodes
on an instance's layers make overrides, which survive edits to the main; get_node_info lists
them, reset_overrides clears them, and detach_instances turns an instance into plain layers.
A component can't hold an instance of itself.
To build a component library: make each main, place instances to check them, then
publish_design_system with notes on which component to use where.

Design system components: get_design_system lists each as `Name WxH`; pass component: \"Name\"
to read one's layers as JSX before using it. After use_design_system, place one with
<x-sketch-instance component=\"Button\"/> in write_html, inside any frame and flex layout.
It keeps the component's size unless style sets width or height. write_html returns each
instance's text layers; change a label with set_text_content. Edits to an instance's layers
override the component, and everything else updates when the system publishes a new version.
get_node_info shows instanceOf for an instance. Don't rebuild a component the system has.

Node ids look like \"3fa9c1:14\" (page prefix and node number). Ids stay valid across calls.";

pub const GUIDE_HTML: &str = "\
# write_html reference

Elements: div/section/header/footer/nav/main/article/aside/ul/ol/li/button/form → frames;
p/h1–h6/span/label/a/strong/em/code/pre (elements holding only text) → text layers (with a
frame around them when they have a background, border, padding or box-shadow); img → image
rectangle;
svg → vector layers; br → line break; x-sketch-clone node-id=ID → copy of an existing layer;
x-sketch-instance component=Name → instance of one of the file's components (see get_basic_info)
or of a design system's (see get_design_system).

Default sizes (like CSS): h1 32px bold, h2 24px bold, h3 20px bold, h4–h6 16px bold, p 16px,
small 13px, code/pre 14px; color is inherited, default black.

Styles: width, height (px, %, fit-content), left/top/right/bottom with position:absolute,
display (flex, block, none), flex-direction, flex-wrap, gap, row-gap, column-gap, padding*,
justify-content, align-items, align-self, flex-grow, overflow, background, background-color,
background-image (url(), linear-gradient(), radial-gradient()), background-size/object-fit,
border, border-color/width/style, border-radius (1–4 values), opacity, mix-blend-mode, color,
font-family, font-size, font-weight, font-style, line-height, letter-spacing, text-align, text-decoration.

Effects: box-shadow with lists, inset and spread, text-shadow on text, filter: blur(),
backdrop-filter: blur(). A shadow whose color is var(--token) stays bound to that color token.
Setting one of these replaces the layer's effects of that kind, and none removes them.
box-shadow: var(--elevation-2) applies an effect style (see get_tokens).
Ignored (reported back): margin, grid, transform, filters other than blur(), transitions.";

pub const GUIDE_WHITEBOARD: &str = "\
# Whiteboard elements

create_shape({type, x, y, width, height, styles}) for rectangle, ellipse, diamond, triangle,
polygon (sides), star (sides = points), sticky (text, background color), text (text).
Lines and arrows: create_shape({type: \"arrow\", points: [[x1,y1],[x2,y2]],
startNodeId, endNodeId, route: \"elbow\", endHead: \"triangle\"}). With start/end nodes the
points can be omitted: the arrow is routed between the shapes and follows them when they move.

Hand-drawn style (Excalidraw-like): styles {sketch: true, roughness: 0|1|2,
fillStyle: hachure|cross-hatch|solid|zigzag|dots, strokeStyle: solid|dashed|dotted, seed}.
Colors: backgroundColor (fill), borderColor/stroke, borderWidth/strokeWidth.

Changing an arrow: update_arrows({updates: [{nodeId, endNodeId: \"<shape>\", route: \"elbow\",
endHead: \"triangle\"}]}). endNodeId \"\" or null unbinds an end where it is; startAnchor /
endAnchor [fx, fy] aim an end at part of the shape (fractions of its box, [0.5, 0] is the top
middle); points [[x,y],...] redraw it, and middle points bend a curved arrow. get_node_info
shows `arrow`: route, heads, start/end bindings and world points.

Pencil: create_stroke({points: [[x, y, pressure?], ...], width, color}) draws a smoothed
freehand stroke; recolor it with update_styles {color} and erase it with delete_nodes.";
