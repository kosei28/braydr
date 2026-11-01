use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Author {
    User,
    Agent,
}

impl Author {
    pub fn as_str(self) -> &'static str {
        match self {
            Author::User => "user",
            Author::Agent => "agent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Author::User),
            "agent" => Some(Author::Agent),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Channel {
    pub channel_id: String,
    pub name: String,
    pub session_id: Option<String>,
    pub cwd: String,
    pub command: String,
    pub prompt: String,
    pub created_at: i64,
}

/// Arguments for starting a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchConfig {
    pub cwd: String,
    pub command: String,
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub message_id: String,
    pub channel_id: String,
    pub thread_id: Option<String>,
    pub child_thread_id: Option<String>,
    pub author: Author,
    pub created_at: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Thread {
    pub thread_id: String,
    pub channel_id: String,
    pub parent_message_id: String,
    pub parent_message: Message,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_message_id: String,
    pub last_message_at: i64,
    pub last_author: Author,
    pub post_count: i64,
    pub reply_count: i64,
    pub resolved_at: Option<i64>,
    pub resolved_by: Option<Author>,
    pub awaiting: Option<Author>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StateAction {
    Resolved,
    Reopened,
}

impl StateAction {
    pub fn as_str(self) -> &'static str {
        match self {
            StateAction::Resolved => "resolved",
            StateAction::Reopened => "reopened",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "resolved" => Some(StateAction::Resolved),
            "reopened" => Some(StateAction::Reopened),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateEvent {
    pub event_id: String,
    pub action: StateAction,
    pub actor: Author,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadFilter {
    #[default]
    Unresolved,
    AwaitingAgent,
    AwaitingUser,
    Resolved,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope<'a> {
    Main { channel_id: &'a str },
    Thread { thread_id: &'a str },
}

/// `awaiting` is the side whose turn it is on an unresolved thread.
pub fn awaiting(resolved: bool, last_author: Author) -> Option<Author> {
    if resolved {
        return None;
    }
    Some(match last_author {
        Author::User => Author::Agent,
        Author::Agent => Author::User,
    })
}

pub fn clamp_limit(limit: Option<u32>, default: u32) -> u32 {
    limit.unwrap_or(default).clamp(1, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn awaiting_follows_last_author() {
        assert_eq!(awaiting(false, Author::User), Some(Author::Agent));
        assert_eq!(awaiting(false, Author::Agent), Some(Author::User));
        assert_eq!(awaiting(true, Author::User), None);
    }

    #[test]
    fn limit_is_rounded_into_range() {
        assert_eq!(clamp_limit(None, 50), 50);
        assert_eq!(clamp_limit(Some(0), 50), 1);
        assert_eq!(clamp_limit(Some(500), 50), 100);
    }
}
