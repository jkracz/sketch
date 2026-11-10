# Paper's design tokens: behaviour notes

Clean-room study of how Paper's design tokens behave, written so Sketch could implement the
feature from these notes. Behaviour only: nothing from Paper is copied into this repository,
and raw material stays outside it. Sketch's code is in `src/model/tokens.rs`,
`src/editor/tokens.rs`, `src/ui/tokens.rs` and the token tools in `src/mcp/`.

## Sources

Paper Desktop 0.5.18, studied on 2026-11-05 with the app closed, so nothing below was seen
running.

* REA `inspect_desktop_application`: Evidence `ev_4ed5ae3f…`. The app is an Electron shell.
* Byte search of `app.asar`: 0 matches for token code. As `paper-mcp.md` found for the MCP
  tools, the feature ships in the web editor, not the desktop bundle.
* REA `analyze_javascript_application` over the newest editor chunks in the renderer's HTTP
  cache (`main-BWZFE6Wh.js`, `MCPHandlers-8vkuIJuQ.js`, cached 2026-11-05): Evidence
  `ev_bbdabcbfa7eb1b00368c20580d404b64083686466d5016194da640d0d36aeb81`. Syntax coverage is
  complete for both files; semantic coverage is partial, and the code was parsed, never run.
* Public docs (`paper.design/docs/tokens`) and the public tool catalogue
  (`app.paper.design/mcp/desktop/config.json`).

**Unverified:** the Paper MCP tools refuse calls while the app is closed, so these notes come
from static reading and the docs. Nothing was confirmed against a running copy.

## Model

* Tokens belong to the file, so all its pages share them. Each has an id, a name, a type, a
  value, an optional description (1024 characters at most) and a sort position.
* Eleven types: color, fontFamily, fontSize, fontWeight, letterSpacing, lineHeight, opacity,
  breakpoint, container, spacing, radius.
* Listing order is grouped by type in that order, then by sort position within a type.
* Properties reference tokens by id, so renaming never breaks a reference.
* A value is a literal or an alias to another token of the same type.

## Names

* The stored name can be free text. Its CSS variable is derived from it: whitespace, dots,
  quotes and long dashes become `-`, `&` becomes `-and-`, other characters outside
  `[a-z0-9_-]` go, runs of `-` collapse, leading and trailing `-` go, and it is lowercased.
* Agents pass the `--` prefix; it is stripped on input.
* Duplicate names are allowed. A lookup by CSS name takes the first match in listing order.
* Duplicating a token names it `x copy`, then `x copy 2`, and so on.

## Rules

* Create is idempotent: same type, name and value returns the existing token.
* A value that doesn't parse for its type is an error, and so is an alias that would form a
  cycle. Resolution also guards against loops at read time.
* Agent calls report errors per entry, so one bad entry doesn't fail the batch.
* Numeric CSS output: px for sizes, percent for line height and opacity, plain for weight.
* Stylesheets are `:root { ... }` for CSS and `@theme { ... }` for Tailwind v4, one
  `--name: value;` line per token. Aliases stay as `var(--other)`.

## Using tokens

* Applying a token links the property to it. Later edits reach every linked property.
* Detaching keeps the current value and drops the link (Backspace in the picker).
* Deleting a token works the same way for everything that used it: properties keep their
  value, and an alias to the deleted token takes on the value it resolved to. This resolves
  through other tokens deleted at the same time.
* The picker's search matches the start of the value, the name, or any word of the name,
  splitting on whitespace, `:`, `-` and `_`.

## Copying

* Copy and paste work between files. Pasted tokens are independent of their source.
* On paste, an alias to a token outside the copied set looks for a token with the same type
  and name in the destination, and otherwise becomes the value it resolved to.
* A name collision gets a copy suffix. Paper also has a paste that overwrites matches.

## How Sketch differs

| Paper | Sketch | Why |
|---|---|---|
| Node properties hold a token reference, resolved when read | Nodes keep the resolved value and a separate binding map | Renderers and exporters stay unchanged |
| The panel sets or clears each reference | A bound property that no longer shows its token's value is detached when the edit commits | One rule covers every edit path |
| One undo history per file, and agent edits skip it | Token edits undo on the page they were made on, agent edits included | Sketch has one history per page, and each MCP call is one undo step |
| Fractional sort keys, reorderable | Creation order, duplicates after their source | No reordering yet |
| `lineHeight` also accepts px | Multiple or percent only | Sketch stores line height as a multiple |

Sketch adds theme modes, which Paper doesn't have publicly: a value per mode on each token,
and a mode per frame. Not carried over: token libraries (not a public feature in Paper),
overwrite on paste, drag to reorder, and reading tokens from a pasted stylesheet. `fontFamily`,
`breakpoint` and `container` tokens are stored and exported but bind to nothing, because Sketch
has one typeface and no breakpoints. Inline `<span>` styles in `write_html` take the token's
value without a binding, because a text run has no binding map.
