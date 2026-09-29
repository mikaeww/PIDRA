use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::OwnedObjectPath,
};

use crate::process::{ProcessIdentity, ProcessSnapshot, ProcessState, procfs::parse_stat};

use super::{
    ControlOutcome, DeliveryMethod, SignalAction,
    signal::{read_identity, send_signal},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartSource {
    SystemdUserUnit {
        unit: String,
    },
    Direct {
        executable: PathBuf,
        arguments: Vec<OsString>,
        working_directory: PathBuf,
        application_scope: Option<String>,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartRequest {
    pub identity: ProcessIdentity,
    pub process_name: String,
    pub source: RestartSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartOutcome {
    Restarted {
        new_identity: ProcessIdentity,
        method: String,
    },
    StillRunning,
    IdentityChanged,
    NotFound,
    PermissionDenied,
    Unavailable(String),
    Failed(String),
}

impl RestartOutcome {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Restarted {
                new_identity,
                method,
            } => format!("RESTARTED as PID {} through {method}", new_identity.pid),
            Self::StillRunning => {
                "STILL RUNNING — restart aborted without force escalation".to_owned()
            }
            Self::IdentityChanged => "IDENTITY CHANGED before restart".to_owned(),
            Self::NotFound => "NOT FOUND before restart".to_owned(),
            Self::PermissionDenied => "PERMISSION DENIED".to_owned(),
            Self::Unavailable(reason) => format!("RESTART UNAVAILABLE — {reason}"),
            Self::Failed(reason) => format!("RESTART FAILED — {reason}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestartResult {
    pub request: RestartRequest,
    pub outcome: RestartOutcome,
}

impl RestartSource {
    #[must_use]
    pub fn is_available(&self) -> bool {
        !matches!(self, Self::Unavailable { .. })
    }

    #[must_use]
    pub fn summary(&self) -> String {
        match self {
            Self::SystemdUserUnit { unit } => format!("systemd user unit {unit}"),
            Self::Direct {
                executable,
                arguments,
                working_directory,
                application_scope,
            } => {
                let scope_note = application_scope
                    .as_deref()
                    .map_or_else(String::new, |scope| {
                        format!("; transient scope {scope} cannot be started by systemd")
                    });
                format!(
                    "direct {} ({} arguments, cwd {}){scope_note}",
                    executable.display(),
                    arguments.len(),
                    working_directory.display()
                )
            }
            Self::Unavailable { reason } => format!("unavailable — {reason}"),
        }
    }
}

#[must_use]
pub fn resolve_restart_source(process: &ProcessSnapshot) -> RestartSource {
    if process.uid != rustix::process::getuid().as_raw() {
        return unavailable("process is not owned by the current user");
    }
    if matches!(
        process.state,
        ProcessState::Zombie | ProcessState::Dead | ProcessState::DiskSleep
    ) {
        return unavailable("process state does not permit a controlled restart");
    }
    if let Some(unit) = systemd_service_unit(&process.cgroups) {
        return RestartSource::SystemdUserUnit { unit };
    }
    let Some(executable) = process.executable.clone() else {
        return unavailable("executable path is unreadable or this is a kernel thread");
    };
    if executable
        .file_name()
        .is_some_and(|name| name == "bwrap" || name == "flatpak")
    {
        return unavailable(
            "sandbox launcher cannot be replayed safely; reopen the application from its launcher",
        );
    }
    if !executable.is_absolute() {
        return unavailable("executable path is not absolute");
    }
    let Some(working_directory) = process.cwd.clone() else {
        return unavailable("working directory is unreadable");
    };
    if !working_directory.is_absolute() {
        return unavailable("working directory is not absolute");
    }

    RestartSource::Direct {
        executable,
        arguments: process.command.iter().skip(1).cloned().collect(),
        working_directory,
        application_scope: systemd_scope_unit(&process.cgroups),
    }
}

#[must_use]
pub fn execute_restart(request: &RestartRequest) -> RestartOutcome {
    match &request.source {
        RestartSource::SystemdUserUnit { unit } => restart_systemd_user_unit(request, unit),
        RestartSource::Direct {
            executable,
            arguments,
            working_directory,
            ..
        } => restart_direct(request, executable, arguments, working_directory),
        RestartSource::Unavailable { reason } => RestartOutcome::Unavailable(reason.clone()),
    }
}

fn restart_direct(
    request: &RestartRequest,
    executable: &Path,
    arguments: &[OsString],
    working_directory: &Path,
) -> RestartOutcome {
    if let Err(reason) = validate_direct_source(executable, working_directory) {
        return RestartOutcome::Unavailable(reason);
    }
    if let Err(outcome) = validate_restart_identity(request.identity) {
        return outcome;
    }
    match send_signal(request.identity, SignalAction::Stop) {
        ControlOutcome::Sent(DeliveryMethod::Pidfd | DeliveryMethod::ValidatedKillFallback) => {}
        ControlOutcome::IdentityChanged => return RestartOutcome::IdentityChanged,
        ControlOutcome::NotFound => return RestartOutcome::NotFound,
        ControlOutcome::PermissionDenied => return RestartOutcome::PermissionDenied,
        ControlOutcome::Protected(reason)
        | ControlOutcome::Unsupported(reason)
        | ControlOutcome::Failed(reason) => return RestartOutcome::Failed(reason),
    }

    if !wait_for_old_identity_to_exit(request.identity, Duration::from_secs(2)) {
        return RestartOutcome::StillRunning;
    }

    let child = match Command::new(executable)
        .args(arguments)
        .current_dir(working_directory)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return RestartOutcome::Failed(error.to_string()),
    };
    let new_pid = match i32::try_from(child.id()) {
        Ok(pid) => pid,
        Err(_) => return RestartOutcome::Failed("new PID does not fit i32".to_owned()),
    };
    wait_for_new_identity(new_pid, Duration::from_secs(1)).map_or_else(
        || RestartOutcome::Failed("new process exited before it could be observed".to_owned()),
        |new_identity| RestartOutcome::Restarted {
            new_identity,
            method: "direct exec (no shell)".to_owned(),
        },
    )
}

fn restart_systemd_user_unit(request: &RestartRequest, unit: &str) -> RestartOutcome {
    if !valid_service_unit(unit) {
        return RestartOutcome::Unavailable("invalid systemd service unit name".to_owned());
    }
    if let Err(outcome) = validate_restart_identity(request.identity) {
        return outcome;
    }
    let connection = match Connection::session() {
        Ok(connection) => connection,
        Err(error) => return RestartOutcome::Unavailable(format!("user D-Bus: {error}")),
    };
    let manager = match systemd_manager(&connection) {
        Ok(proxy) => proxy,
        Err(error) => return RestartOutcome::Unavailable(format!("systemd user manager: {error}")),
    };
    let unit_path: OwnedObjectPath = match manager.call(
        "GetUnitByPID",
        &(u32::try_from(request.identity.pid).unwrap_or_default(),),
    ) {
        Ok(path) => path,
        Err(error) => return RestartOutcome::Failed(format!("GetUnitByPID: {error}")),
    };
    let unit_proxy = match Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        unit_path.as_str(),
        "org.freedesktop.systemd1.Unit",
    ) {
        Ok(proxy) => proxy,
        Err(error) => return RestartOutcome::Failed(format!("unit proxy: {error}")),
    };
    let actual_unit: String = match unit_proxy.get_property("Id") {
        Ok(id) => id,
        Err(error) => return RestartOutcome::Failed(format!("unit identity: {error}")),
    };
    if actual_unit != unit {
        return RestartOutcome::IdentityChanged;
    }
    if let Err(outcome) = validate_restart_identity(request.identity) {
        return outcome;
    }
    let _: OwnedObjectPath = match manager.call("RestartUnit", &(unit, "replace")) {
        Ok(job) => job,
        Err(error) => return RestartOutcome::Failed(format!("RestartUnit: {error}")),
    };

    wait_for_systemd_main_pid(
        &connection,
        &unit_path,
        request.identity,
        Duration::from_secs(5),
    )
    .map_or_else(
        || RestartOutcome::Failed("systemd did not expose a replacement MainPID".to_owned()),
        |new_identity| RestartOutcome::Restarted {
            new_identity,
            method: format!("systemd user unit {unit}"),
        },
    )
}

fn systemd_manager(connection: &Connection) -> zbus::Result<Proxy<'_>> {
    Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )
}

fn wait_for_systemd_main_pid(
    connection: &Connection,
    unit_path: &OwnedObjectPath,
    old_identity: ProcessIdentity,
    timeout: Duration,
) -> Option<ProcessIdentity> {
    let service = Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        unit_path.as_str(),
        "org.freedesktop.systemd1.Service",
    )
    .ok()?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let main_pid: u32 = service.get_property("MainPID").unwrap_or(0);
        if let Ok(pid) = i32::try_from(main_pid)
            && pid > 0
            && let Ok(identity) = read_identity(Path::new("/proc"), pid)
            && identity != old_identity
        {
            return Some(identity);
        }
        thread::sleep(Duration::from_millis(25));
    }
    None
}

fn validate_restart_identity(identity: ProcessIdentity) -> Result<(), RestartOutcome> {
    match read_identity(Path::new("/proc"), identity.pid) {
        Ok(current) if current == identity => Ok(()),
        Ok(_) => Err(RestartOutcome::IdentityChanged),
        Err(ControlOutcome::NotFound) => Err(RestartOutcome::NotFound),
        Err(ControlOutcome::PermissionDenied) => Err(RestartOutcome::PermissionDenied),
        Err(other) => Err(RestartOutcome::Failed(other.message())),
    }
}

fn validate_direct_source(executable: &Path, working_directory: &Path) -> Result<(), String> {
    if !executable.is_absolute() || !working_directory.is_absolute() {
        return Err("executable and working directory must be absolute".to_owned());
    }
    let executable_metadata = fs::metadata(executable)
        .map_err(|error| format!("cannot access executable {}: {error}", executable.display()))?;
    if !executable_metadata.is_file() || executable_metadata.permissions().mode() & 0o111 == 0 {
        return Err(format!("{} is not executable", executable.display()));
    }
    let cwd_metadata = fs::metadata(working_directory).map_err(|error| {
        format!(
            "cannot access working directory {}: {error}",
            working_directory.display()
        )
    })?;
    if !cwd_metadata.is_dir() {
        return Err(format!(
            "{} is not a directory",
            working_directory.display()
        ));
    }
    Ok(())
}

fn wait_for_old_identity_to_exit(identity: ProcessIdentity, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match current_identity_and_state(identity.pid) {
            None => return true,
            Some((current, _)) if current != identity => return true,
            Some((_, ProcessState::Zombie | ProcessState::Dead)) => return true,
            Some(_) => thread::sleep(Duration::from_millis(25)),
        }
    }
    false
}

fn wait_for_new_identity(pid: i32, timeout: Duration) -> Option<ProcessIdentity> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(identity) = read_identity(Path::new("/proc"), pid) {
            return Some(identity);
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

fn current_identity_and_state(pid: i32) -> Option<(ProcessIdentity, ProcessState)> {
    let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
    let parsed = parse_stat(&stat).ok()?;
    Some((
        ProcessIdentity {
            pid: parsed.pid,
            start_time_ticks: parsed.start_time_ticks,
        },
        parsed.state,
    ))
}

fn valid_service_unit(unit: &str) -> bool {
    unit.ends_with(".service")
        && !unit.is_empty()
        && unit.len() <= 255
        && !unit.contains(['/', '\0'])
}

fn unavailable(reason: &str) -> RestartSource {
    RestartSource::Unavailable {
        reason: reason.to_owned(),
    }
}

fn systemd_service_unit(cgroups: &[String]) -> Option<String> {
    cgroups.iter().find_map(|entry| {
        let path = entry
            .split_once("::")
            .map_or(entry.as_str(), |(_, path)| path);
        path.rsplit('/').find_map(|component| {
            let is_service = component.ends_with(".service");
            let is_user_manager = component.starts_with("user@");
            let is_session_core = matches!(
                component,
                "dbus.service" | "pipewire.service" | "wireplumber.service"
            );
            (is_service && !is_user_manager && !is_session_core).then(|| component.to_owned())
        })
    })
}

fn systemd_scope_unit(cgroups: &[String]) -> Option<String> {
    cgroups.iter().find_map(|entry| {
        let path = entry
            .split_once("::")
            .map_or(entry.as_str(), |(_, path)| path);
        path.rsplit('/')
            .find(|component| component.ends_with(".scope"))
            .map(str::to_owned)
    })
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::{RestartSource, resolve_restart_source};
    use crate::process::{ProcessSnapshot, ProcessState};

    fn owned_process() -> ProcessSnapshot {
        let mut process = ProcessSnapshot::fixture("demo", 200, 1);
        process.uid = rustix::process::getuid().as_raw();
        process.executable = Some(PathBuf::from("/usr/bin/demo"));
        process.cwd = Some(PathBuf::from("/tmp"));
        process.command = vec![OsString::from("demo"), OsString::from("--safe")];
        process
    }

    #[test]
    fn systemd_service_has_priority_over_direct_metadata() {
        let mut process = owned_process();
        process.cgroups = vec![
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/demo.service".to_owned(),
        ];

        assert_eq!(
            resolve_restart_source(&process),
            RestartSource::SystemdUserUnit {
                unit: "demo.service".to_owned()
            }
        );
    }

    #[test]
    fn direct_source_keeps_argument_boundaries() {
        let process = owned_process();

        assert_eq!(
            resolve_restart_source(&process),
            RestartSource::Direct {
                executable: PathBuf::from("/usr/bin/demo"),
                arguments: vec![OsString::from("--safe")],
                working_directory: PathBuf::from("/tmp"),
                application_scope: None,
            }
        );
    }

    #[test]
    fn transient_application_scope_uses_direct_restart_with_an_explanation() {
        let mut process = owned_process();
        process.cgroups = vec![
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.chromium.Chromium-2031.scope"
                .to_owned(),
        ];

        let source = resolve_restart_source(&process);

        assert!(matches!(
            &source,
            RestartSource::Direct {
                application_scope: Some(scope),
                ..
            } if scope == "app-org.chromium.Chromium-2031.scope"
        ));
        assert!(source.summary().contains("cannot be started by systemd"));
    }

    #[test]
    fn rejects_unreadable_and_unsafe_process_states() {
        let mut process = owned_process();
        process.executable = None;
        assert!(!resolve_restart_source(&process).is_available());

        process.executable = Some(PathBuf::from("/usr/bin/demo"));
        process.state = ProcessState::Zombie;
        assert!(!resolve_restart_source(&process).is_available());
    }
}
