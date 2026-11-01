//! Workspace behavior through real key dispatch, with no herdr-core behind it.

use std::sync::Arc;
use std::time::Duration;

use braydr_core::{LaunchConfig, Scope};
use braydr_herdr::Client;
use braydr_runtime::Runtime;
use braydr_store::Store;
use gpui_kit::{Entity, TestAppContext, VisualTestContext};

use crate::overlay::Overlay;
use crate::timeline::{Row, Rt};
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
        .agent_post_main(&other, "できました", None)
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
