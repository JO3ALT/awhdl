# Profile v0.2（計画）

日本語（正本） | [English reference translation](PROFILE_v0.2.md)

> **計画文書であり、v0.1 に対する規範ではない。** v0.2 で予定するものを含め、すべての機能の状態を
> 記録するのは [PROFILE_v0.1](PROFILE_v0.1_ja.md) の表だけである。本書は Profile が `v0.2` の行を
> まとめ、それらを範囲に移す条件を示すだけである。

## 候補機能

PROFILE_v0.1 の表で `v0.2` とした行: declassifier、HTTP MCP transport、ホスト単位の network policy の
強制、cloud DLP / egress 検査、taint / 情報フロー検査、高度な cancellation、CLI resume と run lock、
transaction / 期限付きの承認 scope、provider への自動照会による照合。

## 移行の条件

- v0.1 が完了していること（すべての v0.1 行が適合）。
- 範囲に移す各機能について、同じ変更で設計判断の記録、所管する仕様への規範記述、表の行の更新を行う。
- 外部書き込みに関わる機能は Effect・idempotency・承認・capability の規則を維持し、いずれも弱めてはならない。
