# turzx-ai-monitor

A dashboard for the **TURZX 8.8" USB bar LCD** (1920×480) on Windows, replacing the vendor app:
AI plan usage (Claude, Codex, Antigravity), CPU / GPU / RAM / VRAM load and temperatures, fans,
network, clock, calendar and weather.

> **Work in progress.** Settings, languages (Korean / English), holidays by country, an installer
> and full documentation are being added. Until then the code is set up for one machine.

## Build

```powershell
cd rust
cargo build --release
```

Mascot animations are not part of this repository (their art belongs to their owners); the
dashboard builds and runs without them.

## License

MIT, see [`LICENSE`](LICENSE). Bundled fonts and modules: [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
