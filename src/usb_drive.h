// The USB drive (see usb_drive in main.rs): the mass storage part of
// Espressif's TinyUSB component, added in Cargo.toml, over the SD card or
// the board's flash. Rust bindings are generated from this into
// esp_idf_sys::tinyusb. (Only this header: tinyusb.h pulls in TinyUSB's
// own headers, which bindgen can't find FreeRTOS.h for; the one function
// needed from it is declared in main.rs.)
#include "soc/soc_caps.h" // SOC_SDMMC_HOST_SUPPORTED, for the card functions
#include "tusb_msc_storage.h"
