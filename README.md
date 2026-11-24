# NOCTRA
**Product of NOCTA Ecosystem** | **Tagline:** *Beyond Browsing.*

NOCTRA is a cross-platform, Rust-first desktop web browser engineered for high performance ("potato-laptop fast" with aggressive background tab hibernation and zero-bloat native Rust core), strict multi-profile and workspace isolation, an internal floating multi-surface window manager, hardened transport security (HTTPS-Only, TLS 1.3, DoH, ECH, MITM Root CA detection, VPN/Tor kill-switch), a built-in Developer Hub & Network Inspector, and an OS-vault-backed AI & MCP Agent Connection Center.

---

## Core Architecture & Crates

- [`crates/noctra-types`](crates/noctra-types/src/lib.rs) — Centralized design token system (`#17181C` charcoal, `#8B7CF6` deep violet accent), `RedactedSecret` zero-leak wrapper, domain models, and settings.
- [`crates/noctra-platform`](crates/noctra-platform/src/lib.rs) — Cross-platform OS capability detection, native credential storage bridge (`libsecret` / Keychain / WinCred + ChaCha20-Poly1305 AEAD encrypted vault), and notifications.
- [`crates/noctra-database`](crates/noctra-database/src/lib.rs) — SQLite WAL persistence engine, schema migrations, profile-partitioned KV store, bookmarks, history, and Session Time Machine snapshots.
- [`crates/noctra-profiles`](crates/noctra-profiles/src/lib.rs) — Multi-Profile system (`Personal`, `Work`, `Development`, `Testing`, `Guest`) and NOCTRA Spaces (`SharedSession` vs `Isolated` workspace storage partitions & automatic domain routing).
- [`crates/noctra-wm`](crates/noctra-wm/src/lib.rs) — Internal Floating Window Manager (free-floating drag, resize, magnetic edge snap, minimize/maximize/restore, Z-index, Picture-in-Picture, and Smart Tiling: 2-Split, 3-Pane, 4-Grid, Hybrid).
- [`crates/noctra-privacy`](crates/noctra-privacy/src/lib.rs) — Ad & tracker filter, HTTPS-only auto-upgrade, TLS 1.3/DoH/ECH, non-public Root CA corporate MITM proxy detector, VPN/Tor kill-switch, `"What can an observer see?"` residual metadata matrix, and automated wire-capture leak verification.
- [`crates/noctra-network-monitor`](crates/noctra-network-monitor/src/lib.rs) — `WifiInfoProvider` trait (local SSID/BSSID/RSSI/PHY link speed with capability detection), passive NOCTRA throughput counter, and opt-in latency/jitter/throughput speed test.
- [`crates/noctra-downloads`](crates/noctra-downloads/src/lib.rs) — Multi-connection HTTP download manager with directory traversal sanitization, file categorization, duplicate detection, and mandatory SHA-256 checksum verification.
- [`crates/noctra-torrent`](crates/noctra-torrent/src/lib.rs) — Native BitTorrent & Magnet URI client with Bencode `.torrent` parser, selective file priorities, DHT/PEX counts, speed caps, and interface binding.
- [`crates/noctra-devhub`](crates/noctra-devhub/src/lib.rs) — Built-in Developer Hub (JSON/YAML validator, JWT decoder, Base64/URL codec, UUID/Hash generator, Regex tester, DNS lookup) & Network Inspector (API request tester, HAR 1.2 exporter, cURL generator).
- [`crates/noctra-ai`](crates/noctra-ai/src/lib.rs) — Multi-provider AI adapters (OpenAI, Anthropic, Gemini, Ollama, OpenAI-Compatible), MCP JSON-RPC 2.0 Agent Connector, prompt injection boundary guard, and default-deny `AgentPermissionGate` with high-risk user approval queue.
- [`crates/noctra-sync`](crates/noctra-sync/src/lib.rs) — End-to-end encrypted (`ChaCha20-Poly1305`) NOCTRA Sync engine, selective sync, offline queue, device management, and Google OAuth 2.0 PKCE integration.
- [`crates/noctra-chromium`](crates/noctra-chromium/src/lib.rs) — Chromium/Blink + V8 process controller, low-spec memory flags, and live web surface proxy/reader extractor.
- [`crates/noctra-core`](crates/noctra-core/src/lib.rs) — Unified browser orchestrator, intelligent omnibox resolver, Potato-Laptop Resource Governor, Session Time Machine, and Browser Automation engine.
- [`apps/noctra-desktop`](apps/noctra-desktop/src/main.rs) — Native desktop binary, authenticated local IPC server, and multi-surface desktop UI shell.

---

## Building & Running

```bash
# Run full workspace unit, security, and wire-capture tests
cargo test --workspace

# Build optimized release binary
cargo build --workspace --release

# Launch NOCTRA Desktop Browser
cargo run -p noctra-desktop -- --port 9477
```
