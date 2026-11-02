//! Text sent to sessions (`docs/product-spec.md` 7.3 and 10.4).

use crate::{Author, Message};

pub fn new_post_main(message: &Message) -> String {
    format!(
        "[braydr] new post\nscope: main\nmessage_id: {}\n---\n{}",
        message.message_id, message.text
    )
}

pub fn new_post_thread(
    thread_id: &str,
    parent_message_id: &str,
    message: &Message,
    reopened: bool,
) -> String {
    let reopened = if reopened { "reopened: true\n" } else { "" };
    format!(
        "[braydr] new post\nscope: thread\nthread_id: {thread_id}\nmessage_id: {}\nparent_message_id: {parent_message_id}\n{reopened}---\n{}",
        message.message_id, message.text
    )
}

pub fn new_thread(thread_id: &str, parent_message_id: &str, message: &Message) -> String {
    format!(
        "[braydr] new thread\nthread_id: {thread_id}\nmessage_id: {}\nparent_message_id: {parent_message_id}\n---\n{}",
        message.message_id, message.text
    )
}

/// Why a nudge lists a timeline: the user's post is unanswered, or the agent said it continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeState {
    Unanswered,
    Continuing,
}

impl NudgeState {
    /// The state of a timeline that awaits the agent, from its last post's author.
    pub fn of(last_author: Author) -> Self {
        match last_author {
            Author::User => NudgeState::Unanswered,
            Author::Agent => NudgeState::Continuing,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            NudgeState::Unanswered => "unanswered",
            NudgeState::Continuing => "continuing",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgeThread {
    pub thread_id: String,
    pub state: NudgeState,
    pub last_message_id: String,
    pub last_message_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NudgeMain {
    pub state: NudgeState,
    pub last_message_id: String,
    pub last_message_at: String,
}

pub fn nudge(threads: &[NudgeThread], main: Option<&NudgeMain>, more: bool) -> String {
    let mut text = String::from("[braydr] nudge\n");
    if !threads.is_empty() {
        text.push_str("awaiting_agent_threads:\n");
        for thread in threads {
            text.push_str(&format!(
                "- thread_id: {}\n  state: {}\n  last_message_id: {}\n  last_message_at: {}\n",
                thread.thread_id,
                thread.state.as_str(),
                thread.last_message_id,
                thread.last_message_at
            ));
        }
    }
    if let Some(main) = main {
        text.push_str(&format!(
            "main:\n  state: {}\n  last_message_id: {}\n  last_message_at: {}\n",
            main.state.as_str(),
            main.last_message_id,
            main.last_message_at
        ));
    }
    text.push_str(&format!("more: {more}"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> Message {
        Message {
            message_id: "m".into(),
            channel_id: "c".into(),
            thread_id: None,
            child_thread_id: None,
            author: Author::User,
            created_at: 0,
            text: "本文\n\"引用\"".into(),
            agent_continues: false,
        }
    }

    #[test]
    fn main_post_has_the_original_text() {
        assert_eq!(
            new_post_main(&message()),
            "[braydr] new post\nscope: main\nmessage_id: m\n---\n本文\n\"引用\""
        );
    }

    #[test]
    fn reopened_appears_only_when_reopened() {
        assert!(new_post_thread("t", "p", &message(), true).contains("reopened: true\n---"));
        assert!(!new_post_thread("t", "p", &message(), false).contains("reopened"));
    }

    #[test]
    fn nudge_omits_empty_sections() {
        let main = NudgeMain {
            state: NudgeState::Continuing,
            last_message_id: "m".into(),
            last_message_at: "at".into(),
        };
        let text = nudge(&[], Some(&main), false);
        assert!(!text.contains("awaiting_agent_threads"));
        assert!(text.ends_with(
            "main:\n  state: continuing\n  last_message_id: m\n  last_message_at: at\nmore: false"
        ));
        let thread = NudgeThread {
            thread_id: "t".into(),
            state: NudgeState::Unanswered,
            last_message_id: "m".into(),
            last_message_at: "at".into(),
        };
        let text = nudge(&[thread], None, true);
        assert!(text.contains("awaiting_agent_threads:\n- thread_id: t"));
        assert!(!text.contains("main:"));
        assert!(text.ends_with("more: true"));
    }
}
