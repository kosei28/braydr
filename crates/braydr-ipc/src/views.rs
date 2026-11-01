//! JSON shapes returned to agents. Times are UTC ISO 8601 with milliseconds.

use braydr_core::{Error, Message, Page, StateEvent, Thread};
use braydr_store::ChannelSummary;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::macros::format_description;

pub fn iso(ms: i64) -> String {
    let format =
        format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z");
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .expect("the time is in range")
        .format(&format)
        .expect("the format is valid")
}

pub fn message(m: &Message) -> Value {
    json!({
        "message_id": m.message_id,
        "channel_id": m.channel_id,
        "thread_id": m.thread_id,
        "child_thread_id": m.child_thread_id,
        "author": m.author,
        "created_at": iso(m.created_at),
        "text": m.text,
    })
}

pub fn thread(t: &Thread) -> Value {
    json!({
        "thread_id": t.thread_id,
        "channel_id": t.channel_id,
        "parent_message_id": t.parent_message_id,
        "parent_message": message(&t.parent_message),
        "created_at": iso(t.created_at),
        "updated_at": iso(t.updated_at),
        "last_message_id": t.last_message_id,
        "last_message_at": iso(t.last_message_at),
        "last_author": t.last_author,
        "post_count": t.post_count,
        "reply_count": t.reply_count,
        "resolved_at": t.resolved_at.map(iso),
        "resolved_by": t.resolved_by,
        "awaiting": t.awaiting,
    })
}

pub fn state_event(e: &StateEvent) -> Value {
    json!({
        "event_id": e.event_id,
        "action": e.action,
        "actor": e.actor,
        "created_at": iso(e.created_at),
    })
}

pub fn summary(s: &ChannelSummary) -> Value {
    json!({
        "channel_id": s.channel.channel_id,
        "name": s.channel.name,
        "post_count": s.post_count,
        "updated_at": s.updated_at.map(iso),
        "last_author": s.last_author,
        "thread_count_total": s.thread_count_total,
        "thread_count_unresolved": s.thread_count_unresolved,
        "thread_count_resolved": s.thread_count_resolved,
        "thread_count_awaiting_agent": s.thread_count_awaiting_agent,
        "thread_count_awaiting_user": s.thread_count_awaiting_user,
    })
}

pub fn message_page(page: &Page<Message>) -> Value {
    json!({
        "messages": page.items.iter().map(message).collect::<Vec<_>>(),
        "has_more": page.has_more,
        "next_cursor": page.next_cursor,
    })
}

pub fn thread_page(page: &Page<Thread>) -> Value {
    json!({
        "threads": page.items.iter().map(thread).collect::<Vec<_>>(),
        "has_more": page.has_more,
        "next_cursor": page.next_cursor,
    })
}

/// `{"code", "message", "details"}`.
pub fn error(error: &Error) -> Value {
    let mut fields = serde_json::to_value(error).expect("errors serialize");
    let object = fields.as_object_mut().expect("errors are tagged objects");
    let code = object.remove("code").expect("errors are tagged");
    if let Some(at) = object.get("resolved_at").and_then(Value::as_i64) {
        object.insert("resolved_at".into(), json!(iso(at)));
    }
    let mut value = json!({ "code": code, "message": error.to_string() });
    if !object.is_empty() {
        value["details"] = fields;
    }
    value
}
