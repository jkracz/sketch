# Agent activity indicator (Paper)

Behavior notes from Paper's web renderer (`app.paper.design`, main bundle, Nov 2026), read
with REA and by hand. The desktop `app.asar` only holds the Electron shell; the indicator is
drawn by the web app. Only behavior is recorded here, no code.

## State

* One entry per MCP session ("agent"), labeled with the client's `clientInfo.name`
  (Claude Code, Cursor, Codex...), else "Agent".
* Each agent keeps a map of node id → `{readAt, writtenAt}`. Every tool call that reads or
  writes nodes records them. A write to a node also remembers its artboard as the "last
  updated artboard".
* An entry expires 40 s after its last read or write. `finish_working_on_nodes` with
  `nodeIds` drops the entries for those nodes and their descendants; without ids it drops the
  whole agent. Closing the MCP session drops the agent too.
* Agents are shared over multiplayer, so collaborators see each other's agents.

## What the user sees

* **Frame indicator.** For every visible node with a fresh entry on the active page, its
  top-level artboard is marked (one mark per artboard, however many agents). The mark is:
  * a 2 px outline 1 px outside the artboard, following its corner radii, in a light tint
    of the selection blue. Its alpha pulses on a 3 s loop: full, eases down to 0.2, back up,
    holds full;
  * a strip of small dots in the label row, right-aligned on the artboard's top edge and
    stopping short of the artboard name: 32 columns × 2 rows, dot ≈ 3 px. Each column has
    a "drop" that falls through the two rows with a soft tail, at a slightly different
    speed per column, so the strip shimmers like rain;
  * 400 ms fade in and out. On fade-in columns appear one by one as their first drop
    arrives.
* **Write reveal.** New content fades in under a short highlight that sweeps top to bottom
  (≈1.5 s; edits and deletes use a shorter, fainter flash). Up to six outlines of the new
  layers flash in sequence.
* **Presence.** The avatar row in the top bar shows one avatar per active agent (harness
  icon), with the agent's name and owner on hover.
* **Pages.** A page with agent activity swaps its icon for an animated indicator in the page
  list.
* **Desktop tab** switches to a busy state while the user's own agent is active.
* **Camera.** The view never moves for agent edits, except once: the first successful run
  during MCP setup zooms to the updated artboard and shows a toast.
* A "Reduce agent animations" setting turns off the reveal sweep.

## Sketch

Sketch keeps the frame indicator, presence and page marker, and drops what needs multiplayer
or a desktop tab. See `src/agents.rs`.
