# gpui-base 0.8.1 (Braydr patched copy)

crates.io の gpui-base 0.8.1 に、IME の修正 2 つと、コードブロックの操作の表示条件の変更、diff の行の背景、クリックされたリンクの文字の受け渡しを加えたもの。`[patch.crates-io]` で使う。
公開元で直ったら、この複製を消してパッチを外す。

- `src/input/base/state.rs` の `on_blur`: 変換中にフォーカスを失ったら変換を確定し、OS 側の変換も破棄する。
- `src/input/base/element.rs`: 変換中の文節（IME が選択範囲で示す部分）を選択色で表示する。
- `src/text/node.rs`: `code_block_actions` の要素を、ポインタがコードブロックの上にある間だけ表示する。
- `src/global_state.rs` と `src/text/inline.rs`: リンクのクリックの処理の中から、そのリンクの文字を `GlobalState::clicked_link_text` で読めるようにする。リンクの右クリックのメニューで、文字をコピーするのに使う。
- `src/text/node.rs` と `src/text/inline.rs`: コードブロックの色付けに背景色があれば、文字の後ろだけでなく、その行の全幅（左右の余白まで）に敷く。diff の行の色分けに使う。

`benches/` は外し、`Cargo.toml` の `[[bench]]` も除いた。
