//! Tool errors. Codes follow `docs/product-spec.md` 6.5.

use serde::Serialize;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Error {
    #[error("channel not found")]
    ChannelNotFound,
    #[error("invalid argument: {message}")]
    InvalidArgument { message: String },
    #[error("thread not found")]
    ThreadNotFound,
    #[error("message not found")]
    MessageNotFound,
    #[error("the parent must be a main timeline message")]
    ParentNotMain,
    #[error("the parent already has a thread")]
    ThreadExists { thread_id: String },
    #[error("the thread is resolved")]
    ThreadResolved {
        resolved_at: i64,
        resolved_by: Author,
        last_message_id: String,
    },
    #[error("the timeline has newer messages")]
    StaleAfter { current_last_message_id: String },
    #[error("after is required on a non-empty timeline")]
    AfterRequired,
    #[error("the last post is from the user")]
    Unanswered { last_message_id: String },
    #[error("the last post says the agent continues")]
    AgentContinues { last_message_id: String },
    #[error("the thread is already resolved")]
    AlreadyResolved {
        resolved_at: i64,
        resolved_by: Author,
    },
    #[error("the thread is already unresolved")]
    AlreadyUnresolved,
    #[error("invalid cursor")]
    InvalidCursor,
    #[error("invalid query")]
    InvalidQuery,
    #[error("text is empty")]
    EmptyText,
    #[error("text is too long")]
    TextTooLong,
    #[error("internal error: {message}")]
    Internal { message: String },
}

use crate::model::Author;
