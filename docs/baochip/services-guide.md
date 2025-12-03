# Baochip Services Guide (Dabao/Baosec)

- **What’s here:** Which services are included by default on dabao/baosec, how to add more, and quick examples of how to talk to the common ones.
- **Default bundles:**
  - **dabao:** `xous-ticktimer`, `keystore`, `xous-log`, `xous-names`, `usb-bao1x`, `bao1x-hal-service` (plus the sample `dabao-console` app).
  - **baosec:** `xous-ticktimer`, `xous-log`, `xous-names`, `usb-bao1x`, `bao1x-hal-service`, `bao-console`, `modals`, `pddb`, `bao-video` (and `vault2` in swap space). Swap is enabled on baosec.
- **Add more services:** add the crate to the workspace, then pass `--service <crate-name>` to `cargo xtask dabao` or `cargo xtask baosec`, or edit the service lists in `xtask/src/main.rs` if you need it always present.
- **More background on services:** see [services/README.md](../../services/README.md) for the deeper microkernel/services overview.

## Table of Contents
- [Default Service Sets](#default-service-sets)
- [How to Add a Service](#how-to-add-a-service)
- [Service Notes and Examples](#service-notes-and-examples)
  - [Logging: xous-log / log-server](#logging-xous-log--log-server)
  - [Timing: xous-ticktimer](#timing-xous-ticktimer)
  - [Directory: xous-names](#directory-xous-names)
  - [Hardware: bao1x-hal-service](#hardware-bao1x-hal-service)
  - [USB: usb-bao1x](#usb-usb-bao1x)
  - [Keys: keystore](#keys-keystore)
  - [Baosec-only extras: bao-console, modals, pddb, bao-video, vault2](#baosec-only-extras-bao-console-modals-pddb-bao-video-vault2)
- [What’s in `libs/` vs `services/`](#whats-in-libsvs-services)

## Default Service Sets
- **dabao:** `xous-ticktimer`, `keystore`, `xous-log`, `xous-names`, `usb-bao1x`, `bao1x-hal-service` (with `dabao-console` as the starter app).
- **baosec:** `xous-ticktimer`, `xous-log`, `xous-names`, `usb-bao1x`, `bao1x-hal-service`, `bao-console`, `modals`, `pddb`, `bao-video`, and `vault2` in swap.

## How to Add a Service
1. Make sure the service crate is in the workspace (listed in the root `Cargo.toml`).
2. Build with `cargo xtask dabao --service <crate>` or `cargo xtask baosec --service <crate>`.
3. Flash the new `apps.uf2` (or the full set) to the board.
4. For a permanent addition, edit the service arrays in `xtask/src/main.rs`.

## Service Notes and Examples

### Logging: xous-log / log-server
```rust
log_server::init_wait().unwrap(); // connect to log service
log::info!("hello from my app");
```

### Timing: xous-ticktimer
```rust
let tt = ticktimer::Ticktimer::new().unwrap();
tt.sleep_ms(500).unwrap();
let now = tt.elapsed_ms()?;
```

### Directory: xous-names
- It's the phonebook that maps names to service IDs so messages reach the right service.
- Example (service without a fixed SID): resolve the `modals` server through `xous-names` and show a notification.
```rust
let xns = xous_names::XousNames::new().unwrap();           // connect to the phonebook
let mut modals = modals::Modals::new(&xns).unwrap();       // resolved via xous-names
modals.show_notification("Hello from my app", None).unwrap(); // talk to the modals service
```

### Hardware: bao1x-hal-service
```rust
let hal = bao1x_hal_service::Hal::new();
hal.set_preemption(true); // allow preemption in this environment
// Example hardware call (pick what you need; see HAL docs for details):
// hal.gpio_set_output(pin_number, level)?;
```
- Use this service instead of poking hardware registers. It owns on-chip blocks (timers, clocks, USB, watchdog) and off-chip devices via pins/buses.
- Hardware registers are tiny numbered slots; writing/reading them controls hardware. The HAL owns those slots so apps don’t touch raw addresses. You ask “do X” and it picks the right register/bits.
- This keeps the chip safe (no two apps fighting over pins) and makes hardware use approachable if you aren’t a hardware person.
- Example uses: set a GPIO pin high/low, read a button, start/stop I2C/SPI, kick the watchdog, configure clocks.

### USB: usb-bao1x
```rust
#[cfg(feature = "usb")]
{
    let usb = usb_bao1x::UsbHid::new();
    usb.serial_console_input_injection(); // feed serial input into the console
}
```

### Keys: keystore
- Manages device keys. Typical apps don’t talk to it directly; higher-level clients handle that. If you need it, use the keystore client APIs in the repo rather than crafting messages by hand.

### Baosec-only extras: bao-console, modals, pddb, bao-video, vault2
- **bao-console:** serial debug console.
- **modals:** simple UI dialogs.
- **pddb:** plausibly deniable database (filesystem alternative).
- **bao-video:** camera + display pipeline.
- **vault2:** secure storage (lives in swap).
