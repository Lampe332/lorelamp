# Contributing

Bug reports, ideas, translations and pull requests are welcome. For anything bigger than a small fix,
please open an issue first so we can agree on the approach before you spend time on it.

## Build

You need Windows 10 or 11, a current stable Rust toolchain (MSVC) and Node.js 22 or newer.

```powershell
cd apps/web
npm ci
npm run build              # type check + web UI into apps/web/dist (the desktop app embeds it)
cd ../..
cargo build --release -p rp-desktop
```

The program is `target/release/rp-desktop.exe`. The internal names (`rp-core`, `rp-server`,
`rp-desktop`, the data folder `rp-app`) come from the old working title and stay as they are, so
existing installs keep their data.

The server alone, for example to try the phone or tablet view in a browser:
`cargo run -p rp-server -- --lan` prints a local and a phone address. It does not start a model by
itself unless you add `--manage-engine`.

The Android app (`apps/mobile`, Tauri v2) is built with `scripts/release-android.ps1`. It needs the
Android SDK and NDK, JDK 17 and a signing key of your own; the script's header lists the details.
Never commit a key or its password.

## Tests

Run these before you open a pull request:

```powershell
cargo test --workspace
cargo clippy --workspace --all-targets      # no warnings
cd apps/web; npx tsc --noEmit
```

UI tests use Playwright against a separate test server with its own data folder and a small model on
the CPU. The setup is in [apps/web/e2e/README.md](apps/web/e2e/README.md). For a UI change, run at
least the specs of the screens you touched, at phone and desktop size.

## Translations

The interface comes in German, English, Spanish and Russian. German is the source: every text lives
in `apps/web/src/i18n/dict/<area>.de.ts`, with one file per other language next to it, and a missing
key does not compile. [docs/i18n.md](docs/i18n.md) explains keys, plurals and formats. Fixes to a
translation are very welcome; a new language needs all areas at once. The Android app's start page
has its own word list in `apps/mobile/ui/i18n.js`.

## Rules

[CLAUDE.md](CLAUDE.md) holds the project rules in full (it is written for coding agents, but it applies
to everyone). The short version:

- **Real implementations only.** No mock data, stubs or TODO placeholders standing in for a feature.
- **Nothing private in the repo.** No user or model paths, no chats, no character cards of your own.
  Fixtures, screenshots and sample cards must be invented.
- **Database changes** are new entries appended to `MIGRATIONS` in `crates/server/src/store/mod.rs`.
  Never edit one that has shipped.
- **Security checks stay as they are.** Token, host and origin checks, the PC-only routes, the
  allowlist for llama.cpp start options, the network guard of the link import and the fixed options
  and sandbox of *Scene as a picture* are not loosened. [SECURITY.md](SECURITY.md) has the overview.
- **Look and feel.** Colours are CSS tokens in `apps/web/src/style.css`; new styles must work in all
  six themes, from 375 to 2560 px wide, and in the phone and tablet layouts. No UI library. Touch
  targets are at least 44 px (48 px on tablets).
- **Prompt layout.** Prompt building follows layout B in [docs/cache.md](docs/cache.md), so the model's
  cache keeps working across turns.
- **The GPU.** Never start a second model server next to a running one, never start a Vulkan build
  without asking, and measure speed through `llama-server`'s own timings
  ([docs/benchmarks.md](docs/benchmarks.md)).
- **Language.** Code, comments and commit messages in English. Interface texts go into the
  dictionaries, never straight into a component.

By contributing you agree that your work is licensed under the [AGPL-3.0](LICENSE), like the rest of
the project.
