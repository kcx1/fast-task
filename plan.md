# fast-task plan.md

## Context

This plan is the developer-facing companion to `todo.md`. The todo serves as a high-level user wishlist; this file is the prioritized, step-by-step implementation roadmap. Items are organized into three priority tiers based on user impact vs. complexity. Each entry lists the exact files, functions, and patterns to use — drawn from the existing codebase rather than invented from scratch.

**Key patterns to reuse everywhere:**
- Background DB ops: see `task_submit_set_status()` in `tasks.rs` — spawn thread, clone `tx`, send `UpdateMessage`
- Modal popups: `egui::Modal::new(id).show(ctx, |ui| { ... })` — used in `show_help_popup()` in `app.rs`
- Task visibility filtering: `TaskManager::visible_indices()` in `tasks.rs` — all show/hide logic goes here
- Migration: increment `CURRENT_SCHEMA_VERSION` in `migrations.rs`, add `migration_00N_name(db)` fn, add `if version < N` branch in `run()`
- New `UpdateMessage` variants: add to enum in `app.rs`, handle in `thread_sync()` match arm
- Date picker pattern: text field (`due_text` buffer) + `egui_extras::DatePickerButton` + clear button — see `info_editor()` in `info.rs`
- Project list assembly: `assemble_project_list(real)` in `projects.rs`
- Project selection: `FastTask::select_project(idx)` in `app.rs` (persists, fetches tasks); `confirm_project(app, idx)` also switches to Tasks pane

---

## Recommended Execution Order

**Phases 0–2 are complete. Phase 3 is mostly complete.** The table below summarises the remaining work before Phase 4 feature development begins.

### Phase 3 Remaining — Quality gate (do before building new features)

These are the last items between the current state and a clean, warning-free, well-tested tree.

B. **`/` filter focus** (todo.md Issues): pressing any relevant key while a filter is active should
   re-focus the filter TextEdit so the user can continue filtering without reaching for `/` again.

C. **`visible_indices()` per-frame** (todo.md egui-specific): `real_index()` and `get_current_task()`
   each call `visible_indices()` internally, so it still fires 2–3× per frame. Move to a cached
   field on `TaskManager` invalidated on tasks/filter change, or thread the computed slice through
   the call chain.

D. **Shared read-only task card** (todo.md Polish): `detail_panel()` in `tasks.rs` and the
   read-only block in `info.rs` render the same content twice. Extract a `task_card(ui, &Task)`
   widget in `widgets/common.rs`.

E. **H10 — clippy + fmt gate**: Run `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`
   on every push. Add as `.github/workflows/ci.yml` or `.git/hooks/pre-commit`. The tree is now
   warning-clean; add the gate before it drifts.

F. **Remaining Polish (low-risk)**: `Task::new` builder (7 positional args), `if let Some(tags)
   && !tags.is_empty()` let-chain collapses, remaining tooltip coverage (✕ clear buttons,
   status/priority icons in task list), error message friendliness.

### Phase 4 — Features (value/complexity ramp; field-adding ones are safe on H1 + H7)
19. **Item 8 — copy/paste tasks** (small, no migration, high daily utility).
20. **Item 9 — syntax highlighting** (recurring tasks ✓ and annotations ✓ are done; this is next).
21. **Item 7 — dependencies** → **Item 12 — time tracking** (more niche; same field+migration shape).
22. **Item 10 — keybind config** (explicitly "after the codebase stabilizes" — it reads every binding).
23. **Item 11 — local web share** (large; needs H7 ✓).
24. **Item 13 — cloud sync** (largest; needs H1 ✓ + H7 ✓ + H9). Run **H9** (history-log concurrency
    audit) immediately before this — it's the only remaining H, because it only matters once sync
    adds a third concurrent writer.

> The `## Sequencing Dependencies` block at the bottom is the hard-dependency graph behind this
> order; this section is the recommended linear walk through it.

---

## Pre-Production Hardening

All H1–H8 are **complete**. The two remaining items are listed below.

### H9. Concurrency audit of the history log (precedes item 13)
**Files:** `src/database/database.rs`
- Up to 10 spawn sites call into one shared `DB`; `undo` reads "latest record by `_id` desc"
  while another thread may be appending via `append_history`. Confirm PoloDB's
  transactional/isolation guarantees and, if needed, guard append/undo/redo with a mutex so rapid
  edits + undo can't interleave. Must be settled before cloud sync (item 13) adds a third writer.

### H10. clippy + fmt gate
**Files:** new `.github/workflows/ci.yml` or `.git/hooks/pre-commit`
- Run `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` on every push. The tree
  is currently warning-clean; add the gate before that state can regress.

---

## P1 — Model-Touching or Subsystem Work

### 7. Task dependencies (blocked/blocking)
**Migration: v7** (no-op)

- Add `pub blocked_by: Option<Vec<ObjectId>>` to `Task` with `#[serde(default)]`; add to `task_set_doc()` in `database.rs`
- In `task_table()`: when `task.blocked_by` is non-empty, use `colors::OVERLAY0` for text
- In `info_editor()`: add "Blocked by" section — list blocking tasks by `title`, task-search `TextEdit` to add new, remove button per entry
- Add `task_submit_link()` and `task_submit_unlink()` background helpers

### 8. Copy / paste tasks (`y` / `p`)
**Files:** `src/ui/tasks.rs`, `src/ui/app.rs`

`y` copies the current task, `p` pastes it below the cursor as a new task. Key conflict: `p` is currently "Go to Projects" in `set_window_state()` in `keys.rs` — resolve by requiring `p` for paste only when in the Tasks window state (not global).

- Add `pub clipboard_task: Option<Task>` to `TaskManager`
- In `normal_mode_keybinds()`, bind `Key::Y`: clone `get_current_task()` into `clipboard_task` (clear status to `NotStarted`, generate new `id` and `order`)
- Bind `Key::P` in `task_state` (not globally): if `clipboard_task.is_some()`, `task_submit_create` with the cloned task inserted below cursor; else fall through to Projects navigation
- Update help popup

---

## P2 — Large / New Crates / Architecture

### 9. Syntax highlighting in details
**Migration: v8** (no-op)

- Add `syntect = { version = "5", default-features = false, features = ["default-syntaxes", "default-themes"] }` to `Cargo.toml`
- Add `pub enum CodeLanguage { Rust, Python, Lua, JavaScript, CSS, Shell }` to `models.rs`
- Add `pub language: Option<CodeLanguage>` to `Task` and `TaskWriter`; add to `task_set_doc()` in `database.rs`
- In `info_editor()`: show `ComboBox` for `language` when `writer.code == true`
- Create `src/ui/widgets/code_view.rs` with `syntax_highlighted_label(ui, code, language)`:
  - Use `std::sync::OnceLock` to cache `SyntaxSet::load_defaults_frozen()` — expensive, must not run per frame
  - Split `HighlightLines` output into `egui::RichText` fragments; batch same-color spans
  - Render inside `egui::ScrollArea::vertical()`
- Call in `detail_panel()` and `info_state()` read-only when `task.code && task.language.is_some()`
- Register `pub mod code_view` in `src/ui/widgets/mod.rs`

### 10. Keybind config + leader key
**Files:** new `src/config.rs`, `src/ui/app.rs`, `src/ui/keys.rs`

Allow users to rebind keys via a config file with a UI to edit it. Add a `<leader>` prefix key (default `Space`) so the keyspace can grow without conflicts.

- Define a `KeyConfig` struct in `src/config.rs` (serde, stored as TOML at `~/.config/fast-task/keys.toml`)
- Load on startup; fall back to defaults if missing
- Thread `KeyConfig` through to all keybind dispatch functions instead of hardcoded `Key::*` checks
- Add a "Keybinds" screen accessible from the help popup (`?` → `e` to edit) showing all bindings as editable fields
- Write config back to disk on save
- Add a `leader: Key` field to `KeyConfig` (default `Space`); in normal-mode keybinds, intercept the leader key and await a second key to dispatch leader-prefixed chords
- Move any future bindings that conflict with single-key navigation behind the leader (e.g. `<leader>y` / `<leader>p` for copy/paste once `p` conflicts with Projects)

### 11. WASM / local share
**No migration.** Build the desktop app's axum server into a real LAN share: it serves a JSON API
over the local `DB` plus a static WASM bundle that renders a read-only task view in the browser.

`AppType` on `FastTask` is already wired and documented for this branch; gate native-only paths on it.

Delivered as five vertical chunks; each leaves the tree building.

**Decisions needed before 11.4** (documented inline, confirm when implementing):
- *Repo layout:* convert to a Cargo workspace with a `share-web` member, vs. a standalone sibling crate.
- *Model sharing:* extract `models.rs` types into a tiny shared `model` crate vs. duplicate them as
  WASM-side DTOs. A shared crate keeps the wire format honest; it must not pull in `polodb_core`.

#### 11.1 — Server lifecycle + toggle (do first)
**Files:** `src/ui/app.rs`, `src/local_share/server.rs`
- `start_local_share()` currently builds a fresh `tokio::runtime::Runtime` every frame and drops it
  at function end, killing the server. Replace with a runtime that outlives the closure: add
  `share_runtime: Option<tokio::runtime::Runtime>` (or a `JoinHandle`) to `FastTask`; start the
  server exactly once on the `false → true` edge of `local_share`.
- Add a toggle (top panel button or `Shift+W` keybind) flipping `self.local_share`.
- Add a status-bar indicator: show `󰖟 shared @ http://<lan-ip>:8080` when active.
- Add `fn local_addr() -> Option<String>` to resolve the LAN IP for display.
- New crate (optional, display only): `local-ip-address = "0.6"`.

#### 11.2 — Solidify the read API + CORS
**Files:** `src/local_share/server.rs`
- Add `tower_http::cors::CorsLayer::permissive()`.
- Replace `todo!("Send an Error Json")` and `.expect(...)` calls: define `struct ApiError(StatusCode, String)` implementing `IntoResponse`.
- Add `GET /projects` so the browser view can label the current project.
- Make unimplemented `POST/PATCH/DELETE` handlers return `501 Not Implemented`.
- New crate: `tower-http = { version = "0.6", features = ["cors"] }`.

#### 11.3 — Embed web assets for the shipped binary
**Files:** `src/local_share/server.rs`, `Cargo.toml`
- Replace `serve_file`'s `std::fs::read` with `rust-embed`: `#[derive(RustEmbed)] #[folder = "src/local_share/web"] struct WebAssets;`.
- Keep a `#[cfg(debug_assertions)]` branch that reads from disk for fast iteration.
- New crate: `rust-embed = "8"`.

#### 11.4 — Minimal WASM client (the actual browser view)
**Files:** new `share-web/` crate, root `Cargo.toml`, `src/local_share/web/index.html`
- A separate crate is mandatory: the desktop crate pulls in `polodb_core`, `dirs`, `tokio` full, and `LazyLock<DB>`, none of which compile to `wasm32`.
- `share-web` deps: `eframe`/`egui` (web features), `serde`, and `ehttp = "0.5"` for fetch (**not** `reqwest::blocking`).
- Port the render-only code from `tasks.rs`/`info.rs`, stripped of every edit path and keybind.
- Re-poll `/tasks` on a ~5 s interval for the "live" feel.
- Build command: `wasm-pack build share-web --target web --out-dir ../src/local_share/web`.

#### 11.5 — Write-back routes (optional, later)
**Files:** `src/local_share/server.rs`, `src/database/http_database.rs`
- Implement `create_task`/`update_task`/`delete_task` handlers against `DB`.
- Finish `HttpDatabase`: fix commented-out `create_task` body, add `new(base_url)` constructor.
- Gate writes behind a runtime flag so the default share stays read-only.

### 12. Time tracking
**Migration: v9** (no-op)

- Add `pub time_entries: Option<Vec<TimeEntry>>` to `Task`; `TimeEntry { started: DateTime, stopped: Option<DateTime> }`; add to `task_set_doc()` in `database.rs`
- Add `pub active_timer: Option<(ObjectId, DateTime)>` to `AppState` (in memory only, not persisted until stopped)
- Bind `Ctrl+T` to start/stop timer on current task
- In `info_state()` read-only view: show total accumulated time + "running since X" if active
- In status bar: show a ticking indicator (`⏱ 0:14`) when a timer is running
- Background fns `task_submit_start_timer()` / `task_submit_stop_timer()` following spawn-thread pattern

### 13. MongoDB cloud sync
**Migration: v9** (next free slot). PoloDB stays source of truth; a background engine pushes local
changes to Mongo and pulls remote ones, merging last-write-wins by `Task.modify_date`.
The `mongodb` driver is gated behind `[features] cloud-sync = ["dep:mongodb"]`.

**Note:** do not implement Mongo in `http_database.rs` (that file is the HTTP/LAN client). Cloud
sync gets its own `src/database/mongo.rs` and `src/database/sync.rs`.

**Decisions needed:**
- *Delete propagation:* soft-delete via `Task.deleted_at` vs. tombstone collection. Plan assumes soft-delete.
- *Tie-break on equal `modify_date`:* remote wins. Plan assumes remote wins.

#### 13.1 — Config loading (unconditional, no mongodb dep)
**Files:** new `src/config.rs`, `src/ui/app.rs`
- `struct Config { mongo_uri: Option<String> }` (serde, TOML at `dirs::config_dir()/fast-task/config.toml`). `FAST_TASK_MONGO_URI` env var overrides.
- `Config::load() -> Config` never errors; `Config::save(&self)` writes back.
- New crate: `toml = "0.8"`.

#### 13.2 — Sync state plumbing (unconditional UI)
**Files:** `src/ui/app.rs`
- Add `enum SyncState { Disabled, Idle, Syncing, Error(String) }` and `UpdateMessage::SyncStatus(SyncState)`.
- Hold `sync_state: SyncState` on `FastTask`; render in status bar next to existing indicators.

#### 13.3 — `RemoteStore` trait + gated Mongo impl
**Files:** `src/database/mod.rs`, new `src/database/mongo.rs`, `Cargo.toml`
- `RemoteStore` trait: `pull_since`, `push`, `ensure_indexes`.
- `#[cfg(feature = "cloud-sync")] pub mod mongo;` with `struct MongoStore` implementing `RemoteStore`.
- `[features] cloud-sync = ["dep:mongodb"]`.

#### 13.4 — Sync engine (local-first merge)
**Files:** new `src/database/sync.rs`, `src/ui/app.rs`
- `SyncEngine` on a spawned thread: pull → merge → push → persist watermark.
- Strictly follow spawn-thread → `tx.send(SyncStatus(Syncing))` → … → `SyncStatus(Idle)` + `Refresh`.
- Depends on 13.1, 13.2, 13.3, 13.5.

#### 13.5 — Migration: sync watermark + delete tracking
**Files:** `src/database/migrations.rs`, `src/database/models.rs`, `src/database/database.rs`
- Add `pub deleted_at: Option<DateTime>` to `Task`; `delete_task` sets it; `get_tasks` filters `deleted_at.is_none()`.
- Store `last_sync` watermark in `app_state` collection.

---

## Migration Version Map

| Version | Name | Feature |
|---------|------|---------|
| ~~v1~~ | ~~`migration_001_fix_priority_typo`~~ | ~~Priority field rename~~ — **done** |
| ~~v2~~ | ~~`migration_002_remove_duplicate_priority`~~ | ~~Duplicate field cleanup~~ — **done** |
| ~~v3~~ | ~~`migration_003_create_tags_collection`~~ | ~~Tag store~~ — **done** |
| ~~v4~~ | ~~`migration_004_add_wait_until`~~ | ~~Wait date~~ — **done** |
| ~~v5~~ | ~~`migration_005_add_recurrence`~~ | ~~Recurring tasks (item 4)~~ — **done** |
| ~~v6~~ | ~~`migration_006_add_annotations`~~ | ~~Annotations (item 6)~~ — **done** |
| v7 | `migration_007_add_dependencies` | Dependencies (item 7) |
| v8 | `migration_008_add_language` | Syntax highlighting (item 9) |
| v9 | `migration_009_add_sync_fields` | Cloud sync: `deleted_at` + `last_sync` watermark (item 13.5) |

All v5–v9 migrations are no-ops (all new fields are `Option<T>` with `#[serde(default)]`).
Version numbers are assigned in landing order — whichever feature ships first takes the lower
number, and the rest renumber accordingly. Local web share (item 11) needs no migration.

---

## Sequencing Dependencies

```
H9 (history concurrency) — precedes 13 (cloud sync adds a third concurrent writer)
H10 (clippy+fmt gate)    — add before any new feature work so regressions are caught immediately

4  (recurring)          — DONE
6  (annotations)        — DONE
7  (dependencies)       — start anytime
8  (copy/paste)         — start anytime; resolve p key conflict with leader (item 10)
9  (syntax highlight)   — start anytime; adds syntect crate
10 (keybind config)     — best done after codebase stabilizes; leader key lands here too
12 (time tracking)      — start anytime; in-memory timer state + DB entries

11 (local web share)    — read-only MVP does NOT need write routes.
                          Internal order: 11.1 server lifecycle/toggle → 11.2 API + CORS →
                          11.3 embed assets → 11.4 WASM client (separate crate) →
                          11.5 write routes (optional, last).

13 (cloud sync)         — standalone; largest item. Internal order: 13.1 config → 13.2 sync-state UI
                          → 13.3 RemoteStore trait + gated Mongo impl → 13.5 migration (deleted_at +
                          watermark) → 13.4 sync engine (ties the rest together). 13.4 depends on all
                          four preceding chunks.
```

---

## Verification

For each implemented item:
1. `cargo build` — no warnings, no errors
2. `cargo run` — launch the app and exercise the feature manually
3. For model changes: delete `~/.local/share/todo.db`, relaunch to confirm clean migration
4. For DB field additions: create a task, quit, relaunch — confirm field persists correctly
5. For UI items: verify in both narrow and wide window configurations
