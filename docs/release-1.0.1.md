# resonance-chat v1.0.1

## Summary

This release focuses on safety and release hygiene.

## Changes

- Tightened WinDivert capture behavior to inbound `sniff` / `recv_only`.
- Added limits around packet capture, decompression, and TCP reassembly buffers to reduce DoS risk.
- Added WinDivert verification tooling and a release security checklist.
- Added guidance for unsigned hobby releases.

## Notes

- This release is not code signed.
- Windows may show `Unknown publisher` or SmartScreen warnings.
- The app requires administrator rights because WinDivert needs them to capture packets.

## Verification

- Publish SHA-256 hashes for the release artifacts.
- Publish or attach the WinDivert verification report for the release.
- Record the exact git commit used for the build.

## Artifacts

- `src-tauri/target/release/resonance-chat.exe`
- `src-tauri/target/release/bundle/portable/resonance-chat_1.0.1_x64-portable.zip`
- `src-tauri/target/release/bundle/nsis/resonance-chat_1.0.1_x64-setup.exe`
- `src-tauri/target/release/bundle/msi/resonance-chat_1.0.1_x64_en-US.msi`
- `docs/release-1.0.1-sha256.txt`
