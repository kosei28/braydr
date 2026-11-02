//! Ordered migrations. `PRAGMA user_version` records how many have run.

use rusqlite::Connection;

pub(crate) const MIGRATIONS: &[&str] = &[
    r#"
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
        "#,
    r#"
        CREATE TABLE settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    "ALTER TABLE channels ADD COLUMN session_responded INTEGER NOT NULL DEFAULT 0;",
    // Channels used to sort by their newest post, so that is where they start.
    r#"
        ALTER TABLE channels ADD COLUMN position INTEGER NOT NULL DEFAULT 0;
        UPDATE channels SET position = (
            SELECT ranked.rank FROM (
                SELECT c.channel_id, row_number() OVER (
                    ORDER BY coalesce(
                        (SELECT max(m.message_id) FROM messages m
                         WHERE m.channel_id = c.channel_id AND m.thread_id IS NULL),
                        c.channel_id
                    ) DESC
                ) AS rank
                FROM channels c
            ) ranked
            WHERE ranked.channel_id = channels.channel_id
        );
        "#,
    // Set on an agent post that says the agent is still working (`docs/product-spec.md` 6.3).
    "ALTER TABLE messages ADD COLUMN agent_continues INTEGER NOT NULL DEFAULT 0;",
];

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_channels_keep_their_newest_post_order() {
        let mut conn = Connection::open_in_memory().unwrap();
        for sql in &MIGRATIONS[..3] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute_batch(
            "INSERT INTO channels (channel_id, name, cwd, command, prompt, created_at) VALUES
                ('1', 'posted-last', '', '', '', 0),
                ('2', 'no-posts', '', '', '', 0),
                ('3', 'posted-first', '', '', '', 0);
             INSERT INTO messages (message_id, channel_id, author, text, created_at) VALUES
                ('0', '3', 'user', '', 0),
                ('9', '1', 'user', '', 0);",
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let names = conn
            .prepare("SELECT name FROM channels ORDER BY position")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(names, ["posted-last", "no-posts", "posted-first"]);
    }
}
