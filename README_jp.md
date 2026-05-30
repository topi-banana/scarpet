# scarpet

[![CI](https://github.com/topi-banana/scarpet/actions/workflows/ci.yml/badge.svg)](https://github.com/topi-banana/scarpet/actions/workflows/ci.yml)

*[English](README.md) | 日本語*

Minecraft の [Carpet](https://github.com/gnembon/fabric-carpet) Mod に組み込まれているスクリプト言語 [Scarpet](https://github.com/gnembon/fabric-carpet/blob/master/docs/scarpet/Documentation.md) のための Rust 製ツール群です。Scarpet スクリプト（`.sc` ファイル）はゲーム内アプリやサーバー拡張を記述するために使われます。本リポジトリはそれらを対象とした字句解析器、トリビア（コメント・改行）を保持する構文解析器、そしてコードフォーマッタを提供します。

> **ステータス:** 初期段階。構文解析器は式の文法全体をカバーし、実在する 220 ファイルのコーパスの 98.6% を解析できます。フォーマッタはそのコーパスを非破壊的に往復処理できます。API は未安定です。

## ワークスペース構成

3 つのクレートとテストコーパスからなる Cargo ワークスペースです。

| クレート | 内容 |
| --- | --- |
| [`scarpet-syntax`](scarpet-syntax) | 字句解析器（[`logos`](https://crates.io/crates/logos)）と構文解析器（[`logosky`](https://crates.io/crates/logosky) 経由の [`chumsky`](https://crates.io/crates/chumsky)）。コメントと改行を保持した CST を生成します。`wasm32` 向けにもビルドできます。 |
| [`scarpet-fmt`](scarpet-fmt) | コードフォーマッタ。CST を Wadler/Lindig 流のプリティプリント用 IR に変換し、設定可能なスタイルで描画します。 |
| [`scarpet-cli`](scarpet-cli) | `clap` ベースのコマンドラインフロントエンド（`scarpet`）。現在は `format` を提供します。 |
| [`example/`](example) | コミュニティ製 Scarpet スクリプトの git サブモジュール群。解析・整形のコーパスとして使用します。 |

データは一方向に流れます。

```
ソース (.sc) → 字句解析 → 構文解析 → CST（トリビア付き）→ fmt lower → Doc IR → 整形済みテキスト
                                       └─ scarpet-syntax ─┘   └──────── scarpet-fmt ────────┘
```

## はじめに

比較的新しい安定版 Rust ツールチェインが必要です（edition 2024。開発は Rust 1.96 で行っています）。

```sh
# コーパスのサブモジュールごとクローン（任意 — サブモジュールはコーパステストでのみ必要）
git clone --recurse-submodules git@github.com:topi-banana/scarpet.git
cd scarpet

# サブモジュールなしでクローン済みの場合:
git submodule update --init --recursive

cargo build --workspace
cargo test  --workspace
```

## 使い方

フォーマッタはファイルまたは標準入力から読み込みます。バイナリ名は `scarpet-cli` です（`--help` では `scarpet` と名乗ります）。

```sh
# ファイルを整形して結果を標準出力へ表示
cargo run -p scarpet-cli -- format script.sc

# 標準入力から整形
echo "print('hi')" | cargo run -p scarpet-cli -- format

# ファイルをその場で書き換え
cargo run -p scarpet-cli -- format --in-place src/*.sc

# 書き込まずに整形済みかどうかを確認（差分があれば非ゼロ終了）
cargo run -p scarpet-cli -- format --check src/*.sc

# 設定ファイルを明示指定して整形（指定しなければカレントの scarpet-fmt.toml を使用）
cargo run -p scarpet-cli -- format --config scarpet-fmt.toml script.sc
```

スタンドアロンのバイナリとしてインストールする場合:

```sh
cargo install --path scarpet-cli   # `scarpet-cli` がインストールされます
```

終了コード: `0` 成功、`1` 解析エラーまたは `--check` 失敗、`2` 入出力または設定エラー。

### 整形スタイル

スタイルは TOML ファイルで設定できます。`scarpet format` はカレントディレクトリの `scarpet-fmt.toml`、または明示指定した `--config <path>`（こちらが優先）を読み込みます。どちらも無ければ組み込みのデフォルトを使います。各キーは省略可能です。

```toml
# scarpet-fmt.toml
indent = 4               # インデント幅（スペース数）
max_width = 100          # グループが折り返す前の行幅の目標
line_ending = "lf"       # 改行コード: "lf"（Unix、デフォルト）または "crlf"（Windows）
```

未知のキー、`max_width = 0`、および `"lf"`・`"crlf"` 以外の `line_ending` は拒否されます。これらの設定項目を除けばレイアウトは固定です。主な点:

- 二項演算子は前後にスペースを入れます（`a + b`、`a -> b`）。ただし `:`（get）は詰めます: `a:b`。単項前置演算子はオペランドに密着します: `-x`、`!x`、`...xs`。
- `;` による文の並びは 1 行 1 文で配置し、各文を `;` で終端します。括弧で囲まれた `;` チェーンはインデントされたブロックになります。
- リスト・マップ・呼び出し引数は収まる場合は 1 行に保ち、収まらない場合は 1 要素 1 行に折り返して末尾にカンマを付けます。
- コメントは保持されます。独立行のコメントは独立行のまま、行末コメントは付いていた行のまま維持します。連続する空行は 1 つの空行にまとめます。
- 出力は必ず改行 1 つで終わり、行末の空白は残しません。

```sc
foo()->(a;b)
```

は次のように整形されます。

```sc
foo() -> (
    a;
    b;
)
```

フォーマッタは**非破壊的**（出力を再解析すると構造的に同一の木が得られる）かつ**冪等**（2 回整形しても 1 回と同じ）です。いずれの性質も CI でコーパス全体に対して検証されています。

## コーパス

[`example/`](example) はコミュニティ製の Scarpet リポジトリ 9 つを git サブモジュールとして取り込んでおり、合計 220 個の `.sc` ファイルがあります。これらは 2 つの用途で使われます。

- **解析率（parse rate）。** スタンドアロンのランナーが全ファイルを解析し、成功数を報告します。これはゲートではなく進捗の指標です（常に 0 で終了します。既知の上流側の構文エラー 3 件はランナー内に列挙されています）。

  ```sh
  cargo run -p scarpet-syntax --bin corpus            # 人間向けの要約
  cargo run -p scarpet-syntax --bin corpus -- --markdown   # CI/PR 用 Markdown レポート
  ```

- **フォーマッタの安全性。** テスト（`scarpet-fmt` の `corpus` モジュール）が解析可能な全ファイルを整形し、結果が構造的に等しい木に再解析されること、かつ冪等であることを検証します。サブモジュールが未チェックアウトの場合は静かにスキップします。

## 開発

CI（[`.github/workflows/ci.yml`](.github/workflows/ci.yml)）は push と pull request のたびに以下のゲートを実行し、`x86_64-unknown-linux-gnu` と `wasm32-unknown-unknown` の両方をビルドします。ローカルで再現するには:

```sh
cargo fmt --all -- --check                       # rustfmt
taplo fmt --check --diff                          # TOML の整形
cargo clippy --workspace --all-targets -- -D warnings
cargo check --workspace --all-targets
cargo machete                                     # 未使用依存の検出
cargo test --workspace --all-targets
```

依存関係の更新は Dependabot が管理します。各 CI 実行の結果は pull request に sticky な要約コメントとして投稿されます。

## ライセンス

未定です。[`example/`](example) 配下のリポジトリはサードパーティのサブモジュールであり、それぞれ独自のライセンスに従います。
