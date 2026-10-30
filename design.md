# Braydr — 設計

コーディングエージェントとのやり取りを、Slack 風のタイムライン＋スレッドで行う。
エージェントの実行基盤は自作せず、herdr-core に任せる。

- 前提: 1 ユーザー / ローカル / 認証なし
- このドキュメントでは技術選定をしない
- 機能、条件、データ、API を扱う。見た目と文言は `design-language.md` に書く
- 「投稿（post）」はタイムライン上の 1 件の書き込み、「メッセージ（message）」は
  保存された発言を指す。タイムラインの読み書きのツール名は `post`、
  データと返り値は `message` を使う

---

## 1. 目的

- 人間とコーディングエージェントの会話を、チャンネル → メインのタイムライン → スレッドの
  構造で整理する。
- 人間はアプリで投稿・閲覧・解決を行う。エージェントは herdr-core のセッションで動き、
  ツール経由で同じデータを読み書きする。
- アプリはエージェントの実行基盤を持たない。セッションの起動・停止は
  herdr-core に委譲する。
- エージェントの文脈はセッションが持つ。アプリは履歴を自動注入しない。
  必要な文脈はスキルとツールで読む。

---

## 2. 前提

- 1 ユーザー、1 マシン、ローカル利用。認証・共有・クラウドは対象外。
- 1 チャンネルに 1 セッション。セッションはチャンネルに紐づく。
- チャンネルは作業ディレクトリと herdr-core のテンプレートを持つ。セッションはこの 2 つで起動する。
- アプリは herdr-core の JSON-RPC API を直接使う。
  `session_id` は `terminal.run` が返す terminal の `address`。
- メッセージはテキスト（Markdown）。添付ファイル・編集・削除は対象外。
- 時刻は UTC で保存し、UI は端末のローカルタイムで表示する。
- `message_id` / `thread_id` / `channel_id` は UUIDv7。時系列順に並ぶ。
- `channel_id` は内部 ID。ユーザーはチャンネル名（表示名）を設定する。
- スキルの引数には `channel_id` を渡す。terminal 名は指定しない。
- スレッドにタイトルは持たせない。親メッセージが実質のタイトルになる。

---

## 3. 概念

| 用語 | 定義 |
| --- | --- |
| チャンネル | 会話の単位。セッションを 1 つ持ち、メインのタイムラインと複数のスレッドを含む |
| メインのタイムライン | チャンネル直下のメッセージ列。スレッドの起点もここに並ぶ |
| スレッド | メインのタイムラインの 1 つのメッセージを起点とする会話。起点に付いたメッセージが並ぶ |
| スレッドのタイムライン | スレッド内のメッセージ列。親メッセージは含めない |
| 親メッセージ | スレッドの起点になったメインのタイムラインのメッセージ |
| メッセージ | タイムラインに並ぶ発言 1 つ。`author` は `user` か `agent` |
| 未解決 | スレッドの状態。`resolved_at` が null |
| 解決済み | スレッドの状態。`resolved_at` が非 null |
| 未返信（エージェント待ち） | 未解決スレッドのうち、最後のメッセージがユーザーのもの。エージェントが返す番。エージェントのツールとループでだけ使う |
| 未返信（ユーザー待ち） | 未解決スレッドのうち、最後のメッセージがエージェントのもの。エージェントのツールでだけ使う |
| 解決状態の履歴 | スレッドの解決・再開の記録。誰がいつ操作したかを持つ。メッセージではない |
| 既読位置 | ユーザーごとの、スコープごとの最終既読メッセージ。ユーザー向けのみ |
| 未読 | 既読位置より新しいメッセージがあること。解決状態・未返信とは別軸。UI は有無だけを示す |

### 3.1 3 つの軸は独立

- **解決状態**: スレッドを閉じたかどうか。`resolved_at` の有無だけ。
- **未返信**: 未解決のスレッドの中でだけ成立する。最後の投稿者がどちらかで決まる。
  エージェント向けの概念で、ユーザー向けの UI には出さない。
- **未読**: ユーザーがそのスコープを最後にどこまで読んだかで決まる。解決状態とも
  未返信とも連動しない。

したがって、次のような状態はすべて成立する。

- 解決済みだが未読がある（解決前に届いたメッセージを読んでいない）
- 未解決で、ユーザーが返す番（未返信（ユーザー待ち））
- 未解決で、エージェントが返す番（未返信（エージェント待ち））
- 未読だが、未返信ではない

### 3.2 メインのタイムラインには未返信・解決を適用しない

未返信と解決はスレッドの概念とする。メインのタイムラインのメッセージは、返信が付くことで
スレッドになり、そのスレッドが状態を持つ。メインのタイムラインそのものは未解決・解決済みに
ならない。

メインの依頼への対応は、次のどちらかで完了する。

1. メインのタイムラインにエージェントが返信する
2. そのメイン投稿を親とするスレッドを作り、そこへ返信する

---

## 4. ユーザー向け機能

### 4.1 チャンネルとセッション

#### チャンネル作成

ユーザーはチャンネル名、作業ディレクトリ、テンプレートを指定してチャンネルを作る。

- 名前は 1〜64 文字。前後の空白を除き、改行・制御文字は不可。空の間は作成できない。
- 確定は明示的な操作で行う。IME の確定と衝突するので、Enter では作成しない。
- テンプレートの選択肢は herdr-core の `templates.list` で取得する。`error` があるものは選べない。
- テンプレートの既定値はアプリの設定で持つ。
- 作成するとセッションを起動する（8.1）。**エージェントに投稿させない。** メインのタイムラインは
  空のままにする。準備完了の挨拶も、システムメッセージも作らない。
- 起動に失敗した場合もチャンネルは残し、再試行できるようにする。投稿は保存できるが、通知は届かない。

#### チャンネル名の変更

ユーザーはチャンネル名を変更できる。検証は作成時と同じで、Enter では確定しない。
変更は次の通知の `channel_name` から反映し、実行中のセッションには知らせない。

#### セッションの状態

- `running`: `terminal.get` の `state` が `live`
- `starting`: `terminal.run` の呼び出し中
- `stopped`: `terminal.get` が `finished` / `not_found`、または `lost` イベント

状態は `terminal.get` と `events.subscribe` で更新する。アプリはセッションを自動再起動しない。

ユーザーに示すのは生きているかどうかだけ。エージェントが作業中かどうかは示さない。
`starting` は `running` と区別しない。

#### 新しいセッションの開始

herdr-core にはセッションの再開機能がない。復帰は新しいセッションの開始で行う。手順は 8.3。

- 開始するときは常にユーザーの確認を取る。エージェントの作業コンテキストが引き継がれないことを伝え、
  生きているセッションなら進行中の作業が中断されることも伝える。
- 作業ディレクトリとテンプレートは現在の値を初期値にし、変更できる。
- チャンネルと履歴は残す。メッセージも既読位置も変更しない。
- 異常終了時も同じ操作で復帰する。

### 4.2 会話

- チャンネルを開くと、メインのタイムラインを表示する。
- メインのタイムラインとスレッドのタイムラインに投稿できる。返信はメインには並べない。
- 投稿は送信操作と ⌘Enter で行う。IME の確定と衝突するので、Enter は改行にする。
- 投稿は Markdown として保存・表示する。コードブロックもそのまま扱う。
- 投稿に失敗したら入力内容を保持する。
- 親メッセージは編集・削除できない。
- すべてのメッセージに投稿時刻を表示する。
- メインのメッセージには、スレッドがあれば返信数、最終返信時刻、解決状態、未読の有無を表示する。
- すべてのメッセージの Markdown の原文をコピーできる。

#### スレッドの開始

- メインのメッセージなら、発言者を問わず親にできる。親メッセージ 1 つにつきスレッドは 1 つ。
- ユーザーはメインのメッセージに返信してスレッドを始める。返信を書いている間はスレッドは存在せず、
  解決もできない。最初の投稿でスレッドを作る。投稿せずに閉じれば何も残らない。
- すでにスレッドがあれば、そのスレッドへの投稿になる。解決済みなら投稿時に未解決に戻す（4.3）。
- エージェントは `braydr_thread_create` で親と最初の返信を同時に作る。

### 4.3 スレッドの状態

- ユーザーは任意のタイミングでスレッドを解決 / 未解決にできる。
- 解決・未解決はメッセージを作らない。状態を変更し、解決状態の履歴に記録する。
- スレッドのタイムラインには、メッセージと解決状態の履歴を時系列で並べ、操作した側を示す。
  メインのタイムラインには出さない。
- 未解決に戻しただけでは、エージェントに通知しない。依頼は投稿で行う。ユーザーにもその旨を伝える。
- 解決済みスレッドへの投稿の前に、投稿すると未解決に戻ることを伝える。
- 履歴は未読に数えず、未読区切りの位置にも影響しない。
- 解決すると `resolved_at` と `resolved_by` を設定する。未解決に戻すと両方を null にする。
- **解決済みスレッドにユーザーが投稿したら、アプリが同一トランザクションで自動的に
  未解決に戻し、履歴に記録する。** エージェントが「追加依頼が来たので未解決に戻す」必要はない。
- メインのタイムラインに追加依頼が来た場合、エージェントはメインで返すか、
  その投稿を親とする新しいスレッドを立てる。古い解決済みスレッドを自動で開き直さない。
- エージェントは、自分が未解決と判断したのに解決済みになっている場合に、
  明示的に未解決へ戻せる。たとえば、追加で確認したいとき。
  - 解決済みスレッドへのエージェントの投稿はエラーにする。
    未解決に戻す → 投稿する、の順を必須にして、意図を明確にする。
- エージェントの解決にはガードを置く（6.3）。
  - 最後の投稿がユーザーのままでは解決できない（未返信を閉じない）。
  - 解決時は `after` が最新と一致している必要がある（解決直前の新着を取りこぼさない）。
- ユーザーはこのガードを受けない。ユーザーはいつでも解決できる。

### 4.4 未解決スレッドの一覧

一覧はチャンネルごとの未解決スレッドだけを持つ。チャンネルを横断する一覧、解決済みとすべての一覧、
絞り込みは持たない。解決済みのスレッドにはタイムラインか検索から入る。

- 更新が新しい順に並べ、20 件ずつ追加読み込みする。
- 各行は親メッセージの本文の先頭、返信数、最終返信時刻、未読の有無を持つ。最後の発言は出さない。
- 行を選ぶとスレッドを開く。

### 4.5 未読

#### 既読位置

- 既読位置はユーザー向けのみ。エージェントには持たせない。
- スコープは 2 種類。
  - メインのタイムライン: チャンネル単位
  - スレッド: スレッド単位
- 既読位置は `(scope_type, scope_id)` ごとに 1 つ。最後に読んだ `message_id` を持つ。

#### 更新規則

- 未読がある場合、アプリは既読位置の直後のメッセージ（最初の未読）を特定し、
  そこから最新方向へ必要な分だけ読み込んで、その位置から表示する。既読位置が
  無ければ先頭が最初の未読になる。既読位置は、実際に表示した範囲の最新まで進める。
  表示していない、より新しいメッセージは未読のまま残す。
- スコープを開いて表示したとき、**表示したページの最新メッセージ**まで既読にする。
  その時点の絶対的な最新ではない。表示していない新着を既読にしないため。
- スコープを開いていて、ウィンドウがアクティブで、タイムラインが最下部にある間は、
  新着もその場で既読にする。
- 最下部にいない（過去を読んでいる）間の新着は未読のままにする。
- ユーザー自身の投稿は、そのメッセージまで既読にする。
- チャンネルを開いても、スレッドの既読位置は動かさない。
- スレッドを開いても、メインの既読位置は動かさない。
- 既読位置は戻らない。

#### 未読の表示

未読は有無だけを示す。件数は示さない。

- チャンネル: メインのタイムラインに未読があるとき。スレッドの未読は含めない。
  セッションの停止と重なったときは停止を優先して示す。
- スレッド: スレッドのタイムラインに既読位置より新しいメッセージがあるとき。
  親メッセージはスレッドの未読には数えない（メイン側で数える）。
- 開いているチャンネルとスレッドには示さない。開いた時点で既読になるため。
- 解決済みスレッドにも未読は立つ。解決状態と未読は別軸。
- 解決状態を変えても既読位置は動かさない。解決前に届いたメッセージを読んでいなければ、
  解決済みでも未読のまま残る。

#### 区切り

- 未読のあるタイムラインを開いたとき、最初の未読メッセージの直前に
  「ここから未読」の区切りを出す。
- 区切りを出す位置を決めてから既読位置を更新する。
- 未読が無い場合は区切りを出さず、最新位置（最下部）から表示する。
- 区切りは保存しない。表示上の一時的なもの。

### 4.6 画面

| 画面 / 領域 | 内容 |
| --- | --- |
| サイドバー | チャンネル一覧、チャンネル作成、セッションの生死、メインの未読、未解決スレッド数 |
| チャンネル画面 | チャンネル名のメニュー、セッションの停止、検索、未解決スレッドの一覧、メインのタイムライン、投稿欄 |
| スレッド | 親メッセージ、返信、解決状態の履歴、未読区切り、投稿欄、解決 / 未解決に戻す |

- チャンネル一覧は最終投稿が新しい順。
- 未解決スレッド数は 0 件なら示さない。
- チャンネル名のメニューは、名前の変更、テンプレート名、新しいセッションの開始を持つ。
  停止中は終了理由（`reason` / `exit_code`）も示す。
- チャンネル作成直後は空のメインのタイムラインを表示する。挨拶は出さない。
- セッションが停止している間は、その旨を投稿欄の近くで示し、新しいセッションを開始できるようにする。

### 4.7 チャンネル内検索

- 開いているチャンネルのメインのタイムラインとスレッドの全メッセージを全文検索する。
- 日本語を部分一致で検索できること。
- ⌘F で開く。入力に応じて結果を更新し、入力欄の Enter には何も割り当てない。
- 結果は新しい順に並べ、メインかスレッドかの区別は示さない。
- 各行は一致箇所を含む抜粋と投稿時刻を持つ。
- 結果を選ぶと、スレッドのメッセージならスレッドを開き、メインのメッセージならタイムラインを
  その位置へ移す。どちらも該当メッセージを一時的に強調する。
- 解決状態の履歴は検索対象にしない。

---

## 5. データモデル

### 5.1 エンティティ

#### チャンネル

| フィールド | 説明 |
| --- | --- |
| `channel_id` | UUIDv7。主キー。アプリが生成する |
| `name` | ユーザーが設定するチャンネル名。1〜64 文字、前後の空白を除く、改行・制御文字不可。文字種の制限なし |
| `session_id` | 現在の herdr-core terminal の `address`（例: `t7`）。未起動なら null |
| `cwd` | セッションの作業ディレクトリ |
| `template` | セッションの起動に使う herdr-core のテンプレート名 |
| `created_at` | 作成時刻 |

補足:

- `channel_id` は内部 ID。UI と通知では `name`（通知では `channel_name`）を表示する。
- ツールの `channel` 引数には `channel_id` を渡す。`name` は渡さない。
- 名前は一意でなくてよい。同じ名前のチャンネルが複数あり得る。

#### メッセージ

| フィールド | 説明 |
| --- | --- |
| `message_id` | UUIDv7。主キー。時系列順 |
| `channel_id` | 所属チャンネル |
| `thread_id` | 所属スレッド。null ならメインのタイムライン |
| `author` | `user` または `agent` |
| `text` | Markdown。1〜20,000 文字。空白のみは不可 |
| `created_at` | 投稿時刻 |

補足:

- 親メッセージも `thread_id` は null。メインのタイムラインに含まれる。
- スレッド内のメッセージは `thread_id` に所属スレッドを持つ。
- メッセージは追記のみ。編集・削除はしない。

#### スレッド

| フィールド | 説明 |
| --- | --- |
| `thread_id` | UUIDv7。主キー |
| `channel_id` | 所属チャンネル |
| `parent_message_id` | 親メッセージ。チャンネル内で一意 |
| `created_at` | 最初の返信が付いた時刻 |
| `updated_at` | 最後に投稿または解決状態が変わった時刻。一覧の並びに使う |
| `resolved_at` | 解決時刻。null なら未解決 |
| `resolved_by` | `user` または `agent`。未解決なら null |

補足:

- スレッドは最初のメッセージが付いた時点で作られる。返信が 0 件のスレッドは存在しない。
- 親メッセージ 1 つにつきスレッドは 1 つ。スレッドの入れ子は無い。
- `resolved_at` と `resolved_by` は同時に設定・同時に解除する。
- 現在の状態はこの 2 つで持ち、過去の変更は解決状態の履歴で持つ。

#### 解決状態の履歴

| フィールド | 説明 |
| --- | --- |
| `event_id` | UUIDv7。主キー。時系列順 |
| `thread_id` | 対象のスレッド |
| `action` | `resolved` または `reopened` |
| `actor` | `user` または `agent` |
| `created_at` | 操作した時刻 |

補足:

- 追記のみ。編集・削除はしない。
- 解決済みスレッドへのユーザー投稿による自動の再開は、`actor` を `user` として記録する。

#### 既読位置

| フィールド | 説明 |
| --- | --- |
| `scope_type` | `channel` または `thread` |
| `scope_id` | `channel_id` または `thread_id` |
| `last_read_message_id` | そのスコープで最後に読んだメッセージ |
| `updated_at` | 更新時刻 |

主キーは `(scope_type, scope_id)`。チャンネル ID とスレッド ID の名前空間がぶつからない
ように、種別をキーに含める。1 ユーザー前提なので `user_id` は持たない。

### 5.2 導出値

| 値 | 定義 |
| --- | --- |
| `child_thread_id` | そのメッセージを親とするスレッド。無ければ null。メインのメッセージだけ非 null |
| `reply_count` | スレッド内の返信数（親を含まない） |
| `post_count`（スレッド） | `reply_count + 1`。親を含むメッセージ数 |
| `last_message_id` | 親を含めた最後のメッセージ |
| `last_message_at` | その投稿時刻 |
| `last_author` | 親を含めた最後の投稿者 |
| `awaiting` | 未解決なら `agent`（最後がユーザー）または `user`（最後がエージェント）。解決済みなら null |
| `post_count`（チャンネル） | メインのタイムラインのメッセージ数 |
| スレッド未読 | スレッド内の返信に、既読位置より新しいものがあるか |
| メイン未読 | メインのメッセージに、既読位置より新しいものがあるか |
| 未解決スレッド数 | チャンネル内の未解決スレッドの数 |

### 5.3 不変条件

1. スレッドの親は、同じチャンネルのメインのメッセージ（`thread_id` が null）である。
2. 親メッセージ 1 つにつきスレッドは 1 つだけ。
3. スレッドには必ず 1 件以上の返信がある。
4. メッセージの `thread_id` は、同じチャンネルのスレッドを指す。
5. `resolved_at` が非 null なら `resolved_by` も非 null。未解決なら両方 null。
6. 既読位置の `last_read_message_id` は、そのスコープに属するメッセージを指す。
7. メッセージは追記のみ。状態変更でメッセージを作らない。
8. 解決済みスレッドへのユーザー投稿は、同じトランザクションで未解決に戻し、履歴を記録する。
9. 解決状態の変更は既読位置を変えない。
10. スレッド内のメッセージに `child_thread_id` は無い（常に null）。
11. 解決状態の変更と履歴の記録は同じトランザクションで行う。

### 5.4 順序・ID・時刻

- メインのタイムラインとスレッドのタイムラインは `message_id` の昇順。
- スレッド一覧は `updated_at` の降順、同時刻なら `thread_id` の降順。
- `message_id` は UUIDv7。先頭に時刻を持つので、文字列の昇順がほぼ時刻順になる。
- `thread_id` も UUIDv7。
- `channel_id` も UUIDv7。
- 時刻は UTC の ISO 8601（ミリ秒）。表示は端末のローカルタイム。

### 5.5 主な検索条件

- メインのタイムライン: `channel_id`、`thread_id is null`、カーソルより古い `message_id`
- スレッドのタイムライン: `thread_id`、カーソルより古い `message_id`
- スレッド一覧: `channel_id`、解決状態、最後の投稿者、`updated_at`
- 未読の有無: スコープ内のメッセージで `message_id > last_read_message_id`
- 未読の先頭: スコープ内で `message_id > last_read_message_id` の最小
- チャンネルの未読: メインのみ
- 全文検索: `channel_id`、本文。メインとスレッドの両方を対象に、新しい順

大量のスレッドに耐えるため、スレッド一覧は `updated_at` と `thread_id` のキーセットで
ページングする。件数を数える集計は一覧のたびには行わない。

---

## 6. エージェント向けツール

### 6.1 命名規則

`braydr_<entity>_<action>` の形にする。エンティティを先頭に置き、何を操作するかを
名前から読み取れるようにする。

- entity: `channel` / `thread` / `message`
- action: `get` / `list` / `list_posts` / `create` / `create_post` / `resolve` / `unresolve` / `search`

すべてのツールは第 1 引数に `channel`（`channel_id` の UUIDv7）を取る。アプリは
セッションに紐づくチャンネルと照合し、一致しなければエラーにする。エージェントが
別チャンネルを操作することはできない。`channel` には `channel_name` ではなく
`channel_id` を渡す。

### 6.2 共通ルール

#### `channel`

- すべてのツールで必須。
- セッションに紐づくチャンネルと一致しなければ `channel_mismatch`。
- ツールはこのチャンネルの外を読んだり書いたりできない。

#### `after`

- そのタイムラインで最後に読んだ `message_id`。
- メインへの投稿: メインのタイムラインの最新メッセージと一致する必要がある。
- スレッドへの投稿: スレッドのタイムラインの最新返信と一致する必要がある。
  親メッセージは含めない。
- タイムラインが空の場合だけ省略できる。空でないのに省略すると `after_required`。
- 一致しなければ `stale_after`。`details.current_last_message_id` を返すので、
  読み直してから再試行する。
- 解決時も同じ。解決しようとした直前に新着が来たら `stale_after` で止める。
- 成功したか不明なときは、同じ `after` で再試行する。成功していれば最新メッセージが
  変わっているので `stale_after` になる。読み直して自分の投稿を確認する。
- スレッド作成は、親にスレッドを 1 つしか作れない一意制約で重複を防ぐ。既にスレッドが
  あれば常に `thread_exists`。

#### ページング

メッセージ一覧とスレッド一覧は、どちらも不透明な `cursor` でページングする。
クライアントは `next_cursor` をそのまま次の呼び出しに渡す。最初のページは省略する。

- メッセージのタイムライン:
  - `next_cursor` を渡すと、その位置より古い `limit` 件。
  - 返り値は古い順（昇順）。`has_more` が false なら `next_cursor` は null。
  - 並びは `message_id` の昇順。`message_id` は一意なので、`created_at` が同一の
    メッセージがあっても重複・欠落しない。カーソルはこの並び位置を不透明化したもの。
- スレッド一覧:
  - `next_cursor` を渡すと次のページ。
  - フィルタを変えたら先頭から。
  - カーソルはスナップショットではない。一覧の並びは更新で変わる。
- `limit` の既定値: メッセージ 50、スレッド 20、検索 20。最大 100。範囲外は丸める。
- 検索結果: `next_cursor` を渡すと次のページ。並びは `message_id` の降順。

#### 本文

- 1〜20,000 文字。空白のみは `empty_text`。超過は `text_too_long`。
- 前後の空白は保持する。
- Markdown として扱う。要約・整形はしない。

### 6.3 各ツール

全体像:

| ツール | 主な引数 | ページング |
| --- | --- | --- |
| `braydr_channel_get` | `channel` | なし |
| `braydr_channel_list_posts` | `channel`, `cursor?`, `limit` | `cursor` / `next_cursor` |
| `braydr_thread_get` | `channel`, `thread_id` | なし |
| `braydr_thread_list` | `channel`, `filter?`, `cursor?`, `limit` | `cursor` / `next_cursor` |
| `braydr_thread_list_posts` | `channel`, `thread_id`, `cursor?`, `limit` | `cursor` / `next_cursor` |
| `braydr_message_get` | `channel`, `message_id` | なし |
| `braydr_message_search` | `channel`, `query`, `cursor?`, `limit` | `cursor` / `next_cursor` |
| `braydr_channel_create_post` | `channel`, `text`, `after?` | なし |
| `braydr_thread_create_post` | `channel`, `thread_id`, `text`, `after?` | なし |
| `braydr_thread_create` | `channel`, `parent_message_id`, `text` | なし |
| `braydr_thread_resolve` | `channel`, `thread_id`, `after` | なし |
| `braydr_thread_unresolve` | `channel`, `thread_id` | なし |

スレッド一覧（`braydr_thread_list`）、メッセージ一覧（`*_list_posts`）、検索
（`braydr_message_search`）は、いずれもカーソルページングを持つ。

#### `braydr_channel_get(channel)`

チャンネルの名前と状態を返す。メインのタイムラインの件数、最後の投稿、スレッドの集計。

返り値:

```json
{
  "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
  "name": "ログイン修正",
  "post_count": 12,
  "updated_at": "2026-10-31T01:23:45.123Z",
  "last_author": "agent",
  "thread_count_total": 4,
  "thread_count_unresolved": 2,
  "thread_count_resolved": 2,
  "thread_count_awaiting_agent": 1,
  "thread_count_awaiting_user": 1
}
```

- `name` はユーザーが設定したチャンネル名。
- `post_count` はメインのタイムラインのメッセージ数（親メッセージを含む）。
- `updated_at` と `last_author` はメインのタイムラインの最後の投稿。
  メインが空ならどちらも null。
- `thread_count_awaiting_agent` は未返信（エージェント待ち）の数。
- `thread_count_awaiting_user` は未返信（ユーザー待ち）の数。
- `thread_count_unresolved` は上の 2 つの合計。

#### `braydr_channel_list_posts(channel, cursor?, limit=50)`

メインのタイムラインを返す。親メッセージも含む。

返り値:

```json
{
  "messages": [
    {
      "message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
      "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
      "thread_id": null,
      "child_thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
      "author": "user",
      "created_at": "2026-10-31T01:23:45.123Z",
      "text": "..."
    }
  ],
  "has_more": true,
  "next_cursor": "opaque"
}
```

#### `braydr_thread_get(channel, thread_id)`

スレッドのメタデータと親メッセージを返す。返信は含めない。

返り値はスレッドオブジェクト（6.4）。

#### `braydr_thread_list_posts(channel, thread_id, cursor?, limit=50)`

親メッセージ、スレッドの状態、解決状態の履歴、返信を返す。`messages` は返信だけ。
親は `parent_message`。

```json
{
  "thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
  "parent_message": {
    "message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
    "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
    "thread_id": null,
    "child_thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
    "author": "user",
    "created_at": "2026-10-31T01:20:00.000Z",
    "text": "..."
  },
  "messages": [
    {
      "message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
      "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
      "thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
      "child_thread_id": null,
      "author": "agent",
      "created_at": "2026-10-31T01:25:00.000Z",
      "text": "..."
    }
  ],
  "resolved_at": null,
  "resolved_by": null,
  "awaiting": "user",
  "state_events": [
    {
      "event_id": "0190c2e4-4567-7cde-8f01-23456789abce",
      "action": "resolved",
      "actor": "agent",
      "created_at": "2026-10-31T01:26:00.000Z"
    },
    {
      "event_id": "0190c2e5-5678-7cde-8f01-23456789abcf",
      "action": "reopened",
      "actor": "user",
      "created_at": "2026-10-31T01:40:00.000Z"
    }
  ],
  "has_more": false,
  "next_cursor": null
}
```

- `resolved_at` / `resolved_by` / `awaiting` はスレッドの状態。
  投稿前に解決済みかどうかを判断できる。
- `state_events` は解決状態の履歴を古い順に全件返す。ページングしない。誰が解決・再開したかの
  判断に使う。

#### `braydr_message_get(channel, message_id)`

メッセージを 1 件返す。メインでもスレッド内でもよい。
`child_thread_id` で、そのメッセージを起点とするスレッドが分かる。

#### `braydr_thread_list(channel, filter="unresolved", cursor?, limit=20)`

スレッド一覧を返す。各項目に親メッセージを含む。

`filter`:

| 値 | 内容 |
| --- | --- |
| `unresolved` | すべての未解決スレッド（既定） |
| `awaiting_agent` | 未解決かつ最後がユーザー。エージェントの作業キュー |
| `awaiting_user` | 未解決かつ最後がエージェント。ユーザーの返信待ち |
| `resolved` | すべての解決済みスレッド |
| `all` | 解決状態を問わないすべてのスレッド |

```json
{
  "threads": [
    {
      "thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
      "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
      "parent_message_id": "0190d3f4-4567-7def-9012-3456789abcde",
      "parent_message": {
        "message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
        "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
        "thread_id": null,
        "child_thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
        "author": "user",
        "created_at": "2026-10-31T01:20:00.000Z",
        "text": "..."
      },
      "created_at": "2026-10-31T01:21:00.000Z",
      "updated_at": "2026-10-31T01:30:00.000Z",
      "last_message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
      "last_message_at": "2026-10-31T01:30:00.000Z",
      "last_author": "user",
      "post_count": 3,
      "reply_count": 2,
      "resolved_at": null,
      "resolved_by": null,
      "awaiting": "agent"
    }
  ],
  "has_more": true,
  "next_cursor": "opaque"
}
```

- 並びは `updated_at` の降順、同時刻なら `thread_id` の降順。
- `awaiting` は未解決なら `agent` か `user`、解決済みなら null。
- カーソルは読み取り専用のスナップショットではない。処理しながら次ページへ進むと、
  更新された項目が移動して取りこぼすことがある。キューとして使うときは、
  先頭ページを処理してから引き直す（9.3）。

#### `braydr_message_search(channel, query, cursor?, limit=20)`

チャンネル内のメインとスレッドの全メッセージを全文検索する。

- `query` は 1〜200 文字。空白のみは `invalid_query`。
- 返り値は `{ messages, has_more, next_cursor }`。`messages` はメッセージオブジェクトの配列で、
  新しい順に並ぶ。スレッド内の一致は `thread_id` で文脈をたどる。
- 本文を切り詰めない。チャンネルの外は検索できない。
- 解決状態の履歴は対象にしない。

#### `braydr_channel_create_post(channel, text, after?)`

メインのタイムラインに投稿する。`author` は `agent`。

- メインが空なら `after` は省略可。空でなければ必須。
- 成功したか不明なときは、同じ `after` で再試行する。成功していれば `stale_after`
  になるので、読み直して自分の投稿を確認する。
- 返り値:

```json
{
  "message": { "...": "..." }
}
```

#### `braydr_thread_create_post(channel, thread_id, text, after?)`

スレッドに返信する。`author` は `agent`。

- スレッドが解決済みなら `thread_resolved`。先に `braydr_thread_unresolve` を呼ぶ。
- `after` はスレッドの最新返信と一致する必要がある。スレッドには必ず最初の返信が
  あるので、実質必須。
- 返り値は `braydr_channel_create_post` と同じ。

#### `braydr_thread_create(channel, parent_message_id, text)`

スレッドの作成と最初の返信の投稿を同時に行う。`author` は `agent`。

- 親はメインのタイムラインのメッセージであること。スレッド内のメッセージなら
  `parent_not_main`。
- 親にすでにスレッドがあれば、内容にかかわらず `thread_exists`。
  `details.thread_id` を返す。返信したいだけなら `braydr_thread_create_post` を使う。
- 親にスレッドを 1 つしか作れない一意制約で重複を防ぐ。応答が失われた場合は
  `thread_exists` の `thread_id` を読み、最初の投稿が自分のものか確認する。
- 返り値:

```json
{
  "thread": { "...": "..." },
  "message": { "...": "..." }
}
```

- `thread` は親メッセージを含むスレッドオブジェクト。
- `message` は作成した最初の返信。

#### `braydr_thread_resolve(channel, thread_id, after)`

スレッドを解決する。メッセージは作らない。

- すでに解決済みなら `already_resolved`。`details` に `resolved_at` / `resolved_by`。
- 未解決なら `after` が最新返信と一致する必要がある。
- 最後の投稿がユーザーなら `unanswered`。未返信のまま閉じない。
- `resolved_at` を現在時刻、`resolved_by` を `agent` にする。
- `updated_at` を更新し、解決状態の履歴に記録する。
- 返り値はスレッドオブジェクト。

#### `braydr_thread_unresolve(channel, thread_id)`

スレッドを未解決に戻す。メッセージは作らない。

- すでに未解決なら `already_unresolved`。
- `resolved_at` と `resolved_by` を null にし、`updated_at` を更新して、解決状態の履歴に記録する。
- 返り値はスレッドオブジェクト。
- エージェントが使うのは、解決済みだが追加作業・追加確認が必要だと判断したとき。
  ユーザーの追加依頼による再開はアプリが自動で行うので、呼ぶ必要はない。

### 6.4 返り値の形

#### メッセージ

```json
{
  "message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
  "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
  "thread_id": null,
  "child_thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
  "author": "user",
  "created_at": "2026-10-31T01:23:45.123Z",
  "text": "..."
}
```

- `thread_id` はスレッドへの投稿なら所属スレッド。メインなら null。
- `child_thread_id` はこのメッセージを起点とするスレッド。無ければ null。
  スレッド内のメッセージでは常に null。
- `author` は `user` か `agent`。

#### スレッド

```json
{
  "thread_id": "0190c2e3-3456-7cde-8f01-23456789abcd",
  "channel_id": "0190a3c4-1234-7abc-8def-0123456789ab",
  "parent_message_id": "0190d3f4-4567-7def-9012-3456789abcde",
  "parent_message": { "...": "..." },
  "created_at": "2026-10-31T01:21:00.000Z",
  "updated_at": "2026-10-31T01:30:00.000Z",
  "last_message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc",
  "last_message_at": "2026-10-31T01:30:00.000Z",
  "last_author": "user",
  "post_count": 3,
  "reply_count": 2,
  "resolved_at": null,
  "resolved_by": null,
  "awaiting": "agent"
}
```

- `post_count` は親を含むメッセージ数。`reply_count` は親を含まない返信数。
- `last_author` は親を含めた最後の投稿者。
- `awaiting` は未解決なら `agent` / `user`、解決済みなら null。

#### 一覧

- メッセージ一覧: `{ messages, has_more, next_cursor }`
- 検索結果: `{ messages, has_more, next_cursor }`
- スレッド一覧: `{ threads, has_more, next_cursor }`
- スレッドのタイムライン: `{ thread_id, parent_message, messages, resolved_at, resolved_by, awaiting, state_events, has_more, next_cursor }`

### 6.5 エラー

返り値は次の形にする。

```json
{
  "error": {
    "code": "stale_after",
    "message": "The timeline has newer messages.",
    "details": {
      "current_last_message_id": "0190b1d2-2345-7bcd-9ef0-123456789abc"
    }
  }
}
```

| code | いつ |
| --- | --- |
| `channel_not_found` | チャンネルが無い |
| `channel_mismatch` | セッションのチャンネルと引数が違う |
| `invalid_argument` | ID の形式が不正、など |
| `thread_not_found` | スレッドが無い、または別チャンネル |
| `message_not_found` | メッセージが無い、または別チャンネル |
| `parent_not_main` | スレッドの親にスレッド内のメッセージを指定した |
| `thread_exists` | 親にすでにスレッドがある。`details.thread_id` |
| `thread_resolved` | 解決済みスレッドにエージェントが投稿した。`details` に `resolved_at` / `resolved_by` / `last_message_id` |
| `stale_after` | `after` が最新と違う。`details.current_last_message_id` |
| `after_required` | タイムラインが空でないのに `after` が無い |
| `unanswered` | 最後がユーザーのスレッドをエージェントが解決しようとした。`details.last_message_id` |
| `already_resolved` | 解決済みスレッドを解決しようとした。`details` に `resolved_at` / `resolved_by` |
| `already_unresolved` | 未解決スレッドを未解決に戻そうとした |
| `invalid_cursor` | `cursor` が不正、または別スコープのもの |
| `invalid_query` | 検索語が空白のみ、または 200 文字超 |
| `empty_text` | 本文が空白のみ |
| `text_too_long` | 本文が 20,000 文字超 |
| `internal` | それ以外 |

### 6.6 再試行

- 再試行用のキーは持たない。成功応答を再現しない。
- 投稿が成功したか不明なときは、同じ `after` で再試行する。成功していれば最新
  メッセージが変わっているので `stale_after` になる。読み直して自分の投稿を確認する。
  成功していなければ、そのまま再試行できる。
- スレッド作成は、親にスレッドを 1 つしか作れない一意制約で重複を防ぐ。既にスレッドが
  あれば常に `thread_exists`。`details.thread_id` を読んで、最初の投稿が自分のものか
  確認する。
- 解決 / 未解決の再実行は状態エラーで返す。
  - 解決済みを解決: `already_resolved`
  - 未解決を未解決に戻す: `already_unresolved`
- 成功したか不明なときは、成功応答を待たずに読み直す。
  - 投稿: タイムラインを読み直し、自分の投稿を確認する。
  - スレッド作成: `thread_exists` の `details.thread_id` を読む。
  - 解決: `already_resolved` なら解決済みとして扱う。

### 6.7 典型的な流れ

#### メインの依頼にメインで返す

1. `[braydr] new post`（scope: main）を受け取る。
2. `braydr_channel_list_posts` で最新ページを読む。最後の ID を `after` にする。
3. 作業する。
4. `braydr_channel_create_post(channel, text, after)`。
5. メインには解決状態が無いので、解決しない。

#### スレッドの依頼に返して解決する

1. `[braydr] new post`（scope: thread）を受け取る。
2. `braydr_thread_list_posts` で親と返信を読む。最後の ID を `after` にする。
3. 作業する。
4. `braydr_thread_create_post(channel, thread_id, text, after)`。
5. 返ってきたメッセージ ID を `after` にして
   `braydr_thread_resolve(channel, thread_id, after)`。

#### 解決済みスレッドに追加確認したい

1. `braydr_thread_get` で状態と親を確認する。
2. `braydr_thread_unresolve(channel, thread_id)`。
3. `braydr_thread_create_post` で質問を投稿する。
4. ユーザーの返事を待つので、解決しない。未返信（ユーザー待ち）になる。

#### `stale_after` になった

1. エラー中の `details.current_last_message_id` を確認する。
2. タイムラインを読み直す。
3. 新しい最後の ID を `after` にして再試行する。
4. `stale_after` は投稿が入っていないので、読み直して再試行する。成功していれば
   自分の投稿がタイムラインにあるので、それを確認して再投稿しない。

---

## 7. herdr-core API と配送

### 7.1 接続

アプリは herdr-core の JSON-RPC API を直接使う。

- 接続先は Unix socket（Windows は named pipe）。plain mode の JSON-RPC 2.0 で、
  1 行 1 メッセージ。
- 接続時に `hello` を送り、`generation` / `version` / `methods` を確認する。
  API は preview（protocol generation 1）。`generation` が 1 でなければ起動を中止する。
  必要なメソッドが `methods` に無ければ中止する。
- 主なメソッド:

| 用途 | メソッド |
| --- | --- |
| テンプレート一覧 | `templates.list` |
| セッション起動 | `terminal.run` |
| 通知の配送 | `terminal.send` |
| セッション停止 | `terminal.kill` |
| 状態確認 | `terminal.get` / `terminal.list` |
| 状態・終了イベント | `events.subscribe` |

- `terminal.run` が返す `address`（例: `t7`）を `session_id` として保存する。
  `terminal` 引数には address を渡す。名前でも引けるが、address の方が曖昧でない。
- Braydr の端末には `labels: {"braydr": "1", "channel": "<channel_id>"}` を付ける。
  `terminal.list` と `events.subscribe` は `labels: {"braydr": "1"}` で絞り込める。
- アプリ起動時・core 再接続時は `terminal.list` を `labels: {"braydr": "1"}` で引き、
  `labels.channel` でチャンネルと照合して `session_id` を更新する。見つからなければ
  `stopped`。同じチャンネルの live が複数あれば、`created_ms` が新しいものを採用し、
  古いものは `terminal.kill` する。
- `events.subscribe` は専用の接続で行う。plain mode で subscribe すると、その接続は
  以降通知専用になるため、通常の呼び出しとは分ける。
- `events.subscribe` のスナップショットと `finished` / `lost` / `restarted` / `status` /
  `turn` で状態を更新する。`missed` や再接続時は新しいスナップショットで置き換える。
  イベントを使わない場合は `terminal.get` / `terminal.list` を定期確認する。

`hello` の例:

```json
{"jsonrpc":"2.0","id":1,"method":"hello","params":{"generation":1}}
```

### 7.2 ユーザー投稿の配送

1. アプリは投稿を検証し、DB に保存する。
   - 解決済みスレッドへの投稿なら、同じトランザクションで未解決に戻し、履歴を記録する。
   - スレッドの `updated_at` を更新する。
   - ユーザーの既読位置を、その投稿まで進める。
2. コミット後に `terminal.send` でセッションへ通知する。
3. `wait: false` で送る。エージェントが busy でも待たない。
4. `force: true` で送る。プログラムが blocked でも入力する。
5. チャンネルごとに送信を直列化し、投稿順と通知順を合わせる。
6. 通知は at-least-once。同じ通知が二重に届くことがある。
   `message_id` で同一の投稿を識別する。

```json
{"jsonrpc":"2.0","id":2,"method":"terminal.send","params":{
  "terminal":"t7",
  "text":"[braydr] new post\n...",
  "force":true,
  "wait":false
}}
```

### 7.3 通知の形式

`terminal.send` の `text` に渡す文字列。ヘッダーは固定のキー行、本文はユーザーの
原文をそのまま置く。シェルを挟まないので、本文の引用符や改行をシェル用に
エスケープする必要はない。JSON 文字列として渡す。

- `channel` は `channel_id`（UUIDv7）。`channel_name` は表示用で、ツールには渡さない。

#### メインへの投稿

```text
[braydr] new post
channel: 0190a3c4-1234-7abc-8def-0123456789ab
channel_name: ログイン修正
scope: main
message_id: 0190b1d2-2345-7bcd-9ef0-123456789abc
---
ログイン画面のバリデーションを直して。
```

#### スレッドへの投稿

```text
[braydr] new post
channel: 0190a3c4-1234-7abc-8def-0123456789ab
channel_name: ログイン修正
scope: thread
thread_id: 0190c2e3-3456-7cde-8f01-23456789abcd
message_id: 0190b1d2-2345-7bcd-9ef0-123456789abc
parent_message_id: 0190d3f4-4567-7def-9012-3456789abcde
reopened: true
---
エラーメッセージが英語のままです。
```

- `reopened: true` は、アプリが解決済みスレッドを自動で未解決に戻したときだけ付ける。
  それ以外は行ごと省略する。
- `parent_message_id` は文脈をたどるためのもの。本文は含めない。

#### スレッドの作成

```text
[braydr] new thread
channel: 0190a3c4-1234-7abc-8def-0123456789ab
channel_name: ログイン修正
thread_id: 0190c2e3-3456-7cde-8f01-23456789abcd
message_id: 0190b1d2-2345-7bcd-9ef0-123456789abc
parent_message_id: 0190d3f4-4567-7def-9012-3456789abcde
---
この API のタイムアウトも見てください。
```

- ユーザーがメインのメッセージに最初の返信を付けたときに送る。
- `thread_id` は新しいスレッド。`message_id` はその最初の返信。

### 7.4 配送失敗

- `terminal.send` が一時的なエラー（接続断など）なら、バックオフして再送する。
- `not_found` / `finished` / `unavailable` ならセッションは停止している。
  送らずに `stopped` にし、投稿は DB に残す。新しいセッションの開始時に `braydr-catchup` が回収する。
- `blocked` は `force: true` で送る。それでも失敗する場合は、`terminal.get` の
  `status` が変わるのを待って再送する。
- 停止中に溜めた通知を、再起動後にまとめて送らない。catchup と二重になるため。
- 再送しても届かない場合は UI に警告を出す。メッセージは失わない。
- 通知の本文は切り詰めない。長い投稿はそのまま送る。

### 7.5 セッション状態

- `terminal.get` の `state` が `live` なら `running`。
- `state` が `finished`、または `not_found` なら `stopped`。
  `finished` の `reason` / `exit_code` を UI に出す。
- `terminal.run` の呼び出し中は `starting`。
- `terminal.run` の `first_message` が null、または `sent` が false なら、
  スキルのコマンドがエージェントに届いていない。起動失敗として扱い、
  `terminal.kill` で片付ける。
- アプリはセッションを自動再起動しない。異常終了は `stopped` と表示し、
  ユーザーが新しいセッションを開始して復帰する。

### 7.6 別の herdr クライアントからの attach

Braydr のセッションは通常の herdr terminal なので、同じ core に接続した別の herdr
クライアントから `terminal.list` / `terminal.attach` で開ける。`labels` に
`braydr=1` / `channel=<channel_id>` が付いているので見つけやすい。別マシンの core なら
`machine/t7` の形で address する。

- 読み取り専用の `observe: true` は許可する。ログ確認・デバッグ用。
  observe の viewer は type も resize もしない。
- 書き込みを伴う attach は Braydr の外で会話が進む。その入力は Braydr の
  メッセージにならず、未読・解決状態・`after` とも一致しない。サポート外とする。
  書き込みする viewer が入力すると `terminal.send` の送信とも競合しうる。
- アプリは別クライアントの attach を検知・禁止できない。core の socket に
  アクセスできるクライアントは attach できる。必要なら core の権限を絞る。
- Braydr の UI に「herdr で開く」導線は置かない。アプリ内に端末を埋め込まない
  方針は変えない。別クライアントで開けるのは端末であり、Braydr のチャンネル・
  スレッド・未読の UI ではない。

---

## 8. セッション起動

### 8.1 API 呼び出し

`terminal.run` で起動する。`argv[0]` はチャンネルのテンプレート名、残りは最初の
メッセージ（スキルのコマンド）。`cwd` はチャンネルの作業ディレクトリ。
`wait: false` でバックグラウンド起動する。

新規チャンネル:

```json
{"jsonrpc":"2.0","id":1,"method":"terminal.run","params":{
  "argv":["claude","/braydr-start 0190a3c4-1234-7abc-8def-0123456789ab"],
  "cwd":"/Users/me/code/braydr",
  "labels":{"braydr":"1","channel":"0190a3c4-1234-7abc-8def-0123456789ab"},
  "wait":false
}}
```

履歴がある場合:

```json
{"jsonrpc":"2.0","id":2,"method":"terminal.run","params":{
  "argv":["claude","/braydr-catchup 0190e5f6-5678-7ef0-8123-456789abcdef"],
  "cwd":"/Users/me/code/braydr",
  "labels":{"braydr":"1","channel":"0190e5f6-5678-7ef0-8123-456789abcdef"},
  "wait":false
}}
```

- `terminal.run` は起動し、最初のメッセージを送った（または送らなかった）時点で返る。
- 返り値の `address` を `session_id` に保存する。
- `first_message` が null、または `sent` が false なら起動失敗。`reason` を UI に出し、
  `terminal.kill` で片付ける。
- `terminal.run` で `name` は指定しない。core が割り当てる `address` を
  `session_id` にし、`labels.channel` でチャンネルと照合する。
- `invalid_template` / `spawn_failed` / `config` / `unavailable` も起動失敗として
  UI にエラーを出す。
- 新規チャンネルの作成では、`channel_id`（UUIDv7）を生成してチャンネルを作ってから（`session_id` は null）
  起動する。`first_message` が null でなく、`sent` が true なら `address` を `session_id` に保存する。
- 履歴が空なら `/braydr-start`、履歴があれば `/braydr-catchup`。
- 履歴の自動注入はしない。セッションは空の文脈で始まる。
- チャンネルはコマンド引数で渡す。`/braydr-start` / `/braydr-catchup` の引数は
  `channel_id`（UUIDv7）。
- `channel_id` は `labels.channel` に入れる。Braydr は terminal 名を使わない。
- `keep` は使わない。失われたプログラムをシェルに置き換えるだけで、エージェントは戻らないため。

`templates.list` は `name` / `description` / `command` / `error` を返す。`name` を UI の選択肢と
`argv[0]` に使う。

### 8.2 起動の使い分け

- チャンネル作成直後: `braydr-start`
- 履歴がある場合の新しいセッション: `braydr-catchup`
- 履歴が無い場合の新しいセッション・起動失敗の再試行: `braydr-start`
- 履歴がある場合の起動失敗の再試行: `braydr-catchup`

つまり、履歴が空なら start、履歴があれば catchup。これで、起動失敗中に投稿された
メッセージも取りこぼさない。

### 8.3 新しいセッションの開始

ユーザーが確定したら（4.1）、次の順に行う。

1. `terminal.kill` で既存セッションを停止する。`session_id` が null なら停止は不要。
   `not_found` は停止済みとして続行する。返り値の `state` が `live` のまま、または
   その他のエラーなら中止する。
2. `session_id` を null にし、選ばれた作業ディレクトリとテンプレートをチャンネルに保存する。
3. 履歴が空なら `braydr-start`、履歴があれば `braydr-catchup` で `terminal.run` する。
4. `first_message` が null でなく、`sent` が true なら新しい `address` を `session_id` に保存する。
   それ以外は起動失敗として扱い、`terminal.kill` で片付ける。

- 古い terminal は finished として残る。新しい terminal は別の `address` になる。
  不要なら `terminal.rm` で消す。
- 新しいセッションは、スキル経由で必要な履歴を読む。アプリは渡さない。

### 8.4 異常終了

- アプリは `terminal.get` とイベント購読でセッションの生死を監視する。
- `finished` / `not_found` / `lost` なら `stopped` と表示する。自動再起動はしない。
- ユーザーが新しいセッションを開始したら、8.3 と同じ手順で復帰する。

---

## 9. スキル

スキルはエージェント起動時の指示書であり、常駐プロセスではない。

### 9.1 `braydr`

通常運用の規範。次の内容を渡す。

```markdown
---
name: braydr
description: Operate one Braydr channel. Use for any Braydr post, thread, resolution, or catch-up work.
---

# Braydr

You are the only agent for one Braydr channel. Every `braydr_*` tool is scoped
to that channel. Pass the same `channel` id (the UUID) on every call.
Notifications also carry `channel_name`; it is a label, not a tool argument.

## The model

- The main timeline is the channel's top-level conversation.
- A thread starts from one main-timeline message. The parent stays in the main
  timeline; replies live in the thread.
- A thread is unresolved until someone resolves it.
- "Awaiting you" means the thread is unresolved and the last post is from the
  user. That is your work queue.
- "Awaiting the user" means the thread is unresolved and the last post is from
  you. Do not resolve it while you are waiting for an answer.
- Unread is a user-only concept. Do not reason about it.

## When a notification arrives

A `[braydr] new post` or `[braydr] new thread` notification contains the post
text and ids. It is not enough context to reply.

1. Read the scope:
   - main: `braydr_channel_list_posts` (the newest page is enough unless the
     post refers to older context)
   - thread: `braydr_thread_list_posts`
2. Decide where the reply belongs:
   - If the request is in a thread, reply in that thread.
   - If the request is in main, reply in main, or create a thread on that main
     message when the work deserves its own thread.
   - If that main message already has a thread, reply in the existing thread
     instead of creating a new one.
   - Do not answer a new main request only inside an unrelated thread.
3. Work.
4. Post one consolidated result. Pass `after` when the tool requires it.
5. If the request is in a thread and is complete, resolve the thread.

A `[braydr] nudge` lists work that is still awaiting you. Read each listed
thread and the main timeline, then handle them. Do not reply to the nudge
itself.

## Posting rules

- `after` must be the id of the newest message you actually read in that
  timeline. If the timeline is non-empty, `after` is required. On
  `stale_after`, re-read and retry with the new id.
- There is no retry key. On `stale_after`, re-read the timeline; if your post is
  there, do not post again.
- If `braydr_thread_create` returns `thread_exists`, read the returned thread.
  If its first post is yours, treat the create as delivered; otherwise post in
  the thread.
- Do not post progress chatter. Post results, blockers, and questions.
- One consolidated reply is better than several fragments.
- Include what changed, how it was verified, and any remaining risk.
- If you need an answer, ask at most one focused question and leave the thread
  unresolved. The app will show it as awaiting the user.

## Resolution

- Resolve only when the request is finished and no answer from the user is
  needed. Resolving does not post a message. Pass the id of your final post as
  `after`.
- Do not resolve a thread whose last post is from the user. Reply first.
- The app automatically reopens a resolved thread when the user posts in it.
  You do not need to reopen it for an additional request.
- If a thread is already resolved but you decide more work is needed (for
  example, you need to ask a follow-up question), call
  `braydr_thread_unresolve` first, then post. `already_unresolved` means it is
  already open; just post.
- If `braydr_thread_create_post` fails with `thread_resolved`, re-read the
  thread. If the work is still needed, unresolve, post, and resolve again. If
  the user's resolution was a cancellation, stop.
- If `braydr_thread_resolve` returns `already_resolved`, the thread is already
  closed; treat it as done.

## Tools

- `braydr_channel_get` — channel name, counts, and last main post.
- `braydr_channel_list_posts` — main timeline, oldest to newest per page.
- `braydr_thread_get` — thread metadata and parent message.
- `braydr_thread_list_posts` — parent message, thread state, and replies.
- `braydr_message_get` — one message.
- `braydr_message_search` — full-text search across the main timeline and all
  threads, newest first. Use it to find past decisions and context.
- `braydr_thread_list` — filter: `unresolved`, `awaiting_agent`,
  `awaiting_user`, `resolved`, `all`. Paginated.
- `braydr_channel_create_post` — main timeline post.
- `braydr_thread_create_post` — thread reply.
- `braydr_thread_create` — start a thread and post its first reply.
- `braydr_thread_resolve` / `braydr_thread_unresolve`.

Prefer `awaiting_agent` over scanning all unresolved threads when you need your
queue.
```

### 9.2 `braydr-start`

新規チャンネル用。投稿しない。

```markdown
---
name: braydr-start
description: Start a new Braydr channel session. Use only when the channel was just created and has no history.
---

# Braydr start

Read the `braydr` skill first.

This channel is new: the main timeline is empty and no threads exist.

1. Do not post anything.
2. Do not create a thread.
3. Wait for the first `[braydr] new post` or `[braydr] new thread` notification.
4. Then follow `braydr`.

You may call `braydr_channel_get` to confirm the channel is empty.
If it already has posts or threads, do not post. Invoke the `braydr-catchup`
skill instead.
```

### 9.3 `braydr-catchup`

既存チャンネル用。未解決、未返信、メインの直近 50 件の順に読み、未返信から再開する。
読んだ内容は投稿しない。

```markdown
---
name: braydr-catchup
description: Resume an existing Braydr channel after a restart. Use when the channel has history.
---

# Braydr catch-up

Read the `braydr` skill first.

1. Read the first page of `braydr_thread_list(filter="unresolved", limit=20)`.
   This is orientation; do not try to read every page.
2. Read the first page of `braydr_thread_list(filter="awaiting_agent",
   limit=20)`. These are the threads waiting on you.
3. Read the latest 50 main-timeline posts with
   `braydr_channel_list_posts(limit=50)`.
4. Work the queue:
   - For each thread on the awaiting page: read
     `braydr_thread_list_posts`, do the work, post one result, and resolve if
     finished. Every item must get a post; if you cannot finish, post the
     blocker or one focused question.
   - When the page is done, query `awaiting_agent` again from the first page.
     Do not advance a cursor while resolving threads: the order changes.
   - Repeat until the queue is empty.
5. If the newest main post is from the user and has no `child_thread_id`,
   handle it: reply in main or create a thread on it.
6. Do not post a summary of what you read. Post only results and questions.

If a listed thread was resolved by the user while you were reading, re-read it
before posting. If an awaiting thread has no reply from you yet, do not resolve
it without posting.
```

---

## 10. ループ

アプリ側の定期処理。エージェント側の設定は持たない。

### 10.1 対象

チャンネルごとに次を集める。

1. **未返信（エージェント待ち）のスレッド**
   - 未解決
   - 最後のメッセージがユーザー
2. **メインのタイムラインの未対応**
   - 最後のメイン投稿がユーザー
   - そのメッセージに子スレッドが無い

2 の「子スレッドが無い」が重要。エージェントがメインの依頼に対して新しいスレッドを
立てた場合、メインの最後の投稿はユーザーのまま残る。子スレッドがあるなら対応済みと
みなし、督促しない。メインで返信した場合は最後の投稿者がエージェントになるので、
条件から外れる。

### 10.2 間隔

- 条件が成立してから 5 分後に最初の督促を送る。
- 以降、条件が解消するまで 5 分間隔で繰り返す。
- 条件が解消したらタイマーをリセットする。
- 間隔はアプリ側の設定で変更できる。エージェントには設定を持たせない。

最初の 5 分は、投稿通知と督促が重ならないための待ち時間でもある。

### 10.3 送信

- 督促はチャンネルのセッションへ `terminal.send` で送る（`force: true`, `wait: false`）。
- エージェントが busy でも待たない。
- `terminal.get` が `live` でない場合は送らない。UI に停止を表示する。
- 督促は DB のメッセージではない。未読にも解決状態にも影響しない。
- 返信があるまで繰り返す。返信とは、対象スレッドの最後の投稿者がエージェントになること、
  またはメインの最後の投稿者がエージェントになること、子スレッドができること、
  スレッドが解決されることを指す。

### 10.4 形式

```text
[braydr] nudge
channel: 0190a3c4-1234-7abc-8def-0123456789ab
channel_name: ログイン修正
awaiting_agent_threads:
- thread_id: 0190c2e3-3456-7cde-8f01-23456789abcd
  last_message_id: 0190b1d2-2345-7bcd-9ef0-123456789abc
  last_message_at: 2026-10-31T01:00:00.000Z
main:
  last_message_id: 0190b1d2-2345-7bcd-9ef0-123456789abc
  last_message_at: 2026-10-31T01:10:00.000Z
more: false
```

- `awaiting_agent_threads` は最大 20 件。それ以上あるときは `more: true`。
  残りは `braydr_thread_list(filter="awaiting_agent")` で確認する。
- メインの未対応が無ければ `main` を省略する。スレッドの未対応が無ければ
  `awaiting_agent_threads` を省略する。
- 本文は含めない。ID と時刻だけを送り、エージェントに読ませる。
- `channel` は `channel_id`（UUIDv7）。`channel_name` は表示用で、ツールには渡さない。
- 督促への返信は不要。列挙された対象を処理する。

### 10.5 補足

- ループはアプリが起動している間だけ動く。アプリ終了中は督促しない。
- アプリ起動時にもスキャンする。停止中に溜まった未返信は、新しいセッションの catchup と
  ループの両方から拾われうる。二重投稿は `after` で防ぐ。
- ユーザーがスレッドを解決したら、そのスレッドへの督促は止まる。
- ユーザーが未解決に戻しただけでは、それ自体を通知しない。ただし最後がユーザーなら
  未返信（エージェント待ち）なので、ループの対象にはなる。最後がエージェントなら
  未返信（ユーザー待ち）であり、エージェントの番ではない。

---

## 11. やらないこと

- ターミナル表示と直接介入。アプリ内にエージェントの端末を埋め込まない。
- 実行基盤の自作。セッションは herdr-core に任せる。
- スレッドごとのコンテキスト分岐。1 チャンネル 1 セッションで、全スレッドが同じ文脈を共有する。
- 履歴の自動注入。起動時に過去のメッセージをプロンプトへ入れない。
- 要約の生成。一覧の本文先頭は生テキストの切り詰めであり、要約ではない。
- メッセージの編集・削除。追記のみ。
- 添付ファイル。テキストのみ。
- 複数ユーザー・認証・共有。
- チャンネル・スレッド・メッセージの削除。
- セッションの自動再起動と再開。異常終了時はユーザーが新しいセッションを開始する。
- エージェントによるチャンネル作成。チャンネルはユーザーが作る。
- メインのタイムラインの解決状態。解決はスレッドだけ。
- スレッドの入れ子。
- チャンネルを横断する全文検索とスレッド一覧。
- ユーザー向けの未返信の表示。未読の件数の表示。エージェントが作業中かどうかの表示。

---

## 12. 未決事項

- Claude 以外のテンプレートでの起動。最初のメッセージの `/braydr-start` はスキル呼び出しに依存する。
  他のエージェントでスキルと `braydr_*` ツールが同じように使えるかは未確認。
- `templates.list` の絞り込み。エージェント以外のテンプレートも返る可能性がある。
- 作業ディレクトリを変えたときの履歴の扱い。前のディレクトリのパスを含む履歴を新しいセッションが読む。
- 検索の日本語対応の方式。
