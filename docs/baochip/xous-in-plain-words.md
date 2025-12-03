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
- How services get found:
  - Some core helpers have fixed SIDs and skip lookup: `log-server` and `ticktimer` connect directly.
  - Everything else goes through the phonebook (`xous-names`). Typical flow: create `let xns = XousNames::new()?;` once and pass `&xns` into helpers (GAM, HAL, modals, etc.)—they look up the service name for you. You only call `xous-names` yourself when you’re creating a new service and need to register its name, or when you’re experimenting with raw message passing instead of using a client crate.

## When Multiple Apps Call the Same Service
- Each service has a mailbox (Xous calls it a "server queue"). Apps drop messages in; the kernel delivers them.
- Each app also has its own return mailbox for replies. The service pulls from its queue one message at a time and replies to the sender's mailbox.
- Because access goes through a single service, it can keep hardware sane (no two apps toggling the same device at once) and can say "no" if a request isn't allowed.

## Want per-service details and sample code? See the [Baochip services guide](./services-guide.md).

## What About Libraries?
- The `libs/` directory holds various helpful shared Rust crates (no `main`) you pull into services or apps as dependencies. They do not run on their own.
- Use them like any Rust library: add the crate to your `Cargo.toml`, `use` its modules, call its APIs.
