# Braydr — 技術選定とアーキテクチャ

`product-spec.md` の仕様を、Rust と GPUI でどう実装するかを定める。

- 仕様は `product-spec.md`、見た目は `design-system.md` に書く。この文書は実装の方針だけを扱う。
- 当面の対象ハーネスは Claude Code のみ。他のハーネスは考慮しない。
- バージョンと事実は 2026-10-31 時点の調査による。出典は末尾に置く。
- 実装前の検証の結果は 8 章に置く。

---

## 1. 技術スタック

| 領域 | 採用 | バージョン | 理由 |
| --- | --- | --- | --- |
| 言語 | Rust（edition 2024） | stable 1.99.1 | GPUI が最新の stable を前提にする |
| UI 基盤 | `gpui-pre`（`gpui-kit` が固定する版） | =0.3.10 | 3 章を参照 |
| UI 部品 | `gpui-kit` | =0.8.1 | IME 対応の入力、Markdown、仮想リスト、テスト支援を持つ |
| 非同期（アプリ） | GPUI の executor と `smol` | gpui 依存のまま | アプリに tokio を入れずに済む |
| 非同期（MCP） | `tokio` | 1.x | `rmcp` の前提。サイドカーの中に閉じる |
| DB | SQLite（`rusqlite` + `bundled`） | 0.41 | ローカル 1 ユーザーに十分。FTS5 が有効 |
| マイグレーション | 自前（`PRAGMA user_version`） | — | `rusqlite_migration` は最新の rusqlite に追従していない。数行で済む |
| ID | `uuid`（`v7`） | 1.x | `product-spec.md` の UUIDv7 |
| シリアライズ | `serde` / `serde_json` | 1.x | JSON-RPC と MCP の共通基盤 |
| エージェント向けツール | `rmcp`（MCP 公式 SDK） | 3.x | stdio サーバーを作れる |
| Markdown | `gpui-kit` の `TextView` | 同上 | 別途パーサを持たない |
| 通知 | GPUI の `SystemNotification` | 同上 | `tag` の置換、取り消し、選択時の応答を持つ |
| フォルダ選択 | GPUI の `prompt_for_paths` | 同上 | `rfd` を足さずに済む |
| ログ | `tracing` | 0.1 | 構造化ログ |
| テスト | `#[gpui_kit::test]`、`cargo-nextest`、`proptest` | — | 6 章 |

採用しないもの:

| 候補 | 見送る理由 |
| --- | --- |
| `gpui`（crates.io の公式版） | 最新の 0.2.2 は 2025-10 の公開で、1 年更新がない |
| `sqlx` | 非同期 ORM の利点がない。SQLite を 1 スレッドで扱うので rusqlite で足りる |
| `objc2-user-notifications` の直接利用 | GPUI の API で要件を満たせる |
| `tokio` のアプリ本体への導入 | GPUI と 2 つのランタイムを橋渡しする費用に見合わない |

---

## 2. 全体構成

```text
┌─────────────── Braydr.app ────────────────┐
│  GPUI メインスレッド                       │
│   ├ ビュー / Entity（画面状態）            │
│   └ AppService（ユースケース）             │
│        │             │            │        │
│   DB スレッド     herdr client  IPC server │
│   (rusqlite)      (Unix socket) (Unix socket)
└──────┬───────────────┬────────────┬───────┘
    braydr.db          │            │
                       ▼            │
                 herdr-core         │ JSON 行プロトコル
                  (hc core)         │
                       │ 起動        │
                       ▼            │
                 エージェント ─stdio MCP→ braydr-mcp（サイドカー）
```

### プロセス

| プロセス | 役割 |
| --- | --- |
| `Braydr.app` | UI、DB の唯一の所有者、配送、督促ループ、通知 |
| `braydr-mcp` | エージェントごとに起動する MCP stdio サーバー。DB に触らず、アプリへ中継するだけ |
| `herdr-core` | セッションの実行。別途インストールされたものを使う |

DB をアプリだけが触ることで、`after` の検査、解決と履歴の同時記録、通知の順序を 1 か所で保証する。
アプリが起動していないとき、ツールはエラーを返す。

### クレート

```text
crates/
├ braydr-core    ドメインの型と規則。GPUI と SQLite に依存しない
├ braydr-store   SQLite の実装。スキーマ、マイグレーション、検索
├ braydr-herdr   herdr-core の JSON-RPC クライアント
├ braydr-runtime セッション、配送、再通知、応答の監視。UI に依存しない
├ braydr-ipc     ツールの定義と JSON、アプリとサイドカーの間のプロトコル
├ braydr-mcp     サイドカーのバイナリ（rmcp + tokio）
└ braydr-app     アプリのバイナリ（gpui-kit）
```

依存は `app → runtime → core / store / herdr / ipc`、`mcp → ipc` の向きだけにする。
UI なしで規則を試せるよう、仕様の判断は `braydr-core` に集める。

### スレッドと非同期

| 実行場所 | 担当 | 備考 |
| --- | --- | --- |
| GPUI メインスレッド | ビュー、Entity、DB の読み書き | herdr-core を直接呼ばない |
| GPUI background executor | セッションの作成と停止 | 終わったら Entity に結果を返す |
| OS スレッド | IPC サーバー、チャンネルごとの配送キュー、状態の確認と再通知（5 秒ごと）、イベントの購読 | 同期 I/O を使う |

DB は `Mutex<Store>` 1 つで共有し、書き込みはこのロックで直列になる。SQLite の読み書きは 10 万件でも
数十ミリ秒以内なので、当面メインスレッドから呼ぶ。引っかかるようならここを DB スレッドに分ける。

セッションの状態は `Runtime` がキャッシュし、描画中は `cached_state` だけを読む。UI は 300ms ごとに
`Runtime::revision` を見て、変わっていれば読み直す。

---

## 3. GPUI の依存方針

### 調査結果

- 公式の `gpui` は crates.io で 0.2.2（2025-10-22 公開）のまま。Zed 本体の開発は続いているが、
  README は pre-1.0 で破壊的変更が多いと明記している。
- `gpui-pre` は Zed の特定コミットのスナップショットで、ほぼ週次で公開されている。
  ライセンスは Apache-2.0。公開者は `gpui-kit` の開発者（huacnlee）で、Zed の公式配布ではない。
- `gpui-kit` は `gpui-pre` を完全一致で固定し、再公開する。Longbridge Pro の製品で使われている。
- herdr の GPUI クライアント `herdr-gpui` も `gpui-pre` を固定して使っている。

### 方針

- `Cargo.toml` では `gpui-kit = "=0.8.1"` と完全一致で固定する。`gpui` は `gpui_kit` の再公開を通して使う。
- `Cargo.lock` をコミットし、更新は専用のコミットで行う。更新時は 6 章の手動確認も行う。
- 使うのは `gpui-kit` の入力、`TextView`、仮想リスト、`TitleBar` などに絞る。
  見た目は `design-system.md` のトークンを自前のテーマ型に写し、独自のビューで組む。
- `gpui-pre` の公開が止まった場合は、`gpui-kit` を外して Zed のコミットを git 依存で固定する。
  この乗り換えを小さくするため、`gpui-kit` の部品は `braydr-app` の `ui/` の中で包んで使う。

---

## 4. 主要な設計

### データベース

- 場所は `~/Library/Application Support/Braydr/braydr.db`。WAL を使い、外部キーを有効にする。
- `product-spec.md` 5 章のエンティティをそのままテーブルにする。`channels`、`messages`、`threads`、
  `thread_state_events`、`read_positions`。
- 解決状態の変更と履歴の記録、解決済みへの投稿と自動の再開は、1 つのトランザクションで行う。
- UUIDv7 は DB スレッドで生成し、スレッド内で単調に増えることをテストで確かめる。

### 検索

SQLite の FTS5 `trigram` トークナイザで日本語の部分一致を実現する。

| 検索語の長さ | 方法 |
| --- | --- |
| 3 文字以上 | FTS5 の `MATCH` をフレーズで実行 |
| 1〜2 文字 | trigram が使えないので、チャンネルで絞った `LIKE` |

`messages` を外部コンテンツとする FTS テーブルをトリガーで同期する。メッセージは追記のみなので
INSERT のトリガーだけで足りる。1 チャンネル内の検索なので、`LIKE` の速度は問題にならない想定で、
1 万件のフィクスチャで確かめる。

### herdr-core クライアント

- 接続先は `HC_SOCKET`、なければ既定のソケット。`$XDG_RUNTIME_DIR/herdr-core/<session>.sock` を含む
  解決規則は `hc` のものに合わせる。
- 接続は 2 本持つ。通常の呼び出し用と、`events.subscribe` 専用。
- `hello` で `generation` と `methods` を確認する（`product-spec.md` 7.1）。
- 型は `hc.openrpc.json` のうち使うものだけを手で書く。ドリフトは 6 章の契約テストで検出する。
- 使うのは `hello`、`terminal.run`、`terminal.get`、`terminal.list`、`terminal.send`、`terminal.kill`、
  `terminal.rm`、`events.subscribe`。

### エージェント向けツール

`braydr_*` は MCP の stdio サーバーとして公開する。アプリを MCP サーバーにせず、サイドカーを挟む。

```text
エージェント ─(MCP/stdio)→ braydr-mcp ─(JSON 行/Unix socket)→ Braydr.app
```

- MCP の詳細と tokio がアプリに入らない。
- ソケットは `~/Library/Application Support/Braydr/ipc.sock`、権限は 0600。
- サイドカーは環境変数 `BRAYDR_CHANNEL_ID` を要求に付ける。ツールはチャンネルを引数に取らない。
- ツールの判断（`after`、`unanswered` など）はアプリ側の `braydr-store` で行い、サイドカーは検証しない。
- ツールの定義は `braydr-ipc` に置き、サイドカーはそれを公開して中継するだけにする。
- スキルの本文は `skills/braydr/SKILL.md` に置く。

### エージェントの起動とセットアップ

herdr-core のテンプレートは使わない。役割の保存機能で、ハーネスとモデルの選択には合わないため。
起動コマンドとプロンプトをユーザーの自由入力とし、`terminal.run` の `line` で起動する（`product-spec.md` 8.1）。

- `line` はシェルで実行されるので、プロンプトは引用して連結する。単体テストで引用符や改行を確かめる。
- `argv` ではなく `line` を使う。`argv[0]` だと、同名のテンプレートがあるときに解釈が変わるため。
- プロンプトは起動引数で渡すので、エージェントの準備完了を待つ必要がない。
- `env` で `BRAYDR_CHANNEL_ID` を渡す。サイドカーはこの値をアプリに送る。
- core はエージェントの種類を認識しないので、アプリもハーネスを解釈しない。文字列として扱う。

スキルと MCP の設定はハーネスごとに異なるので、ユーザーが行う。アプリは関与せず、設定ファイルも
書き換えない。スキルの本文はリポジトリに置き、手順は README に書く。

### 配送と督促

- `MessagePosted` のコミット後に、チャンネルごとの直列キューから `terminal.send` を呼ぶ（`product-spec.md` 7.2）。
- 送信は at-least-once。アウトボックスは持たない。コミット後に落ちても、督促ループが拾う。
- 督促ループはチャンネルごとの期限を持つタイマーとして実装する。条件の判定は `braydr-core` に置く。

### 既読と通知

既読の進行は `product-spec.md` 4.5 のとおり、画面に表示された範囲で決める。

- タイムラインは `gpui-kit` の可変高の仮想リストで描く。描画後に、下端が表示領域に入った最新の
  メッセージを求める。
- 判定は `braydr-core` の純粋関数にし、書き込みは 500ms 程度にまとめる。既読位置は戻らない。
- 判定には `ListState::bounds_for_item` と `viewport_bounds` を使う（8 章）。

通知は GPUI の API に、タイムライン単位の `tag` を渡すだけで要件を満たす。

| 要件 | 実装 |
| --- | --- |
| タイムラインごとに 1 件 | `tag` を `main:<channel_id>` / `thread:<thread_id>` にする。同じ `tag` は置き換わる |
| 開いたら取り消す | `dismiss_system_notification(tag)` |
| 選ぶと開く | `on_system_notification_response` の `tag` から移動する |
| 前面でも表示する | GPUI が `Banner` と `List` で提示する |

通知を出すかの判定（フォーカス、表示中か、最下部か）は `braydr-core` の純粋関数にする。

`UNUserNotificationCenter` は `.app` バンドルがないと動かない。GPUI は bundle identifier がなければ
通知を無効にして、ログを残すだけにする。開発時に通知を試すときはバンドルして起動する。

### UI

- 画面状態は `Workspace` の Entity に置く。`Timeline` はメインとスレッドで共用し、パネルとダイアログは
  `Overlay` の 1 つの枠で持つので、同時に 2 つ開かない。
- データの変更は `Runtime::revision` で知らせ、`Workspace` が読み直す。
- タイムラインは今のところ全件を読み込む。行の高さの計測は `ListState` が必要な分だけ行う。
  件数が増えて読み直しが重くなったら、ページングに変える。
- ウィンドウは `TitleBar::window_options()` を土台に、信号機の位置を y = 30 に合わせ、
  `app_owns_titlebar_drag` を有効にして、シート上部をドラッグ領域にする。
- トークンは `design-system.md` の値を `ui/theme.rs` に写し、ビューに生の色を書かない。
- 投稿欄は `gpui-kit` の `Textarea` を使う。⌘Enter は独自の `KeyBinding` で送信に割り当て、Enter は既定の改行のままにする。
- `list` はスクロールバーを描かないので、`gpui-base` の `Scrollbar` を `ListState` に組み合わせる。
- フォントは IBM Plex Sans JP と IBM Plex Mono をアプリに同梱する。

---

## 5. エラー処理

| 種類 | 扱い |
| --- | --- |
| ツールのエラー | `product-spec.md` 6.5 のコードを `braydr-core` の enum にし、そのまま返す |
| herdr-core に接続できない | 全チャンネルを `stopped` として示し、バックオフで再接続する。`generation` 不一致は起動を止める |
| DB の書き込み失敗 | 投稿欄に入力を保持し、再試行できるようにする |
| マイグレーション失敗 | 起動を止める。DB を上書きせず、事前にバックアップを取る |
| サイドカーがアプリに接続できない | `internal` で返す |

---

## 6. テスト戦略

仕様の多くは UI に依存しない。規則を `braydr-core` に集め、下の層ほど厚くテストする。

| 層 | 対象 | 道具 | 実行 |
| --- | --- | --- | --- |
| 単体 | 未読、未返信、`after`、通知の判定、督促の条件 | 標準のテスト、`proptest` | 常時 |
| DB | 不変条件、トランザクション、マイグレーション、検索 | 一時ファイルの実 SQLite | 常時 |
| ツール | 全ツールの正常系と全エラーコード | JSON の検証 | 常時 |
| 配送 | 送信順序、再送、停止、督促 | 偽の herdr サーバー（Unix socket）、GPUI の決定的な executor | 常時 |
| UI | クリック、キー入力、スクロール、フォーカス、IME の状態遷移 | `#[gpui_kit::test]`、`test-support` | 常時 |
| 契約 | herdr-core の実 API | 実物の `hc` を `HC_SESSION` で隔離 | CI |
| MCP | サイドカーをプロセスとして起動し、`rmcp` のクライアントで操作 | 結合テスト | CI |
| 手動 | OS の IME、通知、信号機、ピクセル | 確認表 | リリース前と GPUI の更新時 |

### 重点

- **並行性:** `#[gpui_kit::test(iterations = N)]` で実行順を変え、配送の順序とイベントの競合を確かめる。
  失敗は `SEED` で再現できる。
- **性質テスト:** 既読位置は戻らない、解決済みへの投稿で必ず未解決に戻る、`post_count` が実数と合う。
- **通知:** テスト用コンテキストの `shown_system_notifications`、`dismissed_system_notifications`、
  `simulate_system_notification_response` で検証する。
- **IME:** `TestInput` で変換中の Enter が送信にならないことを確かめる。OS の候補ウィンドウは手動で確認する。
- **スキーマの追従:** `hc schema` の出力を保存したものと比べ、使うメソッドの差分を CI で検出する。

### 検証できないこと

`gpui-kit` の文書も、OS の IME、アクセシビリティ、ピクセルは別途の確認が要ると明記している。
次は手動の確認表に残す。

- 日本語入力の変換確定と Enter、⌘Enter
- 通知の表示、置き換え、取り消し、選択時の遷移
- 信号機とヘッダーの位置、全画面表示
- `design/braydr.pen` との見た目の比較

### CI

macOS のランナーで `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo test`。
実物の `hc` を使う確認は例として置き（`braydr-herdr` の `events`、`braydr-runtime` の `e2e`）、当面は手元で実行する。

---

## 7. ビルドと配布

- `.app` は `scripts/bundle.sh` で組む。`Info.plist` と `braydr-mcp` を含める。フォントは実行ファイルに埋め込む。
  `cargo-bundle` は使わない（サイドカーの同梱が単純なスクリプトで済むため）。
- `braydr-mcp` は `Contents/MacOS/` に置く。
- 配布するときは Developer ID で署名し、公証する。当面は個人利用なのでアドホック署名でよい。
- 対象は macOS の Apple Silicon のみ。

---

## 8. 検証結果とリスク

### 検証結果（2026-11-01、hc 0.1.16、Claude Code 2.1.288）

`terminal.run` の `line` と `env` で、対話モードの Claude Code を起動して確かめた。

| 項目 | 結果 |
| --- | --- |
| 起動引数の `/braydr` | スキルとして動いた |
| `BRAYDR_CHANNEL_ID` | MCP サーバーの起動時とツールの呼び出し時に届いた |
| `terminal.send` | 応答待ちのセッションに届き、ツールが呼ばれた |
| `line` の返り値 | `first_message` は null、`template` は null。テンプレートとして解釈されない |
| 状態報告 | `status` は null、`reported` は false。Claude Code は OSC 7501 を送らなかった |
| 初めてのディレクトリ | 信頼の確認で停止した。キー入力で承認すると、続けてプロンプトが実行された |

エージェントが止まる原因は確認画面、MCP やスキルの未設定など複数あり、アプリからは区別できない。
そこで、ツールの呼び出しがないまま再通知の間隔が経ったことだけを検知し、原因は断定しない（`product-spec.md` 4.6）。
IPC サーバーはチャンネルごとに最後のツール呼び出しの時刻を保持する。

### イベントの検証（2026-11-01、hc 0.1.16）

`braydr-herdr` の例 `events` で確かめた。`events.subscribe` はラベルで絞った terminal のスナップショットを返し、
`terminal.kill` の後に `finished` が届く。

### 一周の検証（2026-11-01、hc 0.1.16、Claude Code 2.1.288、Haiku）

アプリなしで `braydr-runtime` の `examples/e2e.rs` を動かし、本物の `hc` と Claude Code で次を確かめた。

| 流れ | 結果 |
| --- | --- |
| チャンネルを作成してセッションを起動 | 起動した。空のチャンネルには投稿しなかった |
| メインへの投稿 → 通知 → エージェントの返信 | 8 秒で返信 |
| スレッドの作成 → 返信 → エージェントの解決 | 8 秒で解決 |
| 解決済みへの投稿 → 再開 → 返信 | 5 秒で返信。履歴に解決と再開が残った |
| 新しいセッションの開始 → 履歴の読み直し → 未対応の依頼に返信 | 10 秒で返信。古いセッションは停止した |

実機で未確認のもの: `events.subscribe`、再通知、停止の検知。これらは偽の core でのテストだけで確かめている。

### 検証結果（2026-11-01、gpui-kit 0.8.1、SQLite 3.54.1）

検証用のアプリと自動テストで確かめた。検証用のコードは削除した。

| 項目 | 結果 |
| --- | --- |
| 未読の位置から開く | `ListState::scroll_to` で 2000 件中の 1200 件目から開けた |
| 既読の判定 | `bounds_for_item` と `viewport_bounds` で、下端が表示領域に入った最新のメッセージを求められた |
| 上下のスクロール | 下へ進むと既読が進み、未読の上へも戻れ、戻っても既読は下がらなかった |
| Enter | 改行になり、送信されない |
| ⌘Enter（`InputEvent::PressEnter`） | 送信できるが、**カーソル位置に改行も入る**。使わない |
| ⌘Enter（独自の `KeyBinding`） | 改行が入らずに送信できた。こちらを採用する |
| FTS5 `trigram` | 10 万件で 3 文字以上の一致が 20ms 以内。英字の大文字小文字を区別しない。2 文字は 0 件 |
| 2 文字の `LIKE` | 1 チャンネル 1000 件で 3ms。十分に速い |

注意点:

- 未計測の範囲を一度に飛ぶと、スクロール位置が大きく先へ進む。1500px の一括移動で 1200 件目から 1994 件目まで進んだ。
  既読が一気に進むので、スクロールバーのドラッグ中は既読を進めない。`overdraw` は余裕を持たせる。
- `list` はスクロールバーを描かない。タイムラインには自前でスクロールバーを付ける。

手動でも確かめ、問題がなかった。

| 手動で確認したこと | 結果 |
| --- | --- |
| 変換中の Enter が確定だけになり、確定後は改行になるか。⌘Enter の送信 | 問題なし |
| `TextView` を 2000 件並べたときのスクロール性能と、テキスト選択 | 問題なし |

その他のリスク:

| リスク | 影響 | 対策 |
| --- | --- | --- |
| `gpui-pre` と `gpui-kit` の更新停止 | 長期の追従ができない | 3 章の乗り換え経路を保つ |
| GPUI の破壊的変更 | 更新のたびに修正が要る | 完全一致で固定し、必要なときだけ更新する |
| herdr-core の API は preview | 仕様が変わる | `generation` の検査と契約テスト |

---

## 9. 実装の順序

1. **土台:** ワークスペース、`braydr-core`、`braydr-store`、単体と DB のテスト、CI。
2. **ツール:** IPC、`braydr-mcp`、全ツール。エージェントだけで往復できる状態にする。
3. **セッション:** herdr クライアント、配送、督促。
4. **UI:** チャンネル、タイムライン、投稿欄、右シート。
5. **UI の残り:** 未読、未解決の一覧、検索、通知、ダイアログ、設定。
6. **仕上げ:** `.app` のバンドル、手動の確認表、`design/braydr.pen` との照合。

ツールとセッションを UI より先に置く。Braydr の価値はエージェントとの往復にあり、最大の不確実性もそこにあるため。

---

## 出典

- [zed-industries/zed: crates/gpui/README.md](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- [crates.io: gpui](https://crates.io/crates/gpui) / [gpui-pre](https://crates.io/crates/gpui-pre) / [gpui-kit](https://crates.io/crates/gpui-kit)
- [longbridge/gpui-kit: README](https://github.com/longbridge/gpui-kit) / [TESTING.md](https://github.com/longbridge/gpui-kit/blob/main/crates/kit/TESTING.md)
- [docs.rs: gpui::test](https://docs.rs/gpui/latest/gpui/attr.test.html)
- [gpui-pre-macos: system_notifications.rs](https://docs.rs/crate/gpui-pre-macos/0.3.1/source/src/system_notifications.rs)
- [penso/herdr-gpui](https://github.com/penso/herdr-gpui)
- [herdrdev/herdr-core: schema/README.md](https://github.com/herdrdev/herdr-core/blob/master/schema/README.md) / [docs/templates.md](https://github.com/herdrdev/herdr-core/blob/master/docs/templates.md) / [docs/security.md](https://github.com/herdrdev/herdr-core/blob/master/docs/security.md)
- [modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk) / [docs.rs: rmcp](https://docs.rs/rmcp/latest/rmcp/)
- [rusqlite: libsqlite3-sys/build.rs](https://github.com/rusqlite/rusqlite/blob/master/libsqlite3-sys/build.rs)
- [Apple Developer Forums: UNUserNotificationCenter と LaunchAgent](https://developer.apple.com/forums/thread/679326)
- [SQLite User Forum: FTS5 trigram](https://sqlite.org/forum/forumpost/dc1a0b736f2adf57?raw)
