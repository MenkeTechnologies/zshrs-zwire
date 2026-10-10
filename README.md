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

### `[THE WHOLE AUTOMATION BUS AS A SHELL BUILTIN — COMPILED]`

> *"`zwire page tables` — the table on the page you are logged into, on stdout."*

## `[NATIVE ZSHRS PLUGIN]`

A native [zshrs](https://github.com/MenkeTechnologies/zshrs) plugin that puts the
**entire zgui automation bus** on the command line: the
[zwire](https://github.com/MenkeTechnologies/zwire) browser (tabs, windows,
tiling, downloads, bookmarks, history, extensions, power, colour scheme), the
**live page** as typed state, the machine under it (filesystem, exec, jobs,
processes, PTY, key/value store, system stats), the **real tmux server**,
lifecycle **hooks**, **transactions** that unwind themselves, the **pub/sub bus**
and peer mesh — and the **other suite apps** on the bus, called by verb.

It speaks [`zwire-host`](https://github.com/MenkeTechnologies/zwire-host)'s
protocol in Rust over its local socket — no `curl`, no remote-debugging port, no
client library, no subprocess on the common path.

### [`zshrs`](https://github.com/MenkeTechnologies/zshrs) &middot; [`znative`](https://github.com/MenkeTechnologies/zshrs/blob/main/docs/ZNATIVE.md) &middot; [`zwire`](https://github.com/MenkeTechnologies/zwire) &middot; [`zwire-host`](https://github.com/MenkeTechnologies/zwire-host)

---

## Table of Contents

- [\[0x00\] Overview](#0x00-overview)
- [\[0x01\] Install](#0x01-install)
- [\[0x02\] The surface](#0x02-the-surface)
- [\[0x03\] Arguments](#0x03-arguments)
- [\[0x04\] Reading the live page](#0x04-reading-the-live-page)
- [\[0x05\] Transactions](#0x05-transactions)
- [\[0x06\] Streams](#0x06-streams)
- [\[0x07\] Audit](#0x07-audit)
- [\[0x08\] How it works](#0x08-how-it-works)
- [\[0x09\] Environment](#0x09-environment)
- [\[0xFF\] License](#0xff-license)

---

## [0x00] OVERVIEW

```text
zwire                          → host, scheme, whether a browser is attached
zwire scheme matrix            → recolour the browser, HUD and every themed app
zwire tab new                  → tab verbs; win, snap, nav, zoom, clear, dl, bm, hist, ext, power
zwire page tables              → the rendered page's tables, as JSON on stdout
zwire fs grep ~/src TODO       → the machine: fs, exec, jobs, ps, kv, sysinfo, pty
zwire tmux send 'make' %3      → the real tmux server, over tmux's own protocol
zwire suite call zcite item.add title=X   → another app on the bus, by verb
zwire txn begin                → a chain of browser calls that unwinds itself
zwire audit                    → every published verb, and whether it is named here
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

## [0x02] THE SURFACE

`zwire help` lists everything; `zwire help GROUP` documents one group, with each
command's arguments. A group with no command prints its commands.

| Group | Commands |
|---|---|
| *(top level)* | `status` `scheme` `schemes` `light` `dark` `toggle` `open` `goto` `back` `forward` `reload` `clip` `exec` `ps` `kill` `which` `screenshot` `notify` `lang` `verbs` `call` `raw` `audit` `help` |
| `tab` | `new close others right left dups reopen dup pin unpin mute unmute muteothers discard next prev first last goto activate moveleft moveright movefirst movelast detach sort muteall unmuteall pinall unpinall group ungroup collapse expand` |
| `win` | `new incognito close min max full restore next prev merge center display` |
| `snap` | `left right top bottom topleft topright bottomleft bottomright` |
| `nav` · `zoom` | `back forward reload hard reloadall home` · `in out reset` |
| `clear` | `cache cookies both all passwords` |
| `dl` | `get pause resume cancel open show retry clear list` |
| `bm` · `read` · `hist` | `add folder remove` · `add remove` · `clear del add` |
| `ext` · `power` | `enable disable uninstall options launch` · `awake display sleep` |
| `page` | `url title text links headings tables forms meta selection extract assert witness batch states` |
| `txn` | `begin commit abort undo` |
| `fs` | `read write append list walk stat mkdir rm ls subdirs info size disk xattrs git hash grep dups compare diff zip unzip b64 head headbytes newdir newfile copy dup rename del trash shred touch chmod relink home watch tail` |
| `job` | `start list result poll` |
| `host` · `sys` | `info hello log theme open notify ping` · `once stream stop meter` |
| `kv` | `get set merge del keys` |
| `hooks` | `list events save delete enable script path setscript test fire` |
| `stryke` | `run lsp send stop` |
| `suite` | `list verbs call get` |
| `bus` · `peer` · `watch` | `pub sub unsub` · `list connect remote` · `list stop` |
| `pty` | `run spawn write resize kill` |
| `tmux` | `status tree sessions panes search capture focus send run command sync broadcast options setopt keys setkey unbindkey buffers buffer setbuffer delbuffer paste export import snaps snap save restore rename rmsnap hud` |

Completion is native: `zwire <TAB>` offers commands and groups, `zwire fs <TAB>`
the fs commands, `zwire scheme <TAB>` the schemes, `zwire page assert <TAB>` the
projections then the predicates. `zwire call <TAB>` and `zwire suite call <TAB>`
ask the **running host** for its surface and for the apps currently up, so
neither can go stale.

---

## [0x03] ARGUMENTS

Each command binds its arguments positionally, then takes `k=v` for anything
else the host accepts:

```sh
zwire fs walk ~/src ext=rs depth=3          # positional path, then extra fields
zwire fs grep ~/src TODO case_insensitive=true max_results=20
zwire kill 4021 kill                        # pid, signal
zwire exec git status --porcelain           # trailing words become argv
zwire suite call zcite item.add title=X year=1999   # k=v go to the CALLED app
```

Values parse as JSON when they can (`3` is a number, `true` a bool, `[1,2]` an
array) and stay strings otherwise, so a URL survives intact. Positionals are
taken from the front *before* anything is read as `k=v`, so `zwire fs write x
'a=b'` writes `a=b` as content.

Replies print as indented JSON, minus the protocol envelope; a reply carrying one
field prints just that field (`zwire which git` → the path), so output pipes
straight into `jq` or a `$(…)`.

---

## [0x04] READING THE LIVE PAGE

```sh
zwire page url          # address of the rendered page
zwire page title
zwire page text         # rendered body text
zwire page links        # [{text, href}] — resolved
zwire page headings     # the outline: level + text
zwire page tables       # every table as rows of cells
zwire page forms        # a form's SHAPE: action, method, field names + types
zwire page meta         # description, canonical, og:*, JSON-LD
zwire page selection
zwire page extract 'h2 a' href           # any selector, optionally one attribute
zwire page assert page.title contains 'Order confirmed'   # exit 1 when it does not
```

These are reads of the DOM in the session you are actually in — after the login,
after the JavaScript. Values typed into form fields are deliberately not
published; `page forms` gives a form's shape and never its contents.

---

## [0x05] TRANSACTIONS

The bus journals reversible calls and can unwind them. From the shell:

```sh
txn=$(zwire txn begin)                   # → the transaction id
zwire call browser.openTab url=https://example.com txn=$txn
zwire page witness page.title contains Example txn=$txn   # a premise of the chain
zwire txn commit $txn                    # premises re-checked; a violation aborts instead
zwire txn abort  $txn                    # compensate every step, in reverse
```

An irreversible verb is refused while a transaction is open rather than stranding
a half-done chain — `zwire verbs` prints each verb's class.

---

## [0x06] STREAMS

`fs watch`, `fs tail`, `sys stream`, `sys meter`, `bus sub` and `pty spawn`
answer with a stream of frames, printed one compact JSON object per line:

```sh
zwire fs tail ~/.cache/zshrs/zshrs.log secs=60
zwire sys stream interval_ms=1000 secs=10
zwire bus sub scheme secs=120
```

`secs=` bounds the wait (default 30, or `$ZWIRE_STREAM_SECS`) — a builtin runs in
the shell's own process, so an unbounded read would hand the terminal to a loop.

Streams work over **every** endpoint, including the bus: a bus connection owns a
real session (zwire-host 0.3.21+), so what it emits unasked arrives as
`{"t":"event"}` frames and is unwrapped here. PTY output is decoded and printed
as the bytes it is, not as base64.

A terminal belongs to the connection that spawned it, so typing into one needs a
single connection — which is what `pty run` is:

```sh
zwire pty run 'make -j8' secs=120      # spawn, type it, stream the output
```

---

## [0x07] AUDIT

```sh
zwire audit     # → "<n> verbs published, all reachable by name"
```

`audit` asks the running host for its surface and prints anything not reachable
by a named command — and any command here whose verb the host no longer
publishes. The claim that this plugin covers the bus is therefore checkable
against the host that is running, not against a second copy of its list kept
here. It exits non-zero when either side has drifted.

---

## [0x08] HOW IT WORKS

`zwire-host` exposes the browser and the machine over local
newline-delimited-JSON sockets. This plugin connects, writes one line, reads one
line, and prints the result — inside the shell process, with no subprocess on the
common path.

Which socket it takes, in order:

1. **`zwire-page`** — bound by the host process the browser's service worker is
   attached to. `browser.*` verbs are delivered by publishing them on a
   *process-global* topic, so this is the only endpoint from which `zwire tab
   new` actually reaches the browser rather than queueing where nobody listens.
2. **`zwire-host.sock`** — the NDJSON daemon, when `zwire-host serve` is running.
3. **`zwire.sock`** — the command bus (`App::open("zwire")`), always up with the
   browser but not attached to it: good for state and for the machine, not for
   browser actions.
4. **`zwire-host call`** — the bundled client, which is what covers Windows'
   named pipe with no second implementation here.

A host command translates to the bus's `{"t":"call"}` frame automatically, so one
message shape reaches every endpoint. Setting the scheme needs a `zwire-host`
carrying the `theme` command (0.3.19+); against an older host over the NDJSON
daemon the plugin falls back to the historical commandless write.

---

## [0x09] ENVIRONMENT

| Variable | Effect |
|---|---|
| `ZWIRE_HOST_SOCK` | override the NDJSON daemon socket path |
| `ZWIRE_TIMEOUT_MS` | reply wait, default `8000` (a page read is a round trip to the renderer) |
| `ZWIRE_STREAM_SECS` | default duration of a streaming command, default `30` |
| `XDG_RUNTIME_DIR` / `TMPDIR` | where the socket directory is looked for, matching zwire-host |

---

## [0xFF] LICENSE

MIT. See [LICENSE](LICENSE).
