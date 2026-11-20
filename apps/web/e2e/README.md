# UI tests (Playwright)

Safety net for the rework: every main screen at 375×812 and 1920×1080, in the themes Graphit and
Papier (4 projects). The tests talk to a separately started test server with its own data folder
and a real model on the CPU. They never touch the owner's app or data.

## 1. Test environment

All paths below are examples; use any temp folder.

1. Web UI into a temp folder (not into the repo):
   `cd apps/web && npx vite build --outDir <temp>/dist --emptyOutDir`
2. Server: `cargo build -p rp-server`, then **copy** `target/debug/rp-server.exe` to `<temp>/` and
   start the copy (so later Rust builds are not blocked):
   `<temp>/rp-server.exe --port 7861 --web-dir <temp>/dist --data-dir <temp>/data --llama http://127.0.0.1:7871`
   The token is in its output (`local: http://127.0.0.1:7861/?token=…`). Use an empty `data` folder
   for a clean start; reusing it for further runs is fine. Set `RP_LMSTUDIO_DIR` to an empty folder
   (e.g. `<temp>/lmstudio`) before starting it, so "Meine Modelle" never lists the real LM Studio models
   (they would land in the screenshots).
3. Model, CPU only (never a second model server on the GPU): the official llama.cpp CPU build
   (`llama-<build>-bin-win-cpu-x64.zip`, SHA-256 checked against the release asset's `digest`) with a
   small instruct model, e.g. Gemma 4 E2B Q4_K_M:
   `llama-server.exe -m <model>.gguf -ngl 0 -c 4096 -np 1 --port 7871 --host 127.0.0.1`
4. Edge must be installed (the tests use `channel: "msedge"`, no browser download).

Alternative to 2 + 3, needed for the profile switch in `model.spec.ts` (it skips itself otherwise):
let the test server start the CPU model itself. Put an `engine.json` into the empty `<temp>/data`
before the first start (it becomes the first model profile), e.g.
`{"llama_server": "<cpu build>/llama-server.exe", "model": "<model>.gguf", "port": 7871, "args": ["-ngl", "0", "-c", "4096", "-np", "1"]}`,
and start `<temp>/rp-server.exe --port 7861 --manage-engine --web-dir <temp>/dist --data-dir <temp>/data`
(no `--llama`). `unload.spec.ts` needs this mode too (it skips itself otherwise); its quiet-time test
also needs a shortened minute: start the server with `RP_IDLE_MINUTE_SECS=2` in its environment and
run the tests with the same value in `RP_E2E_IDLE_MINUTE_SECS`. The switch test adds a second profile with another context, switches to it and back,
so only one model server runs at a time. Check first that no other `llama-server` runs.
`benchmark.spec.ts` (P6) needs this mode too and, so a benchmark takes seconds, shortened runs in the
server's environment: `RP_BENCH_CTX=2048,4096 RP_BENCH_PROMPT_TOKENS=256 RP_BENCH_GEN_TOKENS=16
RP_BENCH_BUDGET_SECS=120` (it checks a 2:00 budget). It runs real benchmarks against the CPU model, applies
"Tempo" and gives the profile its start options back afterwards.
`engine-problem.spec.ts` (R2) needs this mode, the same shortened runs and `RP_E2E_DATA`: it holds a free
port with its own small TCP listener and points `engine_port` there (and back), lets a profile with a
tiny unloadable GGUF crash until the supervisor gives up (about 30 s per project), and writes benchmark
results into the test database; the port, the active profile and the stored results are put back.
`backups.spec.ts` needs `RP_E2E_DATA` for its copy made by hand (`rp-app.db.backup-e2e-hand` next to the
database, removed again; it skips that test otherwise); it deletes the manual backups it makes.
`models-local.spec.ts` needs `RP_E2E_DATA` (it writes tiny GGUF files into the app's model folder
`<data>/models/e2e` and removes them again); with `--manage-engine` it also applies new start options
(2k context, q8_0 KV cache) and checks the context the model server reports, then restores the profile.
Its "Benutzen" test activates a profile for one of those tiny files, so the model server fails to start
once before the test switches back.

`purpose.spec.ts` (Y1) needs `RP_E2E_DATA`: it stores PC scans of other PCs (an 8 GB NVIDIA card, a
24 GB AMD card, no card with 16 GB RAM) and checks the purpose field, its chips, the answer and the
catalog's order against the server's own answer; with `--manage-engine` it also writes a benchmark
result for the active profile into the test database, lets "Beste Einstellung für deinen Zweck" set
it (8k context, DRY, reply length; the CPU model restarts once) and puts profile, results, scan and
purpose back.

`general.spec.ts` (Q2, settings tab "Allgemein") needs `RP_E2E_DATA` (it fills chats and compares the
data folder's path). "Mit Windows starten" writes into the registry: start the server with
`RP_AUTOSTART_KEY` set to a scratch key below `HKEY_CURRENT_USER` (e.g. `Software\Lorelamp-e2e\Run`)
and run the tests with the same key in `RP_E2E_AUTOSTART_KEY`; the test skips itself without it and
refuses the real `Run` key. The data folder test opens one real Explorer window (project
`desktop-graphit` only) and closes it again. Sending with Ctrl + Enter needs a model that answers and
skips itself otherwise.

Model downloads (`suggestions.spec.ts`): the test server must fetch from a local stand-in for Hugging
Face, never from the real site. Start it with `RP_HF_BASE=http://127.0.0.1:7877/` in its environment;
the spec starts the stand-in (`e2e/hf-standin.ts`, tiny files, port `RP_E2E_HF_PORT`, default 7877)
itself. Without `RP_HF_BASE` the download tests skip themselves; the filter test always runs (with
`RP_E2E_DATA` it writes one tiny GGUF under a catalog file name into the app's model folder, so the
"Installiert" filter has something to find). The "Benutzen" test needs `RP_E2E_DATA` too: it puts a tiny
file under the name of the first roleplay pick, activates it (the model server fails to start once),
then switches back to the suite's profile and waits until the model is ready again. Use a
fresh data folder for the first run with the stand-in: the catalog's live check runs at most once per
24 h per repo and keeps its result in `catalog-cache.json`.

SillyTavern search (`import-preview.spec.ts`): the server looks for SillyTavern installations on its
PC; start the test server with `RP_ST_SEARCH_ROOTS=<temp>/st-roots` (an empty folder) so it never
searches the real drives, and run the tests with the same folder in `RP_E2E_ST_ROOT` (they build a
small SillyTavern there and remove it again; without it those two tests skip themselves). The link
preview tests answer the browser's three preview requests with a stand-in (the server's card sources
can only be moved inside the Rust tests); its confirm stores the card through the real server.

Links of any web site (`import-links.spec.ts`): the server fetches them itself and refuses every local
address, so the stand-in site (`e2e/card-site-standin.ts`, started by the spec) has to be allowed: start
the test server with `RP_LINK_TEST_ORIGIN=http://127.0.0.1:7878` (exactly one origin, this PC only) and
run the tests with the same address in `RP_E2E_LINK_SITE`. Without it the link tests skip themselves;
the paste tests (card picture, picture without card data, card as text) always run. The picture link of
a persona (`images-export.spec.ts`, `POST /api/pictures/from-link`) is fetched by the server the same
way and needs the same two settings.

## 2. Run

```
set RP_E2E_URL=http://127.0.0.1:7861
set RP_E2E_TOKEN=<token from the server output>
set RP_E2E_DATA=<temp>\data        the server's --data-dir; the long-chat test writes its 500 messages there
set RP_E2E_SHOTS=<folder>   optional: suggestions.spec.ts saves review screenshots there
npm run test:e2e            # = npx playwright test
npx playwright test --project desktop-graphit   # one project only
npx playwright test --update-snapshots          # new baseline screenshots (check them!)
```

`global-setup.ts` imports the SFW cards from `tests/fixtures`, gives each one chat, adds a second
persona and, if the data folder has no measured answer yet, runs one real exchange with the model in
Mara's chat (the chat stays, so the start screen shows its measurement; the answer text is replaced
by a fixed one so screenshots do not depend on the model).
It only adds what is missing, so runs can repeat against the same data folder.

Theme and start layout are server-wide settings, so the tests run in one worker, one after the other.
Chat tests create their own chats and delete them afterwards. `long-chat.spec.ts` fills its chat to
500 messages directly in the test database (`e2e/db.ts`, Node's built-in `node:sqlite`), because the
API can only add messages through model answers.

`chat-look.spec.ts` (how a chat looks: the settings tab "Chat", a chat's own "Aussehen", the large
picture, backdrops, the fade of new text) needs the model for its two animation tests. It draws its
backdrop picture itself (a PNG built in the test, no file), checks with `RP_E2E_DATA` that the picture
lands as `backdrops/chat-<id>.jpg` in the data folder, and puts the app-wide reading settings and the
test character's default backdrop back after every test. With `RP_E2E_SHOTS` it saves review
screenshots (backdrop, large picture, an answer in the middle of its fade, the settings tab).

`chat-wide.spec.ts` (the layout "Messenger breit" at 1280, 1920 and 2560 px and on a 375 px phone, and the
finer look settings: bubbles, swapped sides, edge and spacing, narration and thought inks, name, time and
picture, new messages, the typing line, Ctrl+K) needs `RP_E2E_DATA` (it writes a short fixed exchange,
`e2e/story.ts`, into its own chats) and, for its test of new messages, the model. With `RP_E2E_SHOTS` it
saves review screenshots.

`backdrop-presets.spec.ts` (the gallery of ready-made backdrops in "Aussehen": for a chat, as a
character's default, "Kein Hintergrund", a tablet window) needs no model. With `RP_E2E_DATA` it checks the
JPEG copies in `backdrops/`. Its restart test (`desktop-graphit` only) needs `RP_E2E_RESTART`: a command that
restarts the test server on the same port and data folder and prints its new `local: …?token=…` line; the
new token is passed on to the later workers of the run (global setup names a file for it in the temp
folder). Without it that test skips itself.

`pc-scan.spec.ts` scans the PC the tests run on (the expected values come from the server's own
answer) and, in `desktop-graphit` only, downloads the newest official CPU build from the llama.cpp
releases on GitHub (~25 MB, needs network), lets the server self-test it and removes it again. It never
downloads or starts a GPU build.

`phone.spec.ts` ("Handy verbinden": QR code decoded with `jsqr`, pairing code, "Alle Handys trennen", the
page with `&shell=android`) switches phones on, which opens a second listener: start the test server with
`RP_LAN_BIND=127.0.0.1` and `RP_DISCOVERY_PORT=<a free port>` so it stays on this PC, and run the tests with
`RP_E2E_LAN=1` (the switch test skips itself otherwise). `engine-setup.bare.ts` ("Engine einrichten" in the
first steps) runs with `first-run.config.ts` against the empty server started with
`RP_GITHUB_API=http://127.0.0.1:<RP_E2E_GH_PORT, default 7879>` (a local stand-in for the llama.cpp releases,
`e2e/github-standin.ts`, never GitHub), plus `RP_E2E_DATA` and `RP_E2E_LLAMA_CPU` (a folder with an official
CPU build; the stand-in packs it, the self-test runs it on the CPU only); it removes the engine again.

`tablet.spec.ts` (the tablet layout, `src/tablet/`) brings its own windows: 800×1280, 1280×800, 1024×1366
and 1366×1024 with touch (`hasTouch`, `isMobile`), plus `&shell=android` with a mouse. It runs once per
theme in the desktop projects and skips itself in the phone projects. Layout is checked by position and
size, not by baseline pictures; with `RP_E2E_SHOTS` it saves review screenshots of every rebuilt screen.
Swipes are real touch input (`Input.dispatchTouchEvent`); the variants test needs `RP_E2E_ST_ROOT` like
`swipes.spec.ts`.

`phone-nav.spec.ts` (a phone's bottom bar and its sheet "Mehr": the four places, the sheet's rows, closing
by Escape, by its backdrop and by dragging it down, the way back to the Android app's start page with
`&shell=android`) needs no model answer; on the desktop projects it only checks that none of it is on
screen.

`remote.spec.ts` serves the built UI from a second, plain static server on a free port (another origin)
and points it at the test server through the stored server address + token (`connectTo` in `src/api.ts`).
The socket test always runs; the library-over-HTTP test skips itself while the server sends no CORS headers.

## 3. Screenshots

`toHaveScreenshot` per screen, baselines in `e2e/__screenshots__/`. Clock times, counters, model
measurements, toasts and the log list are masked; the tolerance is 2 % of the pixels. The
baselines show only the SFW test cards.

## 4. Former bugs

These ran as `test.fixme` until R3a fixed them; they are ordinary tests now:

- Ctrl+K "Logs" jumped to `data-spot="logs"`, which no element had (`settings.spec.ts`).
- The sidebar's tokens/s were empty after a reload (`chat.spec.ts`).
- The live Logs view showed a merged entry's rising "(×N)" only after a reload (`settings.spec.ts`).
