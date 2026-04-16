# shelly-rpc

A Rust workspace for talking to [Shelly](https://www.shelly.com/) smart devices.

- **[`shelly-rpc`](https://crates.io/crates/shelly-rpc)** — a `no_std`-first
  client library. Define your own transport for embedded use, or opt into
  `std` for a batteries-included HTTP transport.
- **[`shellyctl`](https://crates.io/crates/shellyctl)** — a command-line
  client built on top of `shelly-rpc`.

## Status

Skeleton. APIs are placeholders and will change.

## Workspace layout

```
.
├── Cargo.toml        # workspace manifest
├── shelly-rpc/       # library crate (no_std-first)
└── shellyctl/        # binary crate (std)
```

## Installation

### macOS / Linux (curl)

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/pawelchcki/shelly-rpc/releases/latest/download/shellyctl-installer.sh | sh
```

### Windows (PowerShell)

```powershell
powershell -c "irm https://github.com/pawelchcki/shelly-rpc/releases/latest/download/shellyctl-installer.ps1 | iex"
```

### Homebrew

```sh
brew install pawelchcki/shellyctl/shellyctl
```

### npm

```sh
npm install -g shellyctl
```

### From source

```sh
cargo install --git https://github.com/pawelchcki/shelly-rpc shellyctl
```

### Updating (for curl/PowerShell installs)

```sh
shellyctl self-update
```

Binaries installed via Homebrew or npm should be upgraded through those
package managers instead.

## Building

```sh
# default (no_std) build of the library
cargo check -p shelly-rpc

# with the std transport
cargo check -p shelly-rpc --features std

# build the CLI
cargo build -p shellyctl
```

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Both licenses require that the original copyright notice
and license text be preserved in copies and substantial portions of the
software — i.e. attribution is required.
