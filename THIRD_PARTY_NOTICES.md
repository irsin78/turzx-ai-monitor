# Third-party notices

The dashboard's own code is MIT licensed (see `LICENSE`). It ships or builds on the following.

## Bundled files

| Files | Project | License | Text |
|---|---|---|---|
| `fonts/HarmonyOS_Sans_*.ttf` | HarmonyOS Sans, © Huawei Device Co., Ltd. | HarmonyOS Sans Fonts License (redistribution in unmodified form with software allowed, not standalone) | [`licenses/HarmonyOS-Sans-LICENSE.txt`](licenses/HarmonyOS-Sans-LICENSE.txt) |
| `fonts/NotoSansKR-VF.ttf` | Noto Sans KR, © Google / Adobe | SIL Open Font License 1.1 | [`licenses/NotoSansKR-OFL.txt`](licenses/NotoSansKR-OFL.txt) |
| `pawnio_modules/*.bin` | [PawnIO.Modules](https://github.com/namazso/PawnIO.Modules) (IntelMSR, LpcIO, SmbusI801), signed builds by namazso; source at that link | LGPL-2.1 | [`licenses/PawnIO-Modules-LGPL-2.1.txt`](licenses/PawnIO-Modules-LGPL-2.1.txt) |

**This software uses HarmonyOS Sans Fonts.**

The PawnIO modules run on the [PawnIO](https://pawnio.eu) driver, which is not bundled; install it
separately (it also comes with FanControl) for CPU/RAM temperatures and board fan speeds.

## References

Not copied code, but the approach or interface details follow these projects:

- [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) (MPL-2.0): NvAPI
  thermal sensor interface IDs and struct layout; Intel MSR, Nuvoton Super I/O and DDR5 SPD5118
  sensor details.
- [RAMSPDToolkit](https://github.com/Blacktempel/RAMSPDToolkit) (MPL-2.0): DDR5 SPD hub temperature.
- [CodexBar](https://github.com/steipete/CodexBar) (MIT): how the Claude and Codex plan usage endpoints are queried.
- [Claude-Code-Usage-Monitor](https://github.com/Maciek-roboblog/Claude-Code-Usage-Monitor) (MIT): Antigravity token lookup.

Rust crate dependencies keep their own licenses (see `rust/Cargo.lock`).

## Trademarks

Claude, Codex, Antigravity, TURZX and other names are trademarks of their owners. This project is
not affiliated with or endorsed by any of them.
