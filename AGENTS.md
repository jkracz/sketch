## Communication

### Content

The engineers working on this codebase spend their whole days working with AI agents. It requies a lot of reading, and they have short attention spans. Therefore it is critical that you respond as cleanly and concisely as possible. Walls of text are the enemy. Be as concise as possible while still being informative.

Run `/unslop` over anything a person will read, before you commit, post, or
send it: commit messages, the PR title and body, README and doc edits, code
comments, and the closing reply. It strips AI tells (em dashes, filler,
hedging, chatbot phrases, puffery, bold-label lists) and replaces fancy
words with plain ones and passive voice with active. Apply it to text you
wrote or changed, not to prose you didn't touch.

### Requesting Human Help

- End every turn where you're blocked on me, or where the next step needs me, with a short "**What I need from you**" section. Numbered, one concrete action or decision per item: exactly what to do, where (which site, app, file or person), and what to send back. e.g. "Open the Convex dashboard, add `OPENAI_API_KEY` under Settings → Environment Variables, then reply 'done'", not "I need the API key set up".
- If there are several, put the quickest or most blocking one first, and say what you'll get on with in the meantime.
- If you don't need anything from me, say "Nothing needed from you right now", and say whether the work is finished or still going.
- Never bury a request for me in the middle of a long update.

## This repo

This repo is Sketch, a native Rust infinite canvas built on wgpu, winit and
egui, plus its download page in `site/`, a single prerendered SvelteKit page
built for Vercel. The README covers building, running and testing the app, and
`site/README.md` covers the page and its image pipeline. The app used to be
called paper-rs. The GitHub repo is now `jkracz/sketch`, but the
checkout folders keep the old name.

- The project began as a fork of `michaelshimeles/sketch` and is developed
  independently. PRs target `jkracz/sketch`. GitHub still lists the repo as a
  fork and `gh` otherwise defaults to the parent, so open PRs with
  `gh pr create --repo jkracz/sketch --base main`.
- Sketch is a clean-room reimplementation modeled on Paper (paper.design).
  Never add Paper's code, shaders, icons, fonts or assets; `analysis/` records
  behavior only. On the download page, keep Paper's name out of the headline,
  logo and assets, don't imply affiliation, and keep Figma's name out of the
  headline too.
- Code ported from another project starts with a comment naming its source
  and gets an entry in `THIRD_PARTY.md`.
- An agent can do anything a person can do in the app. A change that adds
  something a person can do in the UI also lets an agent create, edit and
  delete it over MCP, with the tool descriptions and the guide in
  `src/mcp/tools.rs` updated to match. Until the MCP side ships, the change
  isn't done.

## Checks in this repo

For changes to the app:

- `cargo fmt --check` passes.
- `cargo clippy --all-targets -- -D warnings` passes.
- `cargo test` passes. The tests rewrite the screenshots in
  `docs/screenshots/`, and on a Mac most come out different from the
  committed ones. Unless the change is meant to update them, restore them with
  `git checkout -- docs/screenshots` before committing.

Keep generated evidence under the ignored `.artifacts/` directory.

## Skill sources

The skills were installed with:

```sh
npx skills add michaelshimeles/skills -s '*' -a cursor -a claude-code -a codex
```

`skills-lock.json` records the hash of each one, and `npx skills update -p`
updates them. Don't edit them here; change them upstream.

| Skill | Source |
|---|---|
| `new-feature`, `code-structure`, `evidence-driven-testing` | michaelshimeles/skills |
| `before-and-after` | michaelshimeles/skills, vendored from [vercel-labs/before-and-after](https://github.com/vercel-labs/before-and-after) |
| `greploop` | michaelshimeles/skills, vendored from [greptileai/skills](https://github.com/greptileai/skills) |
| `greploop-apps` | michaelshimeles/skills, compatibility entrypoint requiring `greploop` |
| `unslop` | michaelshimeles/skills, vendored from [cursor/plugins (pstack)](https://github.com/cursor/plugins/tree/main/pstack/skills/unslop) with frontmatter edited so agents apply it unprompted |
