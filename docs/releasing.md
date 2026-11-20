# Releasing Lorelamp

A release is published only after the owner's explicit go right before it. Everything up to step 4 can be prepared without that.

1. **Version.** Set the same version `X.Y.Z` in `Cargo.toml` (`[workspace.package]`, used by every crate), `apps/web/package.json`, `apps/desktop/package.json`, `apps/desktop/src-tauri/tauri.conf.json`, `apps/mobile/package.json` and `apps/mobile/src-tauri/tauri.conf.json`, plus the two root `version` fields in each `package-lock.json`. Keep `identifier` (`org.rpapp.spike`) and the crate and binary names (`rp-desktop`) as they are: a new identifier would lose the window's stored browser data. The Android app's id (`io.github.lampe332.lorelamp`) stays too: a new one would be a different app. `cargo check` updates `Cargo.lock`.
2. **Changelog and release text.** Add `## X.Y.Z — DD.MM.YYYY` at the top of `CHANGELOG.md` and write the same bullets into `docs/release-notes/vX.Y.Z.md`, which starts with `## What's new?`. English, user-facing, `Area: Change`, one emoji per bullet by meaning, in this order:
   - `📝` notices that need the user to do something (add `BREAKING:` for incompatible changes)
   - `🎉` new features · `⚡️` improvements to working features · `🦺` fixes · `🏗️` restructuring and removals · `⬆️ Updated dependencies` (only when they changed)
   - no commit lists, no claims that were not checked, no invented issue links
3. **Checks.** `cargo test --workspace`, `cargo clippy --workspace --all-targets` (no warnings), `npx tsc --noEmit` in `apps/web`, the Playwright suite.
4. **Build.** `powershell -ExecutionPolicy Bypass -File scripts\release.ps1 [-OutDir <folder>]` builds the web UI, the release EXE and the NSIS installer and writes `Lorelamp-X.Y.Z-setup.exe`, `Lorelamp-X.Y.Z-portable.exe` and `SHA256SUMS.txt` (default `target\release-assets\X.Y.Z\`). It stops when the versions differ or when `target\release\rp-desktop.exe` is running. Check the printed sizes, `ProductName`/`ProductVersion` and checksums.
   Then the Android app: `powershell -ExecutionPolicy Bypass -File scripts\release-android.ps1 [-OutDir <same folder>]` builds the signed arm64 APK `Lorelamp-X.Y.Z.apk` and adds its line to the same `SHA256SUMS.txt` (it replaces only its own line). It stops when the versions differ and verifies the signature with `apksigner` before it copies anything. Needs Node, Rust with the target `aarch64-linux-android`, JDK 17+, the Android SDK and an NDK (`ANDROID_HOME`, `NDK_HOME`). The signing key is never in the repository: the script reads `LORELAMP_SIGNING_STORE` and `LORELAMP_SIGNING_PASSWORD` (optionally `LORELAMP_SIGNING_ALIAS`), or `LORELAMP_SIGNING_DIR` for a folder that holds the key file and its password file. Keep the key safe: Android installs an update only over an app signed with the same key. Check the printed size, `minSdk` and checksum.
5. **Private grep.** Nothing personal may be in the commits that go out: `git grep -nI -iE "C:\\\\Users|D:\\\\llm|<your user name>"` over the tree and `git log -p origin/main..` must find nothing private. Run it on its own, never in the same command as the push.
6. **Push** `main` (after the go).
7. **GitHub release:**
   ```
   gh release create vX.Y.Z <out>\Lorelamp-X.Y.Z-setup.exe <out>\Lorelamp-X.Y.Z-portable.exe <out>\Lorelamp-X.Y.Z.apk <out>\SHA256SUMS.txt --target main --latest --title "Lorelamp X.Y.Z" --notes-file docs/release-notes/vX.Y.Z.md
   ```
