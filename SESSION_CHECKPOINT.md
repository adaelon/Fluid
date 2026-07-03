# SESSION_CHECKPOINT - 2026-07-03 S-FSQ completion

## Freshness
- Commit at write time: `ca5d2b5` - optimize collapsed file tree and function capsule display.
- On read, compare with `git log -1 --oneline`; if different, trust git.
- Working tree is intentionally dirty with S-FSQ implementation/docs.
- Pre-existing untracked files: `defaults`, `todo.md`; leave them untouched unless the user asks.

## Goal State
S-FSQ is implemented as an explicit selected-file-set relationship query:
the user selects 2+ files, switches Query Terminal scope to selected files,
and the backend answers through `WS /api/query-files` using
understand-anything graph summaries/edges first, with bounded on-demand source
slices only for model-named graph nodes inside the selected files.

## Completed Slices
1. S-FSQ-1 frontend selection UI.
   - `web/src/App.vue`: owns `fileSelectionMode` and selected paths; clears on root switch.
   - `web/src/FileTree.vue` and `web/src/TreeNode.vue`: file checkboxes in selection mode.
   - `web/src/QueryPanel.vue`: current/selected scope switch, select/clear controls, local <2-file hint.
   - `web/src/styles.css`: selection controls and selected row styling.
2. S-FSQ-2 backend graph context.
   - `context_assembler.rs`: `assemble_file_set_context`, selected files/symbols/internal edges/boundary edges.
   - `routes.rs`: registered `WS /api/query-files`; clear errors for <2 files, no graph, or missing graph file nodes.
3. S-FSQ-3 bounded source fetch.
   - Fetchable targets come only from selected graph class/function nodes with `lineRange`.
   - Planning prompt asks for node IDs; `slice_file_set_sources` white-lists IDs, dedupes, numbers lines, and caps budget.
   - Planning failure falls back to graph-only answer; no recursion, no file activation, no cache writes.
4. S-FSQ-4 integration/docs.
   - `web/src/api.ts`: `streamQueryFiles`.
   - `QueryPanel.vue`: selected scope calls `/api/query-files` with selected paths.
   - `README.md`, the slice plan doc, and the code-trail doc updated.

## Verification
- `npm run build` in `web/`: passed.
- `cargo test`: passed 116/116 after final rerun.
- One earlier parallel run raced `vite build` against Rust static-asset tests; rerunning `cargo test` alone passed.
- Manual browser plus real LLM smoke is not recorded in this checkpoint.

## Dirty Files To Expect
- Modified: `CONTEXT.md`, `README.md`, `SESSION_CHECKPOINT.md`,
  `crates/fluid-server/src/context_assembler.rs`, `crates/fluid-server/src/routes.rs`,
  S-FSQ docs under `docs/`,
  `web/src/App.vue`, `web/src/FileTree.vue`, `web/src/QueryPanel.vue`,
  `web/src/TreeNode.vue`, `web/src/api.ts`, `web/src/styles.css`.
- Untracked S-FSQ ADR: `docs/adr/0019-*`.
- Untracked unrelated/pre-existing: `defaults`, `todo.md`.

## Cold-Start Reading
1. `CONTEXT.md`: terms for Query Terminal, Selected File Set Query, File Selection Mode, Context Graph, On-Demand Source Fetch.
2. `rg -n "S-FSQ|0019" docs`: locate the selected-file-set ADR, slice plan, and code-trail entries.
3. Slice plan doc: S-FSQ section should be marked completed.
4. Code-trail doc: S-FSQ-1..4 entries should be present.
5. Frontend path: `App.vue` -> `FileTree.vue`/`TreeNode.vue` -> `QueryPanel.vue` -> `api.ts:streamQueryFiles`.
6. Backend path: `routes.rs:query_files_ws` -> `prepare_query_files` -> `run_query_files` -> `context_assembler.rs`.

## Guardrails
- Do not convert S-FSQ into whole-project query.
- Do not use open tabs as implicit selection.
- Do not stuff whole selected files into the prompt.
- Do not write source files, activate selected files, or write `.fluid/` during file-set query.
