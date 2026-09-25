# AWHDL 言語仕様

日本語（正本） | [English reference translation](LANGUAGE_SPEC.md)

> **正本の範囲。** 規範文書: [LANGUAGE_SPEC](LANGUAGE_SPEC_ja.md)、
> [RUNTIME_SPEC](RUNTIME_SPEC_ja.md)、[SECURITY_SPEC](SECURITY_SPEC_ja.md)、
> [IR_SPEC](IR_SPEC_ja.md)、[PROFILE_v0.1](PROFILE_v0.1_ja.md)。非規範:
> [IMPLEMENTATION_GUIDE](IMPLEMENTATION_GUIDE_ja.md)、例、チュートリアル、移行メモ、状態報告。
> 矛盾時は PROFILE_v0.1、次に各規範文書が優先する。v0.1 の実装範囲は PROFILE_v0.1 だけが定める。
> 日本語版を正本とし、英語版は参考訳とする。差異がある場合は日本語版が優先する。

範囲: AWHDL の構文、型システム、静的意味論、言語構成要素。本書は実装範囲を定めない。本書に記述があっても、
PROFILE_v0.1 にない構成要素、または `v0.2` とされた構成要素は v0.1 では未対応である。

## 規範本文

言語は AWHDL 言語仕様 v0.2 で規定する:
[`AWHDL_LANGUAGE_SPEC_v0.2_ja.md`](AWHDL_LANGUAGE_SPEC_v0.2_ja.md)（日本語、正本）と英語の参考訳
[`AWHDL_LANGUAGE_SPEC_v0.2.md`](AWHDL_LANGUAGE_SPEC_v0.2.md)。
上位プロジェクトにある `AWHDL_LANGUAGE_SPEC_v0.1.md` は歴史的な設計資料であり、v0.2 により置き換えられた。

## 言語本文に対する優先関係

- 実行意味論（identity、generation、Value / Event / Invocation、配送、retry、Effect、checkpoint、完了評価）は
  [RUNTIME_SPEC](RUNTIME_SPEC_ja.md) が定める。言語本文が runtime の動作を異なって記述する場合は
  RUNTIME_SPEC が優先する。
- 機密区分、taint、capability、承認、監査は [SECURITY_SPEC](SECURITY_SPEC_ja.md) が定める。
- 直列化形式は [IR_SPEC](IR_SPEC_ja.md) が定める。

## 意味論は確定し構文が未定の構成要素

仕様整理手順書は、以下の構成要素について構文より先に意味論を確定する。ソース構文はまだ決まっておらず、
本節が表記を定めるまで、parser は暫定的な表記を受け付けてはならない。

- Event 宣言と `.changed` / `.completed` の感度（意味論: RUNTIME_SPEC の Value、Event、Invocation）。
- `completion when hard { ... } soft { ... };` または `require hard` / `prefer`（意味論: RUNTIME_SPEC の
  完了）。条件は現在は設定で与える。
- device に対する Effect class・承認・capability の注釈（意味論: RUNTIME_SPEC の Effect と SECURITY_SPEC）。
  現在は設定で与える。

## v0.1 の構文（実装済み）

言語本文 v0.2 のうち、以下の表記を v0.1 として確定し、parser（`awhdl-parser/src/awhdl.pest`）が受け付ける。
それ以外は構文エラーとする。

```vhdl
device name : agent | mcp | deterministic generic (key => value, ...);
    -- keys used by v0.1: route => "<configured action>", location => local | cloud,
    -- clearance => public | internal | restricted
device name : declassifier generic (from => <class>, to => <class>,
    [filter => <deterministic device>, filter_method => <method>,] [approval => human]);
signal a, b : type<class> := <constant>;
timer name : period <n> ms|sec|min|hour|day;
budget name is iterations <= n; wall_time <= <time>; tool_calls <= n; model_calls <= n; end budget;
barrier name (signal, ...);

assert always (<expr>);
assert never (<expr>);
assert never (<class> -> <location>);        -- static flow rule

process(name, name.changed, device.done, device.failed, device.timeout, timer, barrier.ready)
    timeout <time>;                          -- optional
begin
    device.method(<expr>, ...) [timeout <time>] -> signal;
    signal <= <expr>;
    signal <= declassify <expr> using <declassifier>;
    if <expr> then ... elsif <expr> then ... else ... end if;
    parallel device.method(...) -> signal; ... end parallel;
    assert <expr>;
    null;
on timeout                                   -- optional
    ...
end process;
```

式の結合は弱い順に: `or`、`and`、`not`、1つの比較（`=`、`/=`、`<`、`<=`、`>`、`>=`）、整数の `+` / `-`、
リテラル（文字列、整数、`true` / `false`、時間）と名前（`signal`、`signal.field`、`device.done`）。
キーワードは予約語であり、語境界が必要である。ただし感度リストの `device.timeout` の `timeout` は
メンバー名として書ける。

感度名は runtime が発生させるイベントに限る: `signal` / `signal.changed`、`timer`、`barrier` /
`barrier.ready`、`device.done` / `device.failed` / `device.timeout`。これ以外（`device` 単独、
`device.completed`、`signal.field`、`timer.ready` など）は process を決して起動しないため `E206` とする。

静的意味論（checker の診断）: `E2xx` は構造（未知または重複した名前、`in` ポートへの書き込み、
不正な budget の項目や正でない時間、信号でない barrier のメンバー、1つの信号に書き込む2つの parallel
分岐）。`E3xx` は情報フロー（明示のフロー `E301`〜`E303`、暗黙のフロー `E304`〜`E306`。SECURITY_SPEC を参照）。`E4xx` は言語が定義するが PROFILE_v0.1 が除外する
構成要素（`confidential`、`secret`、`sandbox`、`private_cloud`、`external`）または未知の名前。
### 機密解除（実装済み、2026-09-25）

区分を下げられるのは、`declassifier` の device を使う `declassify` 文だけである。declassifier は解除できる範囲
（`from` から `to` へ）と、解除を認める手段を宣言する。手段は次から選び、少なくとも1つを書く。両方を書くと両方が
必要になる。

- `filter => <device>`、`filter_method => <method>`: 決定的な検査。`deterministic` 種別で `local` の device を
  指定する。
- `approval => human`: 人の承認。runtime に設定された承認者（HumanPort の承認者モード）が、解除する内容を見て判断する。

```vhdl
device pii_check : deterministic generic (location => local, route => "pii_filter");
device release : declassifier generic (from => restricted, to => internal,
    filter => pii_check, filter_method => scan, approval => human);
...
summary <= declassify draft using release;
```

静的規則（checker）:

- `E219`: declassifier の宣言が不正。`from` と `to` がない、`to` が `from` より弱くない、手段がない、`filter` が
  `local` の `deterministic` device でない、`filter` があるのに `filter_method` がない、`approval` が `human` でない、
  declassifier の `location` が `local` でない。
- `E220`: `using` の device が declassifier でない。また、declassifier を通常の device のように呼び出した。
- `E307`: 解除する式の区分が declassifier の `from` より強い。
- 解除の結果の区分は `to` である。`to` より弱い信号に格納すると `E303`、文脈の区分が格納先より強いと `E304` とする。
- 解除の成否は解除する内容に依存するため、declassifier の `done`・`failed` のイベントの区分は、解除する式の区分と
  文脈の区分のうち最も強いものとする（SECURITY_SPEC の暗黙のフロー）。

`case`、`await`、`type` / FSM 宣言、`retry`、`approve`、`policy`、`export`、`configuration`、時相アサーションは
v0.1 に含まれず、構文として受け付けない。
