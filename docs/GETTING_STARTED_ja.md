# AWHDLを使い始める

[English](GETTING_STARTED.md) | 日本語

> この日本語版を規範文書（正本）とします。英語版は参考訳であり、差異がある場合は日本語版が優先されます。

## 必要な環境

Milestone 1で必要なのはRustとCargoだけです。特定のモデル、アクセラレータ、エディタ、ネットワークサービス、ファイルシステム配置、APIアカウントには依存しません。

`awhdl`ディレクトリで次を実行します。

```bash
cargo test --workspace
cargo run -p conductor-cli -- check examples/hello.awhdl
```

機械可読な出力を得るには次を実行します。

```bash
cargo run -p conductor-cli -- check examples/hello.awhdl --output json
```

## 最初の設計

```vhdl
entity hello is
    port (
        task   : in  text<internal>;
        result : out text<internal>
    );
end hello;

architecture behavioral of hello is
    device llm : local_llm;
begin
    process(task)
    begin
        llm.run(task) -> result;
    end process;
end behavioral;
```

`entity`は外部インターフェースを宣言します。`architecture`は1個のデバイスと1個のプロセスを宣言します。`task`が変化するとプロセスが起動し、デバイスを呼び出して結果を`result`ポートへ送ります。

Milestone 1の`aic check`が検査するのは、解析、宣言構造、名前の参照、出力先です。`llm`その他のデバイスは実際には呼び出しません。

## 終了状態

| 状態 | 意味 |
| --- | --- |
| 0 | 実装済みの検査に合格 |
| 1 | ソースファイルを読み取れない |
| 2 | 構文解析に失敗 |
| 3 | 構造検査に失敗 |

Milestone 1の検査に合格しても、v0.2言語全体やセキュリティポリシーが検証されたことにはなりません。[実装状況](IMPLEMENTATION_STATUS_ja.md)を参照してください。
