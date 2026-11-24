use chrono::Utc;
use noctra_types::{
    BookmarkEntry, BrowserProfile, BrowserSettings, HistoryEntry, NoctraError, NoctraResult,
    SessionSnapshot, Workspace,
};
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::path::Path;
use uuid::Uuid;

pub struct NoctraDatabase {
    conn: Mutex<Connection>,
}

impl NoctraDatabase {
    pub fn open(path: impl AsRef<Path>) -> NoctraResult<Self> {
        let conn = Connection::open(path)
            .map_err(|e| NoctraError::DatabaseError(format!("SQLite open error: {}", e)))?;

        // Enable WAL mode and synchronous=NORMAL for blazing-fast low-spec disk performance
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA cache_size = -8000;
             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| NoctraError::DatabaseError(format!("PRAGMA setup failed: {}", e)))?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    pub fn open_in_memory() -> NoctraResult<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| NoctraError::DatabaseError(format!("SQLite memory error: {}", e)))?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> NoctraResult<()> {
        let conn = self.conn.lock();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS kv_store (
                namespace TEXT NOT NULL,
                key TEXT NOT NULL,
                json_value TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (namespace, key)
            );

            CREATE TABLE IF NOT EXISTS bookmarks (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                workspace_id TEXT,
                title TEXT NOT NULL,
                url TEXT NOT NULL,
                folder TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_bookmarks_profile ON bookmarks(profile_id);

            CREATE TABLE IF NOT EXISTS browsing_history (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                workspace_id TEXT NOT NULL,
                title TEXT NOT NULL,
                url TEXT NOT NULL,
                visited_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_history_profile ON browsing_history(profile_id, visited_at DESC);

            CREATE TABLE IF NOT EXISTS session_snapshots (
                id TEXT PRIMARY KEY,
                profile_id TEXT NOT NULL,
                workspace_id TEXT,
                label TEXT NOT NULL,
                json_payload TEXT NOT NULL,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS profile_storage_kv (
                profile_id TEXT NOT NULL,
                partition_id TEXT NOT NULL,
                origin TEXT NOT NULL,
                storage_key TEXT NOT NULL,
                storage_value TEXT NOT NULL,
                PRIMARY KEY (profile_id, partition_id, origin, storage_key)
            );",
        )
        .map_err(|e| NoctraError::DatabaseError(format!("Migration failed: {}", e)))?;
        Ok(())
    }

    pub fn save_kv<T: serde::Serialize>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
    ) -> NoctraResult<()> {
        let json = serde_json::to_string(value)
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_store (namespace, key, json_value, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(namespace, key) DO UPDATE SET json_value = excluded.json_value, updated_at = excluded.updated_at",
            params![namespace, key, json, Utc::now().to_rfc3339()],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn load_kv<T: serde::de::DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> NoctraResult<Option<T>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT json_value FROM kv_store WHERE namespace = ?1 AND key = ?2")
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let mut rows = stmt
            .query(params![namespace, key])
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?
        {
            let json: String = row
                .get(0)
                .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
            let parsed = serde_json::from_str(&json)
                .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
            Ok(Some(parsed))
        } else {
            Ok(None)
        }
    }

    pub fn save_settings(&self, settings: &BrowserSettings) -> NoctraResult<()> {
        self.save_kv("global", "browser_settings", settings)
    }

    pub fn load_settings(&self) -> NoctraResult<BrowserSettings> {
        Ok(self
            .load_kv("global", "browser_settings")?
            .unwrap_or_default())
    }

    pub fn save_profiles(&self, profiles: &[BrowserProfile]) -> NoctraResult<()> {
        self.save_kv("profiles", "all", &profiles)
    }

    pub fn load_profiles(&self) -> NoctraResult<Vec<BrowserProfile>> {
        Ok(self.load_kv("profiles", "all")?.unwrap_or_default())
    }

    pub fn save_workspaces(&self, workspaces: &[Workspace]) -> NoctraResult<()> {
        self.save_kv("workspaces", "all", &workspaces)
    }

    pub fn load_workspaces(&self) -> NoctraResult<Vec<Workspace>> {
        Ok(self.load_kv("workspaces", "all")?.unwrap_or_default())
    }

    pub fn add_bookmark(&self, entry: &BookmarkEntry) -> NoctraResult<()> {
        let tags_json = serde_json::to_string(&entry.tags)
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO bookmarks (id, profile_id, workspace_id, title, url, folder, tags_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id.to_string(),
                entry.profile_id.to_string(),
                entry.workspace_id.map(|id| id.to_string()),
                entry.title,
                entry.url,
                entry.folder,
                tags_json,
                entry.created_at.to_rfc3339()
            ],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn list_bookmarks(&self, profile_id: Uuid) -> NoctraResult<Vec<BookmarkEntry>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, profile_id, workspace_id, title, url, folder, tags_json, created_at
                 FROM bookmarks WHERE profile_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let rows = stmt
            .query_map(params![profile_id.to_string()], |row| {
                let id_str: String = row.get(0)?;
                let prof_str: String = row.get(1)?;
                let ws_str: Option<String> = row.get(2)?;
                let title: String = row.get(3)?;
                let url: String = row.get(4)?;
                let folder: String = row.get(5)?;
                let tags_json: String = row.get(6)?;
                let created_str: String = row.get(7)?;
                Ok(BookmarkEntry {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    profile_id: Uuid::parse_str(&prof_str).unwrap_or_default(),
                    workspace_id: ws_str.and_then(|s| Uuid::parse_str(&s).ok()),
                    title,
                    url,
                    folder,
                    tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| NoctraError::DatabaseError(e.to_string()))?);
        }
        Ok(list)
    }

    pub fn delete_bookmark(&self, id: Uuid, profile_id: Uuid) -> NoctraResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM bookmarks WHERE id = ?1 AND profile_id = ?2",
            params![id.to_string(), profile_id.to_string()],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn record_history(&self, entry: &HistoryEntry) -> NoctraResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO browsing_history (id, profile_id, workspace_id, title, url, visited_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.id.to_string(),
                entry.profile_id.to_string(),
                entry.workspace_id.to_string(),
                entry.title,
                entry.url,
                entry.visited_at.to_rfc3339()
            ],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn list_history(&self, profile_id: Uuid, limit: usize) -> NoctraResult<Vec<HistoryEntry>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT id, profile_id, workspace_id, title, url, visited_at
                 FROM browsing_history WHERE profile_id = ?1 ORDER BY visited_at DESC LIMIT ?2",
            )
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let rows = stmt
            .query_map(params![profile_id.to_string(), limit as i64], |row| {
                let id_str: String = row.get(0)?;
                let prof_str: String = row.get(1)?;
                let ws_str: String = row.get(2)?;
                let title: String = row.get(3)?;
                let url: String = row.get(4)?;
                let visited_str: String = row.get(5)?;
                Ok(HistoryEntry {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    profile_id: Uuid::parse_str(&prof_str).unwrap_or_default(),
                    workspace_id: Uuid::parse_str(&ws_str).unwrap_or_default(),
                    title,
                    url,
                    visited_at: chrono::DateTime::parse_from_rfc3339(&visited_str)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r.map_err(|e| NoctraError::DatabaseError(e.to_string()))?);
        }
        Ok(list)
    }

    pub fn clear_profile_history(&self, profile_id: Uuid) -> NoctraResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM browsing_history WHERE profile_id = ?1",
            params![profile_id.to_string()],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    /// Strict Profile & Partition Storage Isolation (Section 6 & 8)
    pub fn set_partition_storage(
        &self,
        profile_id: Uuid,
        partition_id: &str,
        origin: &str,
        key: &str,
        value: &str,
    ) -> NoctraResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO profile_storage_kv (profile_id, partition_id, origin, storage_key, storage_value)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(profile_id, partition_id, origin, storage_key)
             DO UPDATE SET storage_value = excluded.storage_value",
            params![profile_id.to_string(), partition_id, origin, key, value],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn get_partition_storage(
        &self,
        profile_id: Uuid,
        partition_id: &str,
        origin: &str,
        key: &str,
    ) -> NoctraResult<Option<String>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT storage_value FROM profile_storage_kv
                 WHERE profile_id = ?1 AND partition_id = ?2 AND origin = ?3 AND storage_key = ?4",
            )
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let mut rows = stmt
            .query(params![profile_id.to_string(), partition_id, origin, key])
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?
        {
            Ok(Some(
                row.get(0)
                    .map_err(|e| NoctraError::DatabaseError(e.to_string()))?,
            ))
        } else {
            Ok(None)
        }
    }

    pub fn save_snapshot(&self, snapshot: &SessionSnapshot) -> NoctraResult<()> {
        let json = serde_json::to_string(snapshot)
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO session_snapshots (id, profile_id, workspace_id, label, json_payload, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.id.to_string(),
                snapshot.profile_id.to_string(),
                snapshot.workspace_id.map(|id| id.to_string()),
                snapshot.label,
                json,
                snapshot.created_at.to_rfc3339()
            ],
        )
        .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        Ok(())
    }

    pub fn list_snapshots(&self, profile_id: Uuid) -> NoctraResult<Vec<SessionSnapshot>> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT json_payload FROM session_snapshots WHERE profile_id = ?1 ORDER BY created_at DESC",
            )
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
        let rows = stmt
            .query_map(params![profile_id.to_string()], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            })
            .map_err(|e| NoctraError::DatabaseError(e.to_string()))?;

        let mut list = Vec::new();
        for r in rows {
            let json = r.map_err(|e| NoctraError::DatabaseError(e.to_string()))?;
            if let Ok(snap) = serde_json::from_str::<SessionSnapshot>(&json) {
                list.push(snap);
            }
        }
        Ok(list)
    }
}
