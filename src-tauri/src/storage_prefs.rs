//! Effective storage locations for a (project × execution context): stored
//! preferences when the user confirmed them, deterministic defaults otherwise.

use crate::exploration_commands;
use crate::AppState;
use serde::Serialize;
use tauri::State;
use wisp_store::ContextStoragePrefs;

pub(crate) const DEFAULT_REMOTE_WORKDIR_ROOT: &str = ".wisp-science/runs";

fn slug(value: &str) -> String {
    let sanitized: String = value
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let collapsed = sanitized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if collapsed.is_empty() {
        "project".into()
    } else {
        collapsed
    }
}

pub(crate) fn default_prefs(
    project_id: &str,
    project_name: &str,
    context_id: &str,
    context_label: &str,
) -> ContextStoragePrefs {
    let now = chrono::Utc::now().timestamp();
    ContextStoragePrefs {
        project_id: project_id.into(),
        context_id: context_id.into(),
        remote_data_root: format!("~/wisp/{}/data", slug(project_name)),
        remote_workdir_root: DEFAULT_REMOTE_WORKDIR_ROOT.into(),
        local_results_dir: format!("remote/{}", slug(context_label)),
        created_at: now,
        updated_at: now,
    }
}

/// Stored preferences when present, deterministic defaults otherwise. The
/// second value reports whether the user has confirmed (persisted) them.
pub(crate) async fn effective_prefs(
    store: &wisp_store::Store,
    project_id: &str,
    context_id: &str,
) -> Result<(ContextStoragePrefs, bool), String> {
    if let Some(prefs) = store
        .get_context_storage_prefs(project_id, context_id)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok((prefs, true));
    }
    let project_name = store
        .get_project(project_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|(name, _)| name)
        .unwrap_or_default();
    let context_label = store
        .get_execution_context(context_id)
        .await
        .map_err(|e| e.to_string())?
        .map(|context| context.label)
        .unwrap_or_else(|| context_id.to_string());
    Ok((
        default_prefs(project_id, &project_name, context_id, &context_label),
        false,
    ))
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ContextStoragePrefsView {
    pub context_id: String,
    pub remote_data_root: String,
    pub remote_workdir_root: String,
    pub local_results_dir: String,
    /// False while the user has never confirmed locations for this
    /// project × context; the UI prompts once on first enable.
    pub confirmed: bool,
}

#[tauri::command]
pub(crate) async fn get_context_storage_prefs(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
) -> Result<ContextStoragePrefsView, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let (prefs, confirmed) = effective_prefs(&state.store, &ap.id, &context_id).await?;
    Ok(ContextStoragePrefsView {
        context_id,
        remote_data_root: prefs.remote_data_root,
        remote_workdir_root: prefs.remote_workdir_root,
        local_results_dir: prefs.local_results_dir,
        confirmed,
    })
}

#[tauri::command]
pub(crate) async fn set_context_storage_prefs(
    state: State<'_, AppState>,
    window: tauri::WebviewWindow,
    context_id: String,
    remote_data_root: String,
    remote_workdir_root: String,
    local_results_dir: String,
) -> Result<ContextStoragePrefsView, String> {
    let (ap, _) =
        exploration_commands::working_project_for_active_frame(&state, window.label()).await?;
    let prefs = ContextStoragePrefs {
        project_id: ap.id.clone(),
        context_id: context_id.clone(),
        remote_data_root: remote_data_root.trim().to_string(),
        remote_workdir_root: remote_workdir_root.trim().to_string(),
        local_results_dir: local_results_dir
            .trim()
            .trim_end_matches('/')
            .replace('\\', "/"),
        created_at: 0,
        updated_at: 0,
    };
    state
        .store
        .upsert_context_storage_prefs(&prefs)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ContextStoragePrefsView {
        context_id,
        remote_data_root: prefs.remote_data_root,
        remote_workdir_root: prefs.remote_workdir_root,
        local_results_dir: prefs.local_results_dir,
        confirmed: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_collapse_to_safe_lowercase_components() {
        assert_eq!(slug("My T-cell Atlas!"), "my-t-cell-atlas");
        assert_eq!(slug("  "), "project");
        assert_eq!(slug("GPU box #2"), "gpu-box-2");
    }

    #[test]
    fn defaults_follow_project_and_context_names() {
        let prefs = default_prefs("p", "T-cell Atlas", "ssh:gpu", "GPU Box");
        assert_eq!(prefs.remote_data_root, "~/wisp/t-cell-atlas/data");
        assert_eq!(prefs.remote_workdir_root, DEFAULT_REMOTE_WORKDIR_ROOT);
        assert_eq!(prefs.local_results_dir, "remote/gpu-box");
        assert!(prefs.validate().is_ok());
    }
}
