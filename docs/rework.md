# Code rework – measurements

Baseline before the rework (package R0, 2026-11-14), taken on the uncommitted state after `e2a8b3d`.
Later packages add their "after" column; R5 sums up.

Machine: Windows 11, Intel Core i9-13900KF, Edge 157 (headless, via Playwright 1.65).
The test model runs on the CPU only (llama.cpp b12387 CPU build, Gemma 4 E2B Q4_K_M, `-ngl 0 -c 4096`).

## Bundle (`vite build`)

| File | Raw | gzip (Vite report) |
|---|---:|---:|
| `index-*.js` | 378.2 kB | 112.5 kB |
| `index-*.css` | 101.4 kB | 21.4 kB |
| `index.html` | 1.2 kB | 0.6 kB |

One JS chunk, no code splitting, no lazy-loaded screens.

## Desktop EXE

`target/release/rp-desktop.exe`: 16,429,568 bytes (15.7 MiB), built 2026-11-14 19:53. File size only; the
EXE was not started. It still reads the web UI from `apps/web/dist` at run time (not embedded).

## Server start (debug build of `rp-server`)

Time from spawning the process until `GET /health` answers 200 (port 7861, own data folder):

| Case | Time |
|---|---:|
| empty data folder (first start, creates both databases) | 540–561 ms (2 runs) |
| existing test data, 5 starts | 507–567 ms, median 510 ms |

Debug build; the release build is measured in R1.

## Start screen usable

Navigation start until the "continue" link of the newest chat on the start screen (layout "Bisher") is
visible and enabled; fresh browser context per run (empty cache), server on localhost, 7 runs each.

| Viewport | Median | Range | Last run DCL / FCP |
|---|---:|---:|---:|
| 1920×1080 | 127 ms | 125–160 ms | 36 / 76 ms |
| 375×812 | 121 ms | 119–130 ms | 38 / 68 ms |

## UI tests (Playwright)

`apps/web/e2e/` (see its README): 21 tests × 4 projects (375 and 1920 px × Graphit and Papier) = 84,
of which 76 run and 8 are `test.fixme` (2 known bugs, fixed in R3). Includes real model answers on the CPU.

| Run | Result | Duration |
|---|---|---:|
| existing data, run 1 | 76 passed, 8 skipped | 1.1 min (67 s wall) |
| existing data, run 2 | 76 passed, 8 skipped | 1.1 min (65 s wall) |
| empty data folder (setup incl. one warm-up answer) | 76 passed, 8 skipped | 1.1 min (65 s wall) |

Known bugs held as `test.fixme`:
- Ctrl+K "Logs" jumps to `data-spot="logs"`, which no element has (`Palette.tsx`).
- The sidebar's tokens/s are empty after a reload (`App.tsx` live value vs. `stats.last_timings`).

## R1 – embedded web UI + release profile (2026-11-14)

The web UI (`apps/web/dist`) is compiled into the programs with `rust-embed` (feature `embed-ui` of
`rp-server`; `rp-desktop` always uses it, `rp-server` serves it when no `--web-dir` is given). `--web-dir` /
`RP_WEB_DIR` still serve a folder for development. Embedded answers: content type as before, `index.html`
for unknown paths (200), `Cache-Control: no-cache` on `index.html` and other root files,
`public, max-age=31536000, immutable` on `assets/*`, ETag + 304. Without a built `dist` the build stops with
"Run `npm run build` in apps/web first".

Release profile: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`, `strip = true`, `panic` stays unwind.
All builds below into a separate, empty target folder (cold = all dependencies compiled; incremental = after
touching `crates/server/src/lib.rs`), `-p rp-desktop -p rp-server`, same machine.

| | before (default profile, UI on disk) | thin LTO (chosen) | fat LTO |
|---|---:|---:|---:|
| `rp-desktop.exe` | 16,429,568 B (15.7 MiB) | 16,049,664 B (15.3 MiB, UI included) | 15,384,576 B (14.7 MiB) |
| `rp-server.exe` (embed-ui) | 8,247,296 B (7.9 MiB, no UI) | 8,246,272 B (7.9 MiB, UI included) | 7,981,568 B (7.6 MiB) |
| cold release build | 83 s | 181 s | 202 s |
| incremental release build | 24 s | 132 s | 156 s |

Thin over fat: fat saves 0.6 MiB (4 %) on the desktop EXE but adds 24 s to every rebuild (the owner's EXE is
rebuilt after every change). Most of the build time comes from `codegen-units = 1`.

Vite: default target `baseline-widely-available` (Chrome/Edge 111, Safari/iOS 16.4, Firefox 114) covers
WebView2 and current phone browsers; no change.

### Server start (release `rp-server`, until `GET /api/health` 200)

Port 7862, own data folder, 1 start with an empty folder + 5 with existing data.

| Build | empty data folder | existing data (5 starts) |
|---|---:|---:|
| before: default release, `--web-dir` | 281 ms | median 30 ms (28–40) |
| after: thin LTO, embedded UI | 207 ms | median 34 ms (26–44) |

No measurable difference (noise); the debug build in R0 needed ~510 ms.

### Tests

- `cargo test --workspace`: 91 passed (90 + `web::tests::embedded_ui_is_served`: page, JS/CSS with type and
  cache header, fallback for routes/missing files, 304, 405, no log entries for the page fetches).
- `cargo clippy --workspace --all-targets` and `cargo clippy -p rp-server --all-targets` (without the
  feature): no warnings.
- Playwright against a copy of the thin-LTO `rp-server.exe` (embed-ui) started **without** `--web-dir`, empty
  data folder, CPU model: 76 passed, 8 skipped (fixme), 1.0 min.

## R2 – server cleanup (2026-11-14)

No API change; the database schema stays at 11 (`MIGRATIONS` copied unchanged). The public crate API
(`start`, `Config`, `WebUi`, `Running`, `Logger`/`Entry`/`Level`, `logs::*`, `host_allowed`,
`extract_token`, `device_label`, `default_data_dir`, `lan_ip`) is the same; `apps/desktop` needed no change.

### New layout of `crates/server/src`

| File | Job |
|---|---|
| `lib.rs` | config, `AppState`, `start` (puts router, web UI, middleware together) |
| `error.rs` | `ApiError` + `IntoResponse`: status + `{"error": code}`, the code (and storage detail) for the request log |
| `guard.rs` | Host/Origin check (DNS rebinding), API token |
| `reqlog.rs` | request log middleware, device label |
| `routes/` | `mod.rs` (route table, shared picture answer), `library`, `characters`, `personas`, `chats`, `messages`, `logs`, `engine`, `ws` |
| `store/` | `mod.rs` (`Store`, open, `MIGRATIONS`, cleanup), `characters`, `chats`, `messages`, `personas`, `settings`, `stats`, `tests` |
| `logs/` | `mod.rs` (entry, logger, writer thread, schema), `merge` (batches, repeat counter), `limits` (age/count/size, vacuum), `read` (Logs tab), `export`, `tests` |
| `chat.rs`, `web.rs`, `main.rs` | unchanged |

Every handler returns `Result<Response, ApiError>`; storage errors arrive via `?` (`From<StoreError>`,
`From<JoinError>`). The answers stay byte-equal, including which failures answer 404 `no_avatar`
instead of 500.

### `stats` fix

`GET /api/library` → `stats.last_timings` took the newest measured answer from any message that was not
rewound, also from chats and characters deleted in this session (soft delete, purged only at the next
start). It now reads the same live messages as `stats.messages` (live message in a live chat of a live
character). New test `store::tests::stats_leave_out_deleted_chats_and_characters_at_once` (delete, undo,
character delete, rewind).

Side effect on the UI tests: `e2e/global-setup.ts` gets its "last measurement" from a warm-up chat that it
deletes right away; until now that measurement stayed visible only because of this bug. With the fix the
start screen shows no measurement and the 10 screenshot tests `home.spec.ts › Start-Layout
dashboard/bento/kommando` differ (the model card shows "Sobald die erste Antwort fertig ist …"). The
setup has to keep a measured answer in a live chat (and the baselines follow), or the tests mask the panel.

### Checks

- `cargo test --workspace`: 93 passed (91 + the stats test + `error::tests::message_codes_map_to_their_status`).
- `cargo clippy --workspace --all-targets` and `cargo clippy -p rp-server --all-targets`: no warnings.
- API comparison: the same fixed sequence of 71 calls (imports, personas, settings, bindings, chats, chat
  card, delete/restore, pictures, message errors, every GET of library/personas/characters/chats/user-check/
  card, engine, the logged error lines) against the old and the new debug build, each on an empty data
  folder: status, content type and JSON body identical after replacing `created_at`/`updated_at`
  (`cmp` of both dumps: no difference, 34 018 bytes each).
- Playwright (CPU model, `apps/web/dist` as built before R3, empty data folder each run): see the table.

| Server | passed | failed | skipped |
|---|---:|---:|---:|
| old build, specs with 8 fixme | 76 | 0 | 8 |
| new build | 66 | 10 (home screenshots, see above) | 8 |
| new build with only the old `last_timings` query (control) | 76 | 10* | 2 |

| old build again, same specs as the control row | 71 | 15 | 2 |

The last row ran while R3 was changing specs and the shared CPU model was busy (4.5 min instead of 1.1); its
failures cover the same 10 former-fixme tests plus 5 timing/screenshot failures in `characters`, `chat` and
`home` that the new build did not show.

\* At that point R3 had turned 6 fixme tests on (sidebar tokens/s, Strg K → Logs, Logs live count); they
test UI fixes that are not in the stale `dist` yet. All other tests passed, including every home screenshot.

## R3a – web UI cleanup (2026-11-14)

No visible or functional change except the three old bugs below; no new library. Only `apps/web/src`
and `apps/web/e2e` changed.

### New layout of `apps/web/src`

| File | Lines | Job |
|---|---:|---|
| `App.tsx` | 530 | state, actions, which view a route shows, the shell |
| `app/route.ts` | 44 | `Route`, `routeFromHash`, `useRoute` |
| `app/useLibrary.ts` | 24 | library + load error |
| `app/useEngine.ts` | 45 | model status poll |
| `app/useChatSocket.ts` | 129 | the WebSocket: reconnect, open chat, history/tokens/done, notices |
| `app/Sidebar.tsx` | 189 | `Sidebar`, `TopBar`, `TabBar`, `StatusPill` |
| `Hub.tsx` | 72 | picks the start layout |
| `hub/blocks.tsx` | 504 | shared pieces (`Aura`, `EngineStats`, `ChatLine`, `StartWith`, `SoonTile`, `Shelf`, figures, `LeadCard`, `LeadActions`, `CountFacts`, …) |
| `hub/Classic.tsx` · `Dashboard.tsx` · `Bento.tsx` · `Stage.tsx` · `Command.tsx` | 113 · 163 · 151 · 97 · 194 | the five layouts |
| `persona/common.ts` | 33 | `about`, `PRIORITY`, `PersonaSel`, `CHAT_PERSONA_EVENT` |
| `persona/menu.tsx` | 168 | the persona menu / phone sheet |
| `persona/PersonaSwitch.tsx` · `ChatPersona.tsx` · `DefaultPersona.tsx` | 59 · 90 · 108 | switcher, chat persona, default persona |
| `persona/Personas.tsx` · `PersonaEditor.tsx` | 111 · 431 | management screen, editor |

`Persona.tsx` is gone (its importers use `persona/*`). Merged duplicates: the lead card of A/B, the
button row of C/D, the counts of A/B/D, tags/chat count of the figure cards, the two radio
groups of the persona editor (`Choices`), and the "newest measurement" rule (`newestTimings`).

### Bugs fixed

- Ctrl+K "Logs": the filter row of the Logs tab carries `data-spot="logs"`, so it scrolls into view and
  lights up like every other spot (no focus, so a phone keyboard stays closed).
- Sidebar tokens/s after a reload: the pill uses `newestTimings(live, library)`, the same rule as the
  start screen (live value, else `stats.last_timings`).
- Logs live "(×N)": the server merges a repeat only into the entry it wrote last, so the live poll asks
  for `after_id = max_id - 1` and replaces entries whose text changed. No server change needed.

### Checks

- `npx tsc --noEmit`: 0 errors.
- Playwright (release `rp-server` from R1, CPU model, own data folder): 86 passed, 2 skipped (88 =
  84 + the new "Logs live" test × 4). The 8 former `fixme` run as real tests: 6 pass, the 2 phone runs
  of the tokens/s test skip by design (the phone bar hides the speed). Against the build from before
  R3a the 3 tests fail (10 of 10 runs). No baseline screenshot was updated.
- Pixel comparison before/after (fixed browser clock, full page, both themes, 375 and 1920 px: five
  start layouts, characters, character chats + settings, chat, settings, personas list/editor/new,
  persona menu; 56 screenshots): phone identical; desktop identical except the sidebar pill
  (x 12–234, y 907–932), which now shows "Tokens/s · Cache" after a reload (bug 2).
- Bundle: JS 378.2 → 377.3 kB (gzip 111.1 → 112.4 kB), CSS unchanged (same hash).

## R3b – faster UI: React Compiler, windowed long chats, token batching (2026-11-15)

- **React Compiler**: `babel-plugin-react-compiler` 1.0 through `@rolldown/plugin-babel` + `reactCompilerPreset`
  of `@vitejs/plugin-react` 6.3 (the Babel route that plugin documents for Vite 8). 51 components/hooks are
  compiled; 9 are skipped by the compiler and stay as they were (rule breaks or syntax it does not handle yet):
  `App` (`try … finally`), `Chat` (refs read in render), `CharSettings` (eslint-disable), `CommandList`,
  `PersonaSwitch`, `ChatPersona`, `Personas`, `PersonaEditor`, `LogsTab`. Manual `useMemo`/`useCallback`/`memo` left in place.
- **Windowing** (`src/virtual.ts`, own code, ~200 lines; no library: TanStack Virtual places rows absolutely and
  would change the layout/margins of `.msg`, the chat needs end-anchoring that its API leaves to the caller):
  above 80 messages only the part around the visible one is in the page, two empty blocks hold the rest.
  Heights measured per message (ResizeObserver, margin included), unmeasured ones count with the first
  average of the chat. Opens at the end, follows the end while streaming, "Zum Ende" jumps to the last
  screen and glides from there, actions/text selection on drawn messages unchanged. Short chats: same DOM as
  before plus one wrapper `div` without styles.
- **Token batching** (`src/tokens.ts`, used in `onEvent` of `app/useChatSocket.ts`, where R3a moved the socket code): streamed text is collected per message and
  applied once per animation frame; any other socket event hands over waiting text first, so the order stays.
- **Formatters** (`src/api.ts`): `num`/`when` use one `Intl` formatter each instead of `toLocale…String` per call
  (same output, checked over 2000 dates and sample numbers). The CPU profile while streaming showed `when()`
  (the sidebar formats every chat's time on each render) as the largest single JS cost per frame.

### Bundle (`vite build`)

| File | before (R0) | after |
|---|---:|---:|
| `index-*.js` raw / gzip | 378.2 / 112.5 kB | 418.7 / 129.2 kB |
| `index-*.css` raw / gzip | 101.4 / 21.4 kB | 101.4 / 21.4 kB |

+40.5 kB raw / +16.7 kB gzip: the compiler's memo code is +36.4 kB of it (build with only the compiler:
414.6 kB), windowing + batching ~4 kB. Bigger, not smaller; lazy-loading the side screens (rest of R3) is
where the bundle shrinks.

### Long chat, 375×812 (phone-graphit), 500 messages

Same machine, test server on localhost, fresh browser context per run, 3 runs each; values as range. "Usable"
= navigation start until the composer exists and the last message is on screen (checked every frame).
Streaming: the page's socket gets a real stored model answer as token events at about 70 tokens/s for 5 s
(Playwright `routeWebSocket`; the CPU test model's 4096-token context cannot answer in a 500-message chat).
CPU times from `Performance.getMetrics` over those 5 s; frames = `requestAnimationFrame` callbacks (headless,
not capped by a display). "×6" = Chrome CPU throttling 6×, roughly a mid-range phone.

| | before | after |
|---|---:|---:|
| usable after opening | 135–145 ms | 114–125 ms |
| usable after opening, ×6 | 997–1180 ms | 804–827 ms |
| long tasks while opening, ×6 | 3–4, 785–973 ms | 4, 615–631 ms |
| messages / DOM nodes in the page | 500 / 31,570 | 6 / 952 |
| JS heap after GC | 8.3 MB | 3.1 MB |
| streaming: main-thread task time per 5 s | 1196–1253 ms | 626–630 ms |
| streaming: script / layout | 318–323 / 169–176 ms | 196–201 / 53–54 ms |
| streaming: longest frame gap | 15–18 ms | 8 ms |
| streaming ×6: frames per second | 45–51 | 227–230 |
| streaming ×6: longest frame gap | 132–174 ms | 76–79 ms |
| streaming ×6: task time / script / layout | 5537–5575 / ~1650 / ~720 ms | 3794–3915 / 1264–1312 / 343–358 ms |
| streaming ×6: long tasks | 0 | 1 (74–77 ms) |

Long tasks are 0 in "before" although its frames stall longer: its cost is spread over many short tasks
(one per token event).

### Tests

- `npx tsc --noEmit`: 0 errors.
- Playwright against a copy of the R1 thin-LTO `rp-server.exe` with `--web-dir` (after build), CPU model:
  80 passed, 8 skipped (fixme), 1.1 min — the 76 from R0 with unchanged screenshots, plus the new
  `long-chat.spec.ts` (4 projects): opens at the end with < 80 messages drawn, wheel-scrolling up never shows
  an empty screen, top reached, text selection, copy, edit, "Zum Ende" from the top, fork and rewind + undo
  on old messages. It writes its 500 messages straight into the test database (`RP_E2E_DATA`, `e2e/db.ts`).

## R4-Rust – other platforms (2026-11-15)

Goal: the Rust code builds for Linux/macOS desktops and Tauri v2 phone apps later, with no change on Windows.

### Changes

- `crates/server/src/lib.rs` `default_data_dir`: one place for the data folder. Windows unchanged
  (`%APPDATA%\rp-app`); macOS `~/Library/Application Support/rp-app`; other Unix `$XDG_DATA_HOME/rp-app`
  (absolute only), else `~/.local/share/rp-app`. Android/iOS hand out the folder only through the platform
  API, so a phone shell passes `Config::data_dir` itself. New test pins the Windows path.
- `apps/desktop/src-tauri/src/engine.rs`: `config_dir()` uses `rp_server::default_data_dir()` (was a copy).
  `lock_file`: Windows keeps `share_mode(0)`; elsewhere `File::try_lock` (std, flock on Unix) — before, a
  non-Windows build had no lock at all, so two instances could each start a model server. The lock test
  now runs on every system. The "gave up" note names `%APPDATA%\rp-app` only on Windows.
- `crates/server/src/main.rs`: `--help` names the XDG default.
- Already in place and kept: `CREATE_NO_WINDOW` (engine, log export) and `windows_subsystem` behind
  `cfg(windows)`, free disk space via `GetDiskFreeSpaceExW` / `statvfs`, `crates/hw` returns
  `Error::Unsupported` off Windows. No new dependencies.

### Checks

- `cargo test --workspace`: 139 passed, 2 ignored (138 + the new data-folder test).
- `cargo clippy --workspace --all-targets`: 0 warnings (also for `--target x86_64-unknown-linux-gnu`).
- `cargo check --all-targets -p rp-core -p rp-models -p rp-server -p hw` for `x86_64-unknown-linux-gnu`
  and `aarch64-linux-android`: ok. `rp-core` checks with nothing else; `ring` (rustls in `rp-models`) and
  the bundled SQLite (`rusqlite`) compile C in their build scripts, and this machine has no Linux/Android
  C toolchain (no gcc cross compiler, NDK or sysroot). For the check, `CC_<target>` pointed at a
  throw-away wrapper that writes empty object files, so the Rust code was type-checked and the C parts
  were not compiled. A real build needs `gcc`/`clang` with a Linux sysroot or the Android NDK.
- `rp-desktop` for Linux: stops at `libdbus-sys` (pkg-config, no cross sysroot). A Linux build needs
  `pkg-config`, `libdbus-1-dev`, `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`
  (Tauri v2 prerequisites), simplest on Linux itself or in CI. `lock_file`'s non-Windows branch was
  compiled alone for both targets.

### Host/Origin check and a UI the server does not serve (for phase 3, pairing)

Today (`crates/server/src/guard.rs`, unchanged): every request needs a Host that is `localhost` or an IP
literal (private/link-local only in LAN mode); an `Origin`, if sent, must pass the same host test (scheme
and port are ignored); `/api` needs the one global token. A Tauri phone app loads its UI from its own
origin — `http://tauri.localhost` / `https://tauri.localhost` (Android, Windows) or `tauri://localhost`
(iOS, macOS):

- Host is the PC's LAN IP: passes in LAN mode.
- Origin `tauri.localhost`: rejected (403) — HTTP and WebSocket. `tauri://localhost` passes only by
  accident, because the check reads the host part alone.
- No CORS: a cross-origin `fetch` with `Authorization` sends a preflight `OPTIONS` without the token,
  which `require_token` answers with 401, and no `Access-Control-Allow-*` headers come back.

To change in phase 3 (not before): an exact allowlist of app origins (scheme + host + port) accepted
only together with a per-device token from pairing instead of the global one; a CORS layer
(`tower-http` `cors`) in front of `require_token` that answers preflights for exactly those origins
(methods used, headers `Authorization`/`Content-Type`, no cookies); the WebSocket keeps checking Origin
against the same list; tighten the Origin test to compare scheme and port too, so another local page
(`http://localhost:<other port>`) no longer passes on its host alone. A phone shell also needs
`#[cfg_attr(mobile, tauri::mobile_entry_point)]` and must not run the llama-server supervisor
(`engine.rs` spawns a local process; on a phone it would only connect to the PC).

## R3b rest + R4 UI – lazy screens, server address, notch and touch (2026-11-15)

Only `apps/web` changed. Nothing changes on a PC: every screenshot at 1920 is pixel-identical (see Tests).

### Lazy loading

- `App.tsx`: the characters list (`Library`), the character page (`Character` with `UserFit` and
  `DefaultPersona`, also the card for one chat), the app settings with the logs (`Settings`) and the persona
  manager with the editor (`persona/Personas`) are `React.lazy` chunks. Start (all five layouts) and chat stay
  in the main bundle. One `Suspense` around the screen; its fallback is an empty `.view` with `aria-busy`
  (same grid cell, so the sidebar/tab bar do not move).
- `app/route.ts`: a route change is a `startTransition`, so a screen whose chunk is still loading keeps the
  old one on view instead of a blank page. All side chunks are fetched once the browser is idle
  (`requestIdleCallback`, else 500 ms), so in practice a click never waits. A side screen opened straight
  from the address shows the empty page for the moment its chunk loads.
- `commands.ts` (new): `buildCommands` and the `Command` types moved out of `Palette.tsx` into their own chunk.
  It loads at once when the quick actions window opens or the start layout "Kommandozentrale" is on screen,
  else when idle; until then the list is empty. `Palette.tsx` keeps the search list, the window and the
  command field.

### Server address + token (`src/api.ts`)

Default unchanged: same origin, token from `?token=` (kept for the tab in sessionStorage). A UI not served by
the server reads `{ base, token }` from localStorage key `rp-server`, set by `connectTo({ base, token })`
(reloads; `null` goes back to the page's own server; no settings screen, pairing comes in phase 3). A token in
the address always means "this page's server", so a stored pair is never combined with it. `apiUrl()` and
`wsUrl()` build every address: `api()`, the avatars, the log upload (`log.ts`), the log export link and the
socket (`app/useChatSocket.ts`).

### Notch and home bar, touch targets (`style.css`)

All `env(safe-area-inset-*)` values are 0 on a PC, so nothing moves there.
- Phone top bar: its grid row grows by the top inset (before, the inset was padding inside the fixed 52 px
  row and squeezed the bar's content under its own border). Persona editor header and its save bar take
  the top/bottom inset (they had fixed paddings that overrode it). Pages of screens without the tab bar end
  above the home bar; the quick actions window fits between both insets.
- Landscape / tablet: `.app` keeps clear of the notch sides; from 768 px also of status and home bar; the toast
  and the quick actions window follow.
- Already in place before and kept: top bar, chat/character headers, chat bar, tab bar, composer, save bar,
  persona sheet.
- Touch targets at 375 px (audit of every visible control on all screens, 5 start layouts, persona sheet,
  quick actions): "Zum Start" in the top bar 34→44 px wide (negative margin, icon and neighbours stay in
  place), "Alle" links 21→45 px wide (same trick), "Neuer Chat" on the figure cards of "Kommandozentrale"
  32→44 px high (`.btn.sm` won over the 44 px meant there), persona editor buttons 32/40→44, user-check chips
  40→44, "Ältere laden" in the logs 32→44, quick-action examples keep their 36 px look with a 44 px hit area.
  Left below 44 on purpose: the radio inputs inside `.choice` (the whole card is the target), the shelf arrows
  (hidden on touch screens).
- Checked with emulated insets (CDP `Emulation.setSafeAreaInsetsOverride`, 375×812 top 47 / bottom 34, and
  812×375 left/right 47 / bottom 21) in Graphit and Papier: start, chat, settings, personas, persona editor,
  character settings, quick actions, persona sheet.

### Bundle (`vite build`)

| File | before (`main`, R3b) raw / gzip | after raw / gzip |
|---|---:|---:|
| `index-*.js` (main) | 423.2 / 132.7 kB | **345.8 / 109.6 kB** |
| `Character-*.js` | – | 34.2 / 12.0 kB |
| `Settings-*.js` | – | 14.8 / 5.3 kB |
| `Personas-*.js` | – | 13.7 / 4.8 kB |
| `Library-*.js` | – | 10.3 / 4.1 kB |
| `commands-*.js` | – | 8.4 / 3.0 kB |
| `spot-*.js` (shared) | – | 1.2 / 0.6 kB |
| `index-*.css` | 101.4 / 21.4 kB | 102.4 / 21.6 kB |

Main bundle below the R0 baseline before the React Compiler (112.5 kB gzip). Without `Library` and
`commands` as chunks it stayed at 115.6 kB.

### Start screen usable

Same method as R0 (navigation start until the "Zuletzt mit …" link of layout "Bisher" is on screen; fresh
context per run, 7 runs, test server on localhost, same data).

| Viewport | before median (range) | after median (range) |
|---|---:|---:|
| 1920×1080 | 113 ms (109–236) | 116 ms (109–255) |
| 375×812 | 107 ms (105–109) | 103 ms (102–115) |
| 375×812, CPU ×6 | 867 ms (833–885) | 878 ms (855–900) |

No measurable difference: on localhost the start waits for `/api/library`, not for parsing 23 kB less gzip.
The gain is on a slow network (phone over Wi-Fi) and in memory; side screens load idle after the start.

### UI from another origin (`e2e/remote.spec.ts`, new)

A plain Node static server on a free port hands out the built UI under `http://localhost:<port>`; the page
gets the test server only through `rp-server` in localStorage (no token in its address).
- Chat socket: connects to the test server, the chat's history arrives (4 of 4 projects pass).
- Library over HTTP: blocked by the browser today. Preflight `OPTIONS /api/library` answers 401 with no
  `Access-Control-*` headers (as described in "R4-Rust" above). The test skips itself while the server
  sends no CORS headers and runs once it does. With CORS switched off in the browser
  (`--disable-web-security`, one-off check, not committed) the same test passes in all 4 projects: library,
  model status, avatars, log upload all work, so the UI side is complete.
- Server side needed (phase 3): CORS for `/api` in front of `require_token` that answers preflights for the
  allowed origins with methods `GET, POST, PUT, DELETE` and headers `authorization, content-type,
  x-rp-client` (the UI sends `X-RP-Client`), and `Access-Control-Allow-Origin` + `Vary: Origin` on the
  answers; plus the app origins in the Origin check (`http(s)://tauri.localhost`). The socket test passes
  today only because the Origin check ignores the port; if it is tightened to exact origins, the test's
  `http://localhost:<port>` must be allowed in test runs. Not covered by the UI: the log export is an
  `<a download>` link, which a browser ignores across origins (it navigates instead), and an app served
  from `https://` cannot call the PC over plain `http://` (mixed content).

### Tests

- `npx tsc --noEmit`: 0 errors.
- Playwright (debug `rp-server` from `main`, CPU model): 94 passed, 6 skipped (90 + 4 socket tests; skipped:
  the 2 phone runs of the tokens/s test as before + 4 library-over-HTTP tests waiting for CORS). 1.7 min.
- Screenshots: all 56 baselines rendered with the `main` UI and with the new UI on the same data and
  compared pixel by pixel: 51 identical, 5 differ only by data (the log count text in 4 logs shots, 6 pixels
  by one colour level in one bento shot). No baseline needed a new image: the controls that grew at 375 px lie
  outside the screenshotted areas, the top bar change is optically neutral. Phone insets are 0 in the
  tests, as on every device without a notch.
- Known, not changed here: at 768–900 px width (desktop layout, e.g. a phone in landscape) the chat bar's
  buttons run past its right edge (`scrollWidth` 687 px in a 544–676 px bar), already so on `main`.
