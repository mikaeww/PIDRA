# PIDRA — Codex Build Plan

> **Purpose of this document:** This is the implementation contract for Codex.
> Build PIDRA phase by phase. Do not replace this plan with a product pitch or
> GitHub README. Do not skip a phase's tests before moving to the next phase.

## 1. Objective

Build **PIDRA**, a standalone, keyboard-first Linux terminal process manager.

PIDRA must present running processes in this primary table:

```text
 PIDRA                                      42 PROCESSES       CPU 07  MEM 24

 PROCESS NAME                 ID           SIZE       RESTART    STOP    DETAILS
 ──────────────────────────────────────────────────────────────────────────────
›nira                         18422        1.8 GB       [↻]       [■]       [i]
 firefox                      2204         3.1 GB       [↻]       [■]       [i]
 qs                           1198         412 MB       [↻]       [■]       [i]
 pipewire                     806           31 MB       [↻]       [■]       [i]
 ffmpeg                       19102          0 B         --       [■]       [i]

 ↑↓ SELECT   ←→ ACTION   ENTER ACTIVATE   / SEARCH   Q QUIT
```

The selected process is controlled with arrow keys. Mouse clicks and the mouse
wheel are optional secondary inputs when the terminal supports them.

The distinguishing behavior is diagnosis: if a process remains alive or
returns with a new PID, PIDRA explains whether children survived, systemd or a
supervisor restarted it, permissions blocked the action, the PID changed, the
process is a zombie, or it is blocked in kernel state `D`.

## 2. Non-negotiable constraints

1. **Never use Qt or QML.** This remains forbidden until the separate known
   Qt drag/resize bug is proven fixed.
2. Do not use Quickshell, Electron, a webview, GTK or another graphical toolkit.
3. PIDRA is a real terminal user interface, not a GUI launched from a terminal.
4. Use Rust with Ratatui and Crossterm.
5. Linux is the only initial platform.
6. The default view is a raw process table limited to detected graphical
   application roots. Do not replace it with application cards or automatic
   grouping. Background and helper processes remain available through the
   expandable tree in Details.
7. The complete terminal viewport is one surface. No box-in-box dashboard.
8. Keyboard control must implement every feature. Mouse support is optional.
9. `Stop` means `SIGTERM`. Never escalate it automatically to `SIGKILL`.
10. `Force Stop` is available only from Details and requires confirmation.
11. Never signal a process from PID alone. Validate PID plus process start time.
12. Never concatenate process data into a shell command.
13. PIDRA must not run its complete UI/backend as root.
14. Tests must never signal unrelated processes on the user's desktop.
15. Restore the terminal on normal exit, error, panic and handled termination.
16. Keep backend logic independent from Ratatui rendering.
17. The optional developer/server layer is current-user only. It must exclude
    protected system/session targets before they become visible.

## 3. MVP scope

### Required

- read running processes from `/proc`;
- classify graphical application roots using evidence from X11/EWMH,
  systemd user application scopes, desktop-entry metadata and optional
  compositor-specific Wayland adapters;
- show only confirmed or probable graphical application roots in the main
  table while retaining all processes internally for relationships and safety
  analysis;
- provide a keyboard-expandable developer/server table for current-user
  background processes backed by a TCP listener or an explicit dev-server
  command;
- show process name, PID and resident memory (RSS) in the main table;
- show Restart, Stop and Details actions in each row;
- mark Restart unavailable with `--` when no safe restart source exists;
- refresh the process snapshot without resetting selection unnecessarily;
- stable sorting that does not make rows jump on every refresh;
- search by process name and PID;
- keyboard navigation and action focus;
- optional mouse row/action selection and wheel scrolling;
- Details view inside the same terminal viewport;
- parent and child relationships;
- an expandable recursive child-process tree in Details;
- a per-process, evidence-backed termination risk assessment that never claims
  guaranteed safety;
- CPU, RSS, virtual memory, thread count, I/O and process state in Details;
- safe `SIGTERM`, `SIGSTOP`, `SIGCONT` and confirmed `SIGKILL`;
- PID/start-time validation before every signal;
- pidfd signalling where supported;
- systemd user-unit detection when available;
- restart-source resolution;
- clear action result and failure diagnosis;
- no-color and ASCII fallbacks;
- configurable refresh interval and mouse capture;
- focused unit and integration tests.

### Explicitly deferred

- controlling processes owned by another user;
- Polkit helper;
- GPU usage;
- general socket/traffic inspection beyond the listening endpoint used for
  developer/server classification;
- changing nice or I/O priority;
- cgroup resource limits;
- remote-machine management;
- Windows and macOS;
- application icons inside the TUI;
- persistent long-term analytics;
- graphical process charts;
- automatic startup disabling.

## 4. Technology

### Language and runtime

- Rust, current stable toolchain
- Cargo workspace only if separation becomes useful; one package is sufficient
  for the MVP

### Core libraries

- `ratatui`: layout, table, text and terminal rendering
- `crossterm`: raw mode, alternate screen, keyboard, mouse and resize events
- `thiserror`: typed backend errors
- `serde` + `toml`: optional configuration
- `tracing` + `tracing-subscriber`: diagnostics written to a file, never into
  the active alternate screen
- a small Linux syscall wrapper such as `rustix` where it provides the required
  pidfd and signal APIs; otherwise isolate minimal `libc` calls in one reviewed
  module

Do not add a large process-monitoring dependency merely to avoid understanding
procfs. The scanner should be owned by PIDRA and covered by fixtures.

### Linux interfaces

- `/proc/<pid>/stat`
- `/proc/<pid>/status`
- `/proc/<pid>/cmdline`
- `/proc/<pid>/exe`
- `/proc/<pid>/cwd`
- `/proc/<pid>/io`
- `/proc/<pid>/cgroup`
- `/proc/stat` for total CPU deltas
- `/proc/meminfo` for system memory summary
- `pidfd_open` and `pidfd_send_signal` where supported
- systemd user D-Bus for unit ownership and restart

## 5. Repository layout

Create this structure:

```text
pidra/
├── Cargo.toml
├── Cargo.lock
├── BUILDPLAN.md
├── assets/
│   └── pidra.png
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── app.rs
│   ├── event.rs
│   ├── terminal.rs
│   ├── tui/
│   │   ├── mod.rs
│   │   ├── layout.rs
│   │   ├── process_table.rs
│   │   ├── details.rs
│   │   ├── confirm.rs
│   │   ├── search.rs
│   │   └── theme.rs
│   ├── process/
│   │   ├── mod.rs
│   │   ├── identity.rs
│   │   ├── snapshot.rs
│   │   ├── procfs.rs
│   │   ├── cpu.rs
│   │   ├── tree.rs
│   │   └── format.rs
│   ├── control/
│   │   ├── mod.rs
│   │   ├── signal.rs
│   │   ├── pidfd.rs
│   │   ├── restart.rs
│   │   ├── systemd.rs
│   │   └── diagnosis.rs
│   └── history.rs
└── tests/
    ├── fixtures/proc/
    ├── procfs.rs
    ├── navigation.rs
    ├── signal_child.rs
    ├── restart.rs
    └── terminal_restore.rs
```

Files may be combined while small, but keep the boundaries between TUI,
process inspection and destructive actions.

## 6. Core data model

Implement explicit types rather than passing loose PIDs and strings.

```rust
struct ProcessIdentity {
    pid: i32,
    start_time_ticks: u64,
}

enum ProcessState {
    Running,
    Sleeping,
    DiskSleep,
    Stopped,
    Zombie,
    Dead,
    Unknown(char),
}

struct ProcessSnapshot {
    identity: ProcessIdentity,
    name: String,
    executable: Option<PathBuf>,
    command: Vec<OsString>,
    cwd: Option<PathBuf>,
    parent_pid: Option<i32>,
    uid: u32,
    state: ProcessState,
    rss_bytes: u64,
    virtual_bytes: u64,
    cpu_percent: f32,
    cpu_time_ticks: u64,
    thread_count: u32,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
    cgroups: Vec<String>,
}

enum RestartSource {
    SystemdUserUnit { unit: String },
    Direct {
        executable: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
    },
    Unavailable { reason: String },
}

enum ProcessAction {
    Restart,
    Stop,
    ForceStop,
    Freeze,
    Resume,
}

enum ActionTarget {
    Process(ProcessIdentity),
    ProcessTree(ProcessIdentity),
    SystemdUserUnit(String),
}

enum Diagnosis {
    Exited,
    StillRunning,
    Restarted { old_pid: i32, new_pid: i32 },
    ChildrenRemain(Vec<ProcessIdentity>),
    Uninterruptible,
    Zombie,
    PermissionDenied,
    IdentityChanged,
    NotFound,
    Unsupported(String),
}
```

These are target shapes, not an instruction to expose every field publicly.
Prefer immutable snapshots and small action results.

## 7. Procfs scanner

### Rules

- Enumerate numeric directories below `/proc`.
- Treat processes disappearing during a scan as normal.
- Treat unreadable fields as partial data, not a fatal scan error.
- Parse `/proc/<pid>/stat` correctly when the command name contains spaces or
  parentheses. Do not split the complete line naïvely on whitespace.
- Derive page size and clock ticks from the running system.
- Compute CPU percentage from two snapshots and total CPU deltas.
- Keep raw byte counts in the model; format units only in the TUI layer.
- Never block the input/render loop while scanning thousands of processes.
- Run scans on a worker and deliver a complete snapshot/delta to the app state.

### Stable selection

Selection is keyed by `ProcessIdentity`, not the visible row index. After a
refresh:

1. preserve the selected identity when it still exists;
2. otherwise select the nearest previous visible row;
3. never jump to an unrelated reused PID;
4. preserve the focused action column.

Default sorting for the MVP: RSS descending, then process name, then PID. Add a
future sort selector only after the primary table is stable.

## 8. Main TUI

### Header

Show:

- `PIDRA`;
- visible/total process count;
- total CPU percentage;
- total used memory percentage;
- active search text when searching.

### Table columns

1. `PROCESS NAME`
2. `PID`
3. `MEM P/R`
4. `REL. MEM` when at least 64 terminal columns are available

Column requirements:

- Process name receives remaining width.
- PID and memory values are right-aligned.
- Relative memory bars compare against the largest app in the current list.
- Highlight only the selected process marker and name; never draw a full-width
  selection bar.
- Show Restart, Stop and Details once in an action bar directly below the list.
- The action bar names the selected process and uses `[R]`, `[S]`, `[D]` shortcuts.
- Keep the selected name and its actions adjacent; unused terminal width stays
  after the group instead of separating its two parts.
- Unavailable restart shows `[R] --`; narrow terminals use compact action labels.
- Keyboard focus remains visible in both color and no-color modes.
- Never set foreground or background colors. Keep the terminal's own monochrome
  palette and use weight, underline, dim text and reverse video for hierarchy.
- Warnings and force-stop confirmations remain explicit in text and weight.
- Do not draw a border around every row or action.

### Focus model

```rust
enum FocusColumn {
    Restart,
    Stop,
    Details,
}
```

- `Up` / `Down`: select rows.
- `Left` / `Right`: change `FocusColumn`.
- `Enter`: activate the selected row and focused column.
- `R`, `S`, `D`: focus and activate or focus consistently; choose one behavior
  during Phase 2 and test it. Preferred behavior: focus first, Enter confirms.
- `/`: open search mode.
- `Esc`: leave the current mode.
- `Q`: quit only outside text input and confirmation.

### Mouse model

When enabled:

- request Crossterm mouse capture;
- table clicks select a row; shared action-bar clicks use the selected row and
  the same `FocusColumn` used by keyboard;
- left click once selects; clicking an already selected action activates it;
- wheel scrolls without changing the focused column;
- `--no-mouse` and config must disable capture entirely;
- terminal text selection must remain available when mouse capture is disabled.

Do not create separate mouse-only actions.

## 9. Details view

Details replaces the process table within the same terminal viewport. Do not
open a nested modal for ordinary information.

Sections are separated with whitespace and headings, not boxes:

```text
 NIRA                                                RUNNING

 IDENTITY
 PID          18422              OWNER        mika
 EXECUTABLE   /home/user/.local/bin/nira
 STARTED      14:08              AGE          01:42:13

 RESOURCES
 CPU          18.4%              RSS          1.8 GB
 VIRTUAL      4.2 GB             THREADS      34
 READ         2.4 MB/s           WRITE        12 KB/s

 RELATIONSHIPS
 PARENT       1201 systemd       CHILDREN     2
 UNIT         nira.service       RESTART      on-failure

 ACTIONS
 F FREEZE     T STOP             SHIFT+K FORCE STOP

 ESC BACK
```

Required detail content:

- identity and state;
- executable, masked arguments and working directory when readable;
- parent and direct children;
- resource values;
- cgroup and systemd unit when detected;
- restart source and availability reason;
- latest action result and diagnosis;
- Freeze/Resume, Stop and Force Stop.

### Expandable process tree

Details must show the selected graphical application's complete known process
tree. `Up`/`Down` select a node, `Right`/`Enter` expand it, and `Left` collapses
it or returns to the parent. Opening a tree node never signals it. An action
targets only the explicitly selected `ProcessIdentity`; it must never
implicitly signal the whole tree.

### Termination risk assessment

Before Stop, Restart or Force Stop, show one of:

- `LIKELY SAFE TO TERMINATE`;
- `CLOSE FROM APPLICATION FIRST`;
- `CAUTION`;
- `PROTECTED`;
- `UNKNOWN`.

The assessment must list its evidence and confidence. Consider ownership,
process role, parent/children, systemd unit and restart policy, cgroup, session
or compositor role, process state, recent I/O, executable path and the risk of
unsaved user data. It is advisory and must never promise that terminating a
process cannot corrupt data. Normal application shutdown and `SIGTERM` remain
preferred. Active writes increase caution. Force Stop must warn that user
configuration or application databases may be damaged even when system-owned
files are protected by permissions.

PID 1, PIDRA itself, the active compositor/display server, the graphical
session manager and other essential session services are `PROTECTED` unless a
future explicitly reviewed policy says otherwise.

## 9.1 Graphical-process classification

The scanner still reads all accessible processes, but the main table includes
only graphical application roots with `Confirmed` or `Probable` confidence.
Classification must not rely on a process name alone.

Evidence priority:

1. X11 EWMH client window with a validated `_NET_WM_PID`;
2. systemd user unit in `app.slice` using an application-unit naming scheme;
3. desktop-entry application ID, `StartupWMClass`, executable and cgroup
   agreement;
4. a compositor-specific Wayland adapter when available.

Wayland has no portable core protocol for enumerating every application's
top-level window. Generic Wayland support therefore uses application scopes
and desktop metadata, reports confidence honestly and permits optional
compositor adapters. A display-related environment variable alone is not
enough to classify a process as a GUI application.

Full command arguments must be masked by default using conservative patterns
for tokens, passwords, authorization headers and obvious secrets.

## 9.2 Developer/server classification

The developer/server table is a separate layer reached with `V`; it never
changes the GUI-only default table. Name-only guesses are not accepted.

A candidate must be owned by the current user, have an executable, pass the
normal protected-target policy, and provide at least one of these signals:

1. the process owns an inode currently listed as `LISTEN` in `/proc/net/tcp`
   or `/proc/net/tcp6`;
2. its argument vector is an explicit, recognized developer-server invocation
   such as `python -m http.server`, `manage.py runserver` or `npm run dev`.

PID 1, PIDRA and its ancestors, other-user processes, graphical roots, display
servers, session managers, IPC/audio infrastructure and known desktop clients
remain outside this layer. Details show the exact classification evidence and
the existing termination analysis before stronger actions are available.

## 10. Process control

### Identity validation

Before every action:

1. re-read the target's start time from `/proc/<pid>/stat`;
2. compare it with `ProcessIdentity.start_time_ticks`;
3. abort with `IdentityChanged` if it differs;
4. open a pidfd when supported;
5. send the signal through the pidfd;
6. fall back to `kill(2)` only after the identity was revalidated immediately
   before the syscall.

PID 1 and PIDRA's own identity are never valid targets. Add protection warnings
for the parent shell, login session, compositor and core user services.

### Stop

- Send `SIGTERM`.
- Observe the identity for a short configurable period without blocking input.
- Report `Exited`, `StillRunning`, `ChildrenRemain`, `Restarted`,
  `Uninterruptible`, `Zombie`, `PermissionDenied` or `IdentityChanged`.
- Never send `SIGKILL` automatically.

### Force Stop

- Available only inside Details.
- Show exact PID, name and target type in the confirmation view.
- Require a second Enter or `Y`; `Esc` and `N` cancel.
- Send `SIGKILL` only after fresh identity validation.

### Freeze and Resume

- `SIGSTOP` freezes.
- `SIGCONT` resumes.
- The table and details must show `FROZEN` in text, not only color.

## 11. Restart behavior

Restart must be conservative.

### Resolution priority

1. A detected systemd user unit owned by the current user.
2. A direct source containing an absolute executable, exact argument vector and
   readable working directory.
3. Unavailable with a human-readable reason.

### Systemd restart

- Use the systemd user D-Bus API.
- Do not spawn `systemctl` through a shell.
- Ask systemd to restart the exact validated user unit.
- Observe the old identity and locate the replacement PID through the unit.

### Direct restart

- Show the executable, argument count and working directory before confirmation.
- Explain that PIDRA cannot reconstruct the original environment completely.
- Send `SIGTERM` and wait asynchronously.
- If the old identity still lives, abort; do not force kill.
- Spawn with `std::process::Command`, an absolute executable, `args(Vec<OsString>)`
  and explicit `current_dir`.
- Never use `sh -c`, `bash -lc` or a concatenated command string.
- Track the new child PID and show the result.

Restart is disabled for zombies, kernel threads, unreadable executable paths,
invalid working directories and other-user processes.

## 12. Diagnosis engine

After Stop or Restart, compare the before/after snapshot.

### `RESTARTED`

Match a replacement using the strongest available identity:

1. same systemd unit;
2. same cgroup plus executable;
3. same executable, parent/supervisor and close start time.

Never declare a restart from process name alone.

### `CHILDREN REMAIN`

Capture direct and recursive descendants before the action. After the parent
exits, report which captured identities still exist. Do not automatically kill
them.

### `UNINTERRUPTIBLE`

If state is `D`, explain that even `SIGKILL` cannot finish until the kernel wait
returns. Do not recommend repeated force-kill attempts.

### `ZOMBIE`

Explain that the process already exited and the parent has not collected its
status. Link the Details action to the parent.

### `PERMISSION DENIED`

Show the process owner. For the MVP, do not offer privilege escalation.

## 13. Terminal lifecycle

Implement a small RAII terminal guard:

1. enable raw mode;
2. enter alternate screen;
3. optionally enable mouse capture;
4. optionally hide cursor;
5. render;
6. on Drop, disable mouse capture, show cursor, leave alternate screen and
   disable raw mode.

Install a panic hook that attempts restoration before printing the panic. Handle
normal `Ctrl+C`/termination through the event loop where practical. Restoration
must be idempotent.

Logs go to an XDG state/cache file, never stdout while the TUI is active.

## 14. Configuration and CLI

### Planned CLI

```text
pidra [OPTIONS]

  --no-mouse
  --no-color
  --ascii
  --refresh <milliseconds>
  --pid <pid>
  -h, --help
  -V, --version
```

### Configuration

Path:

```text
$XDG_CONFIG_HOME/pidra/config.toml
```

Defaults:

```toml
refresh_interval_ms = 1000
mouse = true
color = "auto"
unicode = true
confirm_force_stop = true
show_kernel_threads = false
mask_command_secrets = true
persistent_history = false
history_capacity = 100
```

CLI flags override config. Missing or partially invalid config must produce a
clear error without corrupting the file.

## 15. Implementation phases

Codex must keep the project buildable at the end of every phase.

### Phase 0 — Scaffold and terminal guard

Deliver:

- Cargo package;
- CLI with `--help` and `--version`;
- Ratatui/Crossterm startup;
- terminal guard and panic restoration;
- static PIDRA table fixture;
- supplied 64×64 RGBA icon retained under `assets/pidra.png`.

Verify:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo run -- --no-mouse
```

Acceptance:

- arrow keys move through fixture rows;
- left/right changes the action focus;
- `Q` exits and restores the terminal;
- forced panic test restores the terminal.

### Phase 0.1 — Public GitHub repository

Run only after Phase 0 passes all quality gates.

Deliver:

- inspect the complete tracked tree for credentials, tokens, private paths and
  unrelated files before publishing;
- use `main` as the default branch;
- create the `PIDRA` repository under the authenticated project owner's GitHub
  account with public visibility;
- add `origin` and push the verified Phase 0 history;
- add a concise README that points to this implementation contract without
  replacing it;
- do not choose a software license without an explicit owner decision.

Acceptance:

- the GitHub repository reports public visibility;
- `origin/main` matches the locally verified `main` commit;
- no secrets or machine-private files are tracked.

### Phase 1 — Read-only procfs backend

Deliver:

- typed process model;
- robust `/proc/<pid>/stat` and status parsers;
- real process enumeration;
- RSS and state;
- partial-data behavior;
- fixture tests.

The scanner retains the complete accessible process set internally. GUI-only
filtering is a separate classification/view concern and must not discard
processes needed to build relationships or assess safety.

Acceptance:

- PIDRA shows real processes without root;
- disappearing processes do not create visible errors;
- stat names containing spaces/parentheses parse correctly;
- no scan blocks keyboard input noticeably.

### Phase 2 — Final main table and search

Deliver:

- exact requested columns;
- stable selection by identity;
- RSS sorting;
- search by name and PID;
- ASCII/no-color modes;
- narrow-terminal handling;
- optional mouse mapping.
- graphical-application classification with explicit confidence;
- main-table filtering to confirmed and probable GUI roots.

Acceptance:

- keyboard can reach every visible action;
- mouse and keyboard call the same commands;
- refresh does not jump away from the selected identity;
- 10,000 fixture processes remain navigable.

### Phase 2.5 — Developer/server layer

Deliver:

- TCP/TCP6 listening-inode discovery with disappearing-file tolerance;
- conservative dev-command recognition without name-only classification;
- a `V`-toggleable table using the same search, action and identity model;
- developer/server evidence in Details;
- a second protected-target filter before any candidate is shown.

Acceptance:

- other-user and essential session processes never enter the layer;
- a process with only a developer-looking name is not enough;
- Stop still means identity-validated `SIGTERM` with no automatic escalation;
- tests never signal an unrelated real process.

### Phase 3 — Details and process relationships

Deliver:

- Details view;
- CPU delta calculation;
- memory, threads and I/O;
- parent and children;
- executable, cwd and masked command;
- cgroup display;
- navigation back to table;
- expandable recursive subprocess tree;
- per-node termination risk assessment with evidence and confidence.

Acceptance:

- details replace the table without nested popup boxes;
- unreadable fields show `unavailable` instead of failing;
- secret-like command arguments are masked by default.
- expanding a child never performs an action;
- an action targets only the explicitly selected identity;
- protected session infrastructure cannot be signalled;
- the advisor never promises that termination is risk-free.

### Phase 4 — Safe signals

Deliver:

- identity revalidation;
- pidfd path plus reviewed fallback;
- Stop, Freeze, Resume and confirmed Force Stop;
- asynchronous result observation;
- integration children created by the test suite.

Acceptance:

- no action uses PID alone;
- Stop never escalates;
- PID reuse fixture/action test returns `IdentityChanged`;
- tests cannot target a process outside their owned group;
- PID 1 and PIDRA itself are protected.

### Phase 5 — Restart sources and systemd

Deliver:

- `RestartSource` resolver;
- systemd user-unit detection and restart;
- guarded direct restart;
- unavailable reasons;
- Restart column state.

Acceptance:

- a test user service restarts through D-Bus;
- a new PID is reported;
- direct restart never uses a shell;
- restart aborts if the old process ignores `SIGTERM`.

### Phase 6 — Diagnosis

Deliver:

- restarted-process detection;
- remaining-child detection;
- Zombie, D-state and permission explanations;
- latest action result in Details;
- action history for the current session.

Acceptance:

- a `Restart=on-failure` fixture is explained as restarted, not failed kill;
- a surviving child is named;
- D-state language does not promise an impossible immediate kill;
- replacement matching never relies on name alone.

### Phase 7 — Hardening

Deliver:

- config file;
- robust resize behavior;
- Kitty, Foot, Alacritty, tmux and SSH smoke-test notes;
- logging outside the alternate screen;
- load/performance tests;
- user-facing help screen.

Acceptance:

- no busy loop when idle;
- terminal restores after all tested exit paths;
- no-color and ASCII modes expose every state;
- mouse capture can be completely disabled.

## 16. Test matrix

### Parser tests

- normal process;
- spaces and parentheses in `comm`;
- process disappears between directory read and file read;
- permission-denied fields;
- zombie;
- stopped process;
- D-state fixture;
- very large counters;
- invalid/truncated proc data;
- cgroup v2 and missing cgroup.

### Model tests

- stable identity selection;
- PID reused with different start time;
- byte formatting boundaries;
- CPU delta math including zero delta;
- stable sorting ties;
- restart availability rules;
- secret masking.

### TUI tests

- row movement;
- action-column movement;
- search mode;
- confirmation cancel/accept;
- details/back;
- small viewport;
- no-color snapshot;
- ASCII snapshot;
- click coordinate mapping;
- wheel scrolling.

### Controlled integration tests

- child exits on SIGTERM;
- child ignores SIGTERM;
- freeze and resume;
- force stop after confirmation logic;
- parent exits while child survives;
- target exits before action;
- systemd user service restarts with new PID;
- terminal restoration after panic.

All integration targets must be spawned by the test, identified by PID plus
start time, and cleaned up by the test-owned process group.

## 17. Quality gates

### Git checkpoint policy

Create an intentional Git commit after every coherent, relatively large step.
A commit must be buildable, contain no unrelated changes and use a concise
message that describes the completed slice. Run formatting, Clippy and all
relevant focused tests before each commit. Run the complete phase quality gates
before the phase's final commit. Never commit a known failing intermediate
state merely to create a checkpoint.

Use `main` for verified phase baselines. After the initial publication, larger
new phases should be developed on a `codex/phase-*` branch unless the owner
requests another workflow.

Run after every phase:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

Before a release build:

```bash
cargo build --release
```

Do not suppress Clippy warnings globally. Any unsafe syscall block must have a
local safety comment that states the validated invariants.

## 18. Definition of done

PIDRA 1.0 is complete when:

- it is a standalone terminal program with no Qt, QML or Quickshell dependency;
- the exact Process Name / ID / Size / Restart / Stop / Details flow works for
  detected graphical application roots;
- background and helper processes are hidden from the main table but available
  through an expandable Details tree;
- developer/server processes are available in a separate evidence-backed layer
  without exposing protected system/session targets;
- every termination assessment states evidence and uncertainty instead of
  promising that closing a process is harmless;
- every action is reachable using only arrow keys and Enter;
- supported terminals can optionally use clicks and the mouse wheel;
- Details explain identity, resources, relationships and action results;
- Stop, Force Stop, Freeze and Resume validate PID plus start time;
- pidfds are used when available;
- Stop never escalates automatically;
- Restart uses a systemd unit or an explicit argument vector, never a shell;
- PIDRA recognizes a replacement process created by a known supervisor;
- Zombie, D-state, remaining children and permission failures are explained;
- terminal state is restored on all tested exits;
- parser, navigation, signalling and diagnosis tests pass;
- the program stays responsive with thousands of process rows;
- no root daemon, telemetry, account or network service is required.

## 19. First Codex instruction

Use this when starting the implementation:

```text
Implement PIDRA according to BUILDPLAN.md.

Start with Phase 0 only. Read the complete build plan before editing files.
Preserve all non-negotiable constraints, especially the ban on Qt/QML, the
keyboard-first TUI, PID-plus-start-time validation, and the prohibition on
automatic SIGKILL escalation. Keep backend logic separate from rendering.

At the end of Phase 0, run every listed verification command and report the
created files, test results, remaining limitations, and the exact next phase.
Do not begin Phase 1 until Phase 0 satisfies its acceptance criteria.
```

## 20. Post-MVP implementation plan

This section defines the next coherent feature phase. The original safety
contract remains unchanged.

### Phase 8.1 — Application-wide resource accounting

- Keep per-process counters as the source of truth.
- Compute CPU, RSS and I/O totals for every visible GUI or developer root and
  all descendants, guarding against cycles and duplicate identities.
- Read `Pss:` from `/proc/<pid>/smaps_rollup` only for members of classified
  application trees. A missing or unreadable file remains partial data.
- Show root and tree-wide values separately in Details. The main table uses
  the complete tree value and labels whether it is PSS or RSS.
- Never present a partial PSS sum as a complete application total; expose its
  process coverage alongside the value.

Acceptance:

- descendants contribute exactly once;
- unrelated processes never contribute;
- incomplete PSS coverage is explicit;
- fixture and model tests cover trees, cycles, saturation and PSS parsing.

### Phase 8.2 — Stable sorting and short-term trends

- Add identity-stable sort modes for memory, CPU, name, PID and write rate.
- Cycle modes with `O`; retain the selected identity and focused action.
- Retain at most the last 30 seconds of in-memory application samples.
- Details show memory change over that window and average CPU. No trend data
  is persisted.

Acceptance:

- every comparator has deterministic name/PID tie breakers;
- switching modes does not select an unrelated process;
- vanished identities are pruned from the trend tracker;
- trends are unavailable, rather than invented, until two samples exist.

### Phase 8.3 — Read-only diagnosis command

- Add `pidra inspect --pid <PID> [--json]`.
- Scan once without entering raw terminal mode or starting control workers.
- Emit identity, masked command, process tree, raw and aggregate resources,
  classification evidence, restart source and termination-risk evidence.
- Return a failure exit status when the requested PID does not exist.

Acceptance:

- JSON is valid and versioned;
- secrets use the same masking rules as Details;
- the command performs no process-control action and creates no history entry.

### Phase 8.4 — Wayland discovery adapters

- Keep compositor discovery isolated behind `WindowHint` adapters.
- Support Hyprland and Sway native JSON tree output without shell commands.
- On KDE Plasma and GNOME, retain systemd application-scope evidence when no
  safe non-interactive compositor PID API is available; make that fallback
  visible in classification evidence instead of guessing window ownership.
- Missing compositor utilities, malformed JSON and timeouts yield no hints and
  never fail the procfs scan.

Acceptance:

- adapter parsers use captured JSON fixtures;
- only mapped/visible toplevels with positive PIDs become hints;
- subprocess invocation uses fixed argument vectors and bounded execution.

### Phase 8.5 — Optional persistent action history

- Add disabled-by-default configuration for persistent history and its bounded
  capacity.
- Store only timestamp, process display name, PID/start time, action and
  result. Never store executable paths, working directories or commands.
- Write a versioned JSONL file below the XDG state directory with atomic
  bounded rewrites; tolerate a malformed individual line.
- Surface persistence status in the History view and log non-fatal I/O errors.

Acceptance:

- default behavior remains session-only;
- capacity applies both in memory and on disk;
- reload preserves ordering and continues sequence numbers;
- tests use temporary, test-owned paths.

### Phase 8.6 — Documentation and release gates

- Update README controls, options, configuration and diagnostic examples.
- Extend terminal smoke tests for sort labels and aggregate values.
- Run formatting, Clippy, all tests and a release build.
