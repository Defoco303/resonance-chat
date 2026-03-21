# Security Release Checklist

Use this file to record the release signing result and the WinDivert verification result for each release.

If this is an unsigned hobby release, mark the code signing section as `not applicable` and fill in the unsigned release notes section instead.

## Release Metadata

- Release version:
- Release date:
- Built from commit:
- Built by:

## Files To Ship

- EXE: `src-tauri\target\release\resonance-chat.exe`
- Installer: `src-tauri\target\release\bundle\nsis\resonance-chat_<version>_x64-setup.exe`
- Local WinDivert DLL: `src-tauri\WinDivert.dll`
- Local WinDivert SYS: `src-tauri\WinDivert64.sys`

## Code Signing

- Certificate type: `OV / EV`
- Certificate subject:
- Certificate thumbprint:
- Timestamp URL:
- SignTool path:
- Verification command:

### Signature Verification Result

- EXE signature status:
- Installer signature status:
- EXE SHA-256:
- Installer SHA-256:

## Unsigned Release Notes

- Distribution mode: `source only / portable zip / installer`
- Unsigned release accepted: `yes / no`
- User-facing warning added to release notes: `yes / no`
- SHA-256 list published: `yes / no`
- Portable zip published: `yes / no`
- Installer published without signing: `yes / no`
- Reason for unsigned release:

### Unsigned Release Recommendation

- Prefer publishing source code and a portable zip before publishing an unsigned installer.
- If you publish an unsigned installer, clearly warn users that Windows may show `Unknown publisher` or SmartScreen warnings.
- Publish SHA-256 for every shipped file.
- Link the WinDivert verification report for the exact release.
- Build from a tagged commit and record the commit hash.

## WinDivert Verification

- Official download source:
- Official version:
- Official extraction directory:
- Verification script command:

### Local Current Hashes

- `WinDivert.dll`: `C1E060EE19444A259B2162F8AF0F3FE8C4428A1C6F694DCE20DE194AC8D7D9A2`
- `WinDivert64.sys`: `8DA085332782708D8767BCACE5327A6EC7283C17CFB85E40B03CD2323A90DDC2`

### Comparison Result

- Official `WinDivert.dll` SHA-256:
- Official `WinDivert64.sys` SHA-256:
- Local `WinDivert.dll` match:
- Local `WinDivert64.sys` match:
- Local `WinDivert64.sys` signature status:
- Official `WinDivert64.sys` signature status:

## Evidence

- `scripts\sign-release.ps1` output saved:
- `scripts\verify-windivert.ps1` output saved:
- Hash list saved:
- Release notes warning saved:
- Additional notes:

## Approval

- Ready to ship: `yes / no`
- Approved by:
- Approval date:
