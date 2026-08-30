//! Best-effort retention for spill files and subagent traces under `.wisp/`.
//! Drops entries older than `max_age_days` and trims total size to `max_bytes`.

use std::path::Path;
use std::time::{Duration, SystemTime};

/// Default retention for explore traces and tool-output spills.
pub const DEFAULT_MAX_AGE_DAYS: u64 = 7;
/// Default total byte budget per archive directory.
pub const DEFAULT_MAX_DIR_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct ArchiveRetention {
    pub max_age_days: u64,
    pub max_bytes: u64,
}

impl Default for ArchiveRetention {
    fn default() -> Self {
        Self {
            max_age_days: DEFAULT_MAX_AGE_DAYS,
            max_bytes: DEFAULT_MAX_DIR_BYTES,
        }
    }
}

#[derive(Debug)]
struct ArchiveEntry {
    path: std::path::PathBuf,
    modified: SystemTime,
    size: u64,
}

/// Delete files in `dir` older than the retention window, then trim oldest
/// files until total size is under the budget. Best-effort: fs errors are ignored.
pub fn prune_dir(dir: &Path, retention: ArchiveRetention) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(retention.max_age_days * 86_400))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if modified < cutoff {
            std::fs::remove_file(&path).ok();
            continue;
        }
        files.push(ArchiveEntry {
            path,
            modified,
            size: meta.len(),
        });
    }
    let mut total: u64 = files.iter().map(|f| f.size).sum();
    if total <= retention.max_bytes {
        return;
    }
    files.sort_by_key(|f| f.modified);
    for file in files {
        if total <= retention.max_bytes {
            break;
        }
        if std::fs::remove_file(&file.path).is_ok() {
            total = total.saturating_sub(file.size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_enforces_size_cap() {
        let dir = std::env::temp_dir().join(format!("wisp-archive-prune-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("part-{i}.txt")), "z".repeat(2_000)).unwrap();
        }

        prune_dir(
            &dir,
            ArchiveRetention {
                max_age_days: 7,
                max_bytes: 5_000,
            },
        );

        let total: u64 = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum();
        assert!(total <= 5_000);
        std::fs::remove_dir_all(&dir).ok();
    }
}
