# Unsigned Release Guidance

This project can be distributed without code signing, but the trust model changes.

## What To Expect

- Windows may show `Unknown publisher` or SmartScreen warnings.
- Some users will assume the download is risky even when it is clean.
- An unsigned installer creates more friction than a portable zip.

## Recommended Distribution Order

1. Publish source code first.
2. Publish SHA-256 hashes for every release artifact.
3. Publish a portable zip before publishing an installer.
4. Publish the installer only if you clearly explain that it is unsigned.

## Minimum Safety Bar For Unsigned Releases

- Build from a clean, tagged commit.
- Keep the exact commit hash in the release notes.
- Publish SHA-256 for:
  - `resonance-chat.exe`
  - installer
  - `WinDivert.dll`
  - `WinDivert64.sys`
- Attach or link the WinDivert verification report for that release.
- State that the app requires administrator rights because WinDivert needs them.
- State that the app is unsigned because it is a hobby project.

## Suggested Release Note Text

This release is not code signed. Windows may show an `Unknown publisher` warning.

The app requires administrator rights because it uses WinDivert to read network packets in `sniff` / `recv_only` mode.

SHA-256 hashes and the WinDivert verification report are included so you can verify the files before running them.

## Practical Advice

- If trust matters more than convenience, distribute source only.
- If convenience matters, prefer a portable zip over an unsigned installer.
- Do not hide the unsigned status. Being explicit improves trust.
- Keep release artifacts small and predictable. Avoid bundling extra tools that are not required.
