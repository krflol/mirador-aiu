# mirador-aiu

Claude Code and Codex account usage inside [Mirador](https://github.com/jchultarsky/mirador). This standalone Rust plugin implements Mirador's external-panel protocol v1 and reads the normalized JSON output of [AIU Rust](https://github.com/krflol/aiu-rs).

The panel shows saved accounts, active logins, recommendations, usage bars, quota locks, reset times, and cached-data status. Select an account to inspect its windows, refresh the display, or confirm a switch to another saved login. Credentials and provider requests stay in AIU.

## Requirements

- Mirador 1.6.0 or a later host advertising external-panel protocol v1.
- AIU Rust 0.2.0 or newer, installed separately and configured with your accounts. The original Go AIU JSON format is not supported.
- Windows, Linux, or macOS. The plugin itself needs no Python, GUI toolkit, or network access. AIU's desktop dependencies apply separately if you use a desktop-enabled AIU build.

## Install

Download the archive for your operating system from [Releases](https://github.com/krflol/mirador-aiu/releases/latest), verify it against `SHA256SUMS`, and extract it. Put `mirador-aiu` (`mirador-aiu.exe` on Windows) on `PATH`, or use its absolute path in the configuration below.

The release targets are Windows x64 MSVC, Linux x64 GNU, macOS Apple Silicon, and macOS Intel. Linux archives are built on Ubuntu 22.04 and require a compatible glibc. macOS binaries are not notarized.

Alternatively, with Rust 1.88 or newer:

```sh
cargo install --git https://github.com/krflol/mirador-aiu --locked
```

First verify AIU independently:

```sh
aiu --version
aiu --json status
```

Use `aiu add`, `aiu login`, or `aiu gui` to set up accounts. The plugin does not add, delete, or log into accounts.

## Add the panel to Mirador

Find your configuration with `mirador --config-path`. Merge this declaration into it, then add `aiu-usage` to an existing layout row or enable it through Mirador's `w` picker:

```toml
[[plugins]]
id = "aiu-usage"
command = ["mirador-aiu"]

[plugins.config]
aiu_command = ["aiu"]
poll_seconds = 60
allow_switch = true
show_email = false

# Example placement; merge with your existing layout rather than replacing it.
[layout]
rows = [
  { height = 100, panels = [{ widget = "aiu-usage", width = 100 }] },
]
```

For Windows paths containing spaces, TOML literal strings make the argv clear:

```toml
[[plugins]]
id = "aiu-usage"
command = ['C:\Tools\mirador-aiu\mirador-aiu.exe']

[plugins.config]
aiu_command = ['C:\Tools\aiu\aiu.exe']
```

Commands are argument arrays, not shell strings. Do not add shell quoting inside an argument. The plugin inherits Mirador's working directory and environment, including AIU's supported configuration-path settings. Do not put credentials in Mirador's TOML or command arguments.

Declaring a plugin does not start it; it must also be placed in the layout or enabled in the picker.

### Try it without accounts

The included demo uses three synthetic accounts. It never launches AIU, reads credentials, or allows account changes:

```sh
mirador --config examples/demo.toml
```

The executable must be on `PATH`, or replace the command in that example with the absolute path to your build. `mirador-aiu --demo` is still a protocol process: it expects Mirador to supply the hello message and render its output.

## Controls

| Key | Action |
| --- | --- |
| Up / Down, k / j | Select an account |
| PageUp / PageDown | Move through account pages |
| Home / End | Select the first / last account |
| r | Refresh through AIU; its request spacing and cooldowns still apply |
| Enter | Ask to switch to the selected eligible account |
| y | Confirm the displayed switch |
| n / Esc | Cancel the switch |

Switching requires a sufficiently large panel, a non-active account, and an AIU login state of `ok` or `expiring`. Read-only, expired, missing, and unknown logins cannot be switched here. Set `allow_switch = false` for an observation-only panel. After a successful switch, start a new CLI session to use the selected login. A completed switch produces one Mirador Watch Log event without account identifiers.

The panel does not capture arbitrary keyboard input or paste. Mirador keeps its navigation bindings and always owns Ctrl+C.

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `aiu_command` | `["aiu"]` | Trusted executable and optional fixed arguments; no shell is invoked |
| `poll_seconds` | `60` | Delay after each completed request, from 30 through 3,600 seconds |
| `slow_after_seconds` | `30` | Show a delayed-request notice after 5 through 300 seconds; this does not terminate AIU |
| `provider` | `"all"` | `"all"`, `"claude"`, or `"codex"`; restricts AIU requests and displayed accounts |
| `allow_switch` | `true` | Enable explicit, confirmed account switching |
| `show_email` | `false` | Include account email addresses in the panel |
| `demo` | `false` | Use built-in synthetic accounts and disable AIU execution |

Unknown setting names are rejected so configuration mistakes are visible. AIU account labels remain visible; the email setting does not conceal an email that you deliberately use as a label.

## Usage and credential ownership

AIU remains the authority for usage normalization, recommendation ranking, storage, token refresh, and account switching. The plugin uses `aiu --json status` and, only after confirmation, `aiu --json switch -- <account-key>`. It does not read AIU's token files, hold provider credentials, or implement another refresh mechanism.

AIU's active-CLI ownership policy continues to apply: automatic monitoring adopts an active CLI's newer credentials and leaves their rotation to that CLI. Independent saved logins can be refreshed by AIU. Explicit switching may update CLI credentials; avoid concurrent CLI login/logout or account switching during that action.

Cached data remains visible if a request fails. The panel suppresses recommendation badges when its read failed, AIU marks the data stale, or the usage is older than 15 minutes. Unknown percentages and explicit locks remain visibly distinct. It does not rerank accounts or infer available capacity from a missing percentage.

### Shutdown and slow operations

An AIU status request can rotate an independent login's credentials. Terminating it midway could interrupt saving the replacement token. Each request therefore runs through a short-lived, separate worker process that owns AIU until it finishes.

Closing or removing the Mirador panel exits promptly and lets an already-started worker finish and reap its AIU child. The worker discards excess output, tolerates its result pipe being closed, and exits after the command. It launches no further requests. A worker-held lock prevents another matching request from overlapping after a panel restart.

The slow-request setting is advisory. A slow command retains the previous snapshot and disables new operations in that panel; it is not force-killed or automatically replaced. This also means a custom `aiu_command` that never exits can leave its worker running. Use the actual AIU executable or a trusted wrapper with equivalent completion behavior. AIU's own HTTP and lock timeouts remain in effect.

The panel bounds retained subprocess output to 2 MiB, limits account/window counts, filters terminal controls, and publishes bounded complete frames. It never forwards raw AIU stderr or free-form provider error text into Mirador. Diagnose detailed failures by running AIU directly.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --release --locked
```

Tests use synthetic data and fake native executables. They do not need real accounts or a running Mirador installation. CI runs on Windows, Linux, and macOS; release builds use Rust 1.88 for all four target architectures. Protocol traffic owns stdout; diagnostics use stderr.

The canonical interface is [Mirador external-panel protocol v1](https://github.com/jchultarsky/mirador/blob/v1.6.0/docs/plugin-protocol.md). The [Mirador sample plugins and optional SDK](https://github.com/krflol/mirador-plugin) provided the integration reference; this implementation is Rust and has no SDK runtime dependency.

## License

MIT. AIU and Mirador are separate projects with their own licenses and release lifecycles.

