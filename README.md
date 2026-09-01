# Email Assistant

A private, on-device email assistant. It connects straight to your mail provider
over IMAP/SMTP and runs every AI feature against a **local** model — nothing about
your mail is sent to a third-party service.

Built with Tauri v2 (Rust backend, vanilla-TypeScript frontend).

## What it does

- **Inbox** — fetches recent INBOX messages and caches them locally in SQLite.
- **Triage** — classifies messages (`urgent` / `needs_reply` / `fyi` / `newsletter` / `spam`),
  with labels and action items, using a small local model.
- **Summaries** — a 2–3 sentence summary per message, generated on demand and cached.
- **Assistant chat** — ask questions about your mail ("what did Dana last send?").
  It answers only from your cached inbox through a bounded set of read-only tools.
- **Reply drafts** — the assistant (or a button) writes a reply. It lands in the
  **Drafts** tab. **Nothing is ever sent without an explicit, confirmed click** —
  there is no auto-send anywhere in the app.

## Try it yourself

Downloads are on the [Releases page](https://github.com/jonsagent8/email-assistant/releases).
The builds are **unsigned**, so:

- **macOS** (Apple Silicon only): after opening the `.dmg` and dragging the app to
  Applications, right-click it → **Open** the first time to get past Gatekeeper.
- **Windows**: SmartScreen will warn — choose **More info → Run anyway**.

### You also need Ollama

The app runs its models through [Ollama](https://ollama.com), which it does **not**
bundle. Before first launch:

1. Install Ollama.
2. Pull the two models the app uses by default:
   ```
   ollama pull qwen3:8b-q4_K_M
   ollama pull qwen3:1.7b
   ```

The app starts `ollama serve` itself if it isn't already running. The 8B chat
model needs ~6–7 GB of free RAM; plan on a 16 GB machine.

### Connecting your account

Use an **app password**, not your main password. For Gmail / Outlook / Yahoo /
iCloud the app fills in the server settings automatically from your address and
links you to the right app-password page. Credentials are stored in the OS
keychain — never in the app's database.

## Development

```
npm install
npm run tauri dev
```

### Tests

```
cd src-tauri
cargo test                        # unit tests — no network, no model
cargo test -- --ignored           # live tests — need a local Ollama + the models above
```

CI runs `cargo test` and `cargo clippy -D warnings` on every push, then builds
unsigned macOS + Windows bundles. Pushing a `v*` tag publishes a Release with the
installers attached.
