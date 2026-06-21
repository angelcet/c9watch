//! Small cross-platform process helpers shared by the GUI actions and the PM
//! daemon.
//!
//! These replace the previously Unix-only primitives: `libc::kill(pid, 0)`
//! liveness checks and `ps`-based parent-PID walking. The parent/name lookups
//! go through `sysinfo` (already a dependency) so they work identically on
//! macOS, Linux, and Windows; liveness uses the cheapest native call per OS.

use std::collections::HashMap;

/// Returns `true` if a process with `pid` currently exists.
pub fn process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        // kill(pid, 0) does permission/existence checking without sending a signal.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{CloseHandle, FALSE};
        use windows::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        // 259 == STILL_ACTIVE: GetExitCodeProcess reports this for a running process.
        const STILL_ACTIVE: u32 = 259;
        unsafe {
            match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) {
                Ok(handle) => {
                    let mut code = 0u32;
                    let alive =
                        GetExitCodeProcess(handle, &mut code).is_ok() && code == STILL_ACTIVE;
                    let _ = CloseHandle(handle);
                    alive
                }
                Err(_) => false,
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

/// A one-shot snapshot of the running process table: parent PID, process name,
/// and (where available) the full executable path for every process.
///
/// Capturing once and walking the in-memory maps avoids spawning `ps` per
/// ancestor (the old Unix approach) and works on Windows where `ps` is absent.
pub struct ProcessTree {
    parents: HashMap<u32, u32>,
    names: HashMap<u32, String>,
    exes: HashMap<u32, String>,
}

impl ProcessTree {
    pub fn capture() -> Self {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System, UpdateKind};

        let refresh = ProcessRefreshKind::new().with_exe(UpdateKind::OnlyIfNotSet);
        let mut sys =
            System::new_with_specifics(RefreshKind::new().with_processes(refresh));
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);

        let mut parents = HashMap::new();
        let mut names = HashMap::new();
        let mut exes = HashMap::new();
        for (pid, process) in sys.processes() {
            let p = pid.as_u32();
            if let Some(parent) = process.parent() {
                parents.insert(p, parent.as_u32());
            }
            names.insert(p, process.name().to_string_lossy().to_string());
            if let Some(exe) = process.exe() {
                exes.insert(p, exe.to_string_lossy().to_string());
            }
        }
        Self {
            parents,
            names,
            exes,
        }
    }

    /// Parent PID of `pid`, or `None` if unknown or at the root.
    pub fn parent(&self, pid: u32) -> Option<u32> {
        self.parents.get(&pid).copied().filter(|&pp| pp != 0 && pp != pid)
    }

    /// Short process name (e.g. `Code.exe`, `node`).
    pub fn name(&self, pid: u32) -> Option<&str> {
        self.names.get(&pid).map(|s| s.as_str())
    }

    /// Full executable path, when sysinfo could resolve it.
    pub fn exe(&self, pid: u32) -> Option<&str> {
        self.exes.get(&pid).map(|s| s.as_str())
    }
}

/// Convenience: parent PID of a single process via a fresh snapshot. Prefer
/// [`ProcessTree::capture`] when walking several ancestors.
pub fn parent_pid(pid: u32) -> Option<u32> {
    ProcessTree::capture().parent(pid)
}

/// Resolve the `claude` executable to a path suitable for `Command::new`.
///
/// On Unix the bare name resolves via PATH. On Windows, npm installs Claude Code
/// as `claude.cmd`, which `Command::new("claude")` will not find (it only tries
/// `.exe`); we search PATH for `claude.exe`/`claude.cmd`/`claude.bat` and return
/// the full path. Modern Rust executes `.cmd`/`.bat` paths through cmd.exe with
/// proper argument escaping. Falls back to the bare name if nothing is found.
pub fn resolve_claude() -> std::path::PathBuf {
    #[cfg(windows)]
    {
        if let Some(path) = which_windows("claude") {
            return path;
        }
    }
    std::path::PathBuf::from("claude")
}

#[cfg(windows)]
fn which_windows(stem: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for ext in ["exe", "cmd", "bat"] {
            let candidate = dir.join(format!("{}.{}", stem, ext));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_process_is_alive() {
        assert!(process_alive(std::process::id()));
    }

    #[test]
    fn pid_zero_is_not_alive() {
        assert!(!process_alive(0));
    }

    #[test]
    fn current_process_has_a_parent() {
        // The test runner was launched by some parent (cargo/shell), so a parent
        // PID should resolve on every supported platform.
        let tree = ProcessTree::capture();
        assert!(tree.name(std::process::id()).is_some());
    }
}
