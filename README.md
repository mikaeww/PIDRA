# PIDRA

PIDRA is a small Linux TUI for answering two questions:

1. Which desktop app is using all that memory?
2. What actually happened after I tried to stop it?

The first screen stays focused on GUI applications instead of dumping every
process on the machine. Press `V` for a separate developer/server list. That
list only accepts current-user processes with a TCP listening socket or an
explicit dev-server command; protected session and system targets stay out.

Open **Details** for the full child process tree, command, cgroup, per-process
and application-wide resources, a 30-second trend, the reason a process was
classified and PIDRA's close-risk notes. The main table totals the root and all
of its descendants instead of showing only a browser or Electron root process.

```text
PIDRA                       6 GUI  [V] 2 DEV               CPU 18  MEM 35

PROCESS NAME                    ID    MEM P/R      RESTART   STOP   DETAILS
>spotify                       2031   1.2 GB P       [R]      [S]      [D]
 zen                         128870   998 MB R       [R]      [S]      [D]

V DEV  UP/DOWN ROW  LEFT/RIGHT ACTION  ENTER USE  O SORT  / SEARCH  H HISTORY
```

`P` means complete proportional set size (PSS); `R` means PIDRA fell back to
the complete tree's RSS because at least one PSS value was unavailable.

## Install

You need a current Rust toolchain and Linux.

```bash
git clone https://github.com/mika2go/PIDRA.git
cd PIDRA
cargo install --path . --locked
```

Then start it with:

```bash
pidra
```

If Cargo's bin directory is not in your `PATH`, either add `~/.cargo/bin` or
link the binary into `~/.local/bin`.

## NixOS Flake

Add PIDRA to the `inputs` of the flake that manages your Home Manager
configuration:

```nix
inputs.pidra.url = "github:mika2go/PIDRA";
```

Add the Home Manager module to a module file such as `apps/pidra/default.nix`:

```nix
{ inputs, ... }:

{
  imports = [
    inputs.pidra.homeManagerModules.default
  ];

  programs.pidra.enable = true;
}
```

If you prefer the package directly instead of the Home Manager module, add it
to `home.packages`:

```nix
home.packages = [
  inputs.pidra.packages.${pkgs.system}.pidra
];

```

## Controls

| Key | What it does |
| --- | --- |
| `↑` / `↓` | Select a process or child process |
| `←` / `→` | Select an action; expand/collapse in Details |
| `Enter` | Run the selected action |
| `/` | Search by name or PID |
| `R`, `S`, `D` | Focus Restart, Stop or Details |
| `V` | Toggle the developer/server process list |
| `O` | Cycle sorting by memory, CPU, name, PID and write rate |
| `F`, `T` | Freeze/resume or send SIGTERM in Details |
| `Shift+K` | Open the Force Stop confirmation |
| `H` | Show the bounded session or optional persistent action history |
| `?` | Open help |
| `Q` | Quit |

Mouse actions use a deliberate two-step click: the first click selects the
button, the second click runs it. `Enter` does the same after the first click.
This keeps an accidental click from stopping an application.

## A few important details

- **Stop is SIGTERM.** PIDRA never turns it into SIGKILL behind your back.
- **Force Stop is explicit.** It only exists in Details and always asks again.
- Every signal checks both the PID and its `/proc` start time. A reused PID is
  rejected.
- PID 1, PIDRA itself, its parent chain and essential desktop-session processes
  are blocked.
- The developer/server list is current-user only. A process needs a real TCP
  listener or a recognized dev command, and protected targets are filtered a
  second time by the normal termination analysis.
- A green-looking risk assessment is not a promise. An application can still
  lose unsaved work.
- Restart uses a real systemd user service when one exists. Transient app
  scopes cannot be started again by systemd, so PIDRA falls back to a guarded
  direct restart only when it has an absolute executable and working directory.
- Hyprland and Sway window ownership is read from their native JSON interfaces.
  KDE Plasma and GNOME use conservative systemd application-scope evidence
  when no safe non-interactive compositor PID mapping is available.

Spotify and other Chromium/Electron apps often have many helper processes and
may handle SIGTERM themselves. If one stays alive, PIDRA reports **STILL
RUNNING**; it does not silently kill the remaining process tree.

## Options

```text
--no-mouse
--no-color
--ascii
--refresh <milliseconds>
--pid <pid>
inspect --pid <pid> [--json]
```

The inspection command is read-only and does not enter raw terminal mode or
start a process-control worker:

```bash
pidra inspect --pid 1234
pidra inspect --pid 1234 --json
```

Configuration is optional. PIDRA reads
`$XDG_CONFIG_HOME/pidra/config.toml` or `~/.config/pidra/config.toml`.
The defaults are shown in [BUILDPLAN.md](BUILDPLAN.md).

For optional persistent action history, add:

```toml
persistent_history = true
history_capacity = 100
```

It writes bounded, versioned JSONL below `$XDG_STATE_HOME/pidra`. Entries contain
only the timestamp, display name, PID/start time, action and result—never a
command, executable path or working directory. The default remains
session-only.

Logs go to `$XDG_STATE_HOME/pidra/pidra.log` or
`~/.local/state/pidra/pidra.log`. They are useful when an app ignores a signal
or a restart source turns out to be unavailable.

## Building and testing

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

The implementation rules are in [BUILDPLAN.md](BUILDPLAN.md), and the manual
terminal checks are in [docs/SMOKE_TESTS.md](docs/SMOKE_TESTS.md).

PIDRA currently targets Linux only. No license has been chosen yet.