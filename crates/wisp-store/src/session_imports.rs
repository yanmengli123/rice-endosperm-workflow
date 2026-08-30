//! Mapping between imported wisp session-export archives and Wisp frames.
//! Keyed by the exporting side's session id so re-imports are idempotent:
//! re-importing the same archive fast-forwards the existing frame instead of
//! creating a duplicate session. Mirrors `codex_imports`.

use super::Store;
use anyhow::Result;

impl Store {
    /// The frame a session archive was already imported into, if any.
    pub async fn find_session_import(&self, source_session_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT frame_id FROM session_imports WHERE source_session_id=?")
                .bind(source_session_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    /// Record (or refresh) the source session → frame mapping after an import.
    pub async fn record_session_import(
        &self,
        source_session_id: &str,
        frame_id: &str,
        source_path: &str,
    ) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        sqlx::query(
            "INSERT INTO session_imports(source_session_id,frame_id,source_path,created_at,updated_at) \
             VALUES(?,?,?,?,?) \
             ON CONFLICT(source_session_id) DO UPDATE SET \
             frame_id=excluded.frame_id, source_path=excluded.source_path, \
             updated_at=excluded.updated_at",
        )
        .bind(source_session_id)
        .bind(frame_id)
        .bind(source_path)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SESSION_IMPORTS_MIGRATION;

    async fn store_with_frame() -> (Store, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!(
            "wisp_store_session_imports_{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let store = Store::open(&path).await.unwrap();
        store
            .create_project("p", "Project", "/workspace")
            .await
            .unwrap();
        store.create_frame("f1", "p", "wisp", "m").await.unwrap();
        (store, path)
    }

    #[tokio::test]
    async fn session_import_round_trips_and_cascades_on_delete() {
        let (store, path) = store_with_frame().await;
        assert_eq!(store.find_session_import("src-1").await.unwrap(), None);
        store
            .record_session_import("src-1", "f1", "/tmp/wisp-session-src-1.zip")
            .await
            .unwrap();
        assert_eq!(
            store.find_session_import("src-1").await.unwrap(),
            Some("f1".to_string())
        );
        // Upsert keeps a single row per source session.
        store
            .record_session_import("src-1", "f1", "/tmp/other.zip")
            .await
            .unwrap();
        assert_eq!(
            store.find_session_import("src-1").await.unwrap(),
            Some("f1".to_string())
        );

        // Deleting the Wisp session frees the source id for re-import.
        store.delete_session("f1", "p").await.unwrap();
        assert_eq!(store.find_session_import("src-1").await.unwrap(), None);
        drop(store);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn session_imports_migration_is_idempotent() {
        let (store, path) = store_with_frame().await;
        store
            .record_session_import("src-1", "f1", "/tmp/wisp-session-src-1.zip")
            .await
            .unwrap();
        sqlx::query("DELETE FROM wisp_schema_migrations WHERE version=?")
            .bind(SESSION_IMPORTS_MIGRATION)
            .execute(&store.pool)
            .await
            .unwrap();
        drop(store);

        let reopened = Store::open(&path).await.unwrap();
        assert_eq!(
            reopened.find_session_import("src-1").await.unwrap(),
            Some("f1".to_string())
        );
        assert!(reopened
            .schema_migrations()
            .await
            .unwrap()
            .contains(&SESSION_IMPORTS_MIGRATION.to_string()));
        drop(reopened);
        let _ = std::fs::remove_file(path);
    }
}
