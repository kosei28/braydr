# gpui-component 0.8.1 (Braydr patched copy)

crates.io の gpui-component 0.8.1 に、diff の行に色を付ける変更を加えたもの。`[patch.crates-io]` で使う。
公開元で直ったら、この複製を消してパッチを外す。

- `src/highlighter/syntect_backend.rs` の `semantic_name`: syntect が付ける `markup.inserted` / `markup.deleted`（行頭の `+` / `-` を含む）を、`diff.plus` / `diff.minus` という色名にする。元は色名がなく、無色になっていた。
- `src/highlighter/wasm_stub.rs`: テーマの `SyntaxColors` に `diff.plus` / `diff.minus` を足す。tree-sitter を使わない構成では、こちらの定義が使われる。
- 同じく `ThemeStyle` に `background_color` を足し、`HighlightStyle` の背景色に渡す。背景は gpui-base 側のパッチで行全体に敷く。

`tests/` は公開パッケージに含まれない資料を読むので、この複製ではテストを実行できない。動作はアプリ側のテストとスクショで確かめる。
