//! The `braydr_*` tools. Channel scope comes from the session, never from arguments.

use braydr_core::{Error, Result, Scope, ThreadFilter};
use braydr_store::Store;
use serde_json::{Value, json};

use crate::views;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
}

pub fn specs() -> Vec<ToolSpec> {
    let id = |what: &str| json!({ "type": "string", "description": what });
    let text = json!({ "type": "string", "description": "Markdown, 1-20000 characters." });
    let after = json!({ "type": "string", "description": "The newest message_id you read in this timeline. Required unless the timeline is empty." });
    let cursor = json!({ "type": "string", "description": "next_cursor from the previous page." });
    let limit = json!({ "type": "integer", "minimum": 1, "maximum": 100 });
    let continues = json!({ "type": "boolean", "description": "true if you will keep working on this after the post, whether or not the user replies (it stays in your queue and in nudges). false if this post finishes your turn and you wait for the user." });
    let spec = |name, description, schema| ToolSpec {
        name,
        description,
        schema,
    };
    vec![
        spec(
            "braydr_channel_get",
            "Channel name, counts, last main post, and whether the main timeline awaits you.",
            schema(json!({}), &[]),
        ),
        spec(
            "braydr_channel_list_posts",
            "Main timeline. Newest page first; each page runs oldest to newest.",
            schema(json!({ "cursor": cursor, "limit": limit }), &[]),
        ),
        spec(
            "braydr_thread_get",
            "Thread metadata and parent message.",
            schema(json!({ "thread_id": id("The thread.") }), &["thread_id"]),
        ),
        spec(
            "braydr_thread_list",
            "Threads, most recently updated first.",
            schema(
                json!({
                    "filter": { "type": "string", "enum": ["unresolved", "awaiting_agent", "awaiting_user", "resolved", "all"] },
                    "cursor": cursor,
                    "limit": limit,
                }),
                &[],
            ),
        ),
        spec(
            "braydr_thread_list_posts",
            "Parent message, thread state, resolution history, and replies.",
            schema(
                json!({ "thread_id": id("The thread."), "cursor": cursor, "limit": limit }),
                &["thread_id"],
            ),
        ),
        spec(
            "braydr_message_get",
            "One message, from the main timeline or a thread.",
            schema(json!({ "message_id": id("The message.") }), &["message_id"]),
        ),
        spec(
            "braydr_message_search",
            "Full-text search across the main timeline and all threads, newest first.",
            schema(
                json!({ "query": { "type": "string", "description": "1-200 characters." }, "cursor": cursor, "limit": limit }),
                &["query"],
            ),
        ),
        spec(
            "braydr_channel_create_post",
            "Post to the main timeline.",
            schema(
                json!({ "text": text, "agent_continues": continues, "after": after }),
                &["text", "agent_continues"],
            ),
        ),
        spec(
            "braydr_thread_create_post",
            "Reply in a thread.",
            schema(
                json!({ "thread_id": id("The thread."), "text": text, "agent_continues": continues, "after": after }),
                &["thread_id", "text", "agent_continues"],
            ),
        ),
        spec(
            "braydr_thread_create",
            "Start a thread on a main message and post its first reply.",
            schema(
                json!({ "parent_message_id": id("A main timeline message."), "text": text, "agent_continues": continues }),
                &["parent_message_id", "text", "agent_continues"],
            ),
        ),
        spec(
            "braydr_thread_resolve",
            "Resolve a thread. Posts no message. Fails while the last post is the user's or says you continue.",
            schema(
                json!({ "thread_id": id("The thread."), "after": after }),
                &["thread_id", "after"],
            ),
        ),
        spec(
            "braydr_thread_unresolve",
            "Reopen a resolved thread. Posts no message.",
            schema(json!({ "thread_id": id("The thread.") }), &["thread_id"]),
        ),
    ]
}

struct Args<'a>(&'a Value);

impl<'a> Args<'a> {
    fn optional(&self, key: &str) -> Result<Option<&'a str>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value)),
            Some(_) => Err(invalid(format!("{key} must be a string"))),
        }
    }

    fn required(&self, key: &str) -> Result<&'a str> {
        self.optional(key)?
            .ok_or_else(|| invalid(format!("{key} is required")))
    }

    fn flag(&self, key: &str) -> Result<bool> {
        match self.0.get(key) {
            None | Some(Value::Null) => Err(invalid(format!("{key} is required"))),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(invalid(format!("{key} must be a boolean"))),
        }
    }

    fn id(&self, key: &str) -> Result<&'a str> {
        let value = self.required(key)?;
        check_id(key, value)?;
        Ok(value)
    }

    fn optional_id(&self, key: &str) -> Result<Option<&'a str>> {
        let value = self.optional(key)?;
        if let Some(value) = value {
            check_id(key, value)?;
        }
        Ok(value)
    }

    fn limit(&self) -> Result<Option<u32>> {
        match self.0.get("limit") {
            None | Some(Value::Null) => Ok(None),
            Some(value) => {
                let limit = value
                    .as_i64()
                    .ok_or_else(|| invalid("limit must be an integer".into()))?;
                Ok(Some(limit.clamp(0, 1000) as u32))
            }
        }
    }

    fn filter(&self) -> Result<ThreadFilter> {
        Ok(match self.optional("filter")? {
            None | Some("unresolved") => ThreadFilter::Unresolved,
            Some("awaiting_agent") => ThreadFilter::AwaitingAgent,
            Some("awaiting_user") => ThreadFilter::AwaitingUser,
            Some("resolved") => ThreadFilter::Resolved,
            Some("all") => ThreadFilter::All,
            Some(other) => return Err(invalid(format!("unknown filter: {other}"))),
        })
    }
}

fn invalid(message: String) -> Error {
    Error::InvalidArgument { message }
}

fn check_id(key: &str, value: &str) -> Result<()> {
    uuid::Uuid::try_parse(value)
        .map(|_| ())
        .map_err(|_| invalid(format!("{key} is not a valid ID")))
}

pub fn call(store: &mut Store, channel: Option<&str>, tool: &str, args: &Value) -> Result<Value> {
    let channel = channel.ok_or(Error::ChannelNotFound)?;
    store.channel(channel)?;
    let args = Args(args);
    Ok(match tool {
        "braydr_channel_get" => views::summary(
            &store.channel_summary(channel)?,
            store.main_awaiting_agent(channel)?.as_ref(),
        ),
        "braydr_channel_list_posts" => views::message_page(&store.list_posts(
            Scope::Main {
                channel_id: channel,
            },
            args.optional("cursor")?,
            args.limit()?,
        )?),
        "braydr_thread_get" => views::thread(&store.thread(channel, args.id("thread_id")?)?),
        "braydr_thread_list" => views::thread_page(&store.list_threads(
            channel,
            args.filter()?,
            args.optional("cursor")?,
            args.limit()?,
        )?),
        "braydr_thread_list_posts" => {
            let thread_id = args.id("thread_id")?;
            let thread = store.thread(channel, thread_id)?;
            let page = store.list_posts(
                Scope::Thread { thread_id },
                args.optional("cursor")?,
                args.limit()?,
            )?;
            let events = store.state_events(thread_id)?;
            json!({
                "thread_id": thread.thread_id,
                "parent_message": views::message(&thread.parent_message),
                "messages": page.items.iter().map(views::message).collect::<Vec<_>>(),
                "resolved_at": thread.resolved_at.map(views::iso),
                "resolved_by": thread.resolved_by,
                "awaiting": thread.awaiting,
                "state_events": events.iter().map(views::state_event).collect::<Vec<_>>(),
                "has_more": page.has_more,
                "next_cursor": page.next_cursor,
            })
        }
        "braydr_message_get" => views::message(&store.message(channel, args.id("message_id")?)?),
        "braydr_message_search" => views::message_page(&store.search(
            channel,
            args.required("query")?,
            args.optional("cursor")?,
            args.limit()?,
        )?),
        "braydr_channel_create_post" => {
            let message = store.agent_post_main(
                channel,
                args.required("text")?,
                args.flag("agent_continues")?,
                args.optional_id("after")?,
            )?;
            json!({ "message": views::message(&message) })
        }
        "braydr_thread_create_post" => {
            let message = store.agent_post_thread(
                channel,
                args.id("thread_id")?,
                args.required("text")?,
                args.flag("agent_continues")?,
                args.optional_id("after")?,
            )?;
            json!({ "message": views::message(&message) })
        }
        "braydr_thread_create" => {
            let (thread, message) = store.agent_create_thread(
                channel,
                args.id("parent_message_id")?,
                args.required("text")?,
                args.flag("agent_continues")?,
            )?;
            json!({ "thread": views::thread(&thread), "message": views::message(&message) })
        }
        "braydr_thread_resolve" => views::thread(&store.agent_resolve(
            channel,
            args.id("thread_id")?,
            args.id("after")?,
        )?),
        "braydr_thread_unresolve" => {
            views::thread(&store.agent_unresolve(channel, args.id("thread_id")?)?)
        }
        other => return Err(invalid(format!("unknown tool: {other}"))),
    })
}
