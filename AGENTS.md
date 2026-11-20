# AGENTS.md

Notes for coding agents (Claude Code, Codex and others) working on Lorelamp.

The full architecture and the project rules live in [CLAUDE.md](CLAUDE.md). Read it first; this file does not repeat it. The release steps are in [docs/releasing.md](docs/releasing.md).

## Repository

- GitHub: `Lampe332/lorelamp`, public, AGPL-3.0, default branch `main`.
- Never push, create a release or change the repository settings without the owner's explicit go right before it. Commit only when asked.
- Run the private grep from `docs/releasing.md` (step 5) before every push, as a command of its own.
- Release files (`release-*/`, `target/release-assets/`) are never committed. They go into the GitHub release as assets.

## Required release-note style

Whenever preparing or publishing an app update to this repository, write its GitHub release description in the style below. Prepare the description before publishing (in `docs/release-notes/vX.Y.Z.md`, passed with `--notes-file`); do not rely on an automatically generated commit list. A normal code push does not require creating a release.

- Write release notes in English, even when the conversation is in German.
- Start the release body with exactly `## What's new?`, followed by a blank line and Markdown bullets. Use this heading once per release.
- Describe actual changes in Lorelamp (the Windows app and the Android remote) since the previous app release. Check the changes and validation results before making claims.
- Use short, concrete, user-facing descriptions. Prefer `Area: Change` after the emoji, such as `Chat: Fixed ...`.
- Order entries: breaking notices, new features, improvements, fixes, maintenance/dependencies. Omit categories with no changes; do not add category headings.
- Emoji meanings are mandatory and consistent across releases. Classify the actual change first; never choose emojis for decoration, variety, or based on the feature's size. Do not substitute other emojis.
- Use these markers according to the change:
  - `📝 BREAKING:` for compatibility changes requiring user action; explain the action. A plain `📝` is a notice the user should read (beta status, SmartScreen warning, which file to download).
  - `🎉` for every new user-facing feature or capability, including small features. If users could not do it before and now can, use `🎉`.
  - `⚡️` only for improvements to existing working features, including performance, usability, and reliability improvements. Do not use it for new capabilities or bug fixes.
  - `🦺` for bug fixes and regressions: restoring behavior that was broken or incorrect. Use it even when the fix also makes the app faster or more reliable.
  - `🏗️` for restructuring, removals, and other maintenance changes worth mentioning; add `BREAKING:` when applicable.
  - `⬆️` for dependency updates. Use `⬆️ Updated dependencies` only if dependencies actually changed.
- For a breaking change, keep the appropriate change marker and add `BREAKING:` (for example `🏗️ BREAKING:` for an incompatible removal). Use `📝 BREAKING:` for a separate migration/action notice. Explain what users must do.
- Split changes with different meanings into separate bullets rather than assigning an arbitrary emoji to a mixed entry. Before publishing, check that every bullet's emoji matches its actual change; do not force unused categories into the release.
- Use indented sub-bullets only when a larger change needs useful details.
- Add `[#123](https://github.com/Lampe332/lorelamp/issues/123)` or the corresponding `/pull/123` link only when it is real and relevant. Omit links when none exist; never invent issue numbers.
- Release notes of llama.cpp or other upstream projects are not this app's changelog. Mention an upstream change only when it was actually integrated and users notice it in this release.
- Avoid raw commit lists, internal testing logs, unsupported claims, and generic filler. Preserve uncertainty when a fix is only a potential fix.
- When updating `CHANGELOG.md`, use the same English bullet style for new entries under the version heading. Keep historical entries intact.
- Release assets: `Lorelamp-X.Y.Z-setup.exe`, `Lorelamp-X.Y.Z-portable.exe`, `Lorelamp-X.Y.Z.apk` and `SHA256SUMS.txt`, nothing else.

### Format example (illustrative only; do not publish unchanged)

```markdown
## What's new?

- 🎉 Chat: Added a search inside a chat
  - Jump to every hit with Enter
- ⚡️ Library: Faster loading of large card collections
- 🦺 Phone: Fixed the connection not coming back after the PC woke up
- ⬆️ Updated dependencies
```

## App notes (keep adding)

Things learned while working on Lorelamp that are easy to get wrong. When you find something like this, add a short bullet here in the same change (fact, then why it matters). Remove a bullet once it is no longer true.

- The app name is Lorelamp, but crate names (`rp-core`, `rp-server`, `rp-desktop`), the data folder `%APPDATA%\rp-app`, the Tauri identifier `org.rpapp.spike` and the `rp-` localStorage prefixes stay as they are. Renaming them loses user data.
- The Android app id `io.github.lampe332.lorelamp` and its signing key never change: Android installs an update only over the same id and key. The key and its password never go into the repository (`LORELAMP_SIGNING_*`).
- The web UI is compiled into the EXE. Build `apps/web` (`npm run build`) before `cargo build --release -p rp-desktop`, or the EXE ships the old UI.
- The owner uses the app while it is being built. A running `rp-desktop.exe` locks the file: ask before closing it, and close it normally, never kill it.
- Before a build that adds a migration, back up the owner's `rp-app.db`. Shipped entries in `MIGRATIONS` are never edited, only appended.
- Never start a second model server next to a running one, and never start the Vulkan backend without asking (it crashed the dev PC once). VRAM numbers come from the WDDM counters in `crates/hw`, not from llama.cpp.
- Agent worktrees under `.claude/worktrees/` each build their own `target/` (10 to 20 GB). Remove a worktree as soon as its branch is merged.
- Test servers for UI work run from a temp folder with their own `--data-dir`, so the owner's chats and settings stay untouched (see CLAUDE.md rule 9).
- Release notes exist for 0.1.0 and 0.2.0 (`docs/release-notes/`). The GitHub repository was created on 2026-11-20 with no releases yet; the built 0.2.0 files were in the local folder `release-0.2.0/` (ignored).
- `README.md` links a `CHANGELOG.md` in the root, and its download badge still says 0.1.0. Check both before the next release.
