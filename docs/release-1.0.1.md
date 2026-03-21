# resonance-chat v1.0.1

## 概要

このリリースは、安全性の改善と配布まわりの整備が中心です。

## 変更点

- WinDivert のキャプチャ条件を見直し、受信側の `sniff` / `recv_only` に寄せました。
- パケット取得、展開処理、TCP 再構成バッファに上限を追加し、DoS 耐性を上げました。
- WinDivert の照合スクリプトとリリース用セキュリティチェックリストを追加しました。
- 未署名の趣味開発向けリリース手順を整理しました。

## 注意事項

- このリリースはコード署名されていません。
- Windows で `Unknown publisher` や SmartScreen 警告が表示される場合があります。
- WinDivert を使ってパケットを読むため、管理者権限が必要です。

## 検証情報

- 配布物の SHA-256 を公開してください。
- WinDivert の照合レポートを添付またはリンクしてください。
- ビルドに使った git commit を記録してください。

## 配布物

- `src-tauri/target/release/resonance-chat.exe`
- `src-tauri/target/release/bundle/portable/resonance-chat_1.0.1_x64-portable.zip`
- `src-tauri/target/release/bundle/nsis/resonance-chat_1.0.1_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/resonance-chat_1.0.1_x64_en-US.msi`
- `docs/release-1.0.1-sha256.txt`
