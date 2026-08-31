//! **zwire** — drive the [zwire](https://github.com/MenkeTechnologies/zwire)
//! browser from the shell, as a native zshrs plugin.
//!
//! The browser publishes its automation surface through
//! [`zwire-host`](https://github.com/MenkeTechnologies/zwire-host): local
//! endpoints (Unix sockets on macOS/Linux, named pipes on Windows) speaking
//! newline-delimited JSON. They answer `{"cmd":…}` host commands, forward
//! `browser.*` automation verbs to the HUD service worker, and project the live
//! DOM as typed `page.*` state.
//!
//! This plugin speaks that protocol directly — one connect, one line out, one
//! line back — so `zwire tab new`, `zwire scheme matrix` and `zwire page tables`
//! are shell builtins running native code, with no `curl`, no remote-debugging
//! port and no client library.
//!
//! See [`endpoints`] for which socket a request takes and why the order matters.
//!
//! Protocol citations (`h:`) refer to zwire-host's sources: `src/lib.rs`
//! (`default_socket`), `src/session.rs` (`handle_cmd`, `theme_write`),
//! `src/zbus.rs` (`serve_conn`, `run_command`, `surface`), `src/page.rs`
//! (`STATES`, `endpoint::bind`) and `src/store.rs` (`SCHEMES`, `write_ui`).
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::io::{BufRead, BufReader, Write};
use std::os::raw::c_int;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use znative::{declare_plugin, Args, Host};

/* ------------------------------------------------------------------ tables */

/// A subcommand → `browser.*` verb table.
type Table = &'static [(&'static str, &'static str)];

/// The built-in colour schemes. h: `store::SCHEMES`. `custom` / `custom-N` are
/// also accepted by the host (a custom scheme carries its colours in the shared
/// palette), so they complete too.
const SCHEMES: &[&str] = &[
    "cyberpunk",
    "midnight",
    "matrix",
    "ember",
    "arctic",
    "crimson",
    "toxic",
    "vapor",
];

/// `zwire tab <sub>` → the `browser.*` verb it runs.
const TAB_VERBS: &[(&str, &str)] = &[
    ("new", "newTab"),
    ("close", "closeTab"),
    ("others", "closeOthers"),
    ("right", "closeRight"),
    ("left", "closeLeft"),
    ("dups", "closeDuplicates"),
    ("reopen", "reopenTab"),
    ("dup", "duplicateTab"),
    ("pin", "pinTab"),
    ("unpin", "unpinTab"),
    ("mute", "muteTab"),
    ("unmute", "unmuteTab"),
    ("muteothers", "muteOthers"),
    ("discard", "discardTab"),
    ("next", "nextTab"),
    ("prev", "prevTab"),
    ("first", "firstTab"),
    ("last", "lastTab"),
    ("moveleft", "moveTabLeft"),
    ("moveright", "moveTabRight"),
    ("movefirst", "moveTabFirst"),
    ("movelast", "moveTabLast"),
    ("detach", "tabToNewWindow"),
    ("sort", "sortTabs"),
    ("muteall", "muteAll"),
    ("unmuteall", "unmuteAll"),
    ("pinall", "pinAll"),
    ("unpinall", "unpinAll"),
    ("group", "groupTabs"),
    ("ungroup", "ungroupTabs"),
    ("collapse", "collapseGroups"),
    ("expand", "expandGroups"),
];

/// `zwire win <sub>` → the `browser.*` verb it runs.
const WIN_VERBS: &[(&str, &str)] = &[
    ("new", "newWindow"),
    ("incognito", "incognitoWindow"),
    ("close", "closeWindow"),
    ("min", "minimizeWindow"),
    ("max", "maximizeWindow"),
    ("full", "fullscreenWindow"),
    ("restore", "restoreWindow"),
    ("next", "nextWindow"),
    ("prev", "prevWindow"),
    ("merge", "mergeWindows"),
    ("center", "centerWindow"),
    ("display", "moveWindowNextDisplay"),
];

/// `zwire snap <dir>` → the `browser.snap*` verb it runs.
const SNAP_VERBS: &[(&str, &str)] = &[
    ("left", "snapLeft"),
    ("right", "snapRight"),
    ("top", "snapTop"),
    ("bottom", "snapBottom"),
    ("topleft", "snapTopLeft"),
    ("topright", "snapTopRight"),
    ("bottomleft", "snapBottomLeft"),
    ("bottomright", "snapBottomRight"),
];

/// `zwire nav <sub>` → the `browser.*` verb it runs. The four common ones are
/// also top-level subcommands (`zwire back`, `zwire reload`, …).
const NAV_VERBS: &[(&str, &str)] = &[
    ("back", "goBack"),
    ("forward", "goForward"),
    ("reload", "reload"),
    ("hard", "reloadHard"),
    ("reloadall", "reloadAll"),
    ("home", "home"),
];

/// `zwire zoom <sub>` → the `browser.zoom*` verb it runs.
const ZOOM_VERBS: &[(&str, &str)] = &[("in", "zoomIn"), ("out", "zoomOut"), ("reset", "zoomReset")];

/// `zwire clear <what>` → the `browser.clear*` verb it runs.
const CLEAR_VERBS: &[(&str, &str)] = &[
    ("cache", "clearCache"),
    ("cookies", "clearCookies"),
    ("both", "clearCacheAndCookies"),
    ("all", "clearAllData"),
    ("passwords", "clearPasswords"),
    ("history", "clearHistory"),
    ("downloads", "clearDownloads"),
];

/// The live-page projections, without their `page.` prefix. h: `page::STATES`
/// plus the `page.extract` escape hatch.
const PAGE_STATES: &[&str] = &[
    "url",
    "title",
    "text",
    "links",
    "headings",
    "tables",
    "forms",
    "meta",
    "selection",
];

/// Top-level subcommands, for `--help` and for completion.
const SUBCOMMANDS: &[(&str, &str)] = &[
    ("status", "host + browser state (default with no arguments)"),
    ("scheme", "print or set the colour scheme"),
    ("schemes", "list the colour schemes"),
    ("light", "switch the HUD to its light variant"),
    ("dark", "switch the HUD to its dark variant"),
    ("toggle", "flip light/dark"),
    ("open", "open URLs in new tabs"),
    ("goto", "navigate the active tab to a URL"),
    ("tab", "tab operations"),
    ("win", "window operations"),
    ("snap", "snap the window to an edge/corner"),
    ("nav", "navigation (back/forward/reload/home)"),
    ("zoom", "zoom the active tab"),
    ("back", "go back"),
    ("forward", "go forward"),
    ("reload", "reload the active tab"),
    ("clear", "clear browsing data"),
    ("bookmark", "bookmark the active tab"),
    ("download", "download a URL"),
    ("screenshot", "capture the visible tab"),
    ("notify", "raise a desktop notification"),
    ("clip", "read or write the clipboard"),
    ("page", "read the live page as typed state"),
    ("verbs", "list the automation surface"),
    ("call", "call any verb: zwire call VERB k=v …"),
    ("raw", "send one raw host command (JSON)"),
    ("help", "this text"),
];

/* --------------------------------------------------------------- transport */

/// Where zwire-host listens. h: `zwire_host::default_socket` — `$ZWIRE_HOST_SOCK`
/// wins everywhere; otherwise a *runtime* dir, never the persistent state dir.
fn socket_path(host: &Host) -> PathBuf {
    if let Some(p) = cfg(host, "ZWIRE_HOST_SOCK").filter(|s| !s.is_empty()) {
        return PathBuf::from(p);
    }
    if let Some(rt) = cfg(host, "XDG_RUNTIME_DIR").filter(|s| !s.is_empty()) {
        return PathBuf::from(rt).join("zwire-host.sock");
    }
    if let Some(tmp) = cfg(host, "TMPDIR").filter(|s| !s.is_empty()) {
        return PathBuf::from(tmp).join("zwire-host.sock");
    }
    let user = cfg(host, "USER").unwrap_or_else(|| "user".into());
    let safe: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    let user = if safe.is_empty() { "user" } else { &safe };
    PathBuf::from("/tmp").join(format!("zwire-host-{user}.sock"))
}

/// A shell parameter, falling back to the process environment — the plugin runs
/// in-process, so an exported variable is visible either way, but a *shell*
/// variable (`TMPDIR=… zwire …` before an export) is only visible through the
/// host callback table.
fn cfg(host: &Host, name: &str) -> Option<String> {
    host.getvar(name)
        .or_else(|| std::env::var(name).ok())
        .filter(|s| !s.is_empty())
}

/// Reply wait, in milliseconds. `$ZWIRE_TIMEOUT_MS` overrides. A page read goes
/// out to the renderer and back, so the default is generous.
fn timeout_ms(host: &Host) -> u64 {
    cfg(host, "ZWIRE_TIMEOUT_MS")
        .and_then(|s| s.parse().ok())
        .filter(|&n: &u64| n > 0)
        .unwrap_or(8000)
}

/// Correlation ids for this shell session. h: `proto::respond` copies `id` onto
/// the reply, which is what lets a reply be told apart from a streaming frame.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One of the bus sockets in the shared socket directory. h:
/// `zbus::platform::socket_dir` / `page::endpoint::socket_dir` —
/// `$XDG_RUNTIME_DIR/zgui`, else `$TMPDIR/zgui`, else `/tmp/zgui`.
fn bus_path(host: &Host, app: &str) -> PathBuf {
    let base = cfg(host, "XDG_RUNTIME_DIR")
        .or_else(|| cfg(host, "TMPDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("zgui").join(format!("{app}.sock"))
}

/// Where a request goes, in preference order.
///
/// `zwire-page` first, and that ordering is the whole trick. A `browser.*` verb
/// is delivered by PUBLISHING it on the `zbus.action` topic, and that topic is
/// process-global (h: `zbus::run_command` → `bus::publish`) — only the host
/// process the browser's service worker is attached to has a subscriber on it.
/// The `zwire-page` endpoint is bound by exactly that process (h:
/// `page::endpoint::bind`, served by the same `zbus::serve_conn`), so dialing it
/// is what makes `zwire tab new` reach the browser instead of queueing into a
/// daemon nobody is listening to.
///
/// Then the NDJSON daemon (a richer command set, but only up when someone ran
/// `zwire-host serve`), then the command bus, then the bundled client binary —
/// which is what covers Windows' named pipe with no second implementation here.
fn endpoints(host: &Host) -> Vec<(PathBuf, bool)> {
    vec![
        (bus_path(host, "zwire-page"), true),
        (socket_path(host), false),
        (bus_path(host, "zwire"), true),
    ]
}

/// Send one host-command request (`{"cmd":…}`), return its reply. Every endpoint
/// reaches the same dispatcher, so the caller writes one message shape and never
/// branches; a bus endpoint gets the command translated into the `{"t":"call"}`
/// frame it speaks (h: `zbus::serve_conn` → `run_command`, which turns the verb
/// back into `{"cmd":verb,…}` and runs it through a `Session`).
fn request(host: &Host, mut msg: Value) -> Result<Value, String> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    if let Some(o) = msg.as_object_mut() {
        o.insert("id".into(), json!(id));
    }
    #[cfg(unix)]
    {
        let ms = timeout_ms(host);
        for (path, is_bus) in endpoints(host) {
            if !path.exists() {
                continue;
            }
            return if is_bus {
                let frame = bus_frame(&msg, id);
                unix_request(&path, &frame, id, ms).map(|r| from_bus_reply(&msg, r))
            } else {
                unix_request(&path, &msg, id, ms)
            };
        }
    }
    cli_request(&msg, id)
}

/// One host command as the bus frame that carries it. A `call` already names a
/// verb, so it maps straight across; anything else becomes a call on the command
/// name itself with the remaining fields as its arguments.
fn bus_frame(msg: &Value, id: u64) -> Value {
    let cmd = msg["cmd"].as_str().unwrap_or("");
    if cmd == "call" {
        return json!({
            "t": "call",
            "id": id,
            "verb": msg["verb"],
            "args": msg.get("args").cloned().unwrap_or_else(|| json!({})),
        });
    }
    let mut args = serde_json::Map::new();
    if let Some(o) = msg.as_object() {
        for (k, v) in o {
            if k != "cmd" && k != "id" {
                args.insert(k.clone(), v.clone());
            }
        }
    }
    json!({ "t": "call", "id": id, "verb": cmd, "args": Value::Object(args) })
}

/// A bus reply (`{"t":"reply","ok":…,"value":…}`) in daemon shape, so everything
/// downstream reads one format. The bus reports a refusal as `error`; the daemon
/// calls it `err`.
fn from_bus_reply(msg: &Value, reply: Value) -> Value {
    if reply.get("ok") == Some(&json!(false)) {
        let e = reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("refused");
        return json!({ "ok": false, "err": e });
    }
    let value = reply.get("value").cloned().unwrap_or(Value::Null);
    // A `call` frame's value IS the verb's result; the daemon wraps it in
    // `{ok,result}`. Every other command's value is already its own reply.
    if msg["cmd"] == json!("call") {
        return json!({ "ok": true, "result": value });
    }
    value
}

#[cfg(unix)]
fn unix_request(sock: &std::path::Path, msg: &Value, id: u64, ms: u64) -> Result<Value, String> {
    use std::os::unix::net::UnixStream;
    let stream =
        UnixStream::connect(sock).map_err(|e| format!("connect {}: {e}", sock.display()))?;
    let wait = Duration::from_millis(ms);
    let _ = stream.set_read_timeout(Some(wait));
    let _ = stream.set_write_timeout(Some(wait));
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    line.push(b'\n');
    w.write_all(&line).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    read_reply(BufReader::new(stream).lines().map_while(Result::ok), id)
}

/// Fall back to the bundled client binary. `zwire-host call` prints the reply
/// (and any frames that arrived first) as NDJSON on stdout.
fn cli_request(msg: &Value, id: u64) -> Result<Value, String> {
    let text = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    let out = Command::new("zwire-host")
        .args(["call", &text])
        .output()
        .map_err(|_| "zwire-host is not running and not on PATH".to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    read_reply(stdout.lines().map(str::to_string), id)
}

/// Pick this request's reply out of the frame stream: the first object carrying
/// our `id`, else the first non-event object (a host old enough to drop `id`
/// still answers in order on a fresh connection).
fn read_reply<I: Iterator<Item = String>>(lines: I, id: u64) -> Result<Value, String> {
    let mut fallback = None;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("id").and_then(Value::as_u64) == Some(id) {
            return Ok(v);
        }
        if v.get("ev").is_none() && fallback.is_none() {
            fallback = Some(v);
        }
    }
    fallback.ok_or_else(|| "no reply from zwire-host".to_string())
}

/* ------------------------------------------------------------------ output */

/// The host's own failure text, if the reply carries one.
fn reply_err(v: &Value) -> Option<String> {
    if v.get("ok") == Some(&json!(false)) {
        return Some(
            v.get("err")
                .and_then(Value::as_str)
                .unwrap_or("refused")
                .to_string(),
        );
    }
    None
}

/// Print a JSON value the way a shell wants it: a string bare, a scalar bare,
/// anything structured as indented JSON that stays pipeable into `jq`.
fn show(host: &Host, v: &Value) {
    match v {
        Value::Null => {}
        Value::String(s) => host.print(&format!("{s}\n")),
        Value::Bool(_) | Value::Number(_) => host.print(&format!("{v}\n")),
        _ => host.print(&format!(
            "{}\n",
            serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
        )),
    }
}

fn fail(msg: &str) -> c_int {
    eprintln!("zwire: {msg}");
    1
}

/// Run one request and report it: `-` on failure, the named field (or the whole
/// reply) on success.
fn run(host: &Host, msg: Value, field: Option<&str>) -> c_int {
    match request(host, msg) {
        Err(e) => fail(&e),
        Ok(v) => {
            if let Some(e) = reply_err(&v) {
                return fail(&e);
            }
            match field {
                Some(f) => show(host, v.get(f).unwrap_or(&Value::Null)),
                None => show(host, &v),
            }
            0
        }
    }
}

/// Call one automation verb. h: `session::handle_cmd` `"call"` →
/// `zbus::call_verb`, which forwards a `browser.*` verb to the HUD worker.
fn call_verb(host: &Host, verb: &str, args: Value, quiet: bool) -> c_int {
    let msg = json!({ "cmd": "call", "verb": verb, "args": args });
    match request(host, msg) {
        Err(e) => fail(&e),
        Ok(v) => {
            if let Some(e) = reply_err(&v) {
                return fail(&e);
            }
            // A `browser.*` reply is a delivery receipt, not a browser result:
            // printing it on every `zwire tab new` would be noise. Structured
            // results (a page read, a host command) still print.
            let result = v.get("result").cloned().unwrap_or(Value::Null);
            let receipt = result.get("action").is_some() && result.get("queued").is_some();
            if !quiet && !receipt {
                show(host, &result);
            }
            if receipt && result.get("delivered") == Some(&json!(0)) {
                // Nothing subscribed: the daemon took the action but no browser
                // is attached to run it. Say so rather than reporting success.
                return fail("no zwire browser attached");
            }
            0
        }
    }
}

/// Look a subcommand up in one of the verb tables.
fn lookup(table: Table, sub: &str) -> Option<&'static str> {
    let sub = sub.to_ascii_lowercase();
    table.iter().find(|(k, _)| *k == sub).map(|(_, v)| *v)
}

/// The keys of a table, for an error message or a completion list.
fn keys(table: Table) -> Vec<&'static str> {
    table.iter().map(|(k, _)| *k).collect()
}

/// `k=v` → a JSON pair. The value parses as JSON when it can (`3`, `true`,
/// `[1,2]`, `"x"`), so `zwire call browser.gotoTab index=3` sends a number, and
/// stays a plain string otherwise — a URL is not valid JSON and must not be
/// mangled into one.
fn kv(arg: &str) -> Option<(String, Value)> {
    let (k, v) = arg.split_once('=')?;
    if k.is_empty() {
        return None;
    }
    let val = serde_json::from_str::<Value>(v).unwrap_or_else(|_| json!(v));
    Some((k.to_string(), val))
}

/// Collect `k=v …` arguments into one object.
fn kv_args(rest: &[String]) -> Result<Value, String> {
    let mut o = serde_json::Map::new();
    for a in rest {
        let (k, v) = kv(a).ok_or_else(|| format!("not a k=v argument: {a}"))?;
        o.insert(k, v);
    }
    Ok(Value::Object(o))
}

/* -------------------------------------------------------------- the builtin */

fn help(host: &Host) -> c_int {
    let mut s = String::from("usage: zwire <command> [args]\n\n");
    for (name, doc) in SUBCOMMANDS {
        s.push_str(&format!("  {name:<11}{doc}\n"));
    }
    s.push_str("\ntab:   ");
    s.push_str(&keys(TAB_VERBS).join(" "));
    s.push_str("\nwin:   ");
    s.push_str(&keys(WIN_VERBS).join(" "));
    s.push_str("\nsnap:  ");
    s.push_str(&keys(SNAP_VERBS).join(" "));
    s.push_str("\nnav:   ");
    s.push_str(&keys(NAV_VERBS).join(" "));
    s.push_str("\nzoom:  ");
    s.push_str(&keys(ZOOM_VERBS).join(" "));
    s.push_str("\nclear: ");
    s.push_str(&keys(CLEAR_VERBS).join(" "));
    s.push_str("\npage:  ");
    s.push_str(&PAGE_STATES.join(" "));
    s.push_str(" | extract SELECTOR [ATTR]\n");
    host.print(&s);
    0
}

/// The shared theme (`{ok, version, scheme, ui, palette, schemes}`).
/// h: `session::handle_cmd` `"get"`.
fn theme(host: &Host) -> Result<Value, String> {
    let v = request(host, json!({ "cmd": "get" }))?;
    match reply_err(&v) {
        Some(e) => Err(e),
        None => Ok(v),
    }
}

/// `zwire` / `zwire status` — the host handshake plus what it is showing.
fn status(host: &Host) -> c_int {
    let hello = match request(host, json!({ "cmd": "hello" })) {
        Ok(v) => v,
        Err(e) => return fail(&e),
    };
    if let Some(e) = reply_err(&hello) {
        return fail(&e);
    }
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("-").to_string();
    let th = theme(host).unwrap_or(Value::Null);
    let light = th
        .get("ui")
        .and_then(|u| u.get("light"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let attached = request(host, json!({ "cmd": "page_states" }))
        .ok()
        .and_then(|v| v.get("serving").and_then(Value::as_bool))
        .unwrap_or(false);
    let endpoint = endpoint_path(host);
    host.print(&format!(
        "host    {} {}\nscheme  {} ({})\nbrowser {}\nendpoint {}\n",
        s(&hello, "host"),
        s(&hello, "version"),
        s(&th, "scheme"),
        if light { "light" } else { "dark" },
        if attached { "attached" } else { "not attached" },
        endpoint.display(),
    ));
    0
}

/// The endpoint a request would actually take, for `status`.
fn endpoint_path(host: &Host) -> PathBuf {
    endpoints(host)
        .into_iter()
        .map(|(p, _)| p)
        .find(|p| p.exists())
        .unwrap_or_else(|| socket_path(host))
}

/// Write part of the shared theme: persisted, published to every subscriber and
/// broadcast to peer hosts, so the browser, the new-tab page and every themed
/// app follow at once. h: `session::theme_write`.
///
/// `theme` is a command only on hosts that carry it. An older host answers
/// `unknown_cmd`, and its one door onto the same write is the historical
/// COMMANDLESS message — which the bus cannot carry (every bus frame names a
/// verb), so the retry is only meaningful on the daemon socket. It costs one
/// round trip and keeps this plugin working against a browser that has not been
/// rebuilt yet.
fn theme_write(host: &Host, fields: Value, field: &str) -> c_int {
    let mut msg = fields.clone();
    if let Some(o) = msg.as_object_mut() {
        o.insert("cmd".into(), json!("theme"));
    }
    match request(host, msg) {
        Err(e) => fail(&e),
        Ok(v) => {
            if v.get("err").and_then(Value::as_str) == Some("unknown_cmd") {
                // Older host. The commandless message is the only other door,
                // and only the NDJSON daemon can carry it — a bus frame always
                // names a verb.
                if socket_path(host).exists() {
                    return run(host, fields, Some(field));
                }
                return fail("this zwire-host has no `theme` command: rebuild it");
            }
            if let Some(e) = reply_err(&v) {
                return fail(&e);
            }
            show(host, v.get(field).unwrap_or(&Value::Null));
            0
        }
    }
}

/// `zwire scheme [NAME]`.
fn scheme(host: &Host, rest: &[String]) -> c_int {
    match rest.first() {
        None => match theme(host) {
            Err(e) => fail(&e),
            Ok(v) => {
                show(host, v.get("scheme").unwrap_or(&Value::Null));
                0
            }
        },
        Some(name) => theme_write(host, json!({ "scheme": name }), "scheme"),
    }
}

/// `zwire light|dark|toggle`. h: `store::write_ui` shallow-merges `[theme.ui]`,
/// so writing `light` alone preserves every other ui setting.
fn set_light(host: &Host, on: Option<bool>) -> c_int {
    let on = match on {
        Some(v) => v,
        None => !theme(host)
            .ok()
            .and_then(|v| {
                v.get("ui")
                    .and_then(|u| u.get("light"))
                    .and_then(Value::as_bool)
            })
            .unwrap_or(false),
    };
    theme_write(host, json!({ "ui": { "light": on } }), "ui")
}

/// `zwire page STATE [SELECTOR [ATTR]]` — the live DOM as typed state.
///
/// Sent as a `call` on the projection itself (`page.url`), never as `page_get`:
/// the daemon answers a page read on its own thread (h: `session::spawn_page_reply`),
/// and the bus's in-process dispatcher reads only the FIRST line a `Session`
/// wrote — so `page_get` over the bus comes back "no reply from host session"
/// while the answer is still in flight. `page.*` as a verb routes straight to
/// `page::request` on both transports.
fn page(host: &Host, rest: &[String]) -> c_int {
    let Some(what) = rest.first() else {
        host.print(&format!("{} extract\n", PAGE_STATES.join(" ")));
        return 0;
    };
    let (state, args) = if what == "extract" {
        let Some(sel) = rest.get(1) else {
            return fail("page extract: needs a selector");
        };
        let mut a = json!({ "selector": sel });
        if let Some(attr) = rest.get(2) {
            a["attr"] = json!(attr);
        }
        ("page.extract".to_string(), a)
    } else {
        let id = format!("page.{what}");
        if !PAGE_STATES.contains(&what.as_str()) {
            return fail(&format!("unknown page state: {what}"));
        }
        (id, json!({}))
    };
    match request(host, json!({ "cmd": "call", "verb": state, "args": args })) {
        Err(e) => fail(&e),
        Ok(v) => {
            if let Some(e) = reply_err(&v) {
                return fail(&e);
            }
            // The verb's own reply: `{ok, state, value}` on success, `{ok:false,
            // err}` when the browser is closed or refused the injection.
            let r = v.get("result").cloned().unwrap_or(Value::Null);
            if let Some(e) = reply_err(&r) {
                return fail(&e);
            }
            show(host, r.get("value").unwrap_or(&Value::Null));
            0
        }
    }
}

/// `zwire verbs [PATTERN]` — the published automation surface, one `id rev` per
/// line. h: `zbus::surface`.
fn verbs(host: &Host, rest: &[String]) -> c_int {
    let filter = rest.first().map(|s| s.to_ascii_lowercase());
    match request(host, json!({ "cmd": "verbs" })) {
        Err(e) => fail(&e),
        Ok(v) => {
            let list = v
                .get("verbs")
                .or_else(|| v.get("result").and_then(|r| r.get("verbs")))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if list.is_empty() {
                return fail("no automation surface in the reply");
            }
            let mut out = String::new();
            for e in &list {
                let id = e.get("id").and_then(Value::as_str).unwrap_or("");
                if filter
                    .as_deref()
                    .is_some_and(|f| !id.to_ascii_lowercase().contains(f))
                {
                    continue;
                }
                let rev = e.get("rev").and_then(Value::as_str).unwrap_or("");
                out.push_str(&format!("{id:<28}{rev}\n"));
            }
            host.print(&out);
            0
        }
    }
}

/// A verb-table subcommand (`tab`, `win`, `snap`, `nav`, `zoom`, `clear`).
fn table_cmd(host: &Host, group: &str, table: Table, rest: &[String], args: Value) -> c_int {
    let Some(sub) = rest.first() else {
        host.print(&format!("{}\n", keys(table).join(" ")));
        return 0;
    };
    match lookup(table, sub) {
        Some(verb) => call_verb(host, &format!("browser.{verb}"), args, true),
        None => fail(&format!("unknown {group} command: {sub}")),
    }
}

fn zwire(host: &Host, args: &Args) -> c_int {
    let rest = args.rest();
    let Some(cmd) = rest.first().map(|s| s.as_str()) else {
        return status(host);
    };
    let tail = &rest[1..];
    match cmd {
        "help" | "-h" | "--help" => help(host),
        "status" => status(host),

        "scheme" => scheme(host, tail),
        "schemes" => {
            host.print(&format!("{}\n", SCHEMES.join(" ")));
            0
        }
        "light" => set_light(host, Some(true)),
        "dark" => set_light(host, Some(false)),
        "toggle" => set_light(host, None),

        // Navigation targets. `open` makes new tabs, `goto` reuses the active
        // one — h: background.js `openTab` vs `open`.
        "open" | "goto" => {
            if tail.is_empty() {
                return fail(&format!("{cmd}: needs a URL"));
            }
            let verb = if cmd == "open" {
                "browser.openTab"
            } else {
                "browser.open"
            };
            for url in tail {
                let rc = call_verb(host, verb, json!({ "url": url }), true);
                if rc != 0 {
                    return rc;
                }
            }
            0
        }

        "tab" => {
            // `tab goto N` carries an index; every other tab verb is bare.
            if tail.first().map(String::as_str) == Some("goto") {
                let Some(n) = tail.get(1).and_then(|s| s.parse::<i64>().ok()) else {
                    return fail("tab goto: needs an index");
                };
                return call_verb(host, "browser.gotoTab", json!({ "index": n }), true);
            }
            table_cmd(host, "tab", TAB_VERBS, tail, json!({}))
        }
        "win" | "window" => table_cmd(host, "win", WIN_VERBS, tail, json!({})),
        "snap" => table_cmd(host, "snap", SNAP_VERBS, tail, json!({})),
        "nav" => table_cmd(host, "nav", NAV_VERBS, tail, json!({})),
        "zoom" => table_cmd(host, "zoom", ZOOM_VERBS, tail, json!({})),
        "clear" => table_cmd(host, "clear", CLEAR_VERBS, tail, json!({})),

        // The three navigation verbs worth a top-level name.
        "back" => call_verb(host, "browser.goBack", json!({}), true),
        "forward" => call_verb(host, "browser.goForward", json!({}), true),
        "reload" => {
            let hard = tail.first().is_some_and(|s| s == "hard" || s == "-f");
            let verb = if hard {
                "browser.reloadHard"
            } else {
                "browser.reload"
            };
            call_verb(host, verb, json!({}), true)
        }

        "bookmark" => call_verb(host, "browser.bookmarkTab", json!({}), true),
        "download" => match tail.first() {
            Some(url) => call_verb(host, "browser.download", json!({ "url": url }), true),
            None => fail("download: needs a URL"),
        },
        "screenshot" => call_verb(host, "browser.screenshot", json!({}), true),
        "notify" => match tail.first() {
            Some(title) => call_verb(
                host,
                "browser.notify",
                json!({ "title": title, "message": tail[1..].join(" ") }),
                true,
            ),
            None => fail("notify: needs a title"),
        },

        // Host-side, not browser-side: the clipboard belongs to the machine.
        "clip" => {
            if tail.is_empty() {
                run(host, json!({ "cmd": "clipboard_get" }), Some("text"))
            } else {
                run(
                    host,
                    json!({ "cmd": "clipboard_set", "text": tail.join(" ") }),
                    Some("ok"),
                )
            }
        }

        "page" => page(host, tail),
        "verbs" => verbs(host, tail),

        // Escape hatches: any verb, any host command.
        "call" => match tail.first() {
            Some(verb) => match kv_args(&tail[1..]) {
                Ok(a) => call_verb(host, verb, a, false),
                Err(e) => fail(&e),
            },
            None => fail("call: needs a verb"),
        },
        "raw" => match tail.first() {
            Some(text) => match serde_json::from_str::<Value>(text) {
                Ok(v) if v.is_object() => run(host, v, None),
                Ok(_) => fail("raw: needs a JSON object"),
                Err(e) => fail(&format!("raw: {e}")),
            },
            None => fail("raw: needs a JSON object"),
        },

        other => fail(&format!("unknown command: {other}")),
    }
}

/* ------------------------------------------------------------- completions */

/// Native completion. The word list is `[CURRENT, word0, word1, …]` — see
/// PLUGINS.md — so `words[1]` is the subcommand and `current` says which word
/// is being completed.
fn zwire_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();
    let current: usize = a.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let words = &a[1..];
    let prefix = current
        .checked_sub(1)
        .and_then(|i| words.get(i))
        .map(String::as_str)
        .unwrap_or("");
    let sub = words.get(1).map(String::as_str).unwrap_or("");

    let emit = |cands: Vec<&str>| {
        for c in cands {
            if c.starts_with(prefix) {
                host.add_match(c);
            }
        }
    };

    // Word 2 is the subcommand itself; later words complete per subcommand.
    if current <= 2 {
        emit(SUBCOMMANDS.iter().map(|(k, _)| *k).collect());
        return 0;
    }
    match sub {
        "scheme" => emit(SCHEMES.to_vec()),
        "tab" => {
            let mut k = keys(TAB_VERBS);
            k.push("goto");
            emit(k);
        }
        "win" | "window" => emit(keys(WIN_VERBS)),
        "snap" => emit(keys(SNAP_VERBS)),
        "nav" => emit(keys(NAV_VERBS)),
        "zoom" => emit(keys(ZOOM_VERBS)),
        "clear" => emit(keys(CLEAR_VERBS)),
        "page" => {
            let mut k = PAGE_STATES.to_vec();
            k.push("extract");
            emit(k);
        }
        "reload" => emit(vec!["hard"]),
        // The live surface, asked of the running host — so a verb added to
        // zwire-host completes here the day it ships, with no table to update.
        "call" | "verbs" => {
            if let Ok(v) = request(host, json!({ "cmd": "verbs" })) {
                let list = v
                    .get("verbs")
                    .or_else(|| v.get("result").and_then(|r| r.get("verbs")))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for e in &list {
                    if let Some(id) = e.get("id").and_then(Value::as_str) {
                        if id.starts_with(prefix) {
                            host.add_match(id);
                        }
                    }
                }
            }
        }
        _ => {}
    }
    0
}

declare_plugin! {
    name: "zwire",
    version: "0.1.0",
    builtins: {
        "zwire" => zwire,
    },
    completions: {
        "zwire" => zwire_complete,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kv_parses_json_values_and_keeps_urls_intact() {
        assert_eq!(kv("index=3"), Some(("index".into(), json!(3))));
        assert_eq!(kv("pinned=true"), Some(("pinned".into(), json!(true))));
        assert_eq!(kv("ids=[1,2]"), Some(("ids".into(), json!([1, 2]))));
        // A URL is not JSON; it must survive as a plain string.
        assert_eq!(
            kv("url=https://example.com/a?b=c"),
            Some(("url".into(), json!("https://example.com/a?b=c")))
        );
        // An `=` inside the value belongs to the value.
        assert_eq!(kv("q=a=b"), Some(("q".into(), json!("a=b"))));
        assert_eq!(kv("noequals"), None);
        assert_eq!(kv("=v"), None);
    }

    #[test]
    fn kv_args_rejects_a_bare_word() {
        assert!(kv_args(&["a=1".to_string()]).is_ok());
        assert!(kv_args(&["oops".to_string()]).is_err());
    }

    #[test]
    fn verb_tables_are_case_insensitive_and_unique() {
        assert_eq!(lookup(TAB_VERBS, "NEW"), Some("newTab"));
        assert_eq!(lookup(TAB_VERBS, "dups"), Some("closeDuplicates"));
        assert_eq!(lookup(WIN_VERBS, "full"), Some("fullscreenWindow"));
        assert_eq!(lookup(SNAP_VERBS, "bottomright"), Some("snapBottomRight"));
        assert_eq!(lookup(TAB_VERBS, "nope"), None);
        for table in [
            TAB_VERBS,
            WIN_VERBS,
            SNAP_VERBS,
            NAV_VERBS,
            ZOOM_VERBS,
            CLEAR_VERBS,
        ] {
            let mut ks = keys(table);
            let n = ks.len();
            ks.sort_unstable();
            ks.dedup();
            assert_eq!(ks.len(), n, "duplicate subcommand in a verb table");
        }
    }

    /// Every verb this plugin can name must exist on zwire-host's published
    /// surface — the tables here are a spelling of `SURFACE_VERBS`, and a typo
    /// would only show up as a silently ignored action in the browser.
    #[test]
    fn every_table_verb_is_camel_cased_browser_action() {
        for table in [
            TAB_VERBS,
            WIN_VERBS,
            SNAP_VERBS,
            NAV_VERBS,
            ZOOM_VERBS,
            CLEAR_VERBS,
        ] {
            for (sub, verb) in table {
                assert!(!verb.is_empty(), "{sub} maps to an empty verb");
                assert!(
                    verb.chars().all(|c| c.is_ascii_alphanumeric()),
                    "{verb} is not a bare action name"
                );
                assert!(
                    verb.starts_with(|c: char| c.is_ascii_lowercase()),
                    "{verb} is not camelCase"
                );
            }
        }
    }

    #[test]
    fn a_reply_is_matched_by_id_past_streaming_frames() {
        let lines = [
            r#"{"ev":"pub","topic":"scheme","data":{}}"#,
            r#"{"ok":true,"id":7,"scheme":"matrix"}"#,
        ]
        .iter()
        .map(|s| s.to_string());
        let v = read_reply(lines, 7).unwrap();
        assert_eq!(v["scheme"], json!("matrix"));
    }

    #[test]
    fn an_unmatched_id_falls_back_to_the_first_non_event_frame() {
        let lines = [
            r#"{"ev":"line","data":"x"}"#,
            r#"{"ok":true,"scheme":"ember"}"#,
        ]
        .iter()
        .map(|s| s.to_string());
        assert_eq!(read_reply(lines, 99).unwrap()["scheme"], json!("ember"));
        // Nothing but event frames is not an answer.
        let only_events = [r#"{"ev":"pub"}"#].iter().map(|s| s.to_string());
        assert!(read_reply(only_events, 1).is_err());
    }

    #[test]
    fn a_host_command_becomes_a_bus_call_on_its_own_name() {
        let f = bus_frame(&json!({"cmd": "theme", "scheme": "matrix", "id": 4}), 4);
        assert_eq!(f["t"], json!("call"));
        assert_eq!(f["verb"], json!("theme"));
        assert_eq!(
            f["args"],
            json!({"scheme": "matrix"}),
            "id/cmd stripped: {f}"
        );
        // A `call` already names its verb and must pass straight through.
        let c = bus_frame(
            &json!({"cmd": "call", "verb": "browser.openTab", "args": {"url": "u"}}),
            5,
        );
        assert_eq!(c["verb"], json!("browser.openTab"));
        assert_eq!(c["args"], json!({"url": "u"}));
    }

    #[test]
    fn a_bus_reply_is_normalised_into_daemon_shape() {
        // A call's value is the verb's result and gets wrapped.
        let r = from_bus_reply(
            &json!({"cmd": "call", "verb": "page.url"}),
            json!({"t": "reply", "ok": true, "value": {"ok": true, "value": "u"}}),
        );
        assert_eq!(r, json!({"ok": true, "result": {"ok": true, "value": "u"}}));
        // Any other command's value IS its reply.
        let g = from_bus_reply(
            &json!({"cmd": "get"}),
            json!({"t": "reply", "ok": true, "value": {"ok": true, "scheme": "toxic"}}),
        );
        assert_eq!(g["scheme"], json!("toxic"));
        // The bus says `error` where the daemon says `err`.
        let e = from_bus_reply(
            &json!({"cmd": "call"}),
            json!({"t": "reply", "ok": false, "error": "verb not reversible: x"}),
        );
        assert_eq!(e, json!({"ok": false, "err": "verb not reversible: x"}));
    }

    #[test]
    fn reply_err_reads_the_hosts_own_reason() {
        assert_eq!(
            reply_err(&json!({"ok": false, "err": "bad_scheme"})).as_deref(),
            Some("bad_scheme")
        );
        assert_eq!(reply_err(&json!({"ok": false})).as_deref(), Some("refused"));
        assert_eq!(reply_err(&json!({"ok": true})), None);
    }

    #[test]
    fn help_lists_every_subcommand() {
        // The help text is built from SUBCOMMANDS, so this pins that the tables
        // it appends stay non-empty rather than that the prose matches.
        assert!(SUBCOMMANDS.iter().any(|(k, _)| *k == "page"));
        assert!(!TAB_VERBS.is_empty() && !PAGE_STATES.is_empty());
    }
}
