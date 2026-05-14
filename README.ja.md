[English](README.md) | 日本語

# tympan-aspl

macOS AudioServerPlugin を Rust で記述するためのフレームワーク。

`tympan-aspl` は Core Audio HAL の AudioServerPlugin インタフェースに対する
Rust の抽象化を提供し、C++ や Objective-C を書くことなく、Rust アプリケーション
から仮想オーディオデバイスやオーディオドライバを macOS 上で実装できるようにします。

## ステータス

**実装初期段階。** クロスプラットフォームな基盤部分が着地しました。リアルタイム
プリミティブ（`RealtimeContext`、ロックフリー SPSC リングバッファ、アトミックな
ライフサイクル状態機械、別スレッドのログシンク）、Core Audio オブジェクトモデル
（`OsStatus`、`FourCharCode`、`AudioObjectId`、`PropertyAddress`、`StreamFormat`）、
`Driver` / `Device` / `Stream` の API サーフェス、`.driver` バンドルの `Info.plist`
生成 — いずれもユニットテストと CI で検証されています。

macOS の FFI ブリッジ（`raw`）と `coreaudiod` への HAL ロードテスト階層が次の段階
です。詳細は
[`docs/decisions/0001-ci-verification-strategy.md`](docs/decisions/0001-ci-verification-strategy.md)
を参照してください。スコープは [`docs/ja/overview.md`](docs/ja/overview.md) を、
API 設計は [`docs/ja/architecture.md`](docs/ja/architecture.md) を参照してください。

## 名前について

*Tympan* は蛾の鼓膜器官 (tympanal organ) を指します。これはメイガ科やヤガ科
などの蛾の腹部にある膜ベースの超音波センサで、コウモリの反響定位を検知する
ために進化しました。この名称はライブラリの役割を反映しています ― OS の
オーディオエンジンとユーザー空間の Rust コードとの間に介在する、薄い膜です。

## ライセンス

以下のいずれかの下でライセンスされています。

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) もしくは
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) もしくは
  <http://opensource.org/licenses/MIT>)

利用者の選択により、いずれを選んでも構いません。

### コントリビューション

明示的に別段の表明がない限り、Apache-2.0 ライセンスで定義される、本著作物への
組込みのために意図的に提出されたあらゆるコントリビューションは、追加の条項や
条件を伴うことなく、上記のとおりデュアルライセンスとなります。

## ドキュメント

| ドキュメント | 内容 |
|---|---|
| [`docs/ja/overview.md`](docs/ja/overview.md) | プロジェクトの目的、スコープ、既存実装との比較 |
| [`docs/ja/architecture.md`](docs/ja/architecture.md) | 計画中の API 設計とモジュール構成 |
| [`docs/ja/testing.md`](docs/ja/testing.md) | 階層化されたテストおよび CI 戦略 |
| [`docs/ja/references.md`](docs/ja/references.md) | Apple のドキュメント、先行実装、関連クレート |
| [`docs/ja/handoff-protocol.md`](docs/ja/handoff-protocol.md) | 長期作業のためのセッション引き継ぎプロトコル |
