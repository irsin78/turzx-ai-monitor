# TURZX 8.8" panel protocol

Reverse-engineered from the vendor app and verified on hardware. Implementation:
[`rust/src/panel/lcd.rs`](../rust/src/panel/lcd.rs) and [`rust/src/panel/jpeg.rs`](../rust/src/panel/jpeg.rs).

## Device

- USB `VID 1CBE`, `PID 0088`, product string "TURZX1.0"; WinUSB through an MS OS descriptor, so no
  driver install is needed.
- Bulk OUT endpoint `0x01`, bulk IN endpoint `0x81`.
- Framebuffer 480×1920 (portrait). Lying landscape, the panel shows it rotated 90° counter-clockwise,
  so landscape images are rotated 90° clockwise before sending.
- One program at a time: the vendor app must be closed.

## Commands

Every command starts with a 500-byte header:

| Bytes | Meaning |
|---|---|
| 0 | command |
| 2, 3 | `0x1A`, `0x6D` |
| 4–7 | milliseconds since start, little-endian |
| 8– | payload |

The header is encrypted with DES-CBC (key = IV = `slv3tuzx`, PKCS#7 padding) to 504 bytes and placed
in a 512-byte block with `[510] = 0xA1`, `[511] = 0x1A`.

Every reply is 512 bytes followed by a zero-length packet; byte 0 echoes the command. Stale replies can
sit in the IN pipe after a reconnect, so drain it after opening.

| Command | Use |
|---|---|
| 10 | firmware version (ASCII at reply[8..40]) |
| 101 | show a JPEG: length big-endian at header[8..12], file bytes appended after the block |
| 102 | show a PNG (same layout); must be RGBA, an RGB PNG fills only part of the screen |

Seen in the vendor app but not verified: 12 shutdown, 13 rotate, 14 brightness, 111 stop, 125 config.

## Images

- JPEG needs **4:4:4** chroma: 4:2:0 smears small colored text into specks that look like dead pixels.
  Quality 95 gives ~190 KB per frame.
- The panel takes about 37 full JPEG frames per second; sending one takes ~16 ms of USB time.
- PNG is much slower (~16 fps).

## Partial re-encoding

Only small parts of the screen change between redraws (here: animated sprites), so the encoder sets a
restart interval (DRI) of one MCU row. Each 8-pixel band of the portrait frame is then entropy-coded on
its own with the DC predictors reset, and a frame is assembled from cached band bytes plus the few bands
that changed (detected with a hash of each band's pixels). This cut the per-frame CPU from ~12.5 ms to
~1.3 ms. The panel's decoder handles restart markers fine.
