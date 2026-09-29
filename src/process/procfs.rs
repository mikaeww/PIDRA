use std::{
    collections::HashSet,
    ffi::OsString,
    fs, io,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
};

use thiserror::Error;

use super::{ProcessIdentity, ProcessSnapshot, ProcessState, tree::ProcessTree};

#[derive(Debug, Error)]
pub enum ProcfsError {
    #[error("cannot read procfs root {path}: {source}")]
    ReadRoot {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStat {
    pub pid: i32,
    pub name: String,
    pub state: ProcessState,
    pub parent_pid: Option<i32>,
    pub tty_foreground_pgid: Option<i32>,
    pub cpu_time_ticks: u64,
    pub thread_count: u32,
    pub start_time_ticks: u64,
    pub virtual_bytes: u64,
    pub rss_pages: i64,
}

pub fn scan_procfs(root: &Path) -> Result<Vec<ProcessSnapshot>, ProcfsError> {
    let entries = fs::read_dir(root).map_err(|source| ProcfsError::ReadRoot {
        path: root.to_path_buf(),
        source,
    })?;
    let page_size = rustix::param::page_size() as u64;
    let mut snapshots = Vec::new();

    for entry in entries.flatten() {
        let Some(pid) = numeric_pid(&entry.file_name()) else {
            continue;
        };
        let process_dir = entry.path();
        if let Some(snapshot) = read_process(&process_dir, pid, page_size) {
            snapshots.push(snapshot);
        }
    }

    Ok(snapshots)
}

pub fn scan_system_procfs() -> Result<Vec<ProcessSnapshot>, ProcfsError> {
    scan_procfs(Path::new("/proc"))
}

fn numeric_pid(name: &OsString) -> Option<i32> {
    name.to_str()?.parse::<i32>().ok().filter(|pid| *pid > 0)
}

fn read_process(process_dir: &Path, directory_pid: i32, page_size: u64) -> Option<ProcessSnapshot> {
    let stat_bytes = fs::read(process_dir.join("stat")).ok()?;
    let stat = parse_stat(&stat_bytes).ok()?;
    if stat.pid != directory_pid {
        return None;
    }

    let status = fs::read(process_dir.join("status")).ok();
    let uid = status
        .as_deref()
        .and_then(parse_status_uid)
        .or_else(|| {
            fs::metadata(process_dir)
                .ok()
                .map(|metadata| metadata.uid())
        })
        .unwrap_or(u32::MAX);
    let (read_bytes, write_bytes) = fs::read(process_dir.join("io"))
        .ok()
        .map_or((None, None), |contents| parse_io(&contents));

    let rss_bytes = u64::try_from(stat.rss_pages)
        .unwrap_or(0)
        .saturating_mul(page_size);

    Some(ProcessSnapshot {
        identity: ProcessIdentity {
            pid: stat.pid,
            start_time_ticks: stat.start_time_ticks,
        },
        name: stat.name,
        executable: fs::read_link(process_dir.join("exe")).ok(),
        command: fs::read(process_dir.join("cmdline"))
            .ok()
            .map_or_else(Vec::new, |bytes| parse_cmdline(&bytes)),
        cwd: fs::read_link(process_dir.join("cwd")).ok(),
        parent_pid: stat.parent_pid,
        tty_foreground_pgid: stat.tty_foreground_pgid,
        uid,
        state: stat.state,
        rss_bytes,
        pss_bytes: None,
        virtual_bytes: stat.virtual_bytes,
        cpu_percent: 0.0,
        cpu_time_ticks: stat.cpu_time_ticks,
        thread_count: stat.thread_count,
        read_bytes,
        write_bytes,
        read_rate_bytes: None,
        write_rate_bytes: None,
        cgroups: fs::read(process_dir.join("cgroup"))
            .ok()
            .map_or_else(Vec::new, |contents| parse_cgroups(&contents)),
    })
}

pub fn enrich_pss(
    root: &Path,
    processes: &mut [ProcessSnapshot],
    roots: impl IntoIterator<Item = ProcessIdentity>,
) {
    let identities: HashSet<_> = {
        let tree = ProcessTree::new(processes);
        roots
            .into_iter()
            .flat_map(|identity| std::iter::once(identity).chain(tree.descendants(identity)))
            .collect()
    };
    for process in processes {
        if !identities.contains(&process.identity) {
            continue;
        }
        let process_dir = root.join(process.identity.pid.to_string());
        let pss = fs::read(process_dir.join("smaps_rollup"))
            .ok()
            .and_then(|contents| parse_smaps_rollup_pss(&contents));
        let identity_still_matches = fs::read(process_dir.join("stat"))
            .ok()
            .and_then(|contents| parse_stat(&contents).ok())
            .is_some_and(|stat| stat.start_time_ticks == process.identity.start_time_ticks);
        if identity_still_matches {
            process.pss_bytes = pss;
        }
    }
}

pub fn parse_stat(input: &[u8]) -> Result<ParsedStat, &'static str> {
    let open = input
        .iter()
        .position(|byte| *byte == b'(')
        .ok_or("missing (")?;
    let close = input
        .iter()
        .rposition(|byte| *byte == b')')
        .ok_or("missing )")?;
    if close <= open {
        return Err("invalid comm field");
    }

    let pid = parse_utf8(&input[..open])?
        .trim()
        .parse::<i32>()
        .map_err(|_| "invalid pid")?;
    let name = String::from_utf8_lossy(&input[open + 1..close]).into_owned();
    let remaining = input.get(close + 1..).ok_or("missing stat fields")?;
    let fields: Vec<&[u8]> = remaining
        .split(|byte| byte.is_ascii_whitespace())
        .filter(|field| !field.is_empty())
        .collect();
    if fields.len() < 22 {
        return Err("truncated stat fields");
    }

    let state_text = parse_utf8(fields[0])?;
    let state_char = state_text.chars().next().ok_or("missing state")?;
    let parent_pid_raw = parse_i64(fields[1], "invalid ppid")?;
    let parent_pid = i32::try_from(parent_pid_raw)
        .ok()
        .filter(|parent| *parent > 0);
    let session = parse_i64(fields[3], "invalid session")?;
    let tty_nr = parse_i64(fields[4], "invalid tty")?;
    let tpgid = parse_i64(fields[5], "invalid tpgid")?;
    // Only a session leader on a tty is the shell a terminal window spawned; tpgid then names what runs in its foreground.
    let tty_foreground_pgid = (tty_nr != 0 && session == i64::from(pid))
        .then(|| i32::try_from(tpgid).ok().filter(|pgid| *pgid > 0))
        .flatten();
    let user_ticks = parse_u64(fields[11], "invalid utime")?;
    let system_ticks = parse_u64(fields[12], "invalid stime")?;

    Ok(ParsedStat {
        pid,
        name,
        state: ProcessState::from_procfs(state_char),
        parent_pid,
        tty_foreground_pgid,
        cpu_time_ticks: user_ticks.saturating_add(system_ticks),
        thread_count: u32::try_from(parse_u64(fields[17], "invalid threads")?)
            .map_err(|_| "thread count overflow")?,
        start_time_ticks: parse_u64(fields[19], "invalid start time")?,
        virtual_bytes: parse_u64(fields[20], "invalid virtual size")?,
        rss_pages: parse_i64(fields[21], "invalid rss")?,
    })
}

fn parse_utf8(value: &[u8]) -> Result<&str, &'static str> {
    std::str::from_utf8(value).map_err(|_| "field is not UTF-8")
}

fn parse_u64(value: &[u8], error: &'static str) -> Result<u64, &'static str> {
    parse_utf8(value)?.parse::<u64>().map_err(|_| error)
}

fn parse_i64(value: &[u8], error: &'static str) -> Result<i64, &'static str> {
    parse_utf8(value)?.parse::<i64>().map_err(|_| error)
}

fn parse_status_uid(contents: &[u8]) -> Option<u32> {
    lines(contents).find_map(|line| {
        let value = line.strip_prefix(b"Uid:")?;
        std::str::from_utf8(value)
            .ok()?
            .split_ascii_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn parse_io(contents: &[u8]) -> (Option<u64>, Option<u64>) {
    let mut read_bytes = None;
    let mut write_bytes = None;
    for line in lines(contents) {
        if let Some(value) = line.strip_prefix(b"read_bytes:") {
            read_bytes = parse_optional_u64(value);
        } else if let Some(value) = line.strip_prefix(b"write_bytes:") {
            write_bytes = parse_optional_u64(value);
        }
    }
    (read_bytes, write_bytes)
}

fn parse_optional_u64(value: &[u8]) -> Option<u64> {
    std::str::from_utf8(value).ok()?.trim().parse().ok()
}

fn parse_smaps_rollup_pss(contents: &[u8]) -> Option<u64> {
    let kib = lines(contents).find_map(|line| {
        let value = line.strip_prefix(b"Pss:")?;
        std::str::from_utf8(value)
            .ok()?
            .split_ascii_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    })?;
    Some(kib.saturating_mul(1024))
}

fn parse_cmdline(contents: &[u8]) -> Vec<OsString> {
    contents
        .split(|byte| *byte == 0)
        .filter(|argument| !argument.is_empty())
        .map(|argument| OsString::from_vec(argument.to_vec()))
        .collect()
}

fn parse_cgroups(contents: &[u8]) -> Vec<String> {
    lines(contents)
        .filter(|line| !line.is_empty())
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect()
}

fn lines(contents: &[u8]) -> impl Iterator<Item = &[u8]> {
    contents
        .split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
}

#[cfg(test)]
mod tests {
    use super::{parse_cmdline, parse_io, parse_smaps_rollup_pss, parse_stat, parse_status_uid};
    use crate::process::ProcessState;

    #[test]
    fn parses_spaces_and_parentheses_in_comm() {
        let input =
            b"101 (name with ) and ( parens) R 1 0 0 0 0 0 0 0 0 0 12 3 0 0 20 0 4 0 12345 4096 10";

        let parsed = parse_stat(input).expect("valid stat");

        assert_eq!(parsed.name, "name with ) and ( parens");
        assert_eq!(parsed.state, ProcessState::Running);
        assert_eq!(parsed.start_time_ticks, 12_345);
        assert_eq!(parsed.rss_pages, 10);
    }

    #[test]
    fn rejects_truncated_stat() {
        assert!(parse_stat(b"1 (short) S 0").is_err());
    }

    #[test]
    fn parses_partial_status_and_io() {
        assert_eq!(
            parse_status_uid(b"Name:\ttest\nUid:\t1000 1000 1000 1000\n"),
            Some(1_000)
        );
        assert_eq!(
            parse_io(b"rchar: 20\nread_bytes: 4096\nwrite_bytes: 8192\n"),
            (Some(4_096), Some(8_192))
        );
    }

    #[test]
    fn preserves_non_utf8_command_arguments() {
        let arguments = parse_cmdline(b"app\0arg\xff\0");
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[0], "app");
    }

    #[test]
    fn parses_pss_from_smaps_rollup_in_kibibytes() {
        assert_eq!(
            parse_smaps_rollup_pss(b"Rss: 20 kB\nPss: 12 kB\n"),
            Some(12 * 1024)
        );
        assert_eq!(parse_smaps_rollup_pss(b"Rss: 20 kB\n"), None);
    }
}
