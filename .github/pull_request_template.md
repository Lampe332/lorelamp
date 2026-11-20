## What and why

<!-- What does this change for someone using Lorelamp? Link the issue if there is one. -->

## Checks

- [ ] `cargo test --workspace` and `cargo clippy --workspace --all-targets` pass without warnings
- [ ] `npx tsc --noEmit` in `apps/web` passes
- [ ] UI change: the touched Playwright specs pass, checked at phone and desktop size
- [ ] New interface text is in all four dictionaries (`apps/web/src/i18n/dict/`)
- [ ] Nothing private in the diff: no user or model paths, chats or own character cards; screenshots and fixtures are safe for work

See [CONTRIBUTING.md](https://github.com/Lampe332/lorelamp/blob/main/CONTRIBUTING.md) for the rules.
