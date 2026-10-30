use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite, Row};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredUser {
    pub node_name: String,
    pub username: String,
    pub password: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSource {
    pub id: String,
    pub name: String,
    pub source_type: String, // "auto_detect", "clash_api", "clash_dir", "clash_file"
    pub target: String,      // URL or directory path or file path
    pub secret: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub is_base: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct Database {
    pool: Pool<Sqlite>,
}

impl Database {
    pub async fn init(database_url: &str) -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;

        // Enable WAL mode, busy timeout, and synchronous=NORMAL for concurrent performance
        sqlx::query("PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000; PRAGMA synchronous = NORMAL;")
            .execute(&pool)
            .await?;

        // Initialize tables
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS users (
                node_name TEXT PRIMARY KEY NOT NULL,
                username TEXT NOT NULL,
                password TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);

            CREATE TABLE IF NOT EXISTS probes (
                node_name TEXT PRIMARY KEY NOT NULL,
                result_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS node_traffic (
                node_name TEXT PRIMARY KEY NOT NULL,
                stats_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS node_health (
                node_name TEXT PRIMARY KEY NOT NULL,
                state_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS sources (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                source_type TEXT NOT NULL,
                target TEXT NOT NULL,
                secret TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                is_base INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
            );
            "#,
        )
        .execute(&pool)
        .await?;

        // Drop legacy deleted_sources blacklist table if it exists
        let _ = sqlx::query("DROP TABLE IF EXISTS deleted_sources")
            .execute(&pool)
            .await;

        // Migration: add is_base column to sources table if missing
        let _ = sqlx::query("ALTER TABLE sources ADD COLUMN is_base INTEGER NOT NULL DEFAULT 0")
            .execute(&pool)
            .await;

        Ok(Self { pool })
    }

    pub async fn save_setting(&self, key: &str, value: &str) -> Result<()> {
        sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("value")))
    }

    pub async fn get_all_users(&self) -> Result<Vec<StoredUser>> {
        let rows = sqlx::query("SELECT node_name, username, password, created_at FROM users ORDER BY node_name ASC")
            .fetch_all(&self.pool)
            .await?;

        let mut users = Vec::new();
        for row in rows {
            let created_at_str: String = row.get("created_at");
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            users.push(StoredUser {
                node_name: row.get("node_name"),
                username: row.get("username"),
                password: row.get("password"),
                created_at,
            });
        }

        Ok(users)
    }

    #[allow(dead_code)]
    pub async fn get_user(&self, node_name: &str) -> Result<Option<StoredUser>> {
        let row = sqlx::query("SELECT node_name, username, password, created_at FROM users WHERE node_name = ?")
            .bind(node_name)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let created_at_str: String = row.get("created_at");
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            Ok(Some(StoredUser {
                node_name: row.get("node_name"),
                username: row.get("username"),
                password: row.get("password"),
                created_at,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn save_user(&self, user: &StoredUser) -> Result<()> {
        let created_at_str = user.created_at.to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO users (node_name, username, password, created_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(node_name) DO UPDATE SET
                username = excluded.username,
                password = excluded.password,
                created_at = excluded.created_at
            "#,
        )
        .bind(&user.node_name)
        .bind(&user.username)
        .bind(&user.password)
        .bind(created_at_str)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn delete_user(&self, node_name: &str) -> Result<()> {
        sqlx::query("DELETE FROM users WHERE node_name = ?")
            .bind(node_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn prune_users_except(&self, keep: &[String]) -> Result<u64> {
        let all = self.get_all_users().await?;
        let keep_set: std::collections::HashSet<&String> = keep.iter().collect();
        let to_remove: Vec<String> = all
            .into_iter()
            .filter(|u| !keep_set.contains(&u.node_name))
            .map(|u| u.node_name)
            .collect();
        let removed = to_remove.len() as u64;
        if !to_remove.is_empty() {
            let mut tx = self.pool.begin().await?;
            for name in to_remove {
                sqlx::query("DELETE FROM users WHERE node_name = ?")
                    .bind(name)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Ok(removed)
    }

    #[allow(dead_code)]
    pub async fn get_all_sources(&self) -> Result<Vec<StoredSource>> {
        let rows = sqlx::query("SELECT id, name, source_type, target, secret, enabled, is_base, created_at FROM sources ORDER BY created_at ASC")
            .fetch_all(&self.pool)
            .await?;

        let mut sources = Vec::new();
        for row in rows {
            let created_at_str: String = row.get("created_at");
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let enabled_int: i64 = row.get("enabled");
            let is_base_int: i64 = row.try_get("is_base").unwrap_or(0);

            sources.push(StoredSource {
                id: row.get("id"),
                name: row.get("name"),
                source_type: row.get("source_type"),
                target: row.get("target"),
                secret: row.get("secret"),
                enabled: enabled_int == 1,
                is_base: is_base_int == 1,
                created_at,
            });
        }

        Ok(sources)
    }

    pub async fn save_source(&self, source: &StoredSource) -> Result<()> {
        let enabled_int: i64 = if source.enabled { 1 } else { 0 };
        let is_base_int: i64 = if source.is_base { 1 } else { 0 };
        let created_at_str = source.created_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO sources (id, name, source_type, target, secret, enabled, is_base, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                source_type = excluded.source_type,
                target = excluded.target,
                secret = excluded.secret,
                enabled = excluded.enabled,
                is_base = excluded.is_base,
                created_at = excluded.created_at
            "#,
        )
        .bind(&source.id)
        .bind(&source.name)
        .bind(&source.source_type)
        .bind(&source.target)
        .bind(&source.secret)
        .bind(enabled_int)
        .bind(is_base_int)
        .bind(created_at_str)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn toggle_source(&self, id: &str, enabled: bool) -> Result<()> {
        let enabled_int: i64 = if enabled { 1 } else { 0 };
        sqlx::query("UPDATE sources SET enabled = ? WHERE id = ?")
            .bind(enabled_int)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn toggle_source_base(&self, id: &str) -> Result<bool> {
        let source = self.get_source(id).await?
            .ok_or_else(|| crate::error::AppError::Config(format!("Source '{}' not found", id)))?;
        let new_is_base = !source.is_base;
        let is_base_int: i64 = if new_is_base { 1 } else { 0 };
        sqlx::query("UPDATE sources SET is_base = ? WHERE id = ?")
            .bind(is_base_int)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(new_is_base)
    }

    pub async fn delete_source(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sources WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_source(&self, id: &str) -> Result<Option<StoredSource>> {
        let row = sqlx::query("SELECT id, name, source_type, target, secret, enabled, is_base, created_at FROM sources WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let created_at_str: String = row.get("created_at");
            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            let enabled_int: i64 = row.get("enabled");
            let is_base_int: i64 = row.try_get("is_base").unwrap_or(0);

            Ok(Some(StoredSource {
                id: row.get("id"),
                name: row.get("name"),
                source_type: row.get("source_type"),
                target: row.get("target"),
                secret: row.get("secret"),
                enabled: enabled_int == 1,
                is_base: is_base_int == 1,
                created_at,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn update_source(
        &self,
        id: &str,
        name: Option<&str>,
        target: Option<&str>,
        secret: Option<Option<&str>>,
        enabled: Option<bool>,
        is_base: Option<bool>,
    ) -> Result<Option<StoredSource>> {
        let existing = self.get_source(id).await?;
        if let Some(mut src) = existing {
            if let Some(n) = name {
                src.name = n.to_string();
            }
            if let Some(t) = target {
                src.target = t.to_string();
            }
            if let Some(s) = secret {
                src.secret = s.map(|x| x.to_string());
            }
            if let Some(e) = enabled {
                src.enabled = e;
            }
            if let Some(b) = is_base {
                src.is_base = b;
            }
            self.save_source(&src).await?;
            Ok(Some(src))
        } else {
            Ok(None)
        }
    }

    // ==================== Node health persistence ====================

    #[allow(dead_code)] // superseded by save_health_many (bulk transaction)
    pub async fn save_health(&self, node_name: &str, state_json: &str) -> Result<()> {
        let now_str = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO node_health (node_name, state_json, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(node_name) DO UPDATE SET
                state_json = excluded.state_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(node_name)
        .bind(state_json)
        .bind(now_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Bulk upsert: one transaction instead of one round trip per node.
    /// A rolling health batch touches hundreds of records, and the per-row
    /// version dominated startup CPU.
    pub async fn save_health_many(&self, rows: &[(String, String)]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let now_str = Utc::now().to_rfc3339();
        let mut tx = self.pool.begin().await?;
        for (node_name, state_json) in rows {
            sqlx::query(
                r#"
                INSERT INTO node_health (node_name, state_json, updated_at)
                VALUES (?, ?, ?)
                ON CONFLICT(node_name) DO UPDATE SET
                    state_json = excluded.state_json,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(node_name)
            .bind(state_json)
            .bind(&now_str)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn load_all_health(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT node_name, state_json FROM node_health")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("node_name"), r.get("state_json")))
            .collect())
    }

    /// Remove persisted health rows for nodes that no longer exist.
    pub async fn prune_health_except(&self, keep: &[String]) -> Result<()> {
        let rows = sqlx::query("SELECT node_name FROM node_health").fetch_all(&self.pool).await?;
        let keep_set: std::collections::HashSet<&String> = keep.iter().collect();
        let to_remove: Vec<String> = rows
            .into_iter()
            .map(|r| r.get::<String, _>("node_name"))
            .filter(|name| !keep_set.contains(name))
            .collect();
        if !to_remove.is_empty() {
            let mut tx = self.pool.begin().await?;
            for name in to_remove {
                sqlx::query("DELETE FROM node_health WHERE node_name = ?")
                    .bind(name)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }

    // ==================== Node traffic persistence ====================

    pub async fn save_traffic_stats(&self, node_name: &str, stats_json: &str) -> Result<()> {
        let now_str = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO node_traffic (node_name, stats_json, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(node_name) DO UPDATE SET
                stats_json = excluded.stats_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(node_name)
        .bind(stats_json)
        .bind(now_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_all_traffic_stats(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT node_name, stats_json FROM node_traffic")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("node_name"), r.get("stats_json")))
            .collect())
    }

    // ==================== Probe history persistence ====================

    pub async fn save_probe(&self, node_name: &str, result_json: &str) -> Result<()> {
        let now_str = Utc::now().to_rfc3339();
        sqlx::query(
            r#"
            INSERT INTO probes (node_name, result_json, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(node_name) DO UPDATE SET
                result_json = excluded.result_json,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(node_name)
        .bind(result_json)
        .bind(now_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Loads all persisted probe results: (node_name, result_json)
    pub async fn load_all_probes(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT node_name, result_json FROM probes")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("node_name"), r.get("result_json")))
            .collect())
    }

    #[allow(dead_code)]
    pub async fn delete_probe(&self, node_name: &str) -> Result<()> {
        sqlx::query("DELETE FROM probes WHERE node_name = ?")
            .bind(node_name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove persisted probes for nodes not in the keep list.
    /// Preserves synthetic region/IP/level-group probes if included in keep list.
    pub async fn prune_probes_except(&self, keep: &[String]) -> Result<()> {
        let rows = sqlx::query("SELECT node_name FROM probes").fetch_all(&self.pool).await?;
        let keep_set: std::collections::HashSet<&String> = keep.iter().collect();
        let to_remove: Vec<String> = rows
            .into_iter()
            .map(|r| r.get::<String, _>("node_name"))
            .filter(|name| !keep_set.contains(name))
            .collect();
        if !to_remove.is_empty() {
            let mut tx = self.pool.begin().await?;
            for name in to_remove {
                sqlx::query("DELETE FROM probes WHERE node_name = ?")
                    .bind(name)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        Ok(())
    }

}
