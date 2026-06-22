use super::detector::LegacySessionSource;
use super::detector_cli::CliSessionSource;
use super::source::{DetectedSession, DetectionDiagnostics, SessionDetectorError, SessionSource};
use std::collections::HashSet;

/// Auto-mode source that runs both the CLI (`claude agents --json`) and legacy
/// (process-scan) backends each cycle and merges their results.
///
/// This surfaces both:
/// - sessions the CLI registry knows about (with rich metadata: official name,
///   busy/idle activity, kind), and
/// - sessions only visible via process scanning — e.g. older `claude` builds
///   that predate the agent registry, or sessions running in an elevated
///   terminal that the CLI registry didn't capture.
///
/// One backend failing (e.g. `claude agents --json` unsupported on an old
/// build) degrades gracefully to the other instead of blanking the list.
pub struct MergedSessionSource {
    cli: CliSessionSource,
    legacy: Option<LegacySessionSource>,
}

impl MergedSessionSource {
    pub fn new() -> Self {
        Self {
            cli: CliSessionSource::new(),
            // Legacy ctor only fails if the home dir can't be resolved; in that
            // case run CLI-only rather than refusing to start.
            legacy: LegacySessionSource::new().ok(),
        }
    }
}

impl Default for MergedSessionSource {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge legacy results into the CLI results, deduplicating by session id and
/// then by pid. CLI entries take precedence (they carry richer metadata), so
/// only legacy sessions not already present are appended.
pub(crate) fn merge_sessions(
    cli: Vec<DetectedSession>,
    legacy: Vec<DetectedSession>,
) -> Vec<DetectedSession> {
    let mut seen_sids: HashSet<String> =
        cli.iter().filter_map(|s| s.session_id.clone()).collect();
    let mut seen_pids: HashSet<u32> = cli.iter().map(|s| s.pid).collect();

    let mut out = cli;
    for s in legacy {
        let dup_sid = s
            .session_id
            .as_ref()
            .map(|id| seen_sids.contains(id))
            .unwrap_or(false);
        if dup_sid || seen_pids.contains(&s.pid) {
            continue;
        }
        if let Some(id) = &s.session_id {
            seen_sids.insert(id.clone());
        }
        seen_pids.insert(s.pid);
        out.push(s);
    }
    out
}

impl SessionSource for MergedSessionSource {
    fn detect(
        &mut self,
    ) -> Result<(Vec<DetectedSession>, DetectionDiagnostics), SessionDetectorError> {
        let cli_res = self.cli.detect();
        let legacy_res = self.legacy.as_mut().map(|l| l.detect());

        match (cli_res, legacy_res) {
            // Both ran: merge, and keep the legacy diagnostics (process counts).
            (Ok((cli_s, _)), Some(Ok((leg_s, leg_diag)))) => {
                Ok((merge_sessions(cli_s, leg_s), leg_diag))
            }
            // CLI ok, legacy failed or absent → CLI only.
            (Ok((cli_s, cli_diag)), _) => Ok((cli_s, cli_diag)),
            // CLI failed, legacy ok → legacy only.
            (Err(_), Some(Ok((leg_s, leg_diag)))) => Ok((leg_s, leg_diag)),
            // Both failed (or CLI failed with no legacy) → surface the CLI error.
            (Err(e), _) => Err(e),
        }
    }

    fn backend_name(&self) -> &'static str {
        "merged"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::source::{CliActivity, DetectedSession, SessionKind};
    use std::path::PathBuf;

    fn cli_session(pid: u32, sid: &str) -> DetectedSession {
        DetectedSession {
            pid,
            cwd: PathBuf::from("/tmp"),
            project_path: PathBuf::from("/tmp/proj"),
            session_id: Some(sid.to_string()),
            project_name: "proj".to_string(),
            kind: SessionKind::Interactive,
            started_at_ms: Some(1),
            official_name: Some("cli-name".to_string()),
            cli_activity: Some(CliActivity::Busy),
        }
    }

    fn legacy_session(pid: u32, sid: Option<&str>) -> DetectedSession {
        DetectedSession::with_legacy_defaults(
            pid,
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp/proj"),
            sid.map(|s| s.to_string()),
            "proj".to_string(),
        )
    }

    #[test]
    fn merges_disjoint_sessions() {
        let cli = vec![cli_session(1, "a")];
        let legacy = vec![legacy_session(2, Some("b"))];
        let out = merge_sessions(cli, legacy);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedups_same_session_id_keeping_cli_entry() {
        let cli = vec![cli_session(1, "a")];
        let legacy = vec![legacy_session(1, Some("a"))];
        let out = merge_sessions(cli, legacy);
        assert_eq!(out.len(), 1);
        // CLI entry wins — its richer metadata is preserved.
        assert_eq!(out[0].official_name.as_deref(), Some("cli-name"));
        assert_eq!(out[0].cli_activity, Some(CliActivity::Busy));
    }

    #[test]
    fn dedups_same_pid_even_when_session_id_differs() {
        let cli = vec![cli_session(1, "a")];
        // Same pid, no/other session id → still treated as the same process.
        let legacy = vec![legacy_session(1, None)];
        let out = merge_sessions(cli, legacy);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn appends_legacy_only_sessions() {
        // Sessions the CLI registry missed (e.g. older build / elevated) appear.
        let cli = vec![cli_session(1, "a")];
        let legacy = vec![
            legacy_session(1, Some("a")), // dup, dropped
            legacy_session(2, Some("b")), // new
            legacy_session(3, None),      // new, no session id
        ];
        let out = merge_sessions(cli, legacy);
        assert_eq!(out.len(), 3);
        let pids: Vec<u32> = out.iter().map(|s| s.pid).collect();
        assert!(pids.contains(&2) && pids.contains(&3));
    }

    #[test]
    fn empty_inputs_yield_empty() {
        assert!(merge_sessions(vec![], vec![]).is_empty());
    }
}
