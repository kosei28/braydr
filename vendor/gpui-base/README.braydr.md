# gpui-base 0.8.1 (Braydr patched copy)

crates.io の gpui-base 0.8.1 に、IME の修正を 2 つ加えたもの。`[patch.crates-io]` で使う。
公開元で直ったら、この複製を消してパッチを外す。

- `src/input/base/state.rs` の `on_blur`: 変換中にフォーカスを失ったら変換を確定し、OS 側の変換も破棄する。
- `src/input/base/element.rs`: 変換中の文節（IME が選択範囲で示す部分）を選択色で表示する。

`benches/` は外し、`Cargo.toml` の `[[bench]]` も除いた。
