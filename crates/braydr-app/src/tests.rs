//! Workspace behavior through real key dispatch, with no herdr-core behind it.

use std::sync::Arc;
use std::time::Duration;

use braydr_core::{LaunchConfig, Scope};
use braydr_herdr::Client;
use braydr_runtime::Runtime;
use braydr_store::Store;
use gpui_kit::{
    Entity, ListOffset, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px,
};

use crate::overlay::Overlay;
use crate::pane_widths::{Edge, PaneWidths};
use crate::timeline::{OwnedScope, Row, Rt};
use crate::workspace::Workspace;

fn launch() -> LaunchConfig {
    LaunchConfig {
        cwd: "/tmp".into(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    }
}

fn runtime() -> Rt {
    Runtime::new(
        Store::in_memory().unwrap(),
        Arc::new(Client::new("/nonexistent/braydr-test.sock")),
        Box::new(|| 1_000),
    )
}

fn open<'a>(cx: &'a mut TestAppContext, rt: &Rt) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::apply_theme(cx);
        crate::bind_keys(cx);
    });
    let rt = rt.clone();
    let (view, cx) = cx.add_window_view(|window, cx| Workspace::new(rt, window, cx));
    cx.run_until_parked();
    (view, cx)
}

fn poll(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
}

fn main_texts(rt: &Rt, channel: &str) -> Vec<String> {
    rt.store()
        .list_posts(
            Scope::Main {
                channel_id: channel,
            },
            None,
            Some(100),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|message| message.text)
        .collect()
}

#[gpui_kit::test]
fn command_enter_posts_and_plain_enter_does_not(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let (_, cx) = open(cx, &rt);
    cx.simulate_input("一行目");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(main_texts(&rt, &channel).is_empty());
    cx.simulate_input("二行目");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert_eq!(main_texts(&rt, &channel), ["一行目\n二行目"]);
}

#[gpui_kit::test]
fn search_opens_finds_and_closes_from_inside_an_input(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    rt.store().user_post_main(&channel, "ログイン修正").unwrap();
    rt.store().user_post_main(&channel, "別の話").unwrap();
    let (view, cx) = open(cx, &rt);
    cx.simulate_keystrokes("cmd-f");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(matches!(this.overlay, Some(Overlay::Search(_))));
    });
    cx.simulate_input("ログイン");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        let Some(Overlay::Search(panel)) = &this.overlay else {
            panic!("search closed");
        };
        assert_eq!(panel.result_texts(), ["ログイン修正"]);
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.overlay.is_none()));
    // Typing returns to the composer, not the closed search.
    cx.simulate_input("次");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(main_texts(&rt, &channel).contains(&"次".to_owned()));
}

#[gpui_kit::test]
fn typing_with_a_panel_open_stays_out_of_the_composer(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let (view, cx) = open(cx, &rt);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.open_menu(window, cx));
    });
    cx.simulate_input("x");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(main_texts(&rt, &channel).is_empty());
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.overlay.is_none()));
}

#[gpui_kit::test]
fn drafts_stay_with_their_channel(cx: &mut TestAppContext) {
    let rt = runtime();
    let (first, second) = {
        let mut store = rt.store();
        let first = store.create_channel("a", &launch()).unwrap().channel_id;
        let second = store.create_channel("b", &launch()).unwrap().channel_id;
        (first, second)
    };
    let (view, cx) = open(cx, &rt);
    let selected = view.read_with(cx, |this, _| this.selected.clone().unwrap());
    let other = if selected == first {
        second.clone()
    } else {
        first.clone()
    };
    cx.simulate_input("下書き");
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_tag(&format!("main:{other}"), window, cx)
        });
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(main_texts(&rt, &other).is_empty());
    assert!(main_texts(&rt, &selected).is_empty());
}

fn thread_texts(rt: &Rt, channel: &str, parent: &str) -> Vec<String> {
    let store = rt.store();
    let Some(thread) = store.thread_for_parent(channel, parent).unwrap() else {
        return Vec::new();
    };
    store
        .list_posts(
            Scope::Thread {
                thread_id: &thread.thread_id,
            },
            None,
            Some(100),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|message| message.text)
        .collect()
}

#[gpui_kit::test]
fn reply_drafts_stay_with_their_parent(cx: &mut TestAppContext) {
    let rt = runtime();
    let (first, second) = {
        let mut store = rt.store();
        let first = store.create_channel("a", &launch()).unwrap().channel_id;
        let second = store.create_channel("b", &launch()).unwrap().channel_id;
        (first, second)
    };
    let (view, cx) = open(cx, &rt);
    let selected = view.read_with(cx, |this, _| this.selected.clone().unwrap());
    let other = if selected == first { second } else { first };
    // One parent has no thread yet, so its reply is a draft that would start one.
    let (unthreaded, threaded) = {
        let mut store = rt.store();
        let unthreaded = store.user_post_main(&selected, "親1").unwrap();
        let threaded = store.user_post_main(&selected, "親2").unwrap();
        store
            .user_reply(&selected, &threaded.message_id, "最初の返信")
            .unwrap();
        (unthreaded, threaded)
    };
    rt.bump();
    poll(cx);

    cx.simulate_input("メイン");
    for (parent, text) in [(&unthreaded, "下書き1"), (&threaded, "下書き2")] {
        let parent = parent.clone();
        cx.update(|window, cx| {
            view.update(cx, |this, cx| this.open_thread(parent, window, cx));
        });
        cx.run_until_parked();
        cx.simulate_input(text);
    }
    // Coming back to the channel reopens the last thread it had open.
    for channel in [&other, &selected] {
        cx.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.open_tag(&format!("main:{channel}"), window, cx);
            });
        });
        cx.run_until_parked();
    }
    // Its reply box holds the unsent reply, while typing goes to the main composer.
    view.read_with(cx, |this, cx| {
        let pane = this.pane.as_ref().unwrap();
        assert_eq!(pane.parent().message_id, threaded.message_id);
        assert_eq!(pane.reply_text(cx), "下書き2");
    });

    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert_eq!(main_texts(&rt, &selected), ["親1", "親2", "メイン"]);
    for (parent, expected) in [
        (&unthreaded, vec!["下書き1"]),
        (&threaded, vec!["最初の返信", "下書き2"]),
    ] {
        let parent = parent.clone();
        let parent_id = parent.message_id.clone();
        cx.update(|window, cx| {
            view.update(cx, |this, cx| this.open_thread(parent, window, cx));
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("cmd-enter");
        cx.run_until_parked();
        assert_eq!(thread_texts(&rt, &selected, &parent_id), expected);
    }
}

#[gpui_kit::test]
fn a_failed_start_keeps_the_channel_and_offers_retry(cx: &mut TestAppContext) {
    let rt = runtime();
    let (channel, failure) = rt.create_channel("c", &launch()).unwrap();
    assert!(failure.is_some());
    let (view, cx) = open(cx, &rt);
    view.read_with(cx, |this, _| {
        let row = this.selected_row().unwrap();
        assert_eq!(row.channel.channel_id, channel.channel_id);
        assert!(row.stopped && row.start_failed);
    });
}

#[gpui_kit::test]
fn notifies_for_other_timelines_and_clears_on_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let (first, second) = {
        let mut store = rt.store();
        let first = store.create_channel("a", &launch()).unwrap().channel_id;
        let second = store.create_channel("b", &launch()).unwrap().channel_id;
        (first, second)
    };
    let (view, cx) = open(cx, &rt);
    let selected = view.read_with(cx, |this, _| this.selected.clone().unwrap());
    let other = if selected == first { second } else { first };
    rt.store()
        .agent_post_main(&other, "できました", false, None)
        .unwrap();
    rt.bump();
    poll(cx);
    let tag = format!("main:{other}");
    view.read_with(cx, |this, _| assert!(this.notified.contains(&tag)));
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.open_tag(&tag, window, cx));
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(other.as_str()));
        assert!(!this.notified.contains(&tag));
    });
}

#[gpui_kit::test]
fn opening_a_thread_from_a_tag_shows_its_channel(cx: &mut TestAppContext) {
    let rt = runtime();
    let (first, second) = {
        let mut store = rt.store();
        let first = store.create_channel("a", &launch()).unwrap().channel_id;
        let second = store.create_channel("b", &launch()).unwrap().channel_id;
        (first, second)
    };
    let (view, cx) = open(cx, &rt);
    let selected = view.read_with(cx, |this, _| this.selected.clone().unwrap());
    let other = if selected == first { second } else { first };
    let thread_id = {
        let mut store = rt.store();
        let parent = store.user_post_main(&other, "親").unwrap();
        store
            .user_reply(&other, &parent.message_id, "返信")
            .unwrap()
            .thread
            .thread_id
    };
    cx.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.open_tag(&format!("thread:{thread_id}"), window, cx);
        });
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(other.as_str()));
        assert!(this.pane.is_some());
    });
}

#[gpui_kit::test]
fn deleting_moves_to_another_channel_then_to_the_creation_form(cx: &mut TestAppContext) {
    let rt = runtime();
    let first = rt
        .store()
        .create_channel("一", &launch())
        .unwrap()
        .channel_id;
    let second = rt
        .store()
        .create_channel("二", &launch())
        .unwrap()
        .channel_id;
    let (view, cx) = open(cx, &rt);

    for _ in 0..2 {
        view.update_in(cx, |this, window, cx| {
            let selected = this.selected.clone().unwrap();
            let thread_ids = this.rt.delete_channel(&selected).unwrap();
            this.finish_delete(&selected, &thread_ids, window, cx);
        });
        cx.run_until_parked();
        view.read_with(cx, |this, _| {
            if this.rt.store().channels().unwrap().is_empty() {
                assert!(this.create.is_some());
                assert!(this.selected.is_none());
            } else {
                let selected = this.selected.as_deref().unwrap();
                assert!(selected == first || selected == second);
                assert!(this.timeline.is_some());
            }
        });
    }
    assert!(rt.store().channels().unwrap().is_empty());
}

#[gpui_kit::test]
fn a_notification_for_a_deleted_channel_opens_nothing(cx: &mut TestAppContext) {
    let rt = runtime();
    let kept = rt
        .store()
        .create_channel("残す", &launch())
        .unwrap()
        .channel_id;
    let (view, cx) = open(cx, &rt);
    view.update_in(cx, |this, window, cx| {
        this.open_tag("main:gone", window, cx);
    });
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(kept.as_str()));
    });
}

#[gpui_kit::test]
fn the_unread_panel_lists_other_channels_and_closes_on_escape(cx: &mut TestAppContext) {
    let rt = runtime();
    let other = rt
        .store()
        .create_channel("別", &launch())
        .unwrap()
        .channel_id;
    rt.store()
        .agent_post_main(&other, "報告", false, None)
        .unwrap();
    rt.store().create_channel("今", &launch()).unwrap();
    let (view, cx) = open(cx, &rt);

    view.update_in(cx, |this, window, cx| this.open_unread(window, cx));
    view.read_with(cx, |this, _| {
        assert!(matches!(this.overlay, Some(Overlay::Unread(_))));
    });
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.overlay.is_none()));

    view.update_in(cx, |this, window, cx| {
        this.open_unread(window, cx);
        this.overlay = None;
        this.open_tag(&format!("main:{other}"), window, cx);
    });
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(other.as_str()));
    });
}

#[gpui_kit::test]
fn with_no_channels_the_creation_form_opens_and_stays(cx: &mut TestAppContext) {
    let rt = runtime();
    let (view, cx) = open(cx, &rt);
    view.read_with(cx, |this, _| assert!(this.create.is_some()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.create.is_some()));
}

#[gpui_kit::test]
fn escape_leaves_the_creation_form_for_a_channel(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let (view, cx) = open(cx, &rt);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.start_create(window, cx));
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.create.is_some()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(this.create.is_none());
        assert_eq!(this.selected.as_deref(), Some(channel.as_str()));
    });
}

#[gpui_kit::test]
fn a_threads_parent_scrolls_with_it_outside_its_read_position(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let (parent, thread_id) = {
        let mut store = rt.store();
        let parent = store.user_post_main(&channel, "親").unwrap();
        let reply = store
            .user_reply(&channel, &parent.message_id, "依頼")
            .unwrap();
        // Everything in the thread is unread: only an agent reply, with nothing read before it.
        store
            .agent_post_thread(
                &channel,
                &reply.thread.thread_id,
                "返事",
                false,
                Some(&reply.message.message_id),
            )
            .unwrap();
        (parent, reply.thread.thread_id)
    };
    let (view, cx) = open(cx, &rt);
    cx.update(|window, cx| {
        window.activate_window();
        view.update(cx, |this, cx| this.open_thread(parent.clone(), window, cx));
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
        assert!(matches!(timeline.rows[0], Row::Parent(_)));
        assert!(matches!(timeline.rows[1], Row::Message(..)));
    });
    let position = rt.store().read_position(Scope::Thread {
        thread_id: &thread_id,
    });
    assert_ne!(
        position.unwrap().as_deref(),
        Some(parent.message_id.as_str())
    );
}

#[gpui_kit::test]
fn the_unresolved_list_closes_when_nothing_is_left(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let thread_id = {
        let mut store = rt.store();
        let parent = store.user_post_main(&channel, "親").unwrap();
        store
            .user_reply(&channel, &parent.message_id, "返信")
            .unwrap()
            .thread
            .thread_id
    };
    let (view, cx) = open(cx, &rt);
    cx.update(|window, cx| {
        view.update(cx, |this, cx| this.open_unresolved(window, cx));
    });
    cx.run_until_parked();
    poll(cx);
    view.read_with(cx, |this, _| assert!(this.overlay.is_some()));
    rt.store().user_resolve(&channel, &thread_id).unwrap();
    rt.bump();
    poll(cx);
    view.read_with(cx, |this, _| assert!(this.overlay.is_none()));
}

#[gpui_kit::test]
fn copying_shows_a_check_for_a_moment(cx: &mut TestAppContext) {
    let rt = runtime();
    rt.store().create_channel("c", &launch()).unwrap();
    let (view, cx) = open(cx, &rt);
    view.update(cx, |this, cx| {
        this.copy("first".to_owned(), "本文".to_owned(), cx);
    });
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.copied.as_deref(), Some("first"))
    });
    let clipboard = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(clipboard.as_deref(), Some("本文"));

    // A second copy restarts the time rather than ending with the first one's timer.
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1000));
    view.update(cx, |this, cx| {
        this.copy("second".to_owned(), "別".to_owned(), cx);
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(1000));
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.copied.as_deref(), Some("second"))
    });
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.copied.is_none()));
}

#[gpui_kit::test]
fn dragging_a_channel_row_reorders_and_clicking_still_selects(cx: &mut TestAppContext) {
    let rt = runtime();
    for name in ["c", "b", "a"] {
        rt.store().create_channel(name, &launch()).unwrap();
    }
    let names = |rt: &Rt| -> Vec<String> {
        let channels = rt.store().channels().unwrap();
        channels.into_iter().map(|channel| channel.name).collect()
    };
    let (view, cx) = open(cx, &rt);
    let none = Modifiers::none();
    let first = cx.debug_bounds("channel-row-0").unwrap().center();
    let last = cx.debug_bounds("channel-row-2").unwrap().center();
    let drag = |cx: &mut VisualTestContext, from, to| {
        cx.simulate_mouse_move(from, None, none);
        cx.simulate_mouse_down(from, MouseButton::Left, none);
        cx.simulate_mouse_move(to, MouseButton::Left, none);
        // Once to start the drag and draw the drop targets, once to land on them.
        cx.run_until_parked();
        cx.simulate_mouse_move(to, MouseButton::Left, none);
        cx.run_until_parked();
        cx.simulate_mouse_up(to, MouseButton::Left, none);
        cx.run_until_parked();
    };

    drag(cx, first, last);
    assert_eq!(names(&rt), ["b", "c", "a"], "downward lands below the row");
    drag(cx, last, first);
    assert_eq!(names(&rt), ["a", "b", "c"], "upward lands above the row");

    let target = rt.store().channels().unwrap()[1].channel_id.clone();
    let row = cx.debug_bounds("channel-row-1").unwrap().center();
    cx.simulate_mouse_move(row, None, none);
    cx.simulate_mouse_down(row, MouseButton::Left, none);
    cx.simulate_mouse_up(row, MouseButton::Left, none);
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(target.as_str()))
    });
    assert_eq!(names(&rt), ["a", "b", "c"]);
}

#[gpui_kit::test]
fn dragging_a_pane_edge_resizes_and_remembers(cx: &mut TestAppContext) {
    let rt = runtime();
    rt.store().create_channel("c", &launch()).unwrap();
    let (view, cx) = open(cx, &rt);
    let start = view.read_with(cx, |this, _| this.shown.sidebar);
    let at = |x: f32| point(px(x), px(100.));
    let none = Modifiers::none();
    // The grip is the sidebar's last 8 points.
    cx.simulate_mouse_move(at(start - 4.), None, none);
    cx.simulate_mouse_down(at(start - 4.), MouseButton::Left, none);
    cx.simulate_mouse_move(at(start + 56.), MouseButton::Left, none);
    cx.simulate_mouse_up(at(start + 56.), MouseButton::Left, none);
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.shown.sidebar),
        start + 60.
    );
    assert_eq!(PaneWidths::load(&rt).sidebar, start + 60.);
    // Further moves after the release do nothing.
    cx.simulate_mouse_move(at(start + 90.), None, none);
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |this, _| this.shown.sidebar),
        start + 60.
    );
    // Double-clicking the edge goes back to the default.
    view.update(cx, |this, cx| this.reset_width(Edge::Sidebar, cx));
    cx.run_until_parked();
    assert_eq!(PaneWidths::load(&rt).sidebar, start);
}

#[gpui_kit::test]
fn a_release_under_a_cover_still_counts(cx: &mut TestAppContext) {
    let rt = runtime();
    rt.store().create_channel("c", &launch()).unwrap();
    let (view, cx) = open(cx, &rt);
    let start = view.read_with(cx, |this, _| this.shown.sidebar);
    let at = point(px(start - 4.), px(100.));
    let none = Modifiers::none();
    // The grip's cover occludes the workspace while the button comes up.
    cx.simulate_mouse_move(at, None, none);
    cx.simulate_mouse_down(at, MouseButton::Left, none);
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.pointer_down));
    cx.simulate_mouse_up(at, MouseButton::Left, none);
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(!this.pointer_down));
}

#[gpui_kit::test]
fn a_release_outside_the_window_shows_on_the_next_move(cx: &mut TestAppContext) {
    let rt = runtime();
    rt.store().create_channel("c", &launch()).unwrap();
    let (view, cx) = open(cx, &rt);
    let at = point(px(600.), px(100.));
    let none = Modifiers::none();
    cx.simulate_mouse_move(at, None, none);
    cx.simulate_mouse_down(at, MouseButton::Left, none);
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(this.pointer_down));
    // No release arrives. The pointer simply comes back with the button up.
    cx.simulate_mouse_move(at, None, none);
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(!this.pointer_down));
}

#[gpui_kit::test]
fn a_timeline_at_its_end_reads_up_to_the_newest_message(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let first = rt.store().user_post_main(&channel, "最初").unwrap();
    let reply = rt
        .store()
        .agent_post_main(&channel, "返事", false, Some(&first.message_id))
        .unwrap();
    let scope = Scope::Main {
        channel_id: &channel,
    };
    // Open with nothing unread, then get a reply: the list stays anchored at its end.
    rt.store().mark_read(scope, &reply.message_id).unwrap();
    let (view, cx) = open(cx, &rt);
    let newest = rt
        .store()
        .agent_post_main(&channel, "続き", false, Some(&reply.message_id))
        .unwrap();
    rt.bump();
    poll(cx);
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        let timeline = this.timeline.as_ref().unwrap();
        assert!(timeline.is_at_bottom());
        timeline.track_read(&rt);
    });
    assert_eq!(
        rt.store().read_position(scope).unwrap().as_deref(),
        Some(newest.message_id.as_str())
    );
}

#[gpui_kit::test]
fn a_code_blocks_button_copies_its_code(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    rt.store()
        .agent_post_main(
            &channel,
            "例です。\n\n```rust\nfn main() {}\n```",
            false,
            None,
        )
        .unwrap();
    let (view, cx) = open(cx, &rt);
    settle(cx);
    // Hidden until the pointer is over the block, which ends the message.
    assert!(cx.debug_bounds("code-copy").is_none());
    let body = cx.debug_bounds("agent-body").unwrap();
    let over = gpui_kit::point(body.center().x, body.bottom() - px(8.));
    cx.simulate_mouse_move(over, None, Modifiers::none());
    settle(cx);
    click(cx, "code-copy");
    let clipboard = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(clipboard.as_deref(), Some("fn main() {}"));
    view.read_with(cx, |this, _| {
        assert!(
            this.copied
                .as_deref()
                .is_some_and(|key| key.starts_with("code-"))
        )
    });
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).unwrap().center();
    let none = Modifiers::none();
    cx.simulate_mouse_move(at, None, none);
    cx.simulate_mouse_down(at, MouseButton::Left, none);
    cx.simulate_mouse_up(at, MouseButton::Left, none);
    cx.run_until_parked();
}

/// Two channels, opened on the first in the list, which gets a thread.
fn two_channels_with_a_thread(rt: &Rt) -> (String, String, braydr_core::Message, String) {
    let mut store = rt.store();
    store.create_channel("後", &launch()).unwrap();
    store.create_channel("先", &launch()).unwrap();
    let channels = store.channels().unwrap();
    let (shown, other) = (
        channels[0].channel_id.clone(),
        channels[1].channel_id.clone(),
    );
    let parent = store.user_post_main(&shown, "親").unwrap();
    let thread_id = store
        .user_reply(&shown, &parent.message_id, "返信")
        .unwrap()
        .thread
        .thread_id;
    (shown, other, parent, thread_id)
}

fn sheet_parent(view: &Entity<Workspace>, cx: &mut VisualTestContext) -> Option<String> {
    view.read_with(cx, |this, _| {
        this.pane
            .as_ref()
            .map(|pane| pane.parent().message_id.clone())
    })
}

#[gpui_kit::test]
fn a_channel_reopens_the_thread_it_had_open(cx: &mut TestAppContext) {
    let rt = runtime();
    let (shown, other, parent, thread_id) = two_channels_with_a_thread(&rt);
    let (view, cx) = open(cx, &rt);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });

    click(cx, "channel-row-1");
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(other.as_str()));
    });
    assert_eq!(sheet_parent(&view, cx), None, "the other channel had none");

    click(cx, "channel-row-0");
    view.read_with(cx, |this, _| {
        assert_eq!(this.selected.as_deref(), Some(shown.as_str()));
        let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
        assert!(timeline.scope == OwnedScope::Thread(thread_id.clone()));
    });
    // Typing still goes to the main composer.
    cx.simulate_input("メインへ");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(main_texts(&rt, &shown).contains(&"メインへ".to_owned()));

    // Closing the sheet forgets it.
    view.update_in(cx, |this, window, cx| this.close_thread(window, cx));
    click(cx, "channel-row-1");
    click(cx, "channel-row-0");
    assert_eq!(sheet_parent(&view, cx), None);
}

#[gpui_kit::test]
fn a_reply_draft_comes_back_as_the_thread_made_meanwhile(cx: &mut TestAppContext) {
    let rt = runtime();
    let (shown, _, _, _) = two_channels_with_a_thread(&rt);
    let lone = rt.store().user_post_main(&shown, "まだ返信なし").unwrap();
    let (view, cx) = open(cx, &rt);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(lone.clone(), window, cx)
    });
    view.read_with(cx, |this, _| {
        assert!(this.pane.as_ref().unwrap().timeline.is_none());
    });

    click(cx, "channel-row-1");
    let thread_id = rt
        .store()
        .user_reply(&shown, &lone.message_id, "別の経路で")
        .unwrap()
        .thread
        .thread_id;
    rt.bump();
    poll(cx);
    click(cx, "channel-row-0");
    view.read_with(cx, |this, _| {
        let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
        assert!(timeline.scope == OwnedScope::Thread(thread_id.clone()));
    });
}

#[gpui_kit::test]
fn a_thread_notification_opens_its_thread_over_the_remembered_one(cx: &mut TestAppContext) {
    let rt = runtime();
    let (shown, _, parent, remembered) = two_channels_with_a_thread(&rt);
    let target = {
        let mut store = rt.store();
        let other_parent = store.user_post_main(&shown, "別の親").unwrap();
        store
            .user_reply(&shown, &other_parent.message_id, "返信")
            .unwrap()
            .thread
            .thread_id
    };
    let (view, cx) = open(cx, &rt);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    click(cx, "channel-row-1");
    {
        let mut store = rt.store();
        let last = store.thread(&shown, &remembered).unwrap().last_message_id;
        store
            .agent_post_thread(&shown, &remembered, "途中経過", false, Some(&last))
            .unwrap();
    }
    rt.bump();
    poll(cx);
    let remembered_tag = format!("thread:{remembered}");
    view.read_with(cx, |this, _| {
        assert!(this.notified.contains(&remembered_tag));
    });

    view.update_in(cx, |this, window, cx| {
        this.open_tag(&format!("thread:{target}"), window, cx)
    });
    view.read_with(cx, |this, _| {
        let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
        assert!(timeline.scope == OwnedScope::Thread(target.clone()));
        // The remembered thread never showed, so its notification stays.
        assert!(this.notified.contains(&remembered_tag));
    });
}

#[gpui_kit::test]
fn choosing_the_shown_channel_keeps_its_sheet(cx: &mut TestAppContext) {
    let rt = runtime();
    let (shown, _, parent, _) = two_channels_with_a_thread(&rt);
    let (view, cx) = open(cx, &rt);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    cx.simulate_input("書きかけ");
    click(cx, "channel-row-0");
    view.read_with(cx, |this, cx| {
        assert_eq!(this.pane.as_ref().unwrap().reply_text(cx), "書きかけ");
    });
    // Typing moves to the main composer, as with any channel chosen.
    cx.simulate_input("メインへ");
    cx.simulate_keystrokes("cmd-enter");
    cx.run_until_parked();
    assert!(main_texts(&rt, &shown).contains(&"メインへ".to_owned()));
}

#[gpui_kit::test]
fn an_open_thread_shown_whole_clears_its_unread_marks(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let parent = rt.store().user_post_main(&channel, "親").unwrap();
    let thread_id = rt
        .store()
        .user_reply(&channel, &parent.message_id, "依頼")
        .unwrap()
        .thread
        .thread_id;
    {
        let mut store = rt.store();
        let last = store.thread(&channel, &thread_id).unwrap().last_message_id;
        store
            .agent_post_thread(&channel, &thread_id, "結果", false, Some(&last))
            .unwrap();
    }
    let link_unread = |view: &Entity<Workspace>, cx: &mut VisualTestContext| {
        view.read_with(cx, |this, _| {
            this.timeline
                .as_ref()
                .unwrap()
                .rows
                .iter()
                .find_map(|row| match row {
                    Row::Message(_, Some(link)) => Some(link.unread),
                    _ => None,
                })
                .unwrap()
        })
    };
    let (view, cx) = open(cx, &rt);
    assert!(link_unread(&view, cx));
    assert!(rt.store().has_unread_unresolved_thread(&channel).unwrap());

    cx.update(|window, _| window.activate_window());
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    // The sheet lays out, then the frame after it reads what it shows.
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    cx.update(|window, cx| window.simulate_next_frame(cx));
    poll(cx);

    assert!(!rt.store().has_unread_unresolved_thread(&channel).unwrap());
    assert!(!link_unread(&view, cx), "the link in main drops its dot");
}

/// Draws frames until scrolling settles, running what each frame leaves for after it.
fn settle(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
}

/// Whether the main timeline's top is the start of the message.
fn at_top(view: &Entity<Workspace>, cx: &mut VisualTestContext, message_id: &str) -> bool {
    view.read_with(cx, |this, _| {
        let timeline = this.timeline.as_ref().unwrap();
        let top = timeline.list.logical_scroll_top();
        Some(top.item_ix) == timeline.row_of(message_id) && top.offset_in_item == px(0.)
    })
}

#[gpui_kit::test]
fn a_thread_opened_from_a_list_scrolls_its_parent_to_the_top(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let mut last = None;
    for i in 0..60 {
        last = Some(
            rt.store()
                .agent_post_main(&channel, &format!("前 {i}"), false, last.as_deref())
                .unwrap()
                .message_id,
        );
    }
    let parent = rt.store().user_post_main(&channel, "親").unwrap();
    rt.store()
        .user_reply(&channel, &parent.message_id, "返信")
        .unwrap();
    let mut last = parent.message_id.clone();
    for i in 0..30 {
        last = rt
            .store()
            .agent_post_main(&channel, &format!("続き {i}"), false, Some(&last))
            .unwrap()
            .message_id;
    }
    rt.store()
        .mark_read(
            Scope::Main {
                channel_id: &channel,
            },
            &last,
        )
        .unwrap();
    let (view, cx) = open(cx, &rt);
    cx.update(|window, _| window.activate_window());
    settle(cx);
    // Two rows above the parent at the top: the parent is already in view.
    view.read_with(cx, |this, _| {
        let timeline = this.timeline.as_ref().unwrap();
        let row = timeline.row_of(&parent.message_id).unwrap();
        timeline.list.scroll_to(ListOffset {
            item_ix: row - 2,
            offset_in_item: px(0.),
        });
    });
    settle(cx);
    view.read_with(cx, |this, _| {
        let timeline = this.timeline.as_ref().unwrap();
        let viewport = timeline.list.viewport_bounds();
        let row = timeline.row_of(&parent.message_id).unwrap();
        let bounds = timeline
            .list
            .bounds_for_item(row)
            .expect("the parent is laid out");
        assert!(bounds.top() > viewport.top() && bounds.bottom() <= viewport.bottom());
    });
    assert!(!at_top(&view, cx, &parent.message_id));

    view.update_in(cx, |this, window, cx| {
        this.open_unresolved(window, cx);
    });
    click(cx, "unresolved-0");
    assert!(at_top(&view, cx, &parent.message_id));
    settle(cx);
    assert!(at_top(&view, cx, &parent.message_id));
    view.read_with(cx, |this, _| assert!(this.pane.is_some()));
}

#[gpui_kit::test]
fn a_channel_reopens_its_thread_with_the_parent_at_the_top_before_painting(
    cx: &mut TestAppContext,
) {
    let rt = runtime();
    let (shown, _, parent, _) = two_channels_with_a_thread(&rt);
    let mut last = parent.message_id.clone();
    for i in 0..60 {
        last = rt
            .store()
            .agent_post_main(&shown, &format!("続き {i}"), false, Some(&last))
            .unwrap()
            .message_id;
    }
    let main = Scope::Main { channel_id: &shown };
    rt.store().mark_read(main, &last).unwrap();
    let (view, cx) = open(cx, &rt);
    cx.update(|window, _| window.activate_window());
    settle(cx);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    settle(cx);
    assert!(!at_top(&view, cx, &parent.message_id));

    click(cx, "channel-row-1");
    settle(cx);
    click(cx, "channel-row-0");
    // Already in place before the first frame of the returned channel.
    assert!(at_top(&view, cx, &parent.message_id));
    settle(cx);
    assert!(at_top(&view, cx, &parent.message_id));
}

#[gpui_kit::test]
fn a_thread_read_to_its_end_stays_there_while_the_reply_grows(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let parent = rt.store().user_post_main(&channel, "親").unwrap();
    let reply = rt
        .store()
        .user_reply(&channel, &parent.message_id, "返信")
        .unwrap();
    let mut last = reply.message.message_id.clone();
    for i in 0..40 {
        last = rt
            .store()
            .agent_post_thread(
                &channel,
                &reply.thread.thread_id,
                &format!("続き {i}"),
                false,
                Some(&last),
            )
            .unwrap()
            .message_id;
    }
    let (view, cx) = open(cx, &rt);
    cx.update(|window, _| window.activate_window());
    settle(cx);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    settle(cx);
    // Read down to the end by scrolling, not by opening there.
    view.read_with(cx, |this, _| {
        let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
        timeline.list.scroll_to(ListOffset {
            item_ix: timeline.rows.len() - 3,
            offset_in_item: px(0.),
        });
    });
    settle(cx);
    let end_in_view = |view: &Entity<Workspace>, cx: &mut VisualTestContext| {
        view.read_with(cx, |this, _| {
            let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
            let viewport = timeline.list.viewport_bounds();
            timeline.follows_end()
                || timeline
                    .list
                    .bounds_for_item(timeline.rows.len() - 1)
                    .is_some_and(|row| row.bottom() <= viewport.bottom() + px(1.))
        })
    };
    assert!(end_in_view(&view, cx));

    for _ in 0..6 {
        cx.simulate_keystrokes("enter");
        settle(cx);
        assert!(end_in_view(&view, cx));
    }
}

#[gpui_kit::test]
fn only_the_last_row_keeps_room_above_the_composer(cx: &mut TestAppContext) {
    let rt = runtime();
    let channel = rt
        .store()
        .create_channel("c", &launch())
        .unwrap()
        .channel_id;
    let parent = rt.store().user_post_main(&channel, "親").unwrap();
    rt.store()
        .user_reply(&channel, &parent.message_id, "一")
        .unwrap();
    rt.store()
        .user_reply(&channel, &parent.message_id, "二")
        .unwrap();
    let (view, cx) = open(cx, &rt);
    cx.update(|window, _| window.activate_window());
    settle(cx);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    settle(cx);
    // Rows: the parent, 一, 二. A thread that fits is laid out from its top.
    let height = |view: &Entity<Workspace>, cx: &mut VisualTestContext, ix: usize| {
        view.read_with(cx, |this, _| {
            let timeline = this.pane.as_ref().unwrap().timeline.as_ref().unwrap();
            timeline.list.bounds_for_item(ix).unwrap().size.height
        })
    };
    let (first, second) = (height(&view, cx, 1), height(&view, cx, 2));
    assert_eq!(second - first, px(crate::workspace::FADE));

    rt.store()
        .user_reply(&channel, &parent.message_id, "三")
        .unwrap();
    rt.bump();
    poll(cx);
    settle(cx);
    assert_eq!(height(&view, cx, 2), first);
    assert_eq!(height(&view, cx, 3), second);
}

#[gpui_kit::test]
fn a_narrow_window_keeps_the_sidebar_in_a_drawer(cx: &mut TestAppContext) {
    let rt = runtime();
    rt.store().create_channel("a", &launch()).unwrap();
    rt.store().create_channel("b", &launch()).unwrap();
    let (view, cx) = open(cx, &rt);
    let first = view.read_with(cx, |this, _| this.selected.clone());
    cx.simulate_resize(gpui_kit::size(px(600.), px(600.)));
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(this.shown.drawer && !this.drawer_open)
    });
    // Not beside the sheets: the rows are not drawn until the drawer comes out.
    assert!(cx.debug_bounds("channel-row-1").is_none());
    click(cx, "main-sidebar-toggle");
    view.read_with(cx, |this, _| assert!(this.drawer_open));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    view.read_with(cx, |this, _| assert!(!this.drawer_open));
    // Choosing a channel puts the drawer away.
    click(cx, "main-sidebar-toggle");
    click(cx, "channel-row-1");
    view.read_with(cx, |this, _| {
        assert_ne!(this.selected, first);
        assert!(!this.drawer_open);
    });
    // A wide window brings the sidebar back beside the sheets, not as a drawer.
    click(cx, "main-sidebar-toggle");
    cx.simulate_resize(gpui_kit::size(px(1200.), px(600.)));
    cx.run_until_parked();
    view.read_with(cx, |this, _| {
        assert!(!this.shown.drawer && !this.drawer_open && this.sidebar_docked());
    });
}

#[gpui_kit::test]
fn a_thread_covering_the_main_sheet_leaves_it_unread(cx: &mut TestAppContext) {
    let rt = runtime();
    let (shown, _, parent, _) = two_channels_with_a_thread(&rt);
    let main = Scope::Main { channel_id: &shown };
    let (view, cx) = open(cx, &rt);
    cx.update(|window, _| window.activate_window());
    cx.simulate_resize(gpui_kit::size(px(700.), px(600.)));
    settle(cx);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    settle(cx);
    view.read_with(cx, |this, _| assert!(this.shown.thread_over));
    rt.store()
        .agent_post_main(&shown, "裏で届いた", false, Some(&parent.message_id))
        .unwrap();
    poll(cx);
    settle(cx);
    assert!(rt.store().has_unread(main).unwrap());
    // Side by side again, the main sheet is in view and read.
    cx.simulate_resize(gpui_kit::size(px(1400.), px(600.)));
    settle(cx);
    view.read_with(cx, |this, _| assert!(!this.shown.thread_over));
    assert!(!rt.store().has_unread(main).unwrap());
}

#[gpui_kit::test]
fn the_drawer_opens_over_a_thread_covering_the_main_sheet(cx: &mut TestAppContext) {
    let rt = runtime();
    let (_, _, parent, _) = two_channels_with_a_thread(&rt);
    let (view, cx) = open(cx, &rt);
    cx.simulate_resize(gpui_kit::size(px(600.), px(600.)));
    settle(cx);
    view.update_in(cx, |this, window, cx| {
        this.open_thread(parent.clone(), window, cx)
    });
    settle(cx);
    view.read_with(cx, |this, _| {
        assert!(this.shown.thread_over && this.shown.drawer)
    });
    click(cx, "thread-sidebar-toggle");
    settle(cx);
    view.read_with(cx, |this, _| assert!(this.drawer_open));
    assert!(cx.debug_bounds("channel-row-1").is_some());
}
