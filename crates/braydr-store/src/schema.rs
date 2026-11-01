//! Ordered migrations. `PRAGMA user_version` records how many have run.

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[r#"
        CREATE TABLE channels (
            channel_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            session_id TEXT,
            cwd TEXT NOT NULL,
            command TEXT NOT NULL,
            prompt TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE messages (
            rowid INTEGER PRIMARY KEY,
            message_id TEXT NOT NULL UNIQUE,
            channel_id TEXT NOT NULL REFERENCES channels(channel_id),
            thread_id TEXT REFERENCES threads(thread_id),
            author TEXT NOT NULL CHECK (author IN ('user', 'agent')),
            text TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX messages_main ON messages(channel_id, message_id) WHERE thread_id IS NULL;
        CREATE INDEX messages_thread ON messages(thread_id, message_id) WHERE thread_id IS NOT NULL;
        CREATE INDEX messages_channel ON messages(channel_id, message_id);
        CREATE TABLE threads (
            thread_id TEXT PRIMARY KEY,
            channel_id TEXT NOT NULL REFERENCES channels(channel_id),
            parent_message_id TEXT NOT NULL UNIQUE REFERENCES messages(message_id),
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            resolved_at INTEGER,
            resolved_by TEXT CHECK (resolved_by IN ('user', 'agent')),
            CHECK ((resolved_at IS NULL) = (resolved_by IS NULL))
        );
        CREATE INDEX threads_list ON threads(channel_id, updated_at DESC, thread_id DESC);
        CREATE TABLE thread_state_events (
            event_id TEXT PRIMARY KEY,
            thread_id TEXT NOT NULL REFERENCES threads(thread_id),
            action TEXT NOT NULL CHECK (action IN ('resolved', 'reopened')),
            actor TEXT NOT NULL CHECK (actor IN ('user', 'agent')),
            created_at INTEGER NOT NULL
        );
        CREATE INDEX thread_state_events_thread ON thread_state_events(thread_id, event_id);
        CREATE TABLE read_positions (
            scope_type TEXT NOT NULL CHECK (scope_type IN ('channel', 'thread')),
            scope_id TEXT NOT NULL,
            last_read_message_id TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (scope_type, scope_id)
        );
        CREATE VIRTUAL TABLE messages_fts USING fts5(
            text, content='messages', content_rowid='rowid', tokenize='trigram'
        );
        CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
            INSERT INTO messages_fts(rowid, text) VALUES (new.rowid, new.text);
        END;
        "#];

pub fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}
