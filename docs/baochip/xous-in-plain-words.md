# Xous in Plain Words (Dabao)

## What Xous Is
- Xous = kernel (traffic cop) + services (helpers) + apps (your code). All three together make the OS.
    - **Kernel** - The kernel is tiny. It keeps programs from crashing into each other and passes messages between them.
    - **Services** are like city departments providing a service to the 'city' of Xous:
        - Each owns one job (time, logging, hardware access).
        - An app files a "request" (message) and the service then does work for your app.
    - **Apps** are what you write.
- It is "microkernel" style: the kernel does almost nothing except schedule and route messages between services and apps. Everything else lives in user space as normal Rust programs.
    - "User space" just means regular programs with no special powers. The services and apps run like normal Rust code, and the kernel keeps them separate so a bug in one cannot smash the others.

## How Xous Starts on Dabao
- The chip powers up and runs built-in boot code (see below). That code can load UF2 files you copy over USB. Day to day you copy `apps.uf2`, but it can also load `loader.uf2` and `xous.uf2` for a full refresh.
    - "Load" here means: boot1 reads the UF2 you dropped on the USB drive, writes it into the board’s flash so it sticks, and on the next boot it runs that code.
- After loading, it jumps into Xous, which starts services and then your apps.

### What the Built-in Boot Code Is (boot0 and boot1)
- `boot0` lives in ROM (hardwired, never changes). It just checks signatures and jumps to boot1.
- `boot1` lives in flash (can be updated). It shows up as the USB drive when you press `PROG`, and it loads your UF2 files (like `loader.uf2`, `xous.uf2`, `apps.uf2`) into place.

## What Gets Created When You Build
#### Running `cargo xtask dabao` produces up to three UF2 files in `target/riscv32imac-unknown-xous-elf/release/`.
- `loader.uf2`: the tiny starter that sets up memory and hands control to Xous. Rarely changed.
- `xous.uf2`: the core OS (kernel plus built-in services).
- **`apps.uf2`**: your app bundle. For most day-to-day updates this is what needs to get copied over to your baochip board.

        For a full refresh (e.g., new loader or OS + services), copy all three to the baochip board.

## How Apps Talk
- Apps and services send messages to each other. Messages are small and fast.
- To find a service, you ask a well-known phonebook called `xous-names`.
- Core helpers you will use first: `log-server` (printing logs), `ticktimer` (sleep/elapsed time), `bao1x-hal-service` (safe hardware access), `xous-names` (lookup).
- As an app author you typically just call these helpers’ Rust APIs; they send the messages under the hood. You only hand-roll message loops if you are creating a brand-new service yourself.

## When Multiple Apps Call the Same Service
- Each service has a mailbox (Xous calls it a "server queue"). Apps drop messages in; the kernel delivers them.
- Each app also has its own return mailbox for replies. The service pulls from its queue one message at a time and replies to the sender’s mailbox.
- Because access goes through a single service, it can keep hardware sane (no two apps toggling the same device at once) and can say “no” if a request isn’t allowed.

## What the bao1x-hal-service (Hardware Abstraction Layer aka HAL) Does
- Hardware here means both on-chip blocks (timers, clocks, USB, watchdog) and off-chip parts you reach through pins/buses (GPIOs, I2C/SPI devices, buttons, display, camera).
- The HAL owns those slots. You ask it to “do X” in Rust; it picks the right register and bits so you never touch raw addresses.
- This keeps the chip safe (no two apps fighting over pins) and makes hardware use approachable if you aren’t a hardware person.
- Example uses: set a GPIO pin high/low, read a button, start or stop I2C/SPI transactions, kick the watchdog, or configure clocks—without ever touching chip registers yourself.
