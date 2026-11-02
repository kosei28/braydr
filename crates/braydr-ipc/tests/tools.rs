use braydr_core::LaunchConfig;
use braydr_ipc::server::handle;
use braydr_ipc::{Request, Response, tools};
use braydr_store::Store;
use serde_json::{Value, json};

fn setup() -> (Store, String) {
    let mut store = Store::in_memory().unwrap();
    let launch = LaunchConfig {
        cwd: ".".into(),
        command: "claude".into(),
        prompt: "/braydr".into(),
    };
    let id = store.create_channel("開発", &launch).unwrap().channel_id;
    (store, id)
}

fn call(store: &mut Store, channel: &str, tool: &str, args: Value) -> Result<Value, Value> {
    let request = Request {
        channel: Some(channel.into()),
        tool: tool.into(),
        args,
    };
    match handle(store, &request) {
        Response::Result(value) => Ok(value),
        Response::Error(error) => Err(error),
    }
}

fn ok(store: &mut Store, channel: &str, tool: &str, args: Value) -> Value {
    call(store, channel, tool, args).unwrap_or_else(|error| panic!("{tool}: {error}"))
}

fn code(store: &mut Store, channel: &str, tool: &str, args: Value) -> String {
    call(store, channel, tool, args).unwrap_err()["code"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn every_tool_has_an_object_schema_and_no_channel_argument() {
    let specs = tools::specs();
    assert_eq!(specs.len(), 12);
    for spec in specs {
        assert_eq!(spec.schema["type"], "object", "{}", spec.name);
        assert!(
            spec.schema["properties"].get("channel").is_none(),
            "{}",
            spec.name
        );
    }
}

#[test]
fn a_full_conversation() {
    let (mut s, ch) = setup();
    let c = ch.as_str();
    let user = s.user_post_main(c, "ログインを直して").unwrap();

    let summary = ok(&mut s, c, "braydr_channel_get", json!({}));
    assert_eq!(summary["name"], "開発");
    assert_eq!(summary["post_count"], 1);
    assert_eq!(summary["last_author"], "user");

    let posts = ok(&mut s, c, "braydr_channel_list_posts", json!({}));
    assert_eq!(posts["messages"][0]["text"], "ログインを直して");
    assert_eq!(posts["has_more"], false);
    assert!(
        posts["messages"][0]["created_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

    let created = ok(
        &mut s,
        c,
        "braydr_thread_create",
        json!({ "parent_message_id": user.message_id, "text": "調べます", "agent_continues": false }),
    );
    let thread_id = created["thread"]["thread_id"].as_str().unwrap().to_owned();
    assert_eq!(created["thread"]["awaiting"], "user");
    let first = created["message"]["message_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let reply = ok(
        &mut s,
        c,
        "braydr_thread_create_post",
        json!({ "thread_id": thread_id, "text": "直しました", "after": first, "agent_continues": false }),
    );
    let last = reply["message"]["message_id"].as_str().unwrap().to_owned();

    let resolved = ok(
        &mut s,
        c,
        "braydr_thread_resolve",
        json!({ "thread_id": thread_id, "after": last }),
    );
    assert_eq!(resolved["resolved_by"], "agent");
    assert!(resolved["resolved_at"].as_str().unwrap().ends_with('Z'));

    let posts = ok(
        &mut s,
        c,
        "braydr_thread_list_posts",
        json!({ "thread_id": thread_id }),
    );
    assert_eq!(posts["messages"].as_array().unwrap().len(), 2);
    assert_eq!(posts["parent_message"]["text"], "ログインを直して");
    assert_eq!(posts["state_events"][0]["action"], "resolved");
    assert_eq!(posts["state_events"][0]["actor"], "agent");

    let listed = ok(
        &mut s,
        c,
        "braydr_thread_list",
        json!({ "filter": "resolved" }),
    );
    assert_eq!(listed["threads"][0]["thread_id"], thread_id);

    let found = ok(
        &mut s,
        c,
        "braydr_message_search",
        json!({ "query": "直しました" }),
    );
    assert_eq!(found["messages"][0]["thread_id"], thread_id);

    let reopened = ok(
        &mut s,
        c,
        "braydr_thread_unresolve",
        json!({ "thread_id": thread_id }),
    );
    assert_eq!(reopened["resolved_at"], Value::Null);
    assert_eq!(
        ok(
            &mut s,
            c,
            "braydr_thread_get",
            json!({ "thread_id": thread_id })
        )["post_count"],
        3
    );
    assert_eq!(
        ok(
            &mut s,
            c,
            "braydr_message_get",
            json!({ "message_id": last })
        )["text"],
        "直しました"
    );
    ok(
        &mut s,
        c,
        "braydr_channel_create_post",
        json!({ "text": "別件", "after": user.message_id, "agent_continues": false }),
    );
}

#[test]
fn error_codes_and_details() {
    let (mut s, ch) = setup();
    let c = ch.as_str();
    let missing = "0190a3c4-1234-7abc-8def-0123456789ab";
    let call_missing = call(&mut s, missing, "braydr_channel_get", json!({})).unwrap_err();
    assert_eq!(call_missing["code"], "channel_not_found");
    assert_eq!(
        match handle(
            &mut s,
            &Request {
                channel: None,
                tool: "braydr_channel_get".into(),
                args: json!({})
            }
        ) {
            Response::Error(error) => error["code"].clone(),
            Response::Result(_) => Value::Null,
        },
        "channel_not_found"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_thread_get",
            json!({ "thread_id": "nope" })
        ),
        "invalid_argument"
    );
    assert_eq!(
        code(&mut s, c, "braydr_thread_get", json!({})),
        "invalid_argument"
    );
    assert_eq!(
        code(&mut s, c, "braydr_unknown", json!({})),
        "invalid_argument"
    );
    assert_eq!(
        code(&mut s, c, "braydr_thread_list", json!({ "filter": "x" })),
        "invalid_argument"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_thread_get",
            json!({ "thread_id": missing })
        ),
        "thread_not_found"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_message_get",
            json!({ "message_id": missing })
        ),
        "message_not_found"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_channel_create_post",
            json!({ "text": " ", "agent_continues": false })
        ),
        "empty_text"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_channel_create_post",
            json!({ "text": "あ".repeat(20_001), "agent_continues": false })
        ),
        "text_too_long"
    );
    assert_eq!(
        code(&mut s, c, "braydr_message_search", json!({ "query": " " })),
        "invalid_query"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_channel_list_posts",
            json!({ "cursor": "x" })
        ),
        "invalid_cursor"
    );

    let a = ok(
        &mut s,
        c,
        "braydr_channel_create_post",
        json!({ "text": "a", "agent_continues": false }),
    );
    let a = a["message"]["message_id"].as_str().unwrap().to_owned();
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_channel_create_post",
            json!({ "text": "b", "agent_continues": false })
        ),
        "after_required"
    );
    let stale = call(
        &mut s,
        c,
        "braydr_channel_create_post",
        json!({ "text": "b", "after": missing, "agent_continues": false }),
    )
    .unwrap_err();
    assert_eq!(stale["code"], "stale_after");
    assert_eq!(stale["details"]["current_last_message_id"], a);

    let t = ok(
        &mut s,
        c,
        "braydr_thread_create",
        json!({ "parent_message_id": a, "text": "t", "agent_continues": false }),
    );
    let (tid, first) = (
        t["thread"]["thread_id"].as_str().unwrap().to_owned(),
        t["message"]["message_id"].as_str().unwrap().to_owned(),
    );
    let exists = call(
        &mut s,
        c,
        "braydr_thread_create",
        json!({ "parent_message_id": a, "text": "t", "agent_continues": false }),
    )
    .unwrap_err();
    assert_eq!(exists["code"], "thread_exists");
    assert_eq!(exists["details"]["thread_id"], tid);
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_thread_create",
            json!({ "parent_message_id": first, "text": "t", "agent_continues": false })
        ),
        "parent_not_main"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_thread_unresolve",
            json!({ "thread_id": tid })
        ),
        "already_unresolved"
    );

    let user = s.user_reply(c, &a, "質問").unwrap();
    let last = user.message.message_id;
    let unanswered = call(
        &mut s,
        c,
        "braydr_thread_resolve",
        json!({ "thread_id": tid, "after": last }),
    )
    .unwrap_err();
    assert_eq!(unanswered["code"], "unanswered");
    let r = ok(
        &mut s,
        c,
        "braydr_thread_create_post",
        json!({ "thread_id": tid, "text": "答え", "after": last, "agent_continues": false }),
    );
    let last = r["message"]["message_id"].as_str().unwrap().to_owned();
    ok(
        &mut s,
        c,
        "braydr_thread_resolve",
        json!({ "thread_id": tid, "after": last }),
    );
    let already = call(
        &mut s,
        c,
        "braydr_thread_resolve",
        json!({ "thread_id": tid, "after": last }),
    )
    .unwrap_err();
    assert_eq!(already["code"], "already_resolved");
    assert!(
        already["details"]["resolved_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    let blocked = call(
        &mut s,
        c,
        "braydr_thread_create_post",
        json!({ "thread_id": tid, "text": "x", "after": last, "agent_continues": false }),
    )
    .unwrap_err();
    assert_eq!(blocked["code"], "thread_resolved");
    assert_eq!(blocked["details"]["resolved_by"], "agent");
}

#[test]
fn agent_continues_is_required_and_keeps_work_in_the_queue() {
    let (mut s, ch) = setup();
    let c = ch.as_str();
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_channel_create_post",
            json!({ "text": "a" })
        ),
        "invalid_argument"
    );
    assert_eq!(
        code(
            &mut s,
            c,
            "braydr_channel_create_post",
            json!({ "text": "a", "agent_continues": "yes" })
        ),
        "invalid_argument"
    );
    for spec in tools::specs() {
        let required = spec.schema["required"].as_array().unwrap();
        let posts = matches!(
            spec.name,
            "braydr_channel_create_post" | "braydr_thread_create_post" | "braydr_thread_create"
        );
        assert_eq!(
            required.contains(&json!("agent_continues")),
            posts,
            "{}",
            spec.name
        );
    }

    let summary = ok(&mut s, c, "braydr_channel_get", json!({}));
    assert_eq!(summary["main_awaiting_agent"], Value::Null);
    let user = s.user_post_main(c, "依頼").unwrap();
    let summary = ok(&mut s, c, "braydr_channel_get", json!({}));
    assert_eq!(summary["main_awaiting_agent"]["state"], "unanswered");
    assert_eq!(
        summary["main_awaiting_agent"]["last_message_id"],
        user.message_id
    );

    let progress = ok(
        &mut s,
        c,
        "braydr_channel_create_post",
        json!({ "text": "途中", "agent_continues": true, "after": user.message_id }),
    );
    assert_eq!(progress["message"]["agent_continues"], true);
    let progress = progress["message"]["message_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let summary = ok(&mut s, c, "braydr_channel_get", json!({}));
    assert_eq!(summary["main_awaiting_agent"]["state"], "continuing");
    assert_eq!(summary["main_awaiting_agent"]["last_message_id"], progress);

    let created = ok(
        &mut s,
        c,
        "braydr_thread_create",
        json!({ "parent_message_id": progress, "text": "続き", "agent_continues": true }),
    );
    assert_eq!(created["thread"]["agent_continues"], true);
    assert_eq!(created["thread"]["awaiting"], "agent");
    let tid = created["thread"]["thread_id"].as_str().unwrap().to_owned();
    let last = created["message"]["message_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let listed = ok(
        &mut s,
        c,
        "braydr_thread_list",
        json!({ "filter": "awaiting_agent" }),
    );
    assert_eq!(listed["threads"][0]["thread_id"], tid);
    let summary = ok(&mut s, c, "braydr_channel_get", json!({}));
    assert_eq!(summary["thread_count_awaiting_agent"], 1);

    let refused = call(
        &mut s,
        c,
        "braydr_thread_resolve",
        json!({ "thread_id": tid, "after": last }),
    )
    .unwrap_err();
    assert_eq!(refused["code"], "agent_continues");
    assert_eq!(refused["details"]["last_message_id"], last);

    let done = ok(
        &mut s,
        c,
        "braydr_thread_create_post",
        json!({ "thread_id": tid, "text": "完了", "agent_continues": false, "after": last }),
    );
    let done = done["message"]["message_id"].as_str().unwrap().to_owned();
    ok(
        &mut s,
        c,
        "braydr_thread_resolve",
        json!({ "thread_id": tid, "after": done }),
    );
}
