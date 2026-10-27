# Hardware

Developed on a [**Waveshare ESP32-S3-LCD-1.47**](https://www.waveshare.com/product/esp32-related/boards-kits/esp32-s3-lcd-1.47.htm):

| | |
|---|---|
| Chip | ESP32-S3, dual-core Xtensa LX7 at 240 MHz |
| RAM | 512 KB internal SRAM, plus 8 MB octal PSRAM |
| Flash | 16 MB |
| Display | 1.47" ST7789 LCD, 172 × 320 |
| Storage | microSD slot (4-bit SDMMC), optional |
| Network | 2.4 GHz Wi-Fi |

![The back of the board, with the microSD card that holds the world](../assets/esp32-waveshare.jpeg)

*The reverse of a Waveshare ESP32-S3-LCD-1.47 with 128GB SD card. You really only need a 16MB one.*

## Other boards

<p align="center">
  <img src="../assets/server-101.jpg" width="48%" alt="The server on an LCDwiki ES3N28P, showing its device page: load on each CPU core, memory, Wi-Fi signal, SD card space and chip temperature">
  <img src="../assets/server-102.jpg" width="48%" alt="The Join page on the ES3N28P, running as its own hotspot: a QR code for the network MTG-6501, its password, and the address to enter in Luanti">
</p>

*The same server on an LCDwiki ES3N28P, here running as its own hotspot. At about 9 cm by 5 cm it is slightly larger and more expensive than the Waveshare, but it runs cooler and its 2.8" screen is easier to read.*

The firmware can also be built for LCDwiki's [**ES3C28P / ES3N28P**](https://www.lcdwiki.com/2.8inch_ESP32-S3_Display) (2.8" screen, with and without touch), with `BOARD=es3c28p` when building:

| | Waveshare ESP32-S3-LCD-1.47 (default) | ES3C28P / ES3N28P (`BOARD=es3c28p`) |
|---|---|---|
| Chip, memory | ESP32-S3 N16R8 | ESP32-S3 N16R8 |
| Display | 1.47" ST7789, 172 × 320 | 2.8" ILI9341V, 240 × 320 (touch on the C, unused) |
| Screen pins | SCLK 40, MOSI 45, CS 42, DC 41, RST 39, backlight 48 | SCLK 12, MOSI 11, CS 10, DC 46, reset with the chip, backlight 45 |
| microSD (4-bit) | CMD 15, CLK 14, D0 16, D1 18, D2 17, D3 21 | CMD 40, CLK 38, D0 39, D1 41, D2 48, D3 47 |
| RGB LED, button | GPIO 38, BOOT (GPIO 0) | GPIO 42, BOOT (GPIO 0) |
| Other | | speaker amplifier (kept off, GPIO 1), battery connector |

On the ES3C28P the status pages are drawn as on the Waveshare, 320 × 172, in the middle of the larger screen, for now. If its colours come out with red and blue swapped, or inverted, or the pages mirrored, flip `lcd_bgr`, `lcd_invert`, `lcd_mirror_x` or `lcd_mirror_y` for it in `src/board.rs` (both mirrors for pages upside down).

Flashing this board is most reliable with the flash erased first:
```
espflash erase-flash
BOARD=es3c28p WIFI_SSID='your network' WIFI_PASS='your password' cargo run --release
```
That wipes a world kept in the board's flash (one on an SD card is untouched). If the monitor still shows "waiting for download" after flashing, the board has restarted into its download mode rather than the new firmware: press its RESET button, and it starts normally.

### LilyGO T-Display-S3

LilyGO's [**T-Display-S3**](https://lilygo.cc/products/t-display-s3) (1.9" screen) is supported with `BOARD=tdisplays3`, or from [the web installer](building.md#releases-and-the-web-installer).

| | LilyGO T-Display-S3 (`BOARD=tdisplays3`) |
|---|---|
| Chip, memory | ESP32-S3R8: 16 MB flash, 8 MB octal PSRAM |
| Display | 1.9" ST7789V, 170 × 320, on an 8-bit parallel (Intel 8080) bus |
| Screen pins | D0 to D7 on 39, 40, 41, 42, 45, 46, 47, 48; WR 8, RD 9 (held high), DC 7, CS 6, RST 5, backlight 38 |
| Power | GPIO 15 held high (powers the screen on battery) |
| microSD | none: the game is kept in the board's flash (11 MB) |
| RGB LED, button | none, BOOT (GPIO 0) |

The status pages are 172 rows tall and the panel shows 170, so their top and bottom rows of pixels are cut off. With no SD card, the world lives in flash: `espflash erase-flash`, or erasing in the web installer, deletes it, and [the USB drive](setup.md#as-a-usb-drive) is the way to back it up. If the pages come out upside down or mirrored, flip `lcd_mirror_x` and `lcd_mirror_y` for it in `src/board.rs`.

### Others

Other ESP32-S3 boards with PSRAM and 16 MB of flash should work with a new entry in `src/board.rs` and their SD card and screen pins in `src/main.rs`; an SD card slot is optional. PSRAM is required: the server keeps its own data there and leaves the internal RAM to Wi-Fi and the system.
