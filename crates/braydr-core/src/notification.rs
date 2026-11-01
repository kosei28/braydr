//! Text sent to sessions (`docs/product-spec.md` 7.3 and 10.4).

use crate::Message;

pub fn new_post_main(channel_name: &str, message: &Message) -> String {
    format!(
        "[braydr] new post\nchannel_name: {channel_name}\nscope: main\nmessage_id: {}\n---\n{}",
        message.message_id, message.text
    )
}

pub fn new_post_thread(
    channel_name: &str,
    thread_id: &str,
    parent_message_id: &str,
    message: &Message,
    reopened: bool,
) -> String {
    let reopened = if reopened { "reopened: true\n" } else { "" };
    format!(
        "[braydr] new post\nchannel_name: {channel_name}\nscope: thread\nthread_id: {thread_id}\nmessage_id: {}\nparent_message_id: {parent_message_id}\n{reopened}---\n{}",
        message.message_id, message.text
    )
}

pub fn new_thread(
    channel_name: &str,
    thread_id: &str,
    parent_message_id: &str,
    message: &Message,
) -> String {
    format!(
        "[braydr] new thread\nchannel_name: {channel_name}\nthread_id: {thread_id}\nmessage_id: {}\nparent_message_id: {parent_message_id}\n---\n{}",
        message.message_id, message.text
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgeThread {
    pub thread_id: String,
    pub last_message_id: String,
    pub last_message_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgeMain {
    pub last_message_id: String,
    pub last_message_at: String,
}

pub fn nudge(
    channel_name: &str,
    threads: &[NudgeThread],
    main: Option<&NudgeMain>,
    more: bool,
) -> String {
    let mut text = format!("[braydr] nudge\nchannel_name: {channel_name}\n");
    if !threads.is_empty() {
        text.push_str("awaiting_agent_threads:\n");
        for thread in threads {
            text.push_str(&format!(
                "- thread_id: {}\n  last_message_id: {}\n  last_message_at: {}\n",
                thread.thread_id, thread.last_message_id, thread.last_message_at
            ));
        }
    }
    if let Some(main) = main {
        text.push_str(&format!(
            "main:\n  last_message_id: {}\n  last_message_at: {}\n",
            main.last_message_id, main.last_message_at
        ));
    }
    text.push_str(&format!("more: {more}"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Author;

    fn message() -> Message {
        Message {
            message_id: "m".into(),
            channel_id: "c".into(),
            thread_id: None,
            child_thread_id: None,
            author: Author::User,
            created_at: 0,
            text: "本文\n\"引用\"".into(),
        }
    }

    #[test]
    fn main_post_has_the_original_text() {
        assert_eq!(
            new_post_main("開発", &message()),
            "[braydr] new post\nchannel_name: 開発\nscope: main\nmessage_id: m\n---\n本文\n\"引用\""
        );
    }

    #[test]
    fn reopened_appears_only_when_reopened() {
        assert!(
            new_post_thread("開発", "t", "p", &message(), true).contains("reopened: true\n---")
        );
        assert!(!new_post_thread("開発", "t", "p", &message(), false).contains("reopened"));
    }

    #[test]
    fn nudge_omits_empty_sections() {
        let main = NudgeMain {
            last_message_id: "m".into(),
            last_message_at: "at".into(),
        };
        let text = nudge("開発", &[], Some(&main), false);
        assert!(!text.contains("awaiting_agent_threads"));
        assert!(text.ends_with("main:\n  last_message_id: m\n  last_message_at: at\nmore: false"));
        let thread = NudgeThread {
            thread_id: "t".into(),
            last_message_id: "m".into(),
            last_message_at: "at".into(),
        };
        let text = nudge("開発", &[thread], None, true);
        assert!(text.contains("awaiting_agent_threads:\n- thread_id: t"));
        assert!(!text.contains("main:"));
        assert!(text.ends_with("more: true"));
    }
}
