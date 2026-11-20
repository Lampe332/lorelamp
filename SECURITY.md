# Security

## Reporting a problem

Please don't open a public issue for a vulnerability. Report it privately through GitHub instead:
**Security → Report a vulnerability** in this repository
([direct link](https://github.com/Lampe332/lorelamp/security/advisories/new)).

Say what an attacker can do, under which conditions (same PC, same Wi-Fi, a web page in the browser, a
crafted character card or link, a chat text), and how to reproduce it. I'll answer there and credit
you in the advisory unless you'd rather not be named. This is a one-person project, so a fix can take
a few days.

## How Lorelamp is exposed

Lorelamp runs an HTTP and WebSocket server on your PC. The desktop window, phone browsers and the
Android app all talk to it.

- **Local only by default.** The server listens on `127.0.0.1`. It listens on your network only while
  *Settings → Phone → Phones may connect* is on (or when started with `--lan` / `RP_LAN=1`). Switching
  it off closes that listener and cuts every phone connection.
- **Two tokens.** Every API request needs a random token. The desktop window's token is made anew at
  each start and only works on `127.0.0.1`. Phones get a separate token, which only works over the
  network listener. A phone receives it by scanning the QR code or by trading the 6-digit pairing
  code (valid for 10 minutes; five wrong tries lock that address for 30 seconds). *Disconnect all
  phones* makes a new phone token. Treat the QR code like a password.
- **PC-only actions.** Model profiles, engines, downloads and folders, the benchmark, the SillyTavern
  import, deleting and restoring backups, the phone access itself and starting with Windows answer
  `403` over the network listener.
- **Host and Origin check** on every request, against DNS rebinding: the `Host` must be `localhost` or
  an IP address, and a non-loopback address must be a private, link-local or shared-range one.
- **Security headers** on every answer: a Content-Security-Policy that only runs the app's own
  scripts, `Referrer-Policy: no-referrer`, `nosniff`, no framing. The desktop window only navigates
  inside the app; the Android app asks before a `lorelamp://` link connects it to a PC.
- **Plain HTTP in the home network.** The traffic between phone and PC is not encrypted. Use it only in
  a network you trust.
- **Link import** fetches cards, and pictures pasted into a picture field, from the internet on the
  server's side; the page itself is not allowed to contact other sites. Those requests go out over
  HTTPS on port 443 only, every resolved address must be public (no requests into your own network),
  redirects are checked again, and there is one time budget per link. Imports share one memory budget
  with limits on file size, ZIP entries and picture size.
- **Model engine.** llama.cpp's `llama-server` runs as a separate process on `127.0.0.1` and ends with
  the app. Lorelamp downloads official builds from the llama.cpp releases on GitHub and checks each
  file against the SHA-256 digest GitHub publishes for it. Start options pass an allowlist: nothing that
  reads files, downloads or overrides the chat template.

Your chats, cards and settings stay in Lorelamp's data folder on your PC. Lorelamp has no account and
no telemetry.

## Supported versions

Only the latest release gets fixes.
