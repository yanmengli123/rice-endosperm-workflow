use super::sessions::SESSION_IS_LISTABLE_SQL;
use super::Store;
use anyhow::Result;
use sqlx::Row;

/// Ephemeral scratch-chat projects use this id prefix and never appear in user-facing lists.
pub const SCRATCH_PROJECT_PREFIX: &str = "scratch:";

pub fn is_scratch_project_id(id: &str) -> bool {
    id.starts_with(SCRATCH_PROJECT_PREFIX)
}

impl Store {
    pub async fn create_project(&self, id: &str, name: &str, workspace_dir: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO projects(id,name,description,workspace_dir,created_at,updated_at) VALUES(?,?,'',?,?,?) \
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, workspace_dir=excluded.workspace_dir, updated_at=excluded.updated_at",
        )
        .bind(id).bind(name).bind(workspace_dir).bind(now).bind(now)
        .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_project(&self, id: &str) -> Result<Option<(String, String)>> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT COALESCE(name,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Full editable metadata for the Project Settings modal: (name, description, workspace_dir).
    pub async fn get_project_meta(&self, id: &str) -> Result<Option<(String, String, String)>> {
        let row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT COALESCE(name,''), COALESCE(description,''), COALESCE(workspace_dir,'') FROM projects WHERE id=?",
        )
        .bind(id).fetch_optional(&self.pool).await?;
        Ok(row)
    }

    /// Update a project's user-visible name and description (touches updated_at).
    pub async fn update_project(&self, id: &str, name: &str, description: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query("UPDATE projects SET name=?, description=?, updated_at=? WHERE id=?")
            .bind(name)
            .bind(description)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// All projects, newest-updated first, each with its session count
    /// (root frames with a user turn or an explicit title — matches
    /// `list_sessions_page`, #888) and artifact count.
    pub async fn list_projects(
        &self,
    ) -> Result<Vec<(String, String, String, i64, i64, i64, String, i64)>> {
        let sql = format!(
            "SELECT p.id AS id, COALESCE(p.name,'') AS name, COALESCE(p.workspace_dir,'') AS ws, \
                    p.created_at AS created_at, p.updated_at AS updated_at, \
                    COALESCE(p.description,'') AS description, \
                    (SELECT COUNT(*) FROM frames f WHERE f.project_id = p.id AND f.parent_frame_id = f.id \
                       AND f.exploration_id IS NULL \
                       AND {listable}) AS sessions, \
                    (SELECT COUNT(*) FROM artifacts a WHERE a.project_id = p.id \
                       AND a.exploration_id IS NULL) AS artifacts \
             FROM projects p \
             WHERE p.id NOT LIKE 'scratch:%' \
             ORDER BY p.updated_at DESC, p.rowid DESC",
            listable = SESSION_IS_LISTABLE_SQL,
        );
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        let mut out = vec![];
        for r in rows {
            out.push((
                r.try_get("id")?,
                r.try_get("name")?,
                r.try_get("ws")?,
                r.try_get("created_at")?,
                r.try_get("updated_at")?,
                r.try_get("sessions")?,
                r.try_get("description")?,
                r.try_get("artifacts")?,
            ));
        }
        Ok(out)
    }

    /// All scratch projects still in the database (e.g. after a crash before close).
    pub async fn list_scratch_projects(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query(
            "SELECT id, COALESCE(workspace_dir,'') AS ws FROM projects WHERE id LIKE 'scratch:%'",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|r| Ok((r.try_get("id")?, r.try_get("ws")?)))
            .collect()
    }

    /// Delete a project and everything under it. Explicit child deletes (SQLite
    /// FKs are OFF by default, so declared CASCADE would not fire). Filesystem
    /// is untouched — only DB rows.
    /// ponytail: explicit cascade of known child tables; switch to
    /// `PRAGMA foreign_keys=ON` if more child tables appear.
    pub async fn delete_project(&self, id: &str) -> Result<()> {
        let mut tx = self.begin_write().await?;
        super::project_transfer::delete_project_children(&mut tx, id).await?;
        sqlx::query("DELETE FROM project_sync_state WHERE project_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM projects WHERE id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
