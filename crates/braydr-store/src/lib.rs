//! SQLite storage. Every write that spec requires to be atomic runs in one transaction.

mod schema;

use std::path::Path;

use braydr_core::validate;
use braydr_core::*;
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};

pub type Clock = Box<dyn Fn() -> i64 + Send>;

pub struct Store {
    conn: Connection,
    clock: Clock,
}

fn internal(error: impl std::fmt::Display) -> Error {
    Error::Internal {
        message: error.to_string(),
    }
}

trait Db<T> {
    fn db(self) -> Result<T>;
}

impl<T> Db<T> for rusqlite::Result<T> {
    fn db(self) -> Result<T> {
        self.map_err(internal)
    }
}

fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

fn author_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Author> {
    let value: String = row.get(index)?;
    Ok(Author::parse(&value).expect("the schema constrains author"))
}

fn optional_author_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<Author>> {
    let value: Option<String> = row.get(index)?;
    Ok(value.map(|value| Author::parse(&value).expect("the schema constrains author")))
}

const MESSAGE_COLUMNS: &str = "m.message_id, m.channel_id, m.thread_id,
    (SELECT t.thread_id FROM threads t WHERE t.parent_message_id = m.message_id),
    m.author, m.created_at, m.text";

fn message_from(row: &Row<'_>) -> rusqlite::Result<Message> {
    Ok(Message {
        message_id: row.get(0)?,
        channel_id: row.get(1)?,
        thread_id: row.get(2)?,
        child_thread_id: row.get(3)?,
        author: author_at(row, 4)?,
        created_at: row.get(5)?,
        text: row.get(6)?,
    })
}

fn channel_from(row: &Row<'_>) -> rusqlite::Result<Channel> {
    Ok(Channel {
        channel_id: row.get(0)?,
        name: row.get(1)?,
        session_id: row.get(2)?,
        cwd: row.get(3)?,
        command: row.get(4)?,
        prompt: row.get(5)?,
        created_at: row.get(6)?,
    })
}

fn message_tail(conn: &Connection, scope: Scope<'_>) -> Result<Option<String>> {
    let found = match scope {
        Scope::Main { channel_id } => conn.query_row(
            "SELECT max(message_id) FROM messages WHERE channel_id = ?1 AND thread_id IS NULL",
            [channel_id],
            |row| row.get(0),
        ),
        Scope::Thread { thread_id } => conn.query_row(
            "SELECT max(message_id) FROM messages WHERE thread_id = ?1",
            [thread_id],
            |row| row.get(0),
        ),
    };
    found.db()
}

fn check_after(tail: Option<&str>, after: Option<&str>) -> Result<()> {
    match (tail, after) {
        (None, _) => Ok(()),
        (Some(_), None) => Err(Error::AfterRequired),
        (Some(tail), Some(after)) if tail == after => Ok(()),
        (Some(tail), Some(_)) => Err(Error::StaleAfter {
            current_last_message_id: tail.to_owned(),
        }),
    }
}

fn insert_message(
    conn: &Connection,
    channel_id: &str,
    thread_id: Option<&str>,
    author: Author,
    text: &str,
    now: i64,
) -> Result<String> {
    let id = new_id();
    conn.execute(
        "INSERT INTO messages (message_id, channel_id, thread_id, author, text, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, channel_id, thread_id, author.as_str(), text, now],
    )
    .db()?;
    Ok(id)
}

fn record_event(
    conn: &Connection,
    thread_id: &str,
    action: StateAction,
    actor: Author,
    now: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO thread_state_events (event_id, thread_id, action, actor, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![new_id(), thread_id, action.as_str(), actor.as_str(), now],
    )
    .db()?;
    Ok(())
}

fn set_read(conn: &Connection, scope: Scope<'_>, message_id: &str, now: i64) -> Result<()> {
    let (kind, id) = scope_key(scope);
    conn.execute(
        "INSERT INTO read_positions (scope_type, scope_id, last_read_message_id, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (scope_type, scope_id) DO UPDATE SET
           last_read_message_id = excluded.last_read_message_id, updated_at = excluded.updated_at
         WHERE excluded.last_read_message_id > read_positions.last_read_message_id",
        params![kind, id, message_id, now],
    )
    .db()?;
    Ok(())
}

/// Trigram needs 3 characters, so shorter queries use `LIKE`. `?2` binds the returned pattern.
fn search_source(query: &str) -> (&'static str, String) {
    if query.chars().count() >= 3 {
        (
            "messages_fts JOIN messages m ON messages_fts.rowid = m.rowid WHERE messages_fts MATCH ?2",
            format!("\"{}\"", query.replace('"', "\"\"")),
        )
    } else {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        (
            "messages m WHERE m.text LIKE ?2 ESCAPE '\\'",
            format!("%{escaped}%"),
        )
    }
}

fn scope_key(scope: Scope<'_>) -> (&'static str, &str) {
    match scope {
        Scope::Main { channel_id } => ("channel", channel_id),
        Scope::Thread { thread_id } => ("thread", thread_id),
    }
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::setup(Connection::open(path).db()?, Box::new(wall_clock))
    }

    pub fn in_memory() -> Result<Self> {
        Self::setup(Connection::open_in_memory().db()?, Box::new(wall_clock))
    }

    pub fn in_memory_with_clock(clock: Clock) -> Result<Self> {
        Self::setup(Connection::open_in_memory().db()?, clock)
    }

    fn setup(mut conn: Connection, clock: Clock) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").db()?;
        conn.pragma_update(None, "foreign_keys", "ON").db()?;
        schema::migrate(&mut conn).db()?;
        Ok(Self { conn, clock })
    }

    fn write<T>(&mut self, body: impl FnOnce(&Connection, i64) -> Result<T>) -> Result<T> {
        let now = (self.clock)();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .db()?;
        let value = body(&tx, now)?;
        tx.commit().db()?;
        Ok(value)
    }

    // Channels

    pub fn create_channel(&mut self, name: &str, launch: &LaunchConfig) -> Result<Channel> {
        let name = validate::channel_name(name)?;
        let id = new_id();
        self.write(|tx, now| {
            tx.execute(
                "INSERT INTO channels (channel_id, name, cwd, command, prompt, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, name, launch.cwd, launch.command, launch.prompt, now],
            )
            .db()?;
            Ok(())
        })?;
        self.channel(&id)
    }

    pub fn channel(&self, channel_id: &str) -> Result<Channel> {
        self.conn
            .query_row(
                "SELECT channel_id, name, session_id, cwd, command, prompt, created_at
                 FROM channels WHERE channel_id = ?1",
                [channel_id],
                channel_from,
            )
            .optional()
            .db()?
            .ok_or(Error::ChannelNotFound)
    }

    /// Newest post first, as the sidebar shows them.
    pub fn channels(&self) -> Result<Vec<Channel>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT c.channel_id, c.name, c.session_id, c.cwd, c.command, c.prompt, c.created_at
                 FROM channels c
                 ORDER BY coalesce((SELECT max(m.message_id) FROM messages m WHERE m.channel_id = c.channel_id AND m.thread_id IS NULL), c.channel_id) DESC",
            )
            .db()?;
        statement
            .query_map([], channel_from)
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()
    }

    pub fn rename_channel(&mut self, channel_id: &str, name: &str) -> Result<()> {
        let name = validate::channel_name(name)?;
        let changed = self
            .conn
            .execute(
                "UPDATE channels SET name = ?2 WHERE channel_id = ?1",
                params![channel_id, name],
            )
            .db()?;
        if changed == 0 {
            return Err(Error::ChannelNotFound);
        }
        Ok(())
    }

    pub fn set_session(&mut self, channel_id: &str, session_id: Option<&str>) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE channels
                 SET session_responded = CASE WHEN session_id IS ?2 THEN session_responded ELSE 0 END,
                     session_id = ?2
                 WHERE channel_id = ?1",
                params![channel_id, session_id],
            )
            .db()?;
        if changed == 0 {
            return Err(Error::ChannelNotFound);
        }
        Ok(())
    }

    /// Records that the current session has called a tool, so it is ready for input.
    pub fn mark_session_responded(&mut self, channel_id: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE channels SET session_responded = 1
                 WHERE channel_id = ?1 AND session_responded = 0",
                [channel_id],
            )
            .db()?;
        Ok(())
    }

    /// Whether the current session has called a tool since it started.
    pub fn session_responded(&self, channel_id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT session_responded FROM channels WHERE channel_id = ?1",
                [channel_id],
                |row| row.get(0),
            )
            .optional()
            .db()?
            .ok_or(Error::ChannelNotFound)
    }

    /// Clears the session and stores the settings of the next one.
    pub fn prepare_new_session(&mut self, channel_id: &str, launch: &LaunchConfig) -> Result<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE channels SET session_id = NULL, session_responded = 0, cwd = ?2, command = ?3, prompt = ?4 WHERE channel_id = ?1",
                params![channel_id, launch.cwd, launch.command, launch.prompt],
            )
            .db()?;
        if changed == 0 {
            return Err(Error::ChannelNotFound);
        }
        Ok(())
    }

    // Reads

    pub fn message(&self, channel_id: &str, message_id: &str) -> Result<Message> {
        self.channel(channel_id)?;
        self.conn
            .query_row(
                &format!("SELECT {MESSAGE_COLUMNS} FROM messages m WHERE m.message_id = ?1 AND m.channel_id = ?2"),
                [message_id, channel_id],
                message_from,
            )
            .optional()
            .db()?
            .ok_or(Error::MessageNotFound)
    }

    /// Newest page first; each page is ordered oldest to newest.
    pub fn list_posts(
        &self,
        scope: Scope<'_>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<Message>> {
        let limit = clamp_limit(limit, 50);
        let before = decode_message_cursor(cursor)?;
        let (filter, key) = match scope {
            Scope::Main { channel_id } => {
                self.channel(channel_id)?;
                ("m.channel_id = ?1 AND m.thread_id IS NULL", channel_id)
            }
            Scope::Thread { thread_id } => ("m.thread_id = ?1", thread_id),
        };
        if let Some(before) = before {
            let in_scope: bool = self
                .conn
                .query_row(
                    &format!("SELECT EXISTS (SELECT 1 FROM messages m WHERE {filter} AND m.message_id = ?2)"),
                    [key, before],
                    |row| row.get(0),
                )
                .db()?;
            if !in_scope {
                return Err(Error::InvalidCursor);
            }
        }
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages m
                 WHERE {filter} AND (?2 IS NULL OR m.message_id < ?2)
                 ORDER BY m.message_id DESC LIMIT ?3"
            ))
            .db()?;
        let mut items: Vec<Message> = statement
            .query_map(params![key, before, limit + 1], message_from)
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()?;
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);
        items.reverse();
        let next_cursor = has_more.then(|| format!("m:{}", items[0].message_id));
        Ok(Page {
            items,
            has_more,
            next_cursor,
        })
    }

    pub fn thread(&self, channel_id: &str, thread_id: &str) -> Result<Thread> {
        self.thread_in(&self.conn, channel_id, thread_id)
    }

    fn thread_in(&self, conn: &Connection, channel_id: &str, thread_id: &str) -> Result<Thread> {
        load_thread(conn, channel_id, thread_id)
    }

    pub fn thread_for_parent(
        &self,
        channel_id: &str,
        parent_message_id: &str,
    ) -> Result<Option<Thread>> {
        let id: Option<String> = self
            .conn
            .query_row(
                "SELECT thread_id FROM threads WHERE parent_message_id = ?1 AND channel_id = ?2",
                [parent_message_id, channel_id],
                |row| row.get(0),
            )
            .optional()
            .db()?;
        id.map(|id| self.thread(channel_id, &id)).transpose()
    }

    pub fn state_events(&self, thread_id: &str) -> Result<Vec<StateEvent>> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT event_id, action, actor, created_at FROM thread_state_events
                 WHERE thread_id = ?1 ORDER BY event_id",
            )
            .db()?;
        statement
            .query_map([thread_id], |row| {
                let action: String = row.get(1)?;
                Ok(StateEvent {
                    event_id: row.get(0)?,
                    action: StateAction::parse(&action).expect("the schema constrains action"),
                    actor: author_at(row, 2)?,
                    created_at: row.get(3)?,
                })
            })
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()
    }

    pub fn list_threads(
        &self,
        channel_id: &str,
        filter: ThreadFilter,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<Thread>> {
        self.channel(channel_id)?;
        let limit = clamp_limit(limit, 20);
        let after = decode_thread_cursor(cursor)?;
        let last_author = "(SELECT author FROM messages l WHERE l.thread_id = t.thread_id ORDER BY l.message_id DESC LIMIT 1)";
        let condition = match filter {
            ThreadFilter::Unresolved => "t.resolved_at IS NULL".to_owned(),
            ThreadFilter::Resolved => "t.resolved_at IS NOT NULL".to_owned(),
            ThreadFilter::All => "1".to_owned(),
            ThreadFilter::AwaitingAgent => {
                format!("t.resolved_at IS NULL AND {last_author} = 'user'")
            }
            ThreadFilter::AwaitingUser => {
                format!("t.resolved_at IS NULL AND {last_author} = 'agent'")
            }
        };
        let (cursor_time, cursor_id) = after.unzip();
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT t.thread_id FROM threads t
                 WHERE t.channel_id = ?1 AND {condition}
                   AND (?2 IS NULL OR (t.updated_at, t.thread_id) < (?2, ?3))
                 ORDER BY t.updated_at DESC, t.thread_id DESC LIMIT ?4"
            ))
            .db()?;
        let ids: Vec<String> = statement
            .query_map(
                params![channel_id, cursor_time, cursor_id, limit + 1],
                |row| row.get(0),
            )
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()?;
        let has_more = ids.len() > limit as usize;
        let items = ids
            .iter()
            .take(limit as usize)
            .map(|id| self.thread(channel_id, id))
            .collect::<Result<Vec<_>>>()?;
        let next_cursor = has_more.then(|| {
            let last = items.last().expect("has_more implies items");
            format!("t:{}:{}", last.updated_at, last.thread_id)
        });
        Ok(Page {
            items,
            has_more,
            next_cursor,
        })
    }

    pub fn channel_summary(&self, channel_id: &str) -> Result<ChannelSummary> {
        let channel = self.channel(channel_id)?;
        let (post_count, updated_at, last_author) = self
            .conn
            .query_row(
                "SELECT (SELECT count(*) FROM messages WHERE channel_id = ?1 AND thread_id IS NULL),
                        (SELECT created_at FROM messages WHERE channel_id = ?1 AND thread_id IS NULL ORDER BY message_id DESC LIMIT 1),
                        (SELECT author FROM messages WHERE channel_id = ?1 AND thread_id IS NULL ORDER BY message_id DESC LIMIT 1)",
                [channel_id],
                |row| Ok((row.get(0)?, row.get(1)?, optional_author_at(row, 2)?)),
            )
            .db()?;
        let count = |filter| -> Result<i64> {
            let last = "(SELECT author FROM messages l WHERE l.thread_id = t.thread_id ORDER BY l.message_id DESC LIMIT 1)";
            let condition = match filter {
                ThreadFilter::All => "1".to_owned(),
                ThreadFilter::Unresolved => "t.resolved_at IS NULL".to_owned(),
                ThreadFilter::Resolved => "t.resolved_at IS NOT NULL".to_owned(),
                ThreadFilter::AwaitingAgent => format!("t.resolved_at IS NULL AND {last} = 'user'"),
                ThreadFilter::AwaitingUser => format!("t.resolved_at IS NULL AND {last} = 'agent'"),
            };
            self.conn
                .query_row(
                    &format!(
                        "SELECT count(*) FROM threads t WHERE t.channel_id = ?1 AND {condition}"
                    ),
                    [channel_id],
                    |row| row.get(0),
                )
                .db()
        };
        Ok(ChannelSummary {
            channel,
            post_count,
            updated_at,
            last_author,
            thread_count_total: count(ThreadFilter::All)?,
            thread_count_unresolved: count(ThreadFilter::Unresolved)?,
            thread_count_resolved: count(ThreadFilter::Resolved)?,
            thread_count_awaiting_agent: count(ThreadFilter::AwaitingAgent)?,
            thread_count_awaiting_user: count(ThreadFilter::AwaitingUser)?,
        })
    }

    pub fn unresolved_count(&self, channel_id: &str) -> Result<i64> {
        self.conn
            .query_row(
                "SELECT count(*) FROM threads WHERE channel_id = ?1 AND resolved_at IS NULL",
                [channel_id],
                |row| row.get(0),
            )
            .db()
    }

    /// Full-text search over one channel, newest first.
    pub fn search(
        &self,
        channel_id: &str,
        query: &str,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Page<Message>> {
        validate::search_query(query)?;
        self.channel(channel_id)?;
        let limit = clamp_limit(limit, 20);
        let before = decode_message_cursor(cursor)?;
        let (source, pattern) = search_source(query);
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM {source}
                 AND m.channel_id = ?1 AND (?3 IS NULL OR m.message_id < ?3)
                 ORDER BY m.message_id DESC LIMIT ?4"
            ))
            .db()?;
        let mut items: Vec<Message> = statement
            .query_map(
                params![channel_id, pattern, before, limit + 1],
                message_from,
            )
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()?;
        let has_more = items.len() > limit as usize;
        items.truncate(limit as usize);
        let next_cursor = has_more.then(|| {
            format!(
                "m:{}",
                items.last().expect("has_more implies items").message_id
            )
        });
        Ok(Page {
            items,
            has_more,
            next_cursor,
        })
    }

    /// How many messages `search` finds in total.
    pub fn search_count(&self, channel_id: &str, query: &str) -> Result<i64> {
        validate::search_query(query)?;
        let (source, pattern) = search_source(query);
        self.conn
            .query_row(
                &format!("SELECT count(*) FROM {source} AND m.channel_id = ?1"),
                params![channel_id, pattern],
                |row| row.get(0),
            )
            .db()
    }

    // Native notifications

    /// The newest message in any channel; agent messages after it have not been announced.
    pub fn newest_message_id(&self) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT max(message_id) FROM messages", [], |row| row.get(0))
            .db()
    }

    /// Agent messages in any channel after `after`, oldest first.
    pub fn agent_messages_after(&self, after: Option<&str>) -> Result<Vec<Message>> {
        let mut statement = self
            .conn
            .prepare(&format!(
                "SELECT {MESSAGE_COLUMNS} FROM messages m
                 WHERE m.author = 'agent' AND (?1 IS NULL OR m.message_id > ?1)
                 ORDER BY m.message_id"
            ))
            .db()?;
        statement
            .query_map([after], message_from)
            .db()?
            .collect::<rusqlite::Result<_>>()
            .db()
    }

    /// The thread's channel, for opening a thread from its id alone.
    pub fn channel_of_thread(&self, thread_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT channel_id FROM threads WHERE thread_id = ?1",
                [thread_id],
                |row| row.get(0),
            )
            .optional()
            .db()
    }

    // Settings

    pub fn settings(&self) -> Result<Settings> {
        let read = |key: &str| -> Result<Option<String>> {
            self.conn
                .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                    row.get(0)
                })
                .optional()
                .db()
        };
        let defaults = Settings::default();
        Ok(Settings {
            default_command: read("default_command")?.unwrap_or(defaults.default_command),
            default_prompt: read("default_prompt")?.unwrap_or(defaults.default_prompt),
            nudge_minutes: read("nudge_minutes")?
                .and_then(|value| value.parse().ok())
                .unwrap_or(defaults.nudge_minutes),
        })
    }

    /// A value the app keeps for itself, outside the settings the user edits.
    pub fn value(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .db()
    }

    pub fn set_value(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .db()?;
        Ok(())
    }

    pub fn save_settings(&mut self, settings: &Settings) -> Result<()> {
        self.write(|tx, _| {
            for (key, value) in [
                ("default_command", settings.default_command.clone()),
                ("default_prompt", settings.default_prompt.clone()),
                ("nudge_minutes", settings.nudge_minutes.to_string()),
            ] {
                tx.execute(
                    "INSERT INTO settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                    params![key, value],
                )
                .db()?;
            }
            Ok(())
        })
    }

    // Agent writes

    pub fn agent_post_main(
        &mut self,
        channel_id: &str,
        text: &str,
        after: Option<&str>,
    ) -> Result<Message> {
        validate::message_text(text)?;
        self.channel(channel_id)?;
        let id = self.write(|tx, now| {
            let tail = message_tail(tx, Scope::Main { channel_id })?;
            check_after(tail.as_deref(), after)?;
            insert_message(tx, channel_id, None, Author::Agent, text, now)
        })?;
        self.message(channel_id, &id)
    }

    pub fn agent_post_thread(
        &mut self,
        channel_id: &str,
        thread_id: &str,
        text: &str,
        after: Option<&str>,
    ) -> Result<Message> {
        validate::message_text(text)?;
        let id = self.write(|tx, now| {
            let thread = load_thread(tx, channel_id, thread_id)?;
            if let (Some(resolved_at), Some(resolved_by)) = (thread.resolved_at, thread.resolved_by)
            {
                return Err(Error::ThreadResolved {
                    resolved_at,
                    resolved_by,
                    last_message_id: thread.last_message_id,
                });
            }
            let tail = message_tail(tx, Scope::Thread { thread_id })?;
            check_after(tail.as_deref(), after)?;
            let id = insert_message(tx, channel_id, Some(thread_id), Author::Agent, text, now)?;
            touch_thread(tx, thread_id, now)?;
            Ok(id)
        })?;
        self.message(channel_id, &id)
    }

    pub fn agent_create_thread(
        &mut self,
        channel_id: &str,
        parent_message_id: &str,
        text: &str,
    ) -> Result<(Thread, Message)> {
        validate::message_text(text)?;
        self.channel(channel_id)?;
        let (thread_id, message_id) = self.write(|tx, now| {
            let parent = load_parent(tx, channel_id, parent_message_id)?;
            if let Some(thread_id) = parent {
                return Err(Error::ThreadExists { thread_id });
            }
            create_thread(tx, channel_id, parent_message_id, Author::Agent, text, now)
        })?;
        Ok((
            self.thread(channel_id, &thread_id)?,
            self.message(channel_id, &message_id)?,
        ))
    }

    pub fn agent_resolve(
        &mut self,
        channel_id: &str,
        thread_id: &str,
        after: &str,
    ) -> Result<Thread> {
        self.write(|tx, now| {
            let thread = load_thread(tx, channel_id, thread_id)?;
            if let (Some(resolved_at), Some(resolved_by)) = (thread.resolved_at, thread.resolved_by)
            {
                return Err(Error::AlreadyResolved {
                    resolved_at,
                    resolved_by,
                });
            }
            check_after(Some(&thread.last_message_id), Some(after))?;
            if thread.last_author == Author::User {
                return Err(Error::Unanswered {
                    last_message_id: thread.last_message_id,
                });
            }
            resolve(tx, thread_id, Author::Agent, now)
        })?;
        self.thread(channel_id, thread_id)
    }

    pub fn agent_unresolve(&mut self, channel_id: &str, thread_id: &str) -> Result<Thread> {
        self.write(|tx, now| {
            let thread = load_thread(tx, channel_id, thread_id)?;
            if thread.resolved_at.is_none() {
                return Err(Error::AlreadyUnresolved);
            }
            reopen(tx, thread_id, Author::Agent, now)
        })?;
        self.thread(channel_id, thread_id)
    }

    // User writes

    pub fn user_post_main(&mut self, channel_id: &str, text: &str) -> Result<Message> {
        validate::message_text(text)?;
        self.channel(channel_id)?;
        let id = self.write(|tx, now| {
            let id = insert_message(tx, channel_id, None, Author::User, text, now)?;
            set_read(tx, Scope::Main { channel_id }, &id, now)?;
            Ok(id)
        })?;
        self.message(channel_id, &id)
    }

    /// Replies on a main message, creating its thread on the first reply. Reopens a resolved thread.
    pub fn user_reply(
        &mut self,
        channel_id: &str,
        parent_message_id: &str,
        text: &str,
    ) -> Result<UserReply> {
        validate::message_text(text)?;
        self.channel(channel_id)?;
        let (thread_id, message_id, created, reopened) = self.write(
            |tx, now| match load_parent(tx, channel_id, parent_message_id)? {
                None => {
                    let (thread_id, message_id) =
                        create_thread(tx, channel_id, parent_message_id, Author::User, text, now)?;
                    set_read(
                        tx,
                        Scope::Thread {
                            thread_id: &thread_id,
                        },
                        &message_id,
                        now,
                    )?;
                    Ok((thread_id, message_id, true, false))
                }
                Some(thread_id) => {
                    let (message_id, reopened) =
                        user_post_in_thread(tx, channel_id, &thread_id, text, now)?;
                    Ok((thread_id, message_id, false, reopened))
                }
            },
        )?;
        Ok(UserReply {
            thread: self.thread(channel_id, &thread_id)?,
            message: self.message(channel_id, &message_id)?,
            created,
            reopened,
        })
    }

    pub fn user_resolve(&mut self, channel_id: &str, thread_id: &str) -> Result<Thread> {
        self.write(|tx, now| {
            let thread = load_thread(tx, channel_id, thread_id)?;
            if let (Some(resolved_at), Some(resolved_by)) = (thread.resolved_at, thread.resolved_by)
            {
                return Err(Error::AlreadyResolved {
                    resolved_at,
                    resolved_by,
                });
            }
            resolve(tx, thread_id, Author::User, now)
        })?;
        self.thread(channel_id, thread_id)
    }

    pub fn user_reopen(&mut self, channel_id: &str, thread_id: &str) -> Result<Thread> {
        self.write(|tx, now| {
            let thread = load_thread(tx, channel_id, thread_id)?;
            if thread.resolved_at.is_none() {
                return Err(Error::AlreadyUnresolved);
            }
            reopen(tx, thread_id, Author::User, now)
        })?;
        self.thread(channel_id, thread_id)
    }

    // Unread

    /// Moves the read position forward. It never moves back.
    pub fn mark_read(&mut self, scope: Scope<'_>, message_id: &str) -> Result<()> {
        self.write(|tx, now| set_read(tx, scope, message_id, now))
    }

    pub fn read_position(&self, scope: Scope<'_>) -> Result<Option<String>> {
        let (kind, id) = scope_key(scope);
        self.conn
            .query_row(
                "SELECT last_read_message_id FROM read_positions WHERE scope_type = ?1 AND scope_id = ?2",
                [kind, id],
                |row| row.get(0),
            )
            .optional()
            .db()
    }

    pub fn first_unread(&self, scope: Scope<'_>) -> Result<Option<String>> {
        let read = self.read_position(scope)?;
        let (filter, key) = match scope {
            Scope::Main { channel_id } => ("channel_id = ?1 AND thread_id IS NULL", channel_id),
            Scope::Thread { thread_id } => ("thread_id = ?1", thread_id),
        };
        self.conn
            .query_row(
                &format!("SELECT min(message_id) FROM messages WHERE {filter} AND (?2 IS NULL OR message_id > ?2)"),
                params![key, read],
                |row| row.get(0),
            )
            .db()
    }

    pub fn has_unread(&self, scope: Scope<'_>) -> Result<bool> {
        Ok(self.first_unread(scope)?.is_some())
    }

    /// Whether any thread in the channel has unread replies: any thread, or only unresolved ones.
    pub fn has_unread_thread(&self, channel_id: &str, unresolved_only: bool) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS (
                    SELECT 1 FROM threads t
                    JOIN messages m ON m.thread_id = t.thread_id
                    LEFT JOIN read_positions r
                        ON r.scope_type = 'thread' AND r.scope_id = t.thread_id
                    WHERE t.channel_id = ?1
                        AND (NOT ?2 OR t.resolved_at IS NULL)
                        AND (r.last_read_message_id IS NULL OR m.message_id > r.last_read_message_id)
                )",
                params![channel_id, unresolved_only],
                |row| row.get(0),
            )
            .db()
    }

    // Nudges (`docs/product-spec.md` 10.1)

    /// The newest main message, if it is from the user and has no child thread.
    pub fn main_awaiting_response(&self, channel_id: &str) -> Result<Option<(String, i64)>> {
        let last: Option<(String, i64, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT m.message_id, m.created_at, m.author,
                        (SELECT t.thread_id FROM threads t WHERE t.parent_message_id = m.message_id)
                 FROM messages m WHERE m.channel_id = ?1 AND m.thread_id IS NULL
                 ORDER BY m.message_id DESC LIMIT 1",
                [channel_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()
            .db()?;
        Ok(last.and_then(|(id, at, author, child)| {
            (author == "user" && child.is_none()).then_some((id, at))
        }))
    }

    pub fn main_needs_response(&self, channel_id: &str) -> Result<bool> {
        Ok(self.main_awaiting_response(channel_id)?.is_some())
    }
}

fn wall_clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_millis() as i64
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSummary {
    pub channel: Channel,
    pub post_count: i64,
    pub updated_at: Option<i64>,
    pub last_author: Option<Author>,
    pub thread_count_total: i64,
    pub thread_count_unresolved: i64,
    pub thread_count_resolved: i64,
    pub thread_count_awaiting_agent: i64,
    pub thread_count_awaiting_user: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserReply {
    pub thread: Thread,
    pub message: Message,
    pub created: bool,
    pub reopened: bool,
}

fn decode_message_cursor(cursor: Option<&str>) -> Result<Option<&str>> {
    cursor
        .map(|cursor| cursor.strip_prefix("m:").ok_or(Error::InvalidCursor))
        .transpose()
}

fn decode_thread_cursor(cursor: Option<&str>) -> Result<Option<(i64, String)>> {
    let Some(cursor) = cursor else {
        return Ok(None);
    };
    let (time, id) = cursor
        .strip_prefix("t:")
        .and_then(|rest| rest.split_once(':'))
        .ok_or(Error::InvalidCursor)?;
    Ok(Some((
        time.parse().map_err(|_| Error::InvalidCursor)?,
        id.to_owned(),
    )))
}

/// Returns the parent's existing thread, after checking it is a main message of this channel.
fn load_parent(
    conn: &Connection,
    channel_id: &str,
    parent_message_id: &str,
) -> Result<Option<String>> {
    let row: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT m.thread_id, (SELECT t.thread_id FROM threads t WHERE t.parent_message_id = m.message_id)
             FROM messages m WHERE m.message_id = ?1 AND m.channel_id = ?2",
            [parent_message_id, channel_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .db()?;
    match row {
        None => Err(Error::MessageNotFound),
        Some((Some(_), _)) => Err(Error::ParentNotMain),
        Some((None, child)) => Ok(child),
    }
}

fn create_thread(
    conn: &Connection,
    channel_id: &str,
    parent_message_id: &str,
    author: Author,
    text: &str,
    now: i64,
) -> Result<(String, String)> {
    let thread_id = new_id();
    conn.execute(
        "INSERT INTO threads (thread_id, channel_id, parent_message_id, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?4)",
        params![thread_id, channel_id, parent_message_id, now],
    )
    .db()?;
    let message_id = insert_message(conn, channel_id, Some(&thread_id), author, text, now)?;
    Ok((thread_id, message_id))
}

fn user_post_in_thread(
    conn: &Connection,
    channel_id: &str,
    thread_id: &str,
    text: &str,
    now: i64,
) -> Result<(String, bool)> {
    let thread = load_thread(conn, channel_id, thread_id)?;
    let reopened = thread.resolved_at.is_some();
    if reopened {
        reopen(conn, thread_id, Author::User, now)?;
    }
    let message_id = insert_message(conn, channel_id, Some(thread_id), Author::User, text, now)?;
    touch_thread(conn, thread_id, now)?;
    set_read(conn, Scope::Thread { thread_id }, &message_id, now)?;
    Ok((message_id, reopened))
}

fn touch_thread(conn: &Connection, thread_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE threads SET updated_at = ?2 WHERE thread_id = ?1",
        params![thread_id, now],
    )
    .db()?;
    Ok(())
}

fn resolve(conn: &Connection, thread_id: &str, actor: Author, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE threads SET resolved_at = ?2, resolved_by = ?3, updated_at = ?2 WHERE thread_id = ?1",
        params![thread_id, now, actor.as_str()],
    )
    .db()?;
    record_event(conn, thread_id, StateAction::Resolved, actor, now)
}

fn reopen(conn: &Connection, thread_id: &str, actor: Author, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE threads SET resolved_at = NULL, resolved_by = NULL, updated_at = ?2 WHERE thread_id = ?1",
        params![thread_id, now],
    )
    .db()?;
    record_event(conn, thread_id, StateAction::Reopened, actor, now)
}

fn load_thread(conn: &Connection, channel_id: &str, thread_id: &str) -> Result<Thread> {
    let thread = conn
        .query_row(
            "SELECT t.parent_message_id, t.created_at, t.updated_at, t.resolved_at, t.resolved_by
             FROM threads t WHERE t.thread_id = ?1 AND t.channel_id = ?2",
            [thread_id, channel_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    optional_author_at(row, 4)?,
                ))
            },
        )
        .optional()
        .db()?
        .ok_or(Error::ThreadNotFound)?;
    let (parent_message_id, created_at, updated_at, resolved_at, resolved_by) = thread;
    let parent_message = conn
        .query_row(
            &format!("SELECT {MESSAGE_COLUMNS} FROM messages m WHERE m.message_id = ?1"),
            [&parent_message_id],
            message_from,
        )
        .db()?;
    let (reply_count, last_message_id, last_message_at, last_author) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM messages WHERE thread_id = ?1), message_id, created_at, author
             FROM messages WHERE thread_id = ?1 ORDER BY message_id DESC LIMIT 1",
            [thread_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?, author_at(row, 3)?)),
        )
        .db()?;
    Ok(Thread {
        thread_id: thread_id.to_owned(),
        channel_id: channel_id.to_owned(),
        parent_message_id,
        parent_message,
        created_at,
        updated_at,
        last_message_id,
        last_message_at,
        last_author,
        post_count: reply_count + 1,
        reply_count,
        resolved_at,
        resolved_by,
        awaiting: awaiting(resolved_at.is_some(), last_author),
    })
}
