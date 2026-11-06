# Paper's dark mode: behaviour notes

Clean-room study of how Paper's dark mode behaves. Nothing from Paper is copied into this
repository except the colour values listed here, which are observed behaviour. Sketch's code
is in `src/theme.rs`, `src/settings.rs` and `CanvasStyle` in `src/render/scene.rs`.

## Sources

Paper Desktop 0.5.18, studied on 2026-11-05.

* REA `analyze_javascript_application` over `app.asar`: partial Evidence
  `ev_8f4672d447d17247c0a1c62e0b41c62ead29643cbd15b1f757b9b4378c0a5c84`. It timed out in
  `dist/main.cjs`, so I searched that file by hand for the window colours.
* REA `analyze_javascript_application` over `main-BbXQHLem.js` from the renderer's HTTP cache,
  the newest copy, cached 2026-11-05: Evidence
  `ev_9a4c13e75b7b1414cb2b0d34d1c10c1f49e0b49229e254afc84e41e9ffac97a7`. Syntax coverage is
  complete and semantic coverage is partial. The theme findings below come from text search
  of that file, and the code was never run.
* Theme tokens from the cached `appBar-pPNCX2SI.css` and `styles-BPe-y59v.css`, same date.

**Unverified:** I read these values from static files and did not compare them with a running
copy of Paper side by side.

## Behaviour

* The chrome follows the system with no setting of its own. `:root` sets
  `color-scheme: light dark`, tokens use `light-dark()`, and a few overrides sit in
  `prefers-color-scheme` media blocks. The editor chunk has no theme setting, and the only
  `light` and `dark` string literals in it are emoji skin tones.
* The desktop shell paints its window `#2a2a2a` or `#f9f9f9` from
  `nativeTheme.shouldUseDarkColors`, and updates the title bar overlay when that changes.
* The canvas is page data, not theme. A page has an optional `canvasColor`, and a missing one
  reads as `#eeeeee`. Pages created while the system is dark, and a new file's first page, get
  `#282828` stored on them. Switching the system later doesn't change existing pages.
* New text keeps its default colour unless it is within 0.1 OKLCH lightness of what's beneath.
  Then it becomes black if that background is lighter than 0.7, otherwise white. New paths
  always get black or white from that test. The background is the top opaque fill under the
  pointer, walking up the parents, and then the canvas.

## Dark tokens

| Token | Value | Sketch |
|---|---|---|
| `--color-canvas-dark` | `#282828` | `CanvasStyle::DARK.background` |
| `--color-panel` | `#2a2a2a` | `window`, `sidebar`, `inspector`, `popover` |
| `--color-control` | `#373737` | `control`, `field` (`.field-control` uses it) |
| `--color-control-pressed` | `#3c3c3c` | `control_pressed` |
| `--color-control-border` | transparent | `control_border` |
| `--color-separator` | `#373737` | `separator` |
| `--color-gray-1`, `-2`, `-4` | white at 90, 65, 50% | `text`, `secondary`, `tertiary` |
| `--color-ghost-hover`, `-pressed` | white at 6, 8.5% | `hover`, `pressed` |
| `--color-popup-outline` | `#444`, `#555` on 2x screens | `popover_border`, always `#555` |
| `--color-thumbnail` | white at 10% | `thumbnail` |

## How Sketch differs

| Paper | Sketch | Why |
|---|---|---|
| Canvas colour is stored per page | Canvas follows the active theme | Stamping pages would leave every existing project light. Sketch has no per-page colour |
| No appearance setting | View > Appearance: System, Light, Dark, default System | Requested. System matches Paper |
| Only opaque chrome | Sidebar keeps macOS vibrancy on Mac | Existing Sketch look, so it isn't exactly `#2a2a2a` |
| Blue accent tokens | System accent colour | Existing Sketch behaviour |
| `--color-selection` and `--color-focus` | Selection and guide colours unchanged | Not part of the palette work |

The light palette is unchanged, so its canvas stays `#eeeef0` where Paper uses `#eeeeee`.

Two dark values are my choices, because I found no matching Paper value. Sketch's dot grid uses
`#444444`, and frame labels use white at 65%, matching `--color-gray-2`.

The pencil follows the same rule as text, because Sketch has an ink colour setting where Paper
has none. Its swatch shows the colour new strokes will get on the canvas.
