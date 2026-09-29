use std::{
    collections::{HashMap, HashSet},
    io::Read,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ConnectionExt as _},
};

use super::{ProcessIdentity, ProcessSnapshot, tree::ProcessTree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GuiConfidence {
    Unclassified,
    Probable,
    Confirmed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiClassification {
    pub identity: ProcessIdentity,
    pub confidence: GuiConfidence,
    pub display_name: Option<String>,
    pub application_scope: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowHint {
    pub pid: i32,
    pub class: Option<String>,
    pub title: Option<String>,
    pub source: &'static str,
}

#[derive(Debug, Deserialize)]
struct HyprlandClient {
    mapped: bool,
    pid: i32,
    class: String,
    title: String,
}

#[derive(Debug, Default, Deserialize)]
struct SwayNode {
    #[serde(rename = "type")]
    node_type: String,
    pid: Option<i32>,
    app_id: Option<String>,
    name: Option<String>,
    window_properties: Option<SwayWindowProperties>,
    #[serde(default)]
    nodes: Vec<SwayNode>,
    #[serde(default)]
    floating_nodes: Vec<SwayNode>,
}

#[derive(Debug, Default, Deserialize)]
struct SwayWindowProperties {
    class: Option<String>,
}

pub fn discover_window_hints() -> Vec<WindowHint> {
    let mut hints = Vec::new();
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        hints.extend(discover_hyprland_windows());
    }
    if std::env::var_os("SWAYSOCK").is_some() {
        hints.extend(discover_sway_windows());
    }
    if std::env::var_os("DISPLAY").is_some() {
        hints.extend(discover_x11_windows());
    }
    hints.sort_by(|left, right| {
        left.pid
            .cmp(&right.pid)
            .then_with(|| left.source.cmp(right.source))
    });
    hints.dedup_by(|left, right| left.pid == right.pid && left.source == right.source);
    hints
}

fn discover_hyprland_windows() -> Vec<WindowHint> {
    let mut command = Command::new("hyprctl");
    command.args(["-j", "clients"]);
    let Some(output) = bounded_output(&mut command, Duration::from_millis(500)) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    parse_hyprland_clients(&output.stdout)
}

fn parse_hyprland_clients(contents: &[u8]) -> Vec<WindowHint> {
    serde_json::from_slice::<Vec<HyprlandClient>>(contents).map_or_else(
        |_| Vec::new(),
        |clients| {
            clients
                .into_iter()
                .filter(|client| client.mapped && client.pid > 0)
                .map(|client| WindowHint {
                    pid: client.pid,
                    class: nonempty(client.class),
                    title: nonempty(client.title),
                    source: "Hyprland mapped client",
                })
                .collect()
        },
    )
}

fn discover_sway_windows() -> Vec<WindowHint> {
    let mut command = Command::new("swaymsg");
    command.args(["-r", "-t", "get_tree"]);
    let Some(output) = bounded_output(&mut command, Duration::from_millis(500)) else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_sway_tree(&output.stdout)
}

fn parse_sway_tree(contents: &[u8]) -> Vec<WindowHint> {
    let Ok(root) = serde_json::from_slice::<SwayNode>(contents) else {
        return Vec::new();
    };
    let mut hints = Vec::new();
    collect_sway_hints(root, &mut hints);
    hints
}

fn collect_sway_hints(node: SwayNode, hints: &mut Vec<WindowHint>) {
    let class = node.app_id.or_else(|| {
        node.window_properties
            .and_then(|properties| properties.class)
    });
    if matches!(node.node_type.as_str(), "con" | "floating_con")
        && node.pid.is_some_and(|pid| pid > 0)
        && class
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
    {
        hints.push(WindowHint {
            pid: node.pid.unwrap_or_default(),
            class,
            title: node.name.filter(|title| !title.trim().is_empty()),
            source: "Sway IPC mapped toplevel",
        });
    }
    for child in node.nodes.into_iter().chain(node.floating_nodes) {
        collect_sway_hints(child, hints);
    }
}

fn bounded_output(command: &mut Command, timeout: Duration) -> Option<Output> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let stdout_reader = thread::spawn(move || {
        let mut contents = Vec::new();
        stdout.read_to_end(&mut contents).map(|_| contents)
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stdout = stdout_reader.join().ok()?.ok()?;
                return Some(Output {
                    status,
                    stdout,
                    stderr: Vec::new(),
                });
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                return None;
            }
        }
    }
}

fn discover_x11_windows() -> Vec<WindowHint> {
    let Ok((connection, screen_index)) = x11rb::connect(None) else {
        return Vec::new();
    };
    let Some(screen) = connection.setup().roots.get(screen_index) else {
        return Vec::new();
    };
    let Ok(client_list_atom) = intern_atom(&connection, b"_NET_CLIENT_LIST") else {
        return Vec::new();
    };
    let Ok(pid_atom) = intern_atom(&connection, b"_NET_WM_PID") else {
        return Vec::new();
    };
    let Ok(client_list_cookie) = connection.get_property(
        false,
        screen.root,
        client_list_atom,
        AtomEnum::WINDOW,
        0,
        u32::MAX,
    ) else {
        return Vec::new();
    };
    let Ok(client_list) = client_list_cookie.reply() else {
        return Vec::new();
    };
    let Some(windows) = client_list.value32() else {
        return Vec::new();
    };

    windows
        .filter_map(|window| {
            let pid_reply = connection
                .get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1)
                .ok()?
                .reply()
                .ok()?;
            let pid = i32::try_from(pid_reply.value32()?.next()?).ok()?;
            let class_reply = connection
                .get_property(
                    false,
                    window,
                    AtomEnum::WM_CLASS,
                    AtomEnum::STRING,
                    0,
                    1_024,
                )
                .ok()
                .and_then(|cookie| cookie.reply().ok());
            let class = class_reply.and_then(|reply| {
                reply
                    .value
                    .split(|byte| *byte == 0)
                    .rfind(|part| !part.is_empty())
                    .map(|part| String::from_utf8_lossy(part).into_owned())
            });
            Some(WindowHint {
                pid,
                class,
                title: None,
                source: "X11 EWMH _NET_WM_PID",
            })
        })
        .collect()
}

fn intern_atom<C: Connection>(connection: &C, name: &[u8]) -> Result<u32, ()> {
    connection
        .intern_atom(false, name)
        .map_err(|_| ())?
        .reply()
        .map(|reply| reply.atom)
        .map_err(|_| ())
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

pub fn classify_gui_processes(
    processes: &[ProcessSnapshot],
    window_hints: &[WindowHint],
) -> Vec<GuiClassification> {
    let current_uid = rustix::process::getuid().as_raw();
    let process_by_pid: HashMap<i32, &ProcessSnapshot> = processes
        .iter()
        .filter(|process| process.uid == current_uid)
        .map(|process| (process.identity.pid, process))
        .collect();
    let mut members_by_scope: HashMap<String, Vec<&ProcessSnapshot>> = HashMap::new();
    for process in process_by_pid.values() {
        if let Some(scope) = application_scope(process) {
            members_by_scope.entry(scope).or_default().push(*process);
        }
    }

    let tree = ProcessTree::new(processes);
    let mut root_by_scope = HashMap::new();
    let mut classifications = HashMap::new();
    for (scope, members) in &members_by_scope {
        let member_pids: HashSet<i32> = members.iter().map(|member| member.identity.pid).collect();
        let root = members
            .iter()
            .copied()
            .filter(|member| {
                member
                    .parent_pid
                    .is_none_or(|parent| !member_pids.contains(&parent))
            })
            .min_by_key(|member| member.identity.start_time_ticks)
            .or_else(|| {
                members
                    .iter()
                    .copied()
                    .min_by_key(|member| member.identity.start_time_ticks)
            });
        let Some(root) = root else {
            continue;
        };
        root_by_scope.insert(scope.clone(), root.identity);
        classifications.insert(
            root.identity,
            GuiClassification {
                identity: root.identity,
                confidence: GuiConfidence::Probable,
                display_name: scope_display_name(scope),
                application_scope: Some(scope.clone()),
                evidence: scope_evidence(scope),
            },
        );
    }

    for hint in window_hints {
        let Some(window_process) = process_by_pid.get(&hint.pid).copied() else {
            continue;
        };
        let scope = application_scope(window_process);
        let root_identity = scope
            .as_ref()
            .and_then(|scope| root_by_scope.get(scope))
            .copied()
            .filter(|root| {
                if *root == window_process.identity {
                    return true;
                }
                let root_process = process_by_pid[&root.pid];
                !matches!(
                    root_process.name.as_str(),
                    "bwrap" | "flatpak" | "bash" | "sh" | "zsh" | "fish" | "env"
                ) && !window_hints
                    .iter()
                    .any(|other| other.pid == root.pid && other.class != hint.class)
                    && tree.descendants(*root).contains(&window_process.identity)
            })
            .unwrap_or(window_process.identity);
        let classification =
            classifications
                .entry(root_identity)
                .or_insert_with(|| GuiClassification {
                    identity: root_identity,
                    confidence: GuiConfidence::Unclassified,
                    display_name: None,
                    application_scope: scope.clone(),
                    evidence: Vec::new(),
                });
        if classification.confidence != GuiConfidence::Confirmed
            && let Some(class) = hint
                .class
                .as_ref()
                .filter(|class| !class.eq_ignore_ascii_case("electron"))
        {
            classification.display_name = Some(class.clone());
        }
        classification.confidence = GuiConfidence::Confirmed;
        let mut evidence = hint.source.to_owned();
        if let Some(class) = &hint.class {
            evidence.push_str(&format!(" class={class}"));
        }
        if let Some(title) = &hint.title {
            evidence.push_str(&format!(" title={title}"));
        }
        if !classification.evidence.contains(&evidence) {
            classification.evidence.push(evidence);
        }
    }

    let confirmed_scopes: HashSet<_> = classifications
        .values()
        .filter(|classification| classification.confidence == GuiConfidence::Confirmed)
        .filter_map(|classification| classification.application_scope.clone())
        .collect();
    classifications.retain(|_, classification| {
        classification.confidence == GuiConfidence::Confirmed
            || !classification
                .application_scope
                .as_ref()
                .is_some_and(|scope| confirmed_scopes.contains(scope))
    });
    for classification in classifications.values_mut() {
        let process = process_by_pid[&classification.identity.pid];
        if matches!(
            process.name.as_str(),
            "kitty" | "foot" | "alacritty" | "wezterm-gui"
        ) {
            let mut tools: Vec<_> = tree
                .descendants(process.identity)
                .iter()
                .filter_map(|identity| process_by_pid.get(&identity.pid))
                .filter_map(|child| match child.name.as_str() {
                    "claude" => Some("Claude"),
                    "codex" => Some("Codex"),
                    _ => None,
                })
                .collect();
            tools.sort_unstable();
            tools.dedup();
            if !tools.is_empty() {
                classification.display_name =
                    Some(format!("{} - {}", process.name, tools.join(" / ")));
            }
        }
    }

    let mut classifications: Vec<_> = classifications
        .into_values()
        .filter(|classification| classification.confidence >= GuiConfidence::Probable)
        .collect();
    classifications.sort_by_key(|classification| classification.identity);
    classifications
}

fn scope_evidence(scope: &str) -> Vec<String> {
    let mut evidence = vec![format!("systemd user application unit {scope}")];
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if desktop.contains("kde") || desktop.contains("plasma") {
        evidence.push(
            "KDE Wayland fallback: application scope used; no safe non-interactive KWin PID mapping available"
                .to_owned(),
        );
    } else if desktop.contains("gnome") {
        evidence.push(
            "GNOME Wayland fallback: application scope used; no safe non-interactive Shell PID mapping available"
                .to_owned(),
        );
    }
    evidence
}

fn application_scope(process: &ProcessSnapshot) -> Option<String> {
    process.cgroups.iter().find_map(|entry| {
        let path = entry
            .rsplit_once(':')
            .map_or(entry.as_str(), |(_, path)| path);
        let mut components = path.split('/').filter(|component| !component.is_empty());
        while let Some(component) = components.next() {
            if component == "app.slice" {
                let unit = components.next()?;
                if unit.starts_with("app-")
                    && (unit.ends_with(".scope") || unit.ends_with(".service"))
                {
                    return Some(unit.to_owned());
                }
            }
        }
        None
    })
}

fn scope_display_name(scope: &str) -> Option<String> {
    let without_prefix = scope.strip_prefix("app-")?;
    let without_suffix = without_prefix
        .strip_suffix(".scope")
        .or_else(|| without_prefix.strip_suffix(".service"))?;
    let name = without_suffix
        .rsplit_once('-')
        .map_or(without_suffix, |(candidate, suffix)| {
            if suffix.chars().all(|character| character.is_ascii_digit()) {
                candidate
            } else {
                without_suffix
            }
        });
    let name = name.strip_prefix("flatpak-").unwrap_or(name);
    Some(display_app_id(name))
}

fn display_app_id(name: &str) -> String {
    if let Some(app) = name.strip_prefix("org.gnome.") {
        return format!("GNOME {app}");
    }
    name.to_owned()
}

#[cfg(test)]
mod tests {
    use super::{
        GuiConfidence, WindowHint, classify_gui_processes, parse_hyprland_clients, parse_sway_tree,
    };
    use crate::process::ProcessSnapshot;

    fn process(name: &str, pid: i32, parent_pid: Option<i32>, cgroup: &str) -> ProcessSnapshot {
        let mut process = ProcessSnapshot::fixture(name, pid, 1_024);
        process.parent_pid = parent_pid;
        process.uid = rustix::process::getuid().as_raw();
        process.cgroups = vec![cgroup.to_owned()];
        process
    }

    #[test]
    fn names_apps_and_terminal_tools_without_merging_independent_windows() {
        let scope = "0::/user.slice/app.slice/app-kitty-100.scope";
        let terminal = process("kitty", 100, Some(1), scope);
        let cli = process("claude", 101, Some(100), scope);
        let electron = process("electron", 102, Some(100), scope);
        let mut other_terminal = process("kitty", 200, Some(1), "0::/session.scope");
        let codex = process("codex", 201, Some(200), "0::/session.scope");
        let sandbox = process(
            "bwrap",
            300,
            Some(1),
            "0::/user.slice/app.slice/app-flatpak-org.gnome.Boxes-123.scope",
        );
        let hints = vec![
            WindowHint {
                pid: 100,
                class: Some("kitty".into()),
                title: None,
                source: "test",
            },
            WindowHint {
                pid: 102,
                class: Some("Notes".into()),
                title: None,
                source: "test",
            },
            WindowHint {
                pid: 200,
                class: Some("kitty".into()),
                title: None,
                source: "test",
            },
        ];
        let processes = vec![
            terminal,
            cli,
            electron,
            other_terminal.clone(),
            codex,
            sandbox.clone(),
        ];
        let classified = classify_gui_processes(&processes, &hints);
        let names: Vec<_> = classified
            .iter()
            .map(|c| (c.identity.pid, c.display_name.as_deref().unwrap()))
            .collect();
        assert_eq!(
            names,
            [
                (100, "kitty - Claude"),
                (102, "Notes"),
                (200, "kitty - Codex"),
                (300, "GNOME Boxes")
            ]
        );
        assert!(matches!(
            crate::control::restart::resolve_restart_source(&ProcessSnapshot {
                executable: Some("/usr/bin/bwrap".into()),
                cwd: Some("/tmp".into()),
                ..sandbox.clone()
            }),
            crate::control::restart::RestartSource::Unavailable { .. }
        ));
        let mut app = crate::app::App::new();
        app.apply_scan_batch(crate::process::ScanBatch {
            processes,
            graphical: classified,
            developer: vec![],
            system: Default::default(),
        });
        app.handle_key(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('/'),
        ));
        for character in "claude".chars() {
            app.handle_key(crossterm::event::KeyEvent::from(
                crossterm::event::KeyCode::Char(character),
            ));
        }
        assert_eq!(app.processes.len(), 1);
        assert_eq!(app.processes[0].identity.pid, 100);
        assert_eq!(app.processes[0].name, "kitty");
        other_terminal.uid = u32::MAX;
        assert!(classify_gui_processes(&[other_terminal], &hints).is_empty());

        let child = process("gnome-boxes", 301, Some(300), &sandbox.cgroups[0]);
        let classified = classify_gui_processes(
            &[sandbox, child],
            &[WindowHint {
                pid: 301,
                class: Some("Boxes".into()),
                title: None,
                source: "test",
            }],
        );
        assert_eq!(classified.len(), 1);
        assert_eq!(classified[0].identity.pid, 301);
        assert_eq!(classified[0].display_name.as_deref(), Some("Boxes"));
    }

    #[test]
    fn collapses_application_scope_to_one_root() {
        let cgroup =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-browser-42.scope";
        let root = process("browser", 100, Some(1), cgroup);
        let helper = process("renderer", 101, Some(100), cgroup);

        let classified = classify_gui_processes(&[root, helper], &[]);

        assert_eq!(classified.len(), 1);
        assert_eq!(classified[0].identity.pid, 100);
        assert_eq!(classified[0].confidence, GuiConfidence::Probable);
    }

    #[test]
    fn mapped_window_confirms_the_application_root() {
        let cgroup =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-browser-42.scope";
        let root = process("browser", 100, Some(1), cgroup);
        let helper = process("renderer", 101, Some(100), cgroup);
        let hint = WindowHint {
            pid: 101,
            class: Some("Browser".to_owned()),
            title: Some("Window".to_owned()),
            source: "test compositor",
        };

        let classified = classify_gui_processes(&[root, helper], &[hint]);

        assert_eq!(classified.len(), 1);
        assert_eq!(classified[0].identity.pid, 100);
        assert_eq!(classified[0].confidence, GuiConfidence::Confirmed);
        assert!(classified[0].evidence[1].contains("class=Browser"));
    }

    #[test]
    fn excludes_unclassified_background_processes() {
        let process = process("daemon", 200, Some(1), "0::/user.slice/background.slice");

        assert!(classify_gui_processes(&[process], &[]).is_empty());
    }

    #[test]
    fn parses_only_sway_toplevels_with_positive_pids_and_app_identity() {
        let hints = parse_sway_tree(
            br#"{
                "type":"root",
                "nodes":[{
                    "type":"workspace",
                    "nodes":[
                        {"type":"con","pid":321,"app_id":"org.demo.App","name":"Demo"},
                        {"type":"con","pid":0,"app_id":"bad","name":"No PID"},
                        {"type":"con","pid":322,"name":"No app id"}
                    ]
                }]
            }"#,
        );

        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].pid, 321);
        assert_eq!(hints[0].class.as_deref(), Some("org.demo.App"));
        assert_eq!(hints[0].source, "Sway IPC mapped toplevel");
    }

    #[test]
    fn parses_only_mapped_hyprland_clients() {
        let hints = parse_hyprland_clients(
            br#"[
                {"mapped":true,"pid":123,"class":"demo","title":"Demo"},
                {"mapped":false,"pid":124,"class":"hidden","title":"Hidden"},
                {"mapped":true,"pid":0,"class":"invalid","title":"Invalid"}
            ]"#,
        );

        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].pid, 123);
        assert_eq!(hints[0].source, "Hyprland mapped client");
    }
}
