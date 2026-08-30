use super::{execution_context_from_row, ExecutionContext, ExecutionContextKind, Store};
use anyhow::Result;

impl Store {
    pub async fn upsert_execution_context(&self, ctx: &ExecutionContext) -> Result<()> {
        ctx.validate()?;
        sqlx::query(
            "INSERT INTO execution_contexts(\
                id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at\
             ) VALUES(?,?,?,?,?,?,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET \
                kind=excluded.kind, label=excluded.label, config_json=excluded.config_json, \
                capabilities_json=excluded.capabilities_json, last_probe_at=excluded.last_probe_at, \
                last_probe_status=excluded.last_probe_status, last_probe_error=excluded.last_probe_error, \
                updated_at=excluded.updated_at",
        )
        .bind(&ctx.id)
        .bind(ctx.kind.as_str())
        .bind(&ctx.label)
        .bind(&ctx.config_json)
        .bind(&ctx.capabilities_json)
        .bind(ctx.last_probe_at)
        .bind(ctx.last_probe_status.as_deref())
        .bind(ctx.last_probe_error.as_deref())
        .bind(ctx.created_at)
        .bind(ctx.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_execution_context(&self, id: &str) -> Result<Option<ExecutionContext>> {
        ExecutionContextKind::from_id(id)?;
        let row = sqlx::query(
            "SELECT id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at \
             FROM execution_contexts WHERE id=?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(execution_context_from_row).transpose()
    }

    pub async fn list_execution_contexts(&self) -> Result<Vec<ExecutionContext>> {
        let rows = sqlx::query(
            "SELECT id,kind,label,config_json,capabilities_json,last_probe_at,last_probe_status,last_probe_error,created_at,updated_at \
             FROM execution_contexts ORDER BY CASE id WHEN 'local' THEN 0 ELSE 1 END, id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(execution_context_from_row).collect()
    }

    pub async fn delete_execution_context(&self, id: &str) -> Result<()> {
        ExecutionContextKind::from_id(id)?;
        sqlx::query("DELETE FROM session_execution_contexts WHERE context_id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM execution_contexts WHERE id=?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_session_execution_context_enabled(
        &self,
        frame_id: &str,
        context_id: &str,
        enabled: bool,
    ) -> Result<()> {
        let context = self
            .get_execution_context(context_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Execution context not found: {context_id}"))?;
        if context.kind == ExecutionContextKind::Local {
            anyhow::bail!("Local compute is always available");
        }
        if self.frame_project_id(frame_id).await?.is_none() {
            anyhow::bail!("Session not found: {frame_id}");
        }
        if enabled {
            sqlx::query(
                "INSERT OR IGNORE INTO session_execution_contexts(frame_id,context_id,created_at) \
                 VALUES(?,?,?)",
            )
            .bind(frame_id)
            .bind(context_id)
            .bind(chrono::Utc::now().timestamp())
            .execute(&self.pool)
            .await?;
        } else {
            sqlx::query("DELETE FROM session_execution_contexts WHERE frame_id=? AND context_id=?")
                .bind(frame_id)
                .bind(context_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn list_session_execution_context_ids(&self, frame_id: &str) -> Result<Vec<String>> {
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT context_id FROM session_execution_contexts \
             WHERE frame_id=? ORDER BY context_id",
        )
        .bind(frame_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|(id,)| id).collect())
    }

    pub async fn session_execution_context_enabled(
        &self,
        frame_id: &str,
        context_id: &str,
    ) -> Result<bool> {
        let row: (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM session_execution_contexts \
             WHERE frame_id=? AND context_id=?)",
        )
        .bind(frame_id)
        .bind(context_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0 != 0)
    }
}
