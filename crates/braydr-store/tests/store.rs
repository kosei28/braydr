use braydr_core::*;
use braydr_store::Store;

fn launch() -> LaunchConfig {
    LaunchConfig {
        cwd: "/repo".into(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    }
}

fn setup() -> (Store, String) {
    let mut store = Store::in_memory().unwrap();
    let channel = store.create_channel(" ログイン修正 ", &launch()).unwrap();
    assert_eq!(channel.name, "ログイン修正");
    (store, channel.channel_id)
}

fn threaded(store: &mut Store, ch: &str) -> (String, Thread) {
    let parent = store.user_post_main(ch, "親").unwrap();
    let reply = store.user_reply(ch, &parent.message_id, "返信").unwrap();
    assert!(reply.created);
    (parent.message_id, reply.thread)
}

#[test]
fn after_guards_main_posts() {
    let (mut s, ch) = setup();
    let first = s.agent_post_main(&ch, "a", None).unwrap();
    assert_eq!(s.agent_post_main(&ch, "b", None), Err(Error::AfterRequired));
    assert_eq!(
        s.agent_post_main(&ch, "b", Some("stale")),
        Err(Error::StaleAfter {
            current_last_message_id: first.message_id.clone()
        })
    );
    assert_eq!(
        s.agent_post_main(&ch, "b", Some(&first.message_id))
            .unwrap()
            .author,
        Author::Agent
    );
}

#[test]
fn validates_text() {
    let (mut s, ch) = setup();
    assert_eq!(s.user_post_main(&ch, "   "), Err(Error::EmptyText));
    assert_eq!(
        s.agent_post_main(&ch, &"あ".repeat(20_001), None),
        Err(Error::TextTooLong)
    );
}

#[test]
fn missing_channel() {
    let mut s = Store::in_memory().unwrap();
    assert_eq!(s.user_post_main("x", "a"), Err(Error::ChannelNotFound));
    assert_eq!(s.rename_channel("x", "a"), Err(Error::ChannelNotFound));
}

#[test]
fn thread_creation_rules() {
    let (mut s, ch) = setup();
    let parent = s.agent_post_main(&ch, "親", None).unwrap();
    assert_eq!(
        s.agent_create_thread(&ch, "missing", "x"),
        Err(Error::MessageNotFound)
    );
    let (thread, first) = s
        .agent_create_thread(&ch, &parent.message_id, "最初")
        .unwrap();
    assert_eq!(thread.reply_count, 1);
    assert_eq!(thread.post_count, 2);
    assert_eq!(thread.awaiting, Some(Author::User));
    assert_eq!(
        s.agent_create_thread(&ch, &parent.message_id, "x"),
        Err(Error::ThreadExists {
            thread_id: thread.thread_id.clone()
        })
    );
    assert_eq!(
        s.agent_create_thread(&ch, &first.message_id, "x"),
        Err(Error::ParentNotMain)
    );
    assert_eq!(
        s.user_reply(&ch, &first.message_id, "x").unwrap_err(),
        Error::ParentNotMain
    );
    let reloaded = s.message(&ch, &parent.message_id).unwrap();
    assert_eq!(
        reloaded.child_thread_id.as_deref(),
        Some(thread.thread_id.as_str())
    );
    assert_eq!(
        s.message(&ch, &first.message_id).unwrap().child_thread_id,
        None
    );
}

#[test]
fn other_channels_are_invisible() {
    let (mut s, ch) = setup();
    let other = s.create_channel("別", &launch()).unwrap().channel_id;
    let (parent, thread) = threaded(&mut s, &ch);
    assert_eq!(s.message(&other, &parent), Err(Error::MessageNotFound));
    assert_eq!(
        s.thread(&other, &thread.thread_id),
        Err(Error::ThreadNotFound)
    );
    assert_eq!(
        s.agent_post_thread(&other, &thread.thread_id, "x", None),
        Err(Error::ThreadNotFound)
    );
    assert_eq!(
        s.agent_create_thread(&other, &parent, "x"),
        Err(Error::MessageNotFound)
    );
}

#[test]
fn agent_resolve_guards() {
    let (mut s, ch) = setup();
    let (_, thread) = threaded(&mut s, &ch);
    let t = thread.thread_id.as_str();
    assert_eq!(thread.awaiting, Some(Author::Agent));
    assert_eq!(
        s.agent_resolve(&ch, t, &thread.last_message_id),
        Err(Error::Unanswered {
            last_message_id: thread.last_message_id.clone()
        })
    );
    assert!(matches!(
        s.agent_resolve(&ch, t, "stale"),
        Err(Error::StaleAfter { .. })
    ));
    let reply = s
        .agent_post_thread(&ch, t, "完了", Some(&thread.last_message_id))
        .unwrap();
    let resolved = s.agent_resolve(&ch, t, &reply.message_id).unwrap();
    assert_eq!(resolved.resolved_by, Some(Author::Agent));
    assert_eq!(resolved.awaiting, None);
    assert!(matches!(
        s.agent_resolve(&ch, t, &reply.message_id),
        Err(Error::AlreadyResolved { .. })
    ));
    assert!(matches!(
        s.agent_post_thread(&ch, t, "x", Some(&reply.message_id)),
        Err(Error::ThreadResolved { .. })
    ));
    let reopened = s.agent_unresolve(&ch, t).unwrap();
    assert_eq!(reopened.resolved_at, None);
    assert_eq!(s.agent_unresolve(&ch, t), Err(Error::AlreadyUnresolved));
}

#[test]
fn user_post_reopens_and_records_history() {
    let (mut s, ch) = setup();
    let (parent, thread) = threaded(&mut s, &ch);
    let t = thread.thread_id.as_str();
    s.user_resolve(&ch, t).unwrap();
    let reply = s.user_reply(&ch, &parent, "追加").unwrap();
    assert!(reply.reopened && !reply.created);
    assert_eq!(reply.thread.resolved_at, None);
    assert_eq!(reply.thread.awaiting, Some(Author::Agent));
    let events = s.state_events(t).unwrap();
    assert_eq!(
        events
            .iter()
            .map(|e| (e.action, e.actor))
            .collect::<Vec<_>>(),
        vec![
            (StateAction::Resolved, Author::User),
            (StateAction::Reopened, Author::User)
        ]
    );
}

#[test]
fn pagination_is_oldest_to_newest_per_page() {
    let (mut s, ch) = setup();
    let ids: Vec<_> = (0..5)
        .map(|i| s.user_post_main(&ch, &format!("m{i}")).unwrap().message_id)
        .collect();
    let scope = Scope::Main { channel_id: &ch };
    let first = s.list_posts(scope, None, Some(2)).unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|m| &m.message_id)
            .collect::<Vec<_>>(),
        vec![&ids[3], &ids[4]]
    );
    assert!(first.has_more);
    let second = s
        .list_posts(scope, first.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(second.items[0].message_id, ids[1]);
    let third = s
        .list_posts(scope, second.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(third.items.len(), 1);
    assert!(!third.has_more && third.next_cursor.is_none());
    assert_eq!(
        s.list_posts(scope, Some("garbage"), None),
        Err(Error::InvalidCursor)
    );
    assert_eq!(
        s.list_posts(scope, Some("m:missing"), None),
        Err(Error::InvalidCursor)
    );
}

#[test]
fn thread_filters_and_order() {
    let (mut s, ch) = setup();
    let (_, waiting_agent) = threaded(&mut s, &ch);
    let (_, waiting_user) = threaded(&mut s, &ch);
    s.agent_post_thread(
        &ch,
        &waiting_user.thread_id,
        "質問",
        Some(&waiting_user.last_message_id),
    )
    .unwrap();
    let (_, done) = threaded(&mut s, &ch);
    s.user_resolve(&ch, &done.thread_id).unwrap();
    let ids = |filter| {
        s.list_threads(&ch, filter, None, None)
            .unwrap()
            .items
            .into_iter()
            .map(|t| t.thread_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(ThreadFilter::AwaitingAgent),
        vec![waiting_agent.thread_id.clone()]
    );
    assert_eq!(
        ids(ThreadFilter::AwaitingUser),
        vec![waiting_user.thread_id.clone()]
    );
    assert_eq!(ids(ThreadFilter::Resolved), vec![done.thread_id.clone()]);
    assert_eq!(ids(ThreadFilter::Unresolved).len(), 2);
    assert_eq!(ids(ThreadFilter::All).len(), 3);
    assert_eq!(s.unresolved_count(&ch).unwrap(), 2);
    let page = s
        .list_threads(&ch, ThreadFilter::All, None, Some(2))
        .unwrap();
    assert!(page.has_more);
    let rest = s
        .list_threads(&ch, ThreadFilter::All, page.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(rest.items.len(), 1);
    assert_eq!(
        s.list_threads(&ch, ThreadFilter::All, Some("x"), None),
        Err(Error::InvalidCursor)
    );
}

#[test]
fn read_positions_only_move_forward() {
    let (mut s, ch) = setup();
    let scope = Scope::Main { channel_id: &ch };
    let a = s.agent_post_main(&ch, "a", None).unwrap();
    let b = s.agent_post_main(&ch, "b", Some(&a.message_id)).unwrap();
    assert_eq!(
        s.first_unread(scope).unwrap().as_deref(),
        Some(a.message_id.as_str())
    );
    s.mark_read(scope, &b.message_id).unwrap();
    assert!(!s.has_unread(scope).unwrap());
    s.mark_read(scope, &a.message_id).unwrap();
    assert_eq!(
        s.read_position(scope).unwrap().as_deref(),
        Some(b.message_id.as_str())
    );
}

#[test]
fn partial_read_leaves_the_rest_unread() {
    let (mut s, ch) = setup();
    let scope = Scope::Main { channel_id: &ch };
    let a = s.agent_post_main(&ch, "a", None).unwrap();
    let b = s.agent_post_main(&ch, "b", Some(&a.message_id)).unwrap();
    s.mark_read(scope, &a.message_id).unwrap();
    assert_eq!(
        s.first_unread(scope).unwrap().as_deref(),
        Some(b.message_id.as_str())
    );
}

#[test]
fn unread_axes_are_independent() {
    let (mut s, ch) = setup();
    let (_, thread) = threaded(&mut s, &ch);
    let t = thread.thread_id.as_str();
    let scope = Scope::Thread { thread_id: t };
    assert!(!s.has_unread(scope).unwrap(), "own reply is read");
    let reply = s
        .agent_post_thread(&ch, t, "返答", Some(&thread.last_message_id))
        .unwrap();
    assert!(s.has_unread(scope).unwrap());
    s.agent_resolve(&ch, t, &reply.message_id).unwrap();
    assert!(s.has_unread(scope).unwrap(), "resolving keeps unread");
    assert!(
        !s.has_unread(Scope::Main { channel_id: &ch }).unwrap(),
        "thread unread does not touch main"
    );
}

#[test]
fn only_unresolved_threads_count_as_unread_unresolved() {
    let (mut s, ch) = setup();
    let (_, thread) = threaded(&mut s, &ch);
    let t = thread.thread_id.as_str();
    assert!(
        !s.has_unread_unresolved_thread(&ch).unwrap(),
        "own reply is read"
    );
    let reply = s
        .agent_post_thread(&ch, t, "返答", Some(&thread.last_message_id))
        .unwrap();
    assert!(s.has_unread_unresolved_thread(&ch).unwrap());

    s.agent_resolve(&ch, t, &reply.message_id).unwrap();
    assert!(!s.has_unread_unresolved_thread(&ch).unwrap());
    assert!(
        s.has_unread(Scope::Thread { thread_id: t }).unwrap(),
        "still unread"
    );
}

#[test]
fn deleting_a_channel_removes_everything_in_it_and_nothing_else() {
    let (mut s, ch) = setup();
    let (_, thread) = threaded(&mut s, &ch);
    let t = thread.thread_id.clone();
    let reply = s
        .agent_post_thread(&ch, &t, "検索語あり", Some(&thread.last_message_id))
        .unwrap();
    s.agent_resolve(&ch, &t, &reply.message_id).unwrap();
    s.mark_read(Scope::Thread { thread_id: &t }, &reply.message_id)
        .unwrap();

    let other = s.create_channel("別", &launch()).unwrap().channel_id;
    let kept = s.user_post_main(&other, "検索語あり").unwrap();

    assert_eq!(s.delete_channel(&ch).unwrap(), vec![t.clone()]);

    assert_eq!(s.channel(&ch), Err(Error::ChannelNotFound));
    assert_eq!(s.channel_of_thread(&t).unwrap(), None);
    assert_eq!(
        s.read_position(Scope::Thread { thread_id: &t }).unwrap(),
        None
    );
    assert_eq!(s.delete_channel(&ch), Err(Error::ChannelNotFound));

    let channels = s.channels().unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0].channel_id, other);
    let found = s.search(&other, "検索語", None, None).unwrap();
    assert_eq!(found.items.len(), 1);
    assert_eq!(found.items[0].message_id, kept.message_id);
    // Messages written after the delete must still be searchable.
    s.user_post_main(&other, "検索語その2").unwrap();
    assert_eq!(
        s.search(&other, "検索語", None, None).unwrap().items.len(),
        2
    );
}

#[test]
fn search_finds_japanese_in_main_and_threads() {
    let (mut s, ch) = setup();
    let other = s.create_channel("別", &launch()).unwrap().channel_id;
    let parent = s.user_post_main(&ch, "バリデーションを直して").unwrap();
    s.user_reply(&ch, &parent.message_id, "VALIDATE に寄せる")
        .unwrap();
    s.user_post_main(&other, "バリデーションは別").unwrap();
    assert_eq!(
        s.search(&ch, "バリデーション", None, None)
            .unwrap()
            .items
            .len(),
        1
    );
    assert_eq!(
        s.search(&ch, "validate", None, None).unwrap().items.len(),
        1
    );
    assert_eq!(
        s.search(&ch, "直し", None, None).unwrap().items.len(),
        1,
        "two characters"
    );
    assert_eq!(
        s.search(&ch, "\"quoted", None, None).unwrap().items.len(),
        0
    );
    assert_eq!(s.search(&ch, "%", None, None).unwrap().items.len(), 0);
    assert_eq!(s.search(&ch, "  ", None, None), Err(Error::InvalidQuery));
}

#[test]
fn search_pages_newest_first() {
    let (mut s, ch) = setup();
    let ids: Vec<_> = (0..3)
        .map(|i| {
            s.user_post_main(&ch, &format!("共通語 {i}"))
                .unwrap()
                .message_id
        })
        .collect();
    let first = s.search(&ch, "共通語", None, Some(2)).unwrap();
    assert_eq!(first.items[0].message_id, ids[2]);
    assert!(first.has_more);
    let second = s
        .search(&ch, "共通語", first.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(second.items[0].message_id, ids[0]);
}

#[test]
fn nudge_condition_for_main() {
    let (mut s, ch) = setup();
    assert!(!s.main_needs_response(&ch).unwrap());
    let posted = s.user_post_main(&ch, "依頼").unwrap();
    assert!(s.main_needs_response(&ch).unwrap());
    s.agent_create_thread(&ch, &posted.message_id, "対応")
        .unwrap();
    assert!(
        !s.main_needs_response(&ch).unwrap(),
        "a child thread counts as handled"
    );
}

#[test]
fn new_session_keeps_history_and_stores_settings() {
    let (mut s, ch) = setup();
    s.user_post_main(&ch, "履歴").unwrap();
    s.set_session(&ch, Some("t7")).unwrap();
    let next = LaunchConfig {
        cwd: "/other".into(),
        command: "claude --model opus".into(),
        prompt: "/braydr 続き".into(),
    };
    s.prepare_new_session(&ch, &next).unwrap();
    let channel = s.channel(&ch).unwrap();
    assert_eq!(channel.session_id, None);
    assert_eq!(channel.cwd, "/other");
    assert_eq!(channel.prompt, "/braydr 続き");
    assert_eq!(
        s.list_posts(Scope::Main { channel_id: &ch }, None, None)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn ids_are_time_ordered_and_migrations_are_idempotent() {
    let dir = std::env::temp_dir().join(format!("braydr-store-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("db.sqlite");
    let ch;
    {
        let mut s = Store::open(&path).unwrap();
        ch = s.create_channel("永続", &launch()).unwrap().channel_id;
        let mut last = String::new();
        for i in 0..200 {
            let id = s.user_post_main(&ch, &format!("{i}")).unwrap().message_id;
            assert!(id > last, "UUIDv7 stays monotonic");
            last = id;
        }
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.channel(&ch).unwrap().name, "永続");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn values_persist_and_do_not_disturb_settings() {
    let mut store = Store::in_memory().unwrap();
    assert_eq!(store.value("main_window").unwrap(), None);
    store.set_value("main_window", "a").unwrap();
    store.set_value("main_window", "b").unwrap();
    assert_eq!(store.value("main_window").unwrap().as_deref(), Some("b"));
    assert_eq!(store.settings().unwrap(), Settings::default());
}

#[test]
fn settings_default_and_persist() {
    let mut store = Store::in_memory().unwrap();
    assert_eq!(store.settings().unwrap(), Settings::default());
    let changed = Settings {
        default_command: "claude --model opus".into(),
        default_prompt: "/x".into(),
        nudge_minutes: 15,
    };
    store.save_settings(&changed).unwrap();
    store.save_settings(&changed).unwrap();
    assert_eq!(store.settings().unwrap(), changed);
}

#[test]
fn search_count_covers_all_pages() {
    let (mut store, channel) = channel_for_notifications();
    let mut last = None;
    for _ in 0..3 {
        last = Some(
            store
                .agent_post_main(&channel, "ログイン修正", last.as_deref())
                .unwrap()
                .message_id,
        );
    }
    assert_eq!(store.search_count(&channel, "ログイン").unwrap(), 3);
    assert_eq!(store.search_count(&channel, "修").unwrap(), 3);
    assert_eq!(store.search_count(&channel, "ない").unwrap(), 0);
}

fn channel_for_notifications() -> (Store, String) {
    let mut store = Store::in_memory().unwrap();
    let launch = LaunchConfig {
        cwd: "/tmp".into(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    };
    let channel = store.create_channel("c", &launch).unwrap().channel_id;
    (store, channel)
}

#[test]
fn agent_messages_after_skips_users_and_older() {
    let (mut store, channel) = channel_for_notifications();
    let first = store.user_post_main(&channel, "依頼").unwrap();
    let watermark = store.newest_message_id().unwrap();
    assert_eq!(watermark.as_deref(), Some(first.message_id.as_str()));
    let reply = store
        .agent_post_main(&channel, "返事", Some(&first.message_id))
        .unwrap();
    store.user_post_main(&channel, "次").unwrap();
    let found = store.agent_messages_after(watermark.as_deref()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].message_id, reply.message_id);
    assert!(
        store
            .agent_messages_after(Some(&reply.message_id))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn finds_the_channel_of_a_thread() {
    let (mut store, channel) = channel_for_notifications();
    let parent = store.user_post_main(&channel, "親").unwrap();
    let reply = store
        .user_reply(&channel, &parent.message_id, "返信")
        .unwrap();
    assert_eq!(
        store.channel_of_thread(&reply.thread.thread_id).unwrap(),
        Some(channel)
    );
    assert_eq!(store.channel_of_thread("missing").unwrap(), None);
}
