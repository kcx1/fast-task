This serves as a running todo list for things that need to be implemented. 

We should create plans based on what we have in here and regularly update both this file and the plan. 

## Issues
- [x] **Urgent** Question mark still does not work. On any pane. In any mode. The `?` is the most natural key - if we have to we can look at another.
- [x] Any key to `/` should focus filter. That way you can easily hop back to a filter
- [x] Question mark doesn't raise a help menu when a text field has focus (the `?` character gets consumed by the text editor)
- [x] Can't move multiple tasks in visual block mode. 
- [x] No status popup when multiple tasks are selected via Tab (currently cycles instead of showing picker)
- [x] Task creation is missing the UI icon elements. We need to work on this. 
- [x] Task creating window is too narrow parts of the UI run off. We can migrate the labels to be on top instead of to the left. 
- [ ] Help menu is larger than the actual fast-task window. I'm fine with it being a float panel that floats above the fast task main window rather than being confined to the parameters of the the parent window
    - [x] Popup now scrolls within the window bounds (ScrollArea wrapping the keybinding grid, max_height = viewport - 120px) — the "true float above window" UX requires a separate OS viewport (ctx.show_viewport_immediate) and is a bigger change.
- [ ] we have a conflict between project and paste. We need to solve this. I'm thinking that we should change project. 
    - [ ] Paste should allow shift p and p to paste above or below. 
    - [ ] Shift and shift L should carousel between projects and tasks (and if we add any new panes)
    - [ ] Maybe we could use esc to leave tasks for projects from normal mode?  We should discuss. 

## Database
- [x] We need a store for tags.

## UI 
- [x] We should add color to the task states so that urgent stands out more
- [x] Projects need to be more consistent with tasks. Right now they both look too different. 
    - [x] Projects should highlight the whole row like tasks
- [x] I like the tall narrow window, but we lose a lot when we toggle in the task details, I wonder if we can clean that up a bit? 
- [x] Let's add an icon pack or nerd fonts so that we can use icons for buttons. 
- [x] Buttons should flip font to black when filled with the light blue. Otherwise they're hard to read.
- [x] Info is now a floating window, but I don't like that either, Mayber should try it at the bottom
- [x] Right now the Info title seems uneeded. I think we could manage with making the actual task the title 
    - But if we move it to the bottom, we should reconsider. 
- [ ] Syntax highlighting in task details — add `language: Option<CodeLanguage>` to the Task model (Lua/Rust/Python/CSS/JS), a dropdown in the editor, and `syntect`-based token rendering in the read-only view. Needs a DB migration. See plan.md for full approach.
- [x] Users can't tell what the status means. Should bring colors to the regular task pane.
    - Green should be for when circle is full with checkmark. 
- [x] Instead of cycling `s` this should be a popup that let's the user pick a state of the task. pending, completed ect. 
- [x] We should be able to show completed via a button or filter. 
- [x] When typing tags, we should try to complete the tag based on one in the database. Otherwise, let the user create a new tag. 
    - We should make this case insensitive. 
- [x] I like the bottom pane, but we need to organize it better. 
- [x] Icons and font can be bigger in the details pane.
- [x] App should be opinable so that it pins above all other apps. (this is an egui feature, just need the button)
- [x] The calendar picker is kinda ugly. I wonder if instead I should just give some drop downs or text fileds that try to conver to date? Or if we can clean up the date pciker.
- [x] tab select shouldn't make the selection jump - maybe just maek it a slightly darker green? 
- [x] Tab selection should hold until I relase it (mayber esc?)
- [x] Move the filter bar down to the bottom status bar
- [x] I think we should get rid of Visual Block mode. I don't think it makes as much sense as I wanted it to. 
    - [x] Instead we should allow the users to sort in various ways: 
        - Due Date 
        - Modified
        - Tags 
        - Status 
        - Free ( This is the current way that we order and allow users to change)
- [x] Task creation should include settable status, and ability to mark as done. 
- [ ] We should add time tracking for tasks.
-[x] I put an svg in assets/svgs/ I'd like to convert it and use it for .ico 
    - we should make sure that we're implementing for all windows, macos, and linux




## Core 
- [x] **Search / filter** — `/` opens a filter bar at the bottom; filter by text, tag, priority, or status. Tasks narrow in real time. Probably the highest-impact missing feature as the list grows.
- [x] **Recurring tasks** — model support for a recurrence rule (daily, weekly, monthly, etc.) on a task, with auto-creation of the next instance on completion.
- [x] **Due date urgency / overdue view** — sort or visually surface overdue tasks at the top; an "overdue" or "today" quick-filter. The due date field exists but there's no urgency surfacing.
- [ ] **Task dependencies** — a task can block or be blocked by another task; blocked tasks visually de-emphasized.
- [x] **Annotations** — timestamped notes appended to a task over time, separate from the details field.
- [x] **Wait date** — a "hidden until" date; tasks don't appear in the list until that date passes.

## Keybinds
- [x] Shift + K should ignore the regular K movement. 
- [x] Create a filter for `/` We should open a small buffer at the bottom showing what we're searching. 
- [x] `?` should open a help popup
- [x] `u` / `r` undo and redo — backed by a history log in the database; wired to `u` (undo) and `r` (redo) in normal mode
- [ ] Let's make this config driven, but offer a ui to write edit the config. This way users can update and change what keys do what
- [ ] y should copy tasks, p should paste ( this will collide with projects so we need to figure out what makes the most sense)
- [ ] As keyboard gets more complicated we should implement a <leader> like vim. It should default to <space>, but be configurable when we implement the config

## Local Share
FEATURE: We want to be able to "share" a project locally by hosting a webserver that will server an egui frame to the browser that mirrors a task and info pane (no projects)
- [ ] We should either make sure that the current pane can be made into a wasm - or we should duplicate out just the task pane for a wasm
- [ ] Toggling "share" on should start the local server and keep it running (today it dies the same frame it starts).
- [ ] When sharing is on, show the LAN address (e.g. `http://192.168.x.x:8080`) in the status bar so I can read it off and open it on my phone.
- [ ] Opening the share URL in a browser should show a live, read-only list of the current project's tasks that refreshes on its own.
- [ ] Clicking a task in the browser view should show its details, mirroring the desktop info pane.
- [ ] The shared view should be read-only by default; let me opt in to allowing edits from the browser later.

## Cloud Sync
FEATURE: we want to be able to point the app at a MongoDB instance and allow cloud sync. However, this needs to be local first and the upon connection - resync. 
- [ ] Let me point the app at a MongoDB connection string, either through an environment variable or a config file.
- [ ] The app must stay fully usable with no network and no MongoDB configured.
- [ ] When connected, my changes should sync to the cloud in the background without the UI ever freezing.
- [ ] When I reconnect after being offline, local and remote changes should reconcile automatically (most recent edit wins).
- [ ] Deleting a task on one device should remove it on the others, not have it reappear on the next sync.
- [ ] Show a small sync status in the status bar (idle / syncing / error) so I know where things stand.

## Hardening (pre-production)
These are correctness, robustness, and coherence items found in a pre-production read of the codebase. They block a polished release and several precede the Local Share / Cloud Sync features.

### Correctness / data integrity
- [x] Undo/redo silently drops fields: undoing an edit doesn't revert `wait_until`; redo drops `code`, `status`, and `wait_until`. Editing then undoing/redoing loses data.
- [x] The task `$set` document is hand-written in three places (`update_task`, `undo`, `redo`) plus the server route, and they've already diverged. Centralize into one builder so every field stays in sync.
- [x] Deleting a project cascade-deletes its tasks with no history record, so they can't be undone — silent data loss.

### Robustness (no panics on realistic input)
- [x] Reading a task or loading the task list panics on any DB/BSON error or a single malformed row; route these through the error UI instead.
- [x] First run with a missing data directory or a locked database panics on startup instead of showing an error.
- [x] Errors are silently swallowed in several places (project selection failing to persist, a DB error rendering as "no projects") — surface them through the existing error UI.
- [x] Remove the unreachable `todo!()` landmines (undo/redo error handler, `update_project`, `Database::close`) so they can't crash the app when later wired.
- [x] Project editing is unimplemented (`update_project` is a stub) — implement it or make the path return a real error.

### Coherence / consistency
- [x] Two unrelated `AppState` structs share the name (UI state vs. persisted DB state) — rename the database one.
- [x] Near-identical `DB` / `DATABASE` statics (handle vs. path) are easy to confuse — clarify the names.
- [x] Enum serialization is inconsistent within `Task` (`Priority` hand-rolls `Into<Bson>`, `status` uses `to_bson`) — pick one approach.
- [x] Rename `single_line` to a real `title` (with a serde alias/migration) and fix the `sinlge_line` / `priorty` typos carried in the model.

### Architecture (precedes Local Share + Cloud Sync)
- [x] The whole app calls the global `DB` singleton directly, so no alternate backend (HTTP, MongoDB) can be swapped in and the background ops aren't testable. Move background ops to take a backend trait. This unblocks both big features.

### Performance / desktop polish
- [x] The app repaints every frame unconditionally, burning CPU/battery while idle — repaint only when work is pending.
- [x] Window size is hardcoded and position/size isn't remembered between launches.

### Concurrency
- [ ] Audit the history log for races: many background threads write through one shared DB, and undo reads "latest record" while another thread may be appending. Confirm guarantees before cloud sync adds a third writer.

### Process / tooling
- [ ] Add a `clippy -D warnings` + `fmt --check` gate (CI or pre-commit) so "polished" doesn't regress as the surface grows.

## Polish
Line-by-line production-readiness findings from a full-codebase audit. Grouped by file, most severe first within each group. Items already in `## Hardening` or `plan.md` are not repeated here.

### src/ui/tasks.rs
- [x] tasks.rs:363,406 — `task_submit_edit` and `task_submit_create` never write `writer.status`, so the editor's Status buttons and "Done" button are silently dropped on save (status only changes via the `s` picker); wire `writer.status` into both paths.
- [x] tasks.rs:90 — `TaskWriter::flush()` resets `priority` but not `status`, leaking a stale status into the next new-task form; reset `status` to `NotStarted` too.
- [x] tasks.rs:129 — `selected_tasks: HashSet<usize>` stores indices that go stale when `UpdateMessage::Tasks` replaces the list, so Tab-selection acts on the wrong rows after a refresh/sort; change to `HashSet<ObjectId>` and update the `filter_map` sites (~786, ~819).
- [x] tasks.rs:383,420,436,460,476 — `tx.send(...).expect("Thread closed unexpectedly")` panics the worker thread on app shutdown; replace with `.ok()` to match the other send sites.
- [x] tasks.rs:363,430,444,470,484 — write ops silently drop the change when `DB.one_task` returns `Err`/`None`; surface the failure via `UpdateMessage::Error`.
- [x] tasks.rs:364-377,394-405 — duplicated tag parsing; extract `fn parse_tags(&str) -> Option<Vec<String>>`.
- [x] tasks.rs:428-494 — `task_submit_complete`/`task_submit_set_status` and their `_many` variants are near-duplicates; collapse into one "update task(s) via closure" helper.
- [ ] tasks.rs:281 + info.rs:60 — the read-only task-info card is rendered twice across files; extract a shared widget.
- [x] tasks.rs:19 + models.rs:129 — `ORDER_GAP` is defined twice; keep one (re-export from `models`).
- [x] tasks.rs:686,733 — `let shift = i.modifiers.shift;` is declared twice in `normal_mode_keybinds`; remove the redundant second binding.
- [x] tasks.rs — `visible_indices()` is recomputed several times per frame (task_state, `real_index`, `get_current_task`, keybinds), each an O(n) allocation; compute once per frame and reuse.

### src/ui/info.rs
- [x] info.rs:307-364,368-423 — the due-date and wait-until editors are the same widget written twice; extract a `date_field` helper.
- [x] info.rs:166 — unused `_details` binding from `ui.code_editor`/`add`; drop the binding.
- [x] info.rs:96 + tasks.rs:310 — `if let Some(tags) { if !tags.is_empty()` can collapse to a let-chain (edition 2024).
- [x] info.rs:312 — stale hint example `2025-06-15`; bump to a current-year example.

### src/ui/projects.rs
- [x] projects.rs:25 — `ProjectManager::default()` calls `DB.all_projects().expect(...)`, panicking at startup on a DB error and doing I/O inside `Default`; load projects fallibly outside `Default`.
- [x] projects.rs:236-276 — `project_submit_create` and `project_submit_delete` are near-identical; share the body.
- [x] projects.rs:244,265 + app.rs:338 + thread_sync — the `[All] + real + [None]` project-list assembly is duplicated four times; extract one function.

### src/ui/app.rs
- [x] app.rs:396 — `Refresh` handler FIXME: it doesn't refresh projects, so the list is stale after undo/redo of a project op; refresh both.
- [x] app.rs:185-193 — the project-dropdown selection logic duplicates `confirm_project` (projects.rs:226); reuse it.
- [x] app.rs:28 — `AppType` enum is set in `default()` but never read; added doc comment + `#[allow(dead_code)]` pending item 11/13.
- [x] app.rs:156,159 — lowercase, period-less comments against the otherwise-capitalized house style.

### src/database/models.rs
- [x] models.rs:197 — `Tags` holds a single tag (`content: String`); rename to `Tag`.
- [ ] models.rs:132 — `Task::new` takes 7 positional args (3 of them `Option`), making call sites unreadable; consider a builder.
- [x] models.rs:180 — `impl Into<Bson> for Priority` trips clippy `from_over_into`; implement `From<Priority> for Bson` instead.
- [x] models.rs — `Project.about` is never set (projects.rs:238) or edited; wire it into the project form or remove the field.

### Dead code (triage each: delete vs. keep with `#[allow(dead_code)]` + rationale)
- [x] widgets/widget.rs — the entire file is commented-out drag-and-drop scaffolding; delete or restore it.
- [x] widgets/popups.rs — the whole module is unused (`centered`, `top_banner`, `focus_on_first_frame`, `esc_pressed`, `enter_pressed`, `WindowKind`); delete or adopt.
- [x] widgets/common.rs:16,43 — `hint()` and `labeled_text_edit()` are unused; delete or use.
- [x] theme.rs:153,158 — `accent()` and `muted()` appear unused; delete or use.
- [x] theme.rs:51 — `MODE_VISUAL_BLOCK` icon is unused (visual-block mode was removed); delete.
- [x] errors.rs:31 — `ErrorUi::push_msg` is unused; delete or use.
- [x] app.rs:338 — `get_projects()` is never called (its logic lives in `thread_sync`); delete.

### Cross-cutting
- [x] Backfill `///` doc comments on all `pub`/`pub(crate)` items in one focused pass (representative gaps: `get_tasks`, `task_submit_*`, `order_below`/`order_above`, `format_due_short`, `due_date_color`, `SortOrder`, `TaskWriter`, `TaskManager`).
- [ ] Cargo.toml — reqwest `blocking`, axum `ws`, and tokio `full` are currently used only by stubbed code; intentionally retained for the Local Share roadmap (plan item 11). Audit/trim at release if still unused. (note, not action)

## UI/UX Review

Findings from a full audit of `src/ui/` against the design principles. Grouped by category, most severe first within each group. Items already tracked in `## Polish` or `## Hardening` are not repeated.

### Visual consistency

- [x] tasks.rs:216 — `ui.heading("Tasks")` renders in the default theme heading style (no color), while every other heading uses `common::heading()` which renders LAVENDER+strong; replace with `common::heading(ui, "Tasks")` so all section heads are uniform.
- [x] tasks.rs:1031–1040 — status color bleeds onto the full title text (InProgress→BLUE, OnHold→YELLOW on the title). BLUE and YELLOW on primary content competes directly with due-date urgency coloring and erodes trust in those colors as signals. Move color to the status icon only; keep titles in `colors::TEXT` (SUBTEXT0 for Completed).
- [x] theme.rs:165 — `status_color(NotStarted)` returns `OVERLAY1`, but `task_table` assigns `SUBTEXT0` to NotStarted titles — two different values for one semantic state. Update `status_color(NotStarted)` to return `SUBTEXT0` so both callers are consistent.
- [x] tasks.rs:287 — `detail_panel` inner margin is `{left:10, right:10, top:6, bottom:6}`; both `10` and `6` are off the 4/8/12/16/24 spacing scale. Change to `Margin::same(8)` or `same(12)`.
- [x] info.rs:152 — `add_space(6.0)` after editor heading is off-scale; change to `8.0`.
- [x] info.rs:425,427 — `add_space(10.0)` before separator and `add_space(6.0)` after it are off-scale; change to `8.0` each.
- [x] projects.rs:113 — `add_space(6.0)` inside project row padding is off-scale; change to `8.0`.
- [x] theme.rs:144 — `item_spacing.y = 5` is off-scale (between 4 and 8); change to `4.0`.
- [x] tasks.rs:302,319 / info.rs:213 / tasks.rs:600,675 / app.rs:539–545 — font size `12` is used for secondary text throughout, but the defined Small tier is `11` and Body is `13`; `12` is an orphan. Collapse all non-monospace secondary text to `11` (Small). Size `12` stays reserved for monospace only.
- [x] info.rs:243,255 — `"  Normal"` uses two leading spaces to fake-align the priority label with icon'd Low/Urgent; fragile and wrong. Removed the leading spaces.
- [x] projects.rs:106 vs tasks.rs:1017 — project row hover has `corner_radius(0.0)` while the task cursor has `corner_radius(3.0)`; pick one (3.0) and apply consistently to both hover and cursor backgrounds across both lists.
- [x] info.rs:109 — `colors::OVERLAY1` for the "hidden until" date label; OVERLAY1 is in the disabled-text range. This is informational secondary content — use `colors::SUBTEXT0`.

### Mouse integration

- [x] tasks.rs — Task rows have no hover visual feedback; project rows paint `SURFACE1` on hover (projects.rs:106). Add a hover highlight to task rows (e.g., `SURFACE0` background or a 2px left border in SURFACE1) so rows feel interactive before click.
- [ ] tasks.rs — No hover action icons on task rows. On cursor hover, reveal icon buttons at the right edge: edit (`e` → pencil icon), complete (`d` → check icon), hard-delete (`Shift+D` → trash icon, danger-styled). The status symbol should also be clickable to open the status picker. This gives mouse users parity with keyboard users.
- [ ] app.rs / tasks.rs — No tooltips exist anywhere in the app. Every icon-only control needs `response.on_hover_text(...)`: ✕ clear buttons (due date / wait until), status symbols in the task list, priority icons. **Partial:** mode indicator, completed toggle, sort label, pin indicator, and project trash icon now have tooltips.
- [x] projects.rs — Projects list has no `ScrollArea`; past ~20 projects (or at small window heights) the list clips off-screen. Wrap the project rows in `egui::ScrollArea::vertical()`.

### Keyboard UX

- [x] projects.rs:keybinds — Project deletion is mouse-only (trash icon). Add `d` in the Projects pane to trigger the existing confirm modal on the hovered project, mirroring task deletion. Also add `r` or `e` to rename (once `update_project` is implemented).
- [x] app.rs:454–503 — Help popup shows Normal-mode task bindings regardless of which pane is active. Add a Projects pane section to the bindings table and switch shown content based on `window_state`, not just `mode`.
- [x] errors.rs:55–87 / projects.rs:172–196 — Fatal error modal and project-delete confirm modal have no keyboard handling (no Esc/Enter). Add `ui.input(|i| i.key_pressed(Key::Escape))` → cancel and `i.key_pressed(Key::Enter)` → confirm. Non-fatal banners can only be dismissed via mouse ✕; add a global `Esc` handler to dismiss the topmost banner.
- [x] app.rs — No in-app hint that `?` opens help. Add a `"? for help"` ghost label (OVERLAY0, size 11, rightmost position) to the status bar so first-time users can discover the popup without guessing.
- [x] tasks.rs:818–839 — Single-task `Shift+D` hard-deletes immediately with no feedback. Show a transient "Deleted — u to undo" status-bar message (3–5 s) so undo becomes discoverable. Multi-select `Shift+D` (line 825) should open the existing keyboard-complete confirm modal before deleting, since blast radius is higher.

### Information architecture

- [x] tasks.rs:204–215 — When the task list is empty the area under "Tasks" is blank. Add a zero-state prompt centered in the list area: "No tasks here.\no / O — new task below / above" matching the Info pane zero-state (info.rs:117–123).
- [ ] app.rs:211–303 — Status bar redesign: (1) remove the project-name segment (already visible in the top dropdown); (2) move sort and show-completed controls to the top panel as a sort dropdown and a toggle button; (3) collapse completed/sort/pinned into small icon-only indicators with tooltips; (4) keep transient state (filter text, N selected) as text since those change frequently and are the most important signal. Goal: ≤ 4 visible items in the steady state.
- [ ] app.rs:177–196 (top panel) — Top panel currently holds only the project dropdown. Add: a sort-order dropdown (shows current sort, opens picker on click) and a "show completed" toggle button. These give mouse users access to features otherwise keyboard-only.
- [ ] tasks.rs:284–338 (detail pane) / info.rs:56–127 (Info pane) — Bottom detail pane and Info pane currently show near-identical content. Differentiate: bottom pane = compact glance only (status icon + priority label + due date + tags, no details/notes body); Info pane = full read including details + edit affordance. Make the boundary explicit so users understand why both exist.

### Feedback and affordances

- [ ] app.rs — No async loading indicator for background ops (get_tasks, undo, redo, project mutations). Add a subtle "⟳" spinner or "loading…" text to the status bar while any background thread is in-flight (e.g., track a pending-ops counter in `BackendManager`).
- [x] app.rs:446–449 — Undo/redo (`u`/`r`) succeeds silently beyond the list refresh. Show a brief status-bar message on success: "Undid: [description]" or at minimum "Undone" / "Redone" so users know the operation applied.
- [ ] errors.rs:23 — Error messages use `format!("{err:#}")` which emits the full anyhow chain including developer-facing context strings (file paths, internal method names). Map common error categories to user-friendly messages before pushing to `ErrorUi`; reserve raw chains for a "details" expandable.

### Configurability / persistence

- [x] app.rs:76–81 — `show_completed`, `sort_order`, `show_detail_pane`, and `always_on_top` all reset to defaults on every launch. Persist these in `PersistedState` (alongside the existing last-project field) and reload them in `init()`. `sort_order` is the most painful to re-set daily; it should survive a restart.

### egui-specific

- [x] tasks.rs:1070–1089 — Task titles are not truncated. In a fixed 26px row with a right-aligned due date, a long title will overflow or push the due date off-screen. Add `.truncate(true)` to the title `Label`, and constrain its available width to `available_width - due_date_reserved_width` before rendering the due date.
- [x] info.rs:205–229 — Tag autocomplete dropdown has no width constraint; a single short suggestion collapses to a narrow strip. Set `ui.set_min_width(ui.available_width())` inside the suggestions Frame so it spans the tags field.
- [ ] tasks.rs:194–201 — `visible_indices()` is called 4–5 times per frame (task_state cursor clamp, `real_index`, `get_current_task`, keybinds, sort). Each call is an O(n) allocation with a `now_ms` syscall. Compute once at the top of `task_state` and pass the slice down, or cache it in `TaskManager` and invalidate on tasks/filter change.
