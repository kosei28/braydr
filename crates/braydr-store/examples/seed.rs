//! Fills a database with the sample conversation from `design/braydr.pen`: `seed <db>`.

use braydr_core::{LaunchConfig, Scope};
use braydr_store::Store;

fn main() {
    let path = std::env::args().nth(1).expect("db path");
    let mut s = Store::open(path).expect("open");
    let launch = LaunchConfig {
        cwd: "~/code/braydr".into(),
        command: "claude --model opus".into(),
        prompt: "/braydr".into(),
    };
    for name in [
        "依存パッケージ更新",
        "ドキュメント整備",
        "E2E安定化",
        "決済APIリファクタ",
    ] {
        let id = s.create_channel(name, &launch).unwrap().channel_id;
        s.user_post_main(&id, "進めて").unwrap();
    }
    let ch = s
        .create_channel("ログイン修正", &launch)
        .unwrap()
        .channel_id;
    let main = Scope::Main { channel_id: &ch };
    let a = s
        .user_post_main(
            &ch,
            "ログイン画面のバリデーションを直して。メールアドレスが空でも送信できてしまう。",
        )
        .unwrap();
    let b = s.agent_post_main(&ch, "送信前の必須チェックが抜けていました。LoginForm にメールとパスワードの検証を追加しました。\n\n```diff\n- if (password.length === 0) return\n+ const errors = validate({ email, password })\n+ if (errors.length > 0) return setErrors(errors)\n```\n\n単体テスト 14 件がすべて通過しています。", Some(&a.message_id)).unwrap();
    s.mark_read(main, &b.message_id).unwrap();
    let c = s
        .user_post_main(&ch, "エラーの文言、英語のままになっていない？")
        .unwrap();
    let r1 = s.user_reply(&ch, &c.message_id, "確認して").unwrap();
    s.agent_post_thread(&ch, &r1.thread.thread_id, "3 箇所が英語のままでした。\n\n- Email is required\n- Password is required\n- Invalid email format", Some(&r1.message.message_id)).unwrap();
    let d = s.agent_post_main(&ch, "セッション切れの判定も同じ箇所にあったので確認しました。現状の挙動は正しく、変更は不要です。", Some(&c.message_id)).unwrap();
    let (t, first) = s
        .agent_create_thread(&ch, &d.message_id, "念のためテストも追加しました。")
        .unwrap();
    s.agent_resolve(&ch, &t.thread_id, &first.message_id)
        .unwrap();
    s.mark_read(main, &d.message_id).unwrap();
    let e = s
        .agent_post_main(
            &ch,
            "パスワード再設定画面にも同じ抜けがあります。共通の **validate** に寄せられます。",
            Some(&d.message_id),
        )
        .unwrap();
    println!("{}", e.message_id);
}
