```
███████╗██╗    ██╗██╗██████╗ ███████╗
╚══███╔╝██║    ██║██║██╔══██╗██╔════╝
  ███╔╝ ██║ █╗ ██║██║██████╔╝█████╗
 ███╔╝  ██║███╗██║██║██╔══██╗██╔══╝
███████╗╚███╔███╔╝██║██║  ██║███████╗
╚══════╝ ╚══╝╚══╝ ╚═╝╚═╝  ╚═╝╚══════╝
```

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![zshrs plugin](https://img.shields.io/badge/zshrs-native%20plugin-blue.svg)](https://github.com/MenkeTechnologies/zshrs)

### `[THE BROWSER AS A SHELL BUILTIN — COMPILED]`

> *"`zwire page tables` — the table on the page you are logged into, on stdout."*

## `[NATIVE ZSHRS PLUGIN]`

A native [zshrs](https://github.com/MenkeTechnologies/zshrs) plugin that drives
the [zwire](https://github.com/MenkeTechnologies/zwire) browser: tabs, windows,
tiling, navigation, colour scheme, browsing data — and reads back what the
browser is *showing*, from the live DOM, after the login and after the
JavaScript.

It talks to [`zwire-host`](https://github.com/MenkeTechnologies/zwire-host) over
its local socket in Rust — no `curl`, no remote-debugging port, no client
library, no subprocess on the common path.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) &middot; [`znative`](https://github.com/MenkeTechnologies/zshrs/blob/main/docs/ZNATIVE.md) &middot; [`zwire`](https://github.com/MenkeTechnologies/zwire) &middot; [`zwire-host`](https://github.com/MenkeTechnologies/zwire-host)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] Usage](#0x02-usage)
- [\[0x03\] Reading the live page](#0x03-reading-the-live-page)
- [\[0x04\] How it works](#0x04-how-it-works)
- [\[0x05\] Environment](#0x05-environment)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

```text
zwire                       → host, scheme, whether a browser is attached
zwire scheme matrix         → recolour the browser, the HUD and every themed app
zwire open URL…             → open each URL in a new tab
zwire tab next              → tab / window / tiling / navigation verbs
zwire page tables           → the rendered page's tables, as JSON on stdout
zwire call VERB k=v …       → any verb on the published automation surface
```

---

## [0x01] INSTALL

```sh
znative load MenkeTechnologies/zshrs-zwire
```

Put that one line in your `.zshrc`.
[znative](https://github.com/MenkeTechnologies/zshrs/blob/main/docs/ZNATIVE.md),
zshrs's package manager, installs the plugin on the first shell start — clones
it, runs `cargo build --release`, and `zmodload -R`s the resulting `libzwire` —
then loads it from the store, zero-network, on every start after. Load it after
`compinit` so the native completion wires up.

### Manual build

```sh
cargo build --release
zmodload -R ./target/release/libzwire.dylib   # .so on Linux
zwire status
```

---

## [0x02] USAGE

| Command | Effect |
|---|---|
| `zwire` / `zwire status` | host + version, current scheme, whether a browser is attached, which endpoint |
| `zwire scheme [NAME]` | print the colour scheme, or set it (`cyberpunk` `midnight` `matrix` `ember` `arctic` `crimson` `toxic` `vapor`) |
| `zwire schemes` | list the built-in schemes |
| `zwire light` / `dark` / `toggle` | the light/dark variant of the current scheme |
| `zwire open URL…` | open each URL in a new tab |
| `zwire goto URL` | navigate the active tab |
| `zwire tab SUB` | `new close others right left dups reopen dup pin unpin mute unmute muteothers discard next prev first last goto N moveleft moveright movefirst movelast detach sort muteall unmuteall pinall unpinall group ungroup collapse expand` |
| `zwire win SUB` | `new incognito close min max full restore next prev merge center display` |
| `zwire snap DIR` | `left right top bottom topleft topright bottomleft bottomright` |
| `zwire nav SUB` | `back forward reload hard reloadall home` |
| `zwire back` / `forward` / `reload [hard]` | the three worth a short name |
| `zwire zoom in\|out\|reset` | zoom the active tab |
| `zwire clear WHAT` | `cache cookies both all passwords history downloads` |
| `zwire bookmark` | bookmark the active tab |
| `zwire download URL` | download through the browser |
| `zwire screenshot` | capture the visible tab |
| `zwire notify TITLE [BODY]` | desktop notification from the browser |
| `zwire clip [TEXT]` | read / write the machine's clipboard |
| `zwire page STATE` | read the live page (below) |
| `zwire verbs [PATTERN]` | the published automation surface, with each verb's reversibility class |
| `zwire call VERB [k=v…]` | call any verb; values parse as JSON when they can, else stay strings |
| `zwire raw JSON` | send one raw host command |

A subcommand with no argument prints what it accepts, and `zwire help` prints
everything. Completion is native: `zwire <TAB>` offers the subcommands, `zwire
scheme <TAB>` the schemes, `zwire tab <TAB>` the tab verbs — and `zwire call
<TAB>` asks the *running* host for its surface, so a verb added to zwire-host
completes the day it ships.

---

## [0x03] READING THE LIVE PAGE

```sh
zwire page url          # address of the rendered page
zwire page title
zwire page text         # rendered body text
zwire page links        # [{text, href}] — resolved
zwire page headings     # the outline: level + text
zwire page tables       # every table as rows of cells
zwire page forms        # a form's SHAPE: action, method, field names + types
zwire page meta         # description, canonical, og:*, JSON-LD
zwire page selection    # what is selected right now
zwire page extract 'h2 a' href     # any selector, optionally one attribute
```

Structured results print as indented JSON, so `zwire page tables | jq '.[0]'`
works. These are reads of the DOM in the session you are actually in — after the
login, after the JavaScript. Values typed into form fields are deliberately not
published; `page forms` gives a form's shape and never its contents.

---

## [0x04] HOW IT WORKS

`zwire-host` exposes the browser over local newline-delimited-JSON sockets. This
plugin connects, writes one line, reads one line, and prints the result — inside
the shell process, with no subprocess on the common path.

Which socket it takes, in order:

1. **`zwire-page`** — bound by the host process the browser's service worker is
   attached to. `browser.*` verbs are delivered by publishing them on a
   *process-global* topic, so this is the only endpoint from which `zwire tab
   new` actually reaches the browser rather than queueing where nobody listens.
2. **`zwire-host.sock`** — the NDJSON daemon, when `zwire-host serve` is running.
3. **`zwire.sock`** — the command bus (`App::open("zwire")`), always up with the
   browser but not attached to it: good for state, not for actions.
4. **`zwire-host call`** — the bundled client, which is what covers Windows'
   named pipe with no second implementation here.

A host command translates to the bus's `{"t":"call"}` frame automatically, so
one message shape reaches every endpoint. Setting the scheme needs a `zwire-host`
carrying the `theme` command; against an older host over the NDJSON daemon the
plugin falls back to the historical commandless write.

---

## [0x05] ENVIRONMENT

| Variable | Effect |
|---|---|
| `ZWIRE_HOST_SOCK` | override the NDJSON daemon socket path |
| `ZWIRE_TIMEOUT_MS` | reply wait, default `8000` (a page read is a round trip to the renderer) |
| `XDG_RUNTIME_DIR` / `TMPDIR` | where the socket directory is looked for, matching zwire-host |

---

## [0xFF] LICENSE

MIT. See [LICENSE](LICENSE).
