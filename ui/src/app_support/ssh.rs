use super::*;

/// First open vs after a failed probe (failed phase must not keep probing).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SshCheckPhase {
    /// Never probed (or status unknown): one intentional probe is allowed.
    NeedConfirm,
    /// Probe already failed: show diagnosis and fix actions, not re-probe as primary.
    Failed,
}

/// Modal asking the user to confirm SSH reachability before the agent can use a host.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SshConnectivityModal {
    pub(crate) context_id: String,
    pub(crate) label: String,
    pub(crate) detail: String,
    /// When true, a successful probe enables this context for the current session.
    pub(crate) enable_after_probe: bool,
    pub(crate) phase: SshCheckPhase,
}

impl SshConnectivityModal {
    pub(crate) fn need_confirm(
        context_id: String,
        label: String,
        detail: String,
        enable_after_probe: bool,
    ) -> Self {
        Self {
            context_id,
            label,
            detail,
            enable_after_probe,
            phase: SshCheckPhase::NeedConfirm,
        }
    }

    pub(crate) fn failed(
        context_id: String,
        label: String,
        detail: String,
        enable_after_probe: bool,
    ) -> Self {
        Self {
            context_id,
            label,
            detail,
            enable_after_probe,
            phase: SshCheckPhase::Failed,
        }
    }

    /// Prefer Failed when we already know the last probe error.
    pub(crate) fn from_gap(
        context_id: String,
        label: String,
        detail: String,
        enable_after_probe: bool,
    ) -> Self {
        let phase = if detail == "not probed yet" {
            SshCheckPhase::NeedConfirm
        } else {
            SshCheckPhase::Failed
        };
        Self {
            context_id,
            label,
            detail,
            enable_after_probe,
            phase,
        }
    }
}

/// Classified SSH failure for diagnosis copy and fix guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SshFailKind {
    PasswordAuth,
    KeyAuth,
    Auth,
    IdentityMissing,
    Timeout,
    Resolve,
    HostKey,
    ProbeOutput,
    Other,
}

pub(crate) fn classify_ssh_failure(detail: &str) -> SshFailKind {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("ssh password authentication failed") {
        SshFailKind::PasswordAuth
    } else if lower.contains("ssh key authentication failed") {
        SshFailKind::KeyAuth
    } else if lower.contains("ssh connection succeeded")
        || lower.contains("ssh authentication succeeded")
        || lower.contains("probe command returned no output")
    {
        SshFailKind::ProbeOutput
    } else if lower.contains("identity file")
        || lower.contains("not accessible")
        || lower.contains("no such identity")
    {
        SshFailKind::IdentityMissing
    } else if lower.contains("permission denied")
        || lower.contains("publickey")
        || lower.contains("too many authentication failures")
        || lower.contains("authentication failed")
    {
        SshFailKind::Auth
    } else if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("no route to host")
        || lower.contains("network is unreachable")
    {
        SshFailKind::Timeout
    } else if lower.contains("could not resolve")
        || lower.contains("name or service not known")
        || lower.contains("nodename nor servname")
    {
        SshFailKind::Resolve
    } else if lower.contains("host key verification failed")
        || lower.contains("remote host identification has changed")
    {
        SshFailKind::HostKey
    } else {
        SshFailKind::Other
    }
}

/// i18n keys for bullet causes under the failed-diagnosis phase.
pub(crate) fn ssh_fail_cause_keys(kind: SshFailKind) -> &'static [&'static str] {
    match kind {
        SshFailKind::PasswordAuth => &[
            "ssh_check.cause.password.1",
            "ssh_check.cause.password.2",
            "ssh_check.cause.password.3",
        ],
        SshFailKind::KeyAuth => &[
            "ssh_check.cause.key.1",
            "ssh_check.cause.key.2",
            "ssh_check.cause.key.3",
        ],
        SshFailKind::Auth => &[
            "ssh_check.cause.auth.1",
            "ssh_check.cause.auth.2",
            "ssh_check.cause.auth.3",
            "ssh_check.cause.auth.4",
        ],
        SshFailKind::IdentityMissing => {
            &["ssh_check.cause.identity.1", "ssh_check.cause.identity.2"]
        }
        SshFailKind::Timeout => &[
            "ssh_check.cause.timeout.1",
            "ssh_check.cause.timeout.2",
            "ssh_check.cause.timeout.3",
        ],
        SshFailKind::Resolve => &["ssh_check.cause.resolve.1", "ssh_check.cause.resolve.2"],
        SshFailKind::HostKey => &["ssh_check.cause.hostkey.1", "ssh_check.cause.hostkey.2"],
        SshFailKind::ProbeOutput => &[
            "ssh_check.cause.probe_output.1",
            "ssh_check.cause.probe_output.2",
            "ssh_check.cause.probe_output.3",
        ],
        SshFailKind::Other => &[
            "ssh_check.cause.other.1",
            "ssh_check.cause.other.2",
            "ssh_check.cause.other.3",
        ],
    }
}

/// Returns a human detail when SSH connectivity is not known-good.
pub(crate) fn ssh_connectivity_gap(ctx: &ExecutionContext) -> Option<String> {
    if ctx.kind != "ssh" {
        return None;
    }
    match ctx.last_probe_status.as_deref() {
        Some("ok") => None,
        Some("error") => Some(
            ctx.last_probe_error
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "probe failed".into()),
        ),
        _ => Some("not probed yet".into()),
    }
}

pub(crate) fn ssh_context_known_good(ctx: &ExecutionContext) -> bool {
    ssh_connectivity_gap(ctx).is_none()
}

/// Errors that need host configuration / Probe, not a blind retry.
pub(crate) fn is_ssh_setup_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("ssh connectivity is not confirmed")
        || lower.contains("ssh connectivity gate blocked")
        || lower.contains("identity file is not accessible")
        || lower.contains("no successful probe")
}

/// Prefer the active remote source; fall back to ``ssh:alias`` embedded in the error.
pub(crate) fn ssh_setup_context_id(preferred: Option<&str>, error: &str) -> Option<String> {
    if let Some(id) = preferred.filter(|id| id.starts_with("ssh:")) {
        return Some(id.to_string());
    }
    // Messages use backticks: `ssh:host-alias`
    let Some(start) = error.find("`ssh:") else {
        return None;
    };
    let rest = &error[start + 1..];
    let end = rest.find('`').unwrap_or(rest.len());
    let id = &rest[..end];
    id.starts_with("ssh:").then(|| id.to_string())
}
