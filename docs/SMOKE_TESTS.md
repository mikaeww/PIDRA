# PIDRA terminal smoke tests

Run these checks after terminal/input changes. Never confirm a destructive
action against a real desktop process during a smoke test; open confirmation
screens and cancel with `N`.

## Automated coverage

`cargo test --all-targets --all-features` covers:

- tiny, narrow, ASCII and no-color Ratatui viewports;
- process-tree aggregation, partial PSS coverage, stable sorting and trends;
- versioned read-only inspection JSON and bounded persistent history reloads;
- Hyprland/Sway compositor JSON parsing;
- keyboard navigation, GUI/developer switching, help, history and both confirmation screens;
- TCP-listener parsing and conservative developer-command classification;
- mouse-disabled terminal setup and idempotent panic restoration;
- 10,000-row selection/viewport behavior;
- test-owned SIGTERM, freeze/resume, surviving-child and restart processes;
- a transient test-owned systemd user service when a user manager is present.

## Manual matrix

For Kitty, Foot and Alacritty, then inside tmux and over SSH:

1. Run `cargo run --release -- --no-mouse --ascii`.
2. Resize from a normal window down to roughly 20×5 and back.
3. Verify arrow navigation, `/` search, `H` history and `?` help. Press `O`
   through all five sort modes; verify the selected application stays selected.
4. Press `V`, verify the developer/server header and return with `V` or `Esc`.
   Do not stop a real server during this check.
5. Open Details, verify separate process and `APP TREE` resources plus a trend,
   expand/collapse subprocess nodes, switch Overview/Technical with `Tab`,
   scroll with Page Up/Page Down and return with `Esc`.
6. Open Restart and Force Stop confirmation, verify exact PID/start time, then
   verify the restart steps remain understandable at narrow sizes, scroll them,
   then cancel with `N`.
7. Quit with `Q`; verify cursor, echo, canonical input and the original screen
   are restored.
8. Repeat without `--no-mouse`; verify wheel/row selection and the shared action-bar click mapping.
   Row clicks must never execute an action. Check that the bar follows selection
   and remains usable after narrowing the window. Repeat with
   `--no-mouse`; verify ordinary terminal text selection remains available.
9. Repeat with `NO_COLOR=1` and confirm all states remain textual.
10. Run `pidra inspect --pid $$ --json`; verify valid JSON appears without an
    alternate screen or terminal mouse capture.

Record terminal version, `$TERM`, multiplexer/SSH status and any rendering
artifact when reporting a failure. This workspace had Kitty and OpenSSH
available during development; Foot, Alacritty and tmux require separate manual
coverage on systems where they are installed.
