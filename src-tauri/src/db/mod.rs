//! SQLite persistence.
//!
//! The whole database is a handful of small tables, so a single connection
//! behind a mutex is cheaper than a pool — every statement here runs in
//! microseconds and never blocks on the network.

pub mod models;

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::AppResult;
use models::*;

const SCHEMA_VERSION: i32 = 4;

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // WAL keeps the UI readable while the update watcher writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> AppResult<()> {
        let conn = self.conn.lock();
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS torrents (
                info_hash     TEXT PRIMARY KEY,
                name          TEXT NOT NULL,
                output_folder TEXT NOT NULL,
                total_bytes   INTEGER NOT NULL DEFAULT 0,
                added_at      INTEGER NOT NULL,
                completed_at  INTEGER,
                source        TEXT NOT NULL DEFAULT 'file',
                topic_id      INTEGER,
                torrent_file  BLOB
            );
            CREATE INDEX IF NOT EXISTS idx_torrents_topic ON torrents(topic_id);

            CREATE TABLE IF NOT EXISTS library_items (
                id             INTEGER PRIMARY KEY,
                info_hash      TEXT NOT NULL UNIQUE
                                 REFERENCES torrents(info_hash) ON DELETE CASCADE,
                title          TEXT NOT NULL,
                cover_path     TEXT,
                hero_path      TEXT,
                exe_path       TEXT,
                install_dir    TEXT,
                category       TEXT NOT NULL DEFAULT 'game',
                last_played_at INTEGER,
                play_seconds   INTEGER NOT NULL DEFAULT 0,
                favorite       INTEGER NOT NULL DEFAULT 0,
                hidden         INTEGER NOT NULL DEFAULT 0,
                notes          TEXT
            );

            CREATE TABLE IF NOT EXISTS tracked_topics (
                topic_id        INTEGER PRIMARY KEY,
                tracker         TEXT NOT NULL DEFAULT 'rutracker',
                info_hash       TEXT NOT NULL,
                title           TEXT,
                size_bytes      INTEGER,
                reg_time        INTEGER,
                last_checked_at INTEGER,
                enabled         INTEGER NOT NULL DEFAULT 1
            );

            CREATE TABLE IF NOT EXISTS topic_updates (
                id             INTEGER PRIMARY KEY,
                topic_id       INTEGER NOT NULL,
                old_info_hash  TEXT NOT NULL,
                new_info_hash  TEXT NOT NULL,
                old_size_bytes INTEGER,
                new_size_bytes INTEGER,
                new_reg_time   INTEGER,
                detected_at    INTEGER NOT NULL,
                status         TEXT NOT NULL DEFAULT 'pending',
                UNIQUE(topic_id, new_info_hash)
            );
            CREATE INDEX IF NOT EXISTS idx_updates_status ON topic_updates(status);

            -- Preview artwork for search results. `image_url` stays NULL when a
            -- topic genuinely has no usable image, so a miss is remembered and
            -- the topic page is not re-fetched on every scroll.
            CREATE TABLE IF NOT EXISTS topic_previews (
                topic_id   INTEGER PRIMARY KEY,
                image_url  TEXT,
                fetched_at INTEGER NOT NULL
            );

            -- Releases the user marked to get later. Deliberately not tied to
            -- `torrents`: nothing has been downloaded yet, and the whole point
            -- is to keep the plan around before any bytes exist.
            CREATE TABLE IF NOT EXISTS wishlist (
                topic_id   INTEGER PRIMARY KEY,
                title      TEXT NOT NULL,
                image_url  TEXT,
                size_bytes INTEGER,
                category   TEXT NOT NULL DEFAULT 'movie',
                added_at   INTEGER NOT NULL
            );

            -- What has been watched, including releases streamed without being
            -- kept. Those leave no torrent behind, so without this row there
            -- would be no trace of them at all.
            CREATE TABLE IF NOT EXISTS watch_history (
                id         INTEGER PRIMARY KEY,
                topic_id   INTEGER,
                info_hash  TEXT,
                title      TEXT NOT NULL,
                file_name  TEXT,
                image_url  TEXT,
                watched_at INTEGER NOT NULL,
                -- 1 when it was a "just watch it" stream that gets deleted.
                temporary  INTEGER NOT NULL DEFAULT 0,
                UNIQUE(topic_id, file_name)
            );
            CREATE INDEX IF NOT EXISTS idx_history_time ON watch_history(watched_at DESC);

            CREATE TABLE IF NOT EXISTS kv (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    // ---------------------------------------------------------------- kv

    pub fn kv_get(&self, key: &str) -> AppResult<Option<String>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv(key, value) VALUES(?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn kv_delete(&self, key: &str) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM kv WHERE key = ?1", params![key])?;
        Ok(())
    }

    // ---------------------------------------------------------- torrents

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_torrent(
        &self,
        info_hash: &str,
        name: &str,
        output_folder: &str,
        total_bytes: i64,
        source: TorrentSource,
        topic_id: Option<i64>,
        torrent_file: Option<&[u8]>,
    ) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO torrents(info_hash, name, output_folder, total_bytes, added_at, source, topic_id, torrent_file)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(info_hash) DO UPDATE SET
                name          = excluded.name,
                output_folder = excluded.output_folder,
                total_bytes   = MAX(excluded.total_bytes, torrents.total_bytes),
                source        = excluded.source,
                topic_id      = COALESCE(excluded.topic_id, torrents.topic_id),
                torrent_file  = COALESCE(excluded.torrent_file, torrents.torrent_file)",
            params![
                info_hash,
                name,
                output_folder,
                total_bytes,
                now(),
                source.as_str(),
                topic_id,
                torrent_file
            ],
        )?;
        Ok(())
    }

    pub fn get_torrent(&self, info_hash: &str) -> AppResult<Option<TorrentRecord>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT info_hash, name, output_folder, total_bytes, added_at, completed_at, source, topic_id
                 FROM torrents WHERE info_hash = ?1",
                params![info_hash],
                map_torrent,
            )
            .optional()?)
    }

    pub fn get_torrent_file(&self, info_hash: &str) -> AppResult<Option<Vec<u8>>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT torrent_file FROM torrents WHERE info_hash = ?1",
                params![info_hash],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?
            .flatten())
    }

    pub fn list_torrents(&self) -> AppResult<Vec<TorrentRecord>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT info_hash, name, output_folder, total_bytes, added_at, completed_at, source, topic_id
             FROM torrents ORDER BY added_at DESC",
        )?;
        let rows = stmt.query_map([], map_torrent)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn mark_torrent_completed(&self, info_hash: &str) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE torrents SET completed_at = COALESCE(completed_at, ?2) WHERE info_hash = ?1",
            params![info_hash, now()],
        )?;
        Ok(())
    }

    pub fn delete_torrent(&self, info_hash: &str) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM torrents WHERE info_hash = ?1",
            params![info_hash],
        )?;
        Ok(())
    }

    // ----------------------------------------------------------- library

    pub fn upsert_library_item(
        &self,
        info_hash: &str,
        title: &str,
        install_dir: Option<&str>,
        category: &str,
    ) -> AppResult<i64> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO library_items(info_hash, title, install_dir, category)
             VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(info_hash) DO UPDATE SET
                title       = excluded.title,
                install_dir = COALESCE(excluded.install_dir, library_items.install_dir)",
            params![info_hash, title, install_dir, category],
        )?;
        Ok(conn.query_row(
            "SELECT id FROM library_items WHERE info_hash = ?1",
            params![info_hash],
            |r| r.get(0),
        )?)
    }

    pub fn list_library(&self, include_hidden: bool) -> AppResult<Vec<LibraryItem>> {
        let conn = self.conn.lock();
        // The correlated EXISTS carries the pending-update flag so the grid can
        // badge a card without a per-item round trip.
        let sql = format!(
            "SELECT l.id, l.info_hash, l.title, l.cover_path, l.hero_path, l.exe_path,
                    l.install_dir, l.category, l.last_played_at, l.play_seconds,
                    l.favorite, l.hidden, t.topic_id,
                    EXISTS(SELECT 1 FROM topic_updates u
                           WHERE u.topic_id = t.topic_id AND u.status = 'pending') AS pending
             FROM library_items l
             JOIN torrents t ON t.info_hash = l.info_hash
             {}
             ORDER BY l.favorite DESC, l.last_played_at DESC, l.title COLLATE NOCASE",
            if include_hidden { "" } else { "WHERE l.hidden = 0" }
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            Ok(LibraryItem {
                id: r.get(0)?,
                info_hash: r.get(1)?,
                title: r.get(2)?,
                cover_path: r.get(3)?,
                hero_path: r.get(4)?,
                exe_path: r.get(5)?,
                install_dir: r.get(6)?,
                category: r.get(7)?,
                last_played_at: r.get(8)?,
                play_seconds: r.get(9)?,
                favorite: r.get::<_, i64>(10)? != 0,
                hidden: r.get::<_, i64>(11)? != 0,
                topic_id: r.get(12)?,
                has_pending_update: r.get::<_, i64>(13)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_library_cover(&self, info_hash: &str, cover: Option<&str>) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE library_items SET cover_path = ?2 WHERE info_hash = ?1",
            params![info_hash, cover],
        )?;
        Ok(())
    }

    pub fn set_library_title(&self, info_hash: &str, title: &str) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE library_items SET title = ?2 WHERE info_hash = ?1",
            params![info_hash, title],
        )?;
        Ok(())
    }

    pub fn set_library_exe(&self, info_hash: &str, exe: Option<&str>) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE library_items SET exe_path = ?2 WHERE info_hash = ?1",
            params![info_hash, exe],
        )?;
        Ok(())
    }

    pub fn set_library_flag(&self, info_hash: &str, flag: &str, value: bool) -> AppResult<()> {
        // `flag` never carries user input — the command layer maps a closed enum onto it.
        let sql = match flag {
            "favorite" => "UPDATE library_items SET favorite = ?2 WHERE info_hash = ?1",
            "hidden" => "UPDATE library_items SET hidden = ?2 WHERE info_hash = ?1",
            _ => return Err(crate::error::AppError::msg("unknown library flag")),
        };
        let conn = self.conn.lock();
        conn.execute(sql, params![info_hash, value as i64])?;
        Ok(())
    }

    pub fn record_play(&self, info_hash: &str, seconds: i64) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE library_items
             SET last_played_at = ?2, play_seconds = play_seconds + ?3
             WHERE info_hash = ?1",
            params![info_hash, now(), seconds.max(0)],
        )?;
        Ok(())
    }

    // ------------------------------------------------- watch history

    pub fn history_add(
        &self,
        topic_id: Option<i64>,
        info_hash: Option<&str>,
        title: &str,
        file_name: Option<&str>,
        image_url: Option<&str>,
        temporary: bool,
    ) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO watch_history(topic_id, info_hash, title, file_name, image_url, watched_at, temporary)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(topic_id, file_name) DO UPDATE SET
                watched_at = excluded.watched_at,
                info_hash  = COALESCE(excluded.info_hash, watch_history.info_hash),
                image_url  = COALESCE(excluded.image_url, watch_history.image_url)",
            params![
                topic_id,
                info_hash,
                title,
                file_name,
                image_url,
                now(),
                temporary as i64
            ],
        )?;
        Ok(())
    }

    pub fn history_list(&self, limit: usize) -> AppResult<Vec<WatchHistoryItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, topic_id, info_hash, title, file_name, image_url, watched_at, temporary
             FROM watch_history ORDER BY watched_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            Ok(WatchHistoryItem {
                id: r.get(0)?,
                topic_id: r.get(1)?,
                info_hash: r.get(2)?,
                title: r.get(3)?,
                file_name: r.get(4)?,
                image_url: r.get(5)?,
                watched_at: r.get(6)?,
                temporary: r.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Fills in artwork for rows that were written before it was known.
    ///
    /// A film is recorded in the history the moment it starts playing, which is
    /// usually before its topic page has been fetched — so the picture arrives
    /// a little later and is patched in here.
    pub fn history_set_image(&self, topic_id: i64, image_url: &str) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE watch_history SET image_url = ?2
             WHERE topic_id = ?1 AND (image_url IS NULL OR image_url = '')",
            params![topic_id, image_url],
        )?;
        Ok(())
    }

    pub fn history_remove(&self, id: i64) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM watch_history WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn history_clear(&self) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM watch_history", [])?;
        Ok(())
    }

    // ------------------------------------------------------- wishlist

    pub fn wishlist_add(
        &self,
        topic_id: i64,
        title: &str,
        image_url: Option<&str>,
        size_bytes: Option<i64>,
        category: &str,
    ) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO wishlist(topic_id, title, image_url, size_bytes, category, added_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(topic_id) DO UPDATE SET
                title      = excluded.title,
                image_url  = COALESCE(excluded.image_url, wishlist.image_url),
                size_bytes = COALESCE(excluded.size_bytes, wishlist.size_bytes),
                category   = excluded.category",
            params![topic_id, title, image_url, size_bytes, category, now()],
        )?;
        Ok(())
    }

    pub fn wishlist_remove(&self, topic_id: i64) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM wishlist WHERE topic_id = ?1", params![topic_id])?;
        Ok(())
    }

    pub fn wishlist_list(&self) -> AppResult<Vec<WishlistItem>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT topic_id, title, image_url, size_bytes, category, added_at
             FROM wishlist ORDER BY added_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(WishlistItem {
                topic_id: r.get(0)?,
                title: r.get(1)?,
                image_url: r.get(2)?,
                size_bytes: r.get(3)?,
                category: r.get(4)?,
                added_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // -------------------------------------------------- preview artwork

    /// Returns `Some(url)` for a cached image, `Some(None)` for a topic known
    /// to have none, and `None` when the topic has never been looked at.
    #[allow(clippy::option_option)]
    pub fn get_topic_preview(&self, topic_id: i64) -> AppResult<Option<Option<String>>> {
        let conn = self.conn.lock();
        Ok(conn
            .query_row(
                "SELECT image_url FROM topic_previews WHERE topic_id = ?1",
                params![topic_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?)
    }

    pub fn set_topic_preview(&self, topic_id: i64, image_url: Option<&str>) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO topic_previews(topic_id, image_url, fetched_at)
             VALUES(?1, ?2, ?3)
             ON CONFLICT(topic_id) DO UPDATE SET
                image_url = excluded.image_url,
                fetched_at = excluded.fetched_at",
            params![topic_id, image_url, now()],
        )?;
        Ok(())
    }

    // ---------------------------------------------------- tracked topics

    pub fn upsert_tracked_topic(
        &self,
        topic_id: i64,
        info_hash: &str,
        title: Option<&str>,
        size_bytes: Option<i64>,
        reg_time: Option<i64>,
    ) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO tracked_topics(topic_id, info_hash, title, size_bytes, reg_time)
             VALUES(?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(topic_id) DO UPDATE SET
                info_hash  = excluded.info_hash,
                title      = COALESCE(excluded.title, tracked_topics.title),
                size_bytes = COALESCE(excluded.size_bytes, tracked_topics.size_bytes),
                reg_time   = COALESCE(excluded.reg_time, tracked_topics.reg_time)",
            params![topic_id, info_hash, title, size_bytes, reg_time],
        )?;
        Ok(())
    }

    pub fn list_tracked_topics(&self, only_enabled: bool) -> AppResult<Vec<TrackedTopic>> {
        let conn = self.conn.lock();
        let sql = format!(
            "SELECT topic_id, tracker, info_hash, title, size_bytes, reg_time, last_checked_at, enabled
             FROM tracked_topics {} ORDER BY topic_id",
            if only_enabled { "WHERE enabled = 1" } else { "" }
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            Ok(TrackedTopic {
                topic_id: r.get(0)?,
                tracker: r.get(1)?,
                info_hash: r.get(2)?,
                title: r.get(3)?,
                size_bytes: r.get(4)?,
                reg_time: r.get(5)?,
                last_checked_at: r.get(6)?,
                enabled: r.get::<_, i64>(7)? != 0,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_topic_enabled(&self, topic_id: i64, enabled: bool) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE tracked_topics SET enabled = ?2 WHERE topic_id = ?1",
            params![topic_id, enabled as i64],
        )?;
        Ok(())
    }

    pub fn touch_topics_checked(&self, topic_ids: &[i64]) -> AppResult<()> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        {
            let mut stmt =
                tx.prepare("UPDATE tracked_topics SET last_checked_at = ?2 WHERE topic_id = ?1")?;
            let ts = now();
            for id in topic_ids {
                stmt.execute(params![id, ts])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_tracked_topic(&self, topic_id: i64) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM tracked_topics WHERE topic_id = ?1",
            params![topic_id],
        )?;
        Ok(())
    }

    /// Moves the library card, and the tracked topic, from an old info hash to
    /// the one of a freshly downloaded release.
    ///
    /// Order matters: the card is repointed *before* the old torrent row goes
    /// away, otherwise `ON DELETE CASCADE` would take the card with it and the
    /// user would lose their cover, playtime and favourite flag.
    pub fn migrate_to_new_hash(&self, old_hash: &str, new_hash: &str) -> AppResult<()> {
        // Guard against the degenerate case: the DELETE below would otherwise
        // remove the row we just repointed to and cascade the card away.
        if old_hash.eq_ignore_ascii_case(new_hash) {
            return Ok(());
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE library_items SET info_hash = ?2 WHERE info_hash = ?1",
            params![old_hash, new_hash],
        )?;
        tx.execute(
            "UPDATE tracked_topics SET info_hash = ?2 WHERE info_hash = ?1",
            params![old_hash, new_hash],
        )?;
        tx.execute("DELETE FROM torrents WHERE info_hash = ?1", params![old_hash])?;
        tx.commit()?;
        Ok(())
    }

    /// Records that a tracked topic now holds this release, so the next poll
    /// compares against the new baseline instead of re-reporting the update.
    pub fn set_topic_current(
        &self,
        topic_id: i64,
        info_hash: &str,
        size_bytes: Option<i64>,
        reg_time: Option<i64>,
    ) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE tracked_topics
             SET info_hash = ?2,
                 size_bytes = COALESCE(?3, size_bytes),
                 reg_time = COALESCE(?4, reg_time)
             WHERE topic_id = ?1",
            params![topic_id, info_hash, size_bytes, reg_time],
        )?;
        Ok(())
    }

    pub fn count_pending_updates(&self) -> AppResult<i64> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM topic_updates WHERE status = 'pending'",
            [],
            |r| r.get(0),
        )?)
    }

    // ----------------------------------------------------- topic updates

    /// Returns `true` when this is a newly seen re-upload, so the caller knows
    /// whether to raise a notification. Re-detecting the same hash is a no-op.
    pub fn insert_update(
        &self,
        topic_id: i64,
        old_info_hash: &str,
        new_info_hash: &str,
        old_size_bytes: Option<i64>,
        new_size_bytes: Option<i64>,
        new_reg_time: Option<i64>,
    ) -> AppResult<bool> {
        let conn = self.conn.lock();
        let changed = conn.execute(
            "INSERT OR IGNORE INTO topic_updates
                (topic_id, old_info_hash, new_info_hash, old_size_bytes, new_size_bytes, new_reg_time, detected_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                topic_id,
                old_info_hash,
                new_info_hash,
                old_size_bytes,
                new_size_bytes,
                new_reg_time,
                now()
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn list_updates(&self, status: Option<UpdateStatus>) -> AppResult<Vec<TopicUpdate>> {
        let conn = self.conn.lock();
        let sql = "SELECT u.id, u.topic_id, t.title, u.old_info_hash, u.new_info_hash,
                          u.old_size_bytes, u.new_size_bytes, u.new_reg_time, u.detected_at, u.status
                   FROM topic_updates u
                   LEFT JOIN tracked_topics t ON t.topic_id = u.topic_id
                   WHERE (?1 IS NULL OR u.status = ?1)
                   ORDER BY u.detected_at DESC";
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map(params![status.map(|s| s.as_str())], |r| {
            Ok(TopicUpdate {
                id: r.get(0)?,
                topic_id: r.get(1)?,
                title: r.get(2)?,
                old_info_hash: r.get(3)?,
                new_info_hash: r.get(4)?,
                old_size_bytes: r.get(5)?,
                new_size_bytes: r.get(6)?,
                new_reg_time: r.get(7)?,
                detected_at: r.get(8)?,
                status: UpdateStatus::from_str(&r.get::<_, String>(9)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_update_status(&self, id: i64, status: UpdateStatus) -> AppResult<()> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE topic_updates SET status = ?2 WHERE id = ?1",
            params![id, status.as_str()],
        )?;
        Ok(())
    }

    pub fn get_update(&self, id: i64) -> AppResult<Option<TopicUpdate>> {
        Ok(self.list_updates(None)?.into_iter().find(|u| u.id == id))
    }
}

fn map_torrent(r: &rusqlite::Row<'_>) -> rusqlite::Result<TorrentRecord> {
    Ok(TorrentRecord {
        info_hash: r.get(0)?,
        name: r.get(1)?,
        output_folder: r.get(2)?,
        total_bytes: r.get(3)?,
        added_at: r.get(4)?,
        completed_at: r.get(5)?,
        source: TorrentSource::from_str(&r.get::<_, String>(6)?),
        topic_id: r.get(7)?,
    })
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
