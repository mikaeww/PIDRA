use std::{ffi::OsString, path::PathBuf};

use super::{ProcessIdentity, ProcessState};

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessSnapshot {
    pub identity: ProcessIdentity,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub command: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub parent_pid: Option<i32>,
    pub tty_foreground_pgid: Option<i32>,
    pub uid: u32,
    pub state: ProcessState,
    pub rss_bytes: u64,
    pub pss_bytes: Option<u64>,
    pub virtual_bytes: u64,
    pub cpu_percent: f32,
    pub cpu_time_ticks: u64,
    pub thread_count: u32,
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub read_rate_bytes: Option<f64>,
    pub write_rate_bytes: Option<f64>,
    pub cgroups: Vec<String>,
}

impl ProcessSnapshot {
    #[must_use]
    pub fn fixture(name: &str, pid: i32, rss_bytes: u64) -> Self {
        Self {
            identity: ProcessIdentity {
                pid,
                start_time_ticks: pid.unsigned_abs().into(),
            },
            name: name.to_owned(),
            executable: None,
            command: Vec::new(),
            cwd: None,
            parent_pid: None,
            tty_foreground_pgid: None,
            uid: 1_000,
            state: ProcessState::Sleeping,
            rss_bytes,
            pss_bytes: None,
            virtual_bytes: 0,
            cpu_percent: 0.0,
            cpu_time_ticks: 0,
            thread_count: 1,
            read_bytes: None,
            write_bytes: None,
            read_rate_bytes: None,
            write_rate_bytes: None,
            cgroups: Vec::new(),
        }
    }
}
