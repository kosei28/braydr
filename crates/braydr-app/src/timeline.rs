//! One scrolling timeline and its read tracking.

use std::sync::Arc;

use braydr_core::read::{self, Laid};
use braydr_core::{Message, Scope, StateEvent, Thread};
use braydr_herdr::Client;
use braydr_runtime::Runtime;
use gpui_kit::{ListAlignment, ListOffset, ListState, px};

pub type Rt = Arc<Runtime<Client>>;

#[derive(Clone, PartialEq, Eq, Hash)]
pub enum OwnedScope {
    Main(String),
    Thread(String),
}

impl OwnedScope {
    pub fn as_scope(&self) -> Scope<'_> {
        match self {
            OwnedScope::Main(channel_id) => Scope::Main { channel_id },
            OwnedScope::Thread(thread_id) => Scope::Thread { thread_id },
        }
    }

    pub fn of(message: &Message) -> Self {
        match &message.thread_id {
            Some(thread_id) => OwnedScope::Thread(thread_id.clone()),
            None => OwnedScope::Main(message.channel_id.clone()),
        }
    }
}

#[derive(Clone)]
pub struct ThreadLink {
    pub thread: Thread,
    pub unread: bool,
}

#[derive(Clone)]
pub enum Row {
    /// A thread's parent. It scrolls with the replies but belongs to the main timeline, so it
    /// takes no part in the thread's read position.
    Parent(Message),
    Message(Message, Option<Box<ThreadLink>>),
    Event(StateEvent),
    Unread,
}

impl Row {
    fn same_item(&self, other: &Row) -> bool {
        match (self, other) {
            (Row::Message(a, _), Row::Message(b, _)) => a.message_id == b.message_id,
            (Row::Event(a), Row::Event(b)) => a.event_id == b.event_id,
            (Row::Parent(_), Row::Parent(_)) | (Row::Unread, Row::Unread) => true,
            _ => false,
        }
    }
}

pub struct Timeline {
    pub scope: OwnedScope,
    pub rows: Arc<Vec<Row>>,
    pub list: ListState,
    parent: Option<Message>,
    divider_before: Option<String>,
}

impl Timeline {
    pub fn open(rt: &Rt, scope: OwnedScope, parent: Option<Message>) -> Self {
        let divider_before = rt.store().first_unread(scope.as_scope()).ok().flatten();
        // Main reads from the bottom up; a thread reads down from its parent.
        let alignment = match scope {
            OwnedScope::Main(_) => ListAlignment::Bottom,
            OwnedScope::Thread(_) => ListAlignment::Top,
        };
        let mut timeline = Self {
            scope,
            rows: Arc::new(Vec::new()),
            list: ListState::new(0, alignment, px(600.)),
            parent,
            divider_before,
        };
        timeline.reload(rt);
        let divider = timeline
            .rows
            .iter()
            .position(|row| matches!(row, Row::Unread));
        // When every reply is unread, keep the parent they answer in view.
        let first_row = timeline.parent.is_some().then_some(1);
        match divider {
            Some(row) => timeline.list.scroll_to(ListOffset {
                item_ix: if Some(row) == first_row { 0 } else { row },
                offset_in_item: px(0.),
            }),
            None => timeline.list.scroll_to_end(),
        }
        timeline
    }

    pub fn reload(&mut self, rt: &Rt) {
        let mut messages = Vec::new();
        let mut cursor = None;
        loop {
            let Ok(page) =
                rt.store()
                    .list_posts(self.scope.as_scope(), cursor.as_deref(), Some(100))
            else {
                break;
            };
            let more = page.next_cursor.clone();
            messages.splice(0..0, page.items);
            match more {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let events = match &self.scope {
            OwnedScope::Thread(thread_id) => rt.store().state_events(thread_id).unwrap_or_default(),
            OwnedScope::Main(_) => Vec::new(),
        };
        let mut rows = Vec::with_capacity(messages.len() + events.len() + 2);
        rows.extend(self.parent.clone().map(Row::Parent));
        let mut events = events.into_iter().peekable();
        for message in messages {
            while events
                .peek()
                .is_some_and(|event| event.event_id < message.message_id)
            {
                rows.push(Row::Event(events.next().expect("peeked")));
            }
            if self.divider_before.as_deref() == Some(message.message_id.as_str()) {
                rows.push(Row::Unread);
            }
            let link = thread_link(rt, &message);
            rows.push(Row::Message(message, link));
        }
        rows.extend(events.map(Row::Event));
        // Timelines only grow, so keep the unchanged start and with it the scroll position.
        let kept = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(old, new)| old.same_item(new))
            .count();
        let (old, new) = (self.rows.len(), rows.len());
        let at_end = self.is_at_bottom();
        self.rows = Arc::new(rows);
        if kept != old || kept != new {
            self.list.splice(kept..old, new - kept);
            if at_end {
                self.list.scroll_to_end();
            }
        }
    }

    pub fn is_at_bottom(&self) -> bool {
        let Some(last) = self.rows.len().checked_sub(1) else {
            return true;
        };
        let viewport = self.list.viewport_bounds();
        self.list
            .bounds_for_item(last)
            .is_some_and(|bounds| bounds.bottom() <= viewport.bottom() + px(2.))
    }

    /// Scrolls so the message is at the top of the view.
    pub fn reveal(&self, message_id: &str) {
        let row = self.rows.iter().position(
            |row| matches!(row, Row::Message(message, _) if message.message_id == message_id),
        );
        if let Some(item_ix) = row {
            self.list.scroll_to(ListOffset {
                item_ix,
                offset_in_item: px(0.),
            });
        }
    }

    /// Advances the read position to what the user has actually seen.
    pub fn track_read(&self, rt: &Rt) {
        let viewport = self.list.viewport_bounds();
        let mut laid = Vec::new();
        let mut ix = self.list.logical_scroll_top().item_ix;
        while let (Some(row), Some(bounds)) = (self.rows.get(ix), self.list.bounds_for_item(ix)) {
            if bounds.top() >= viewport.bottom() {
                break;
            }
            if let Row::Message(message, _) = row {
                laid.push((message.message_id.clone(), f32::from(bounds.bottom())));
            }
            ix += 1;
        }
        let refs: Vec<Laid<'_>> = laid
            .iter()
            .map(|(id, bottom)| Laid {
                message_id: id,
                bottom: *bottom,
            })
            .collect();
        let Some(seen) = read::newest_seen(&refs, f32::from(viewport.bottom())) else {
            return;
        };
        let scope = self.scope.as_scope();
        let current = rt.store().read_position(scope).ok().flatten();
        if read::advance(current.as_deref(), Some(seen)) != current.as_deref()
            && rt.store().mark_read(scope, seen).is_ok()
        {
            rt.bump();
        }
    }
}

fn thread_link(rt: &Rt, message: &Message) -> Option<Box<ThreadLink>> {
    let thread_id = message.child_thread_id.as_ref()?;
    let store = rt.store();
    let thread = store.thread(&message.channel_id, thread_id).ok()?;
    let unread = store
        .has_unread(Scope::Thread { thread_id })
        .unwrap_or(false);
    Some(Box::new(ThreadLink { thread, unread }))
}
