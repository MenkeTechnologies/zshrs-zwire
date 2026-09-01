//! **zwire** — the whole zgui automation bus as a shell builtin.
//!
//! [zwire](https://github.com/MenkeTechnologies/zwire)'s browser and the machine
//! under it are published by
//! [`zwire-host`](https://github.com/MenkeTechnologies/zwire-host): local
//! endpoints (Unix sockets on macOS/Linux, named pipes on Windows) speaking
//! newline-delimited JSON. One dispatcher answers `{"cmd":…}` host commands,
//! forwards `browser.*` automation verbs to the HUD service worker, projects the
//! live DOM as typed `page.*` state, and — through `suite_*` — dials the OTHER
//! apps on the bus.
//!
//! This plugin puts every one of those on the command line. `SUBS` is the
//! whole named surface: one row per command, each naming the verb it sends and
//! the arguments it binds positionally, so `zwire fs grep ~/src TODO`,
//! `zwire tmux send %3 'make'`, `zwire suite call zcite item.add`, `zwire txn
//! begin` and `zwire page tables` all take the same path. `zwire audit` asks the
//! running host for its surface and prints anything not reachable by name — the
//! table cannot silently fall behind the host it drives.
//!
//! See `endpoints` for which socket a request takes and why the order matters.
//!
//! Protocol citations (`h:`) refer to zwire-host's sources: `src/lib.rs`
//! (`default_socket`), `src/session.rs` (`handle_cmd`, `theme_write`),
//! `src/zbus.rs` (`serve_conn`, `run_command`, `surface`, `SURFACE_VERBS`),
//! `src/page.rs` (`STATES`, `endpoint::bind`), `src/fsx.rs`, `src/tmuxops.rs`,
//! `src/hooks.rs`, `src/suite.rs` and `src/store.rs`; `background.js` (`execZbAction`)
//! is the browser half of every `browser.*` verb.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::io::{BufRead, BufReader, Write};
use std::os::raw::c_int;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use znative::{declare_plugin, Args, Host};

/* ----------------------------------------------------------------- surface */

/// One named command: `zwire <group> <name>` (or `zwire <name>` when `group` is
/// empty) sends `verb`, binding `args` from the words that follow.
///
/// `args` entries are positional in order, with three markers:
/// * `name?` — optional, bound only if a word is there.
/// * `*name` — every remaining word, as an array.
/// * `**name` — every remaining `k=v`, as a nested object (what `suite call`
///   needs, since the called app's arguments must not collide with this one's).
///
/// Whatever is left over is parsed as top-level `k=v`, so a row never has to
/// name every optional field the host accepts — `zwire fs walk ~/src ext=rs
/// depth=3` reaches fields this table does not mention.
struct Sub {
    group: &'static str,
    name: &'static str,
    verb: &'static str,
    args: &'static [&'static str],
    doc: &'static str,
    /// The reply is a STREAM of frames rather than one answer (`fs_watch`,
    /// `sysinfo_start`, `sub`, …). Printed until `secs=` elapses.
    stream: bool,
}

/// Shorthand for a row.
const fn s(
    group: &'static str,
    name: &'static str,
    verb: &'static str,
    args: &'static [&'static str],
    doc: &'static str,
) -> Sub {
    Sub {
        group,
        name,
        verb,
        args,
        doc,
        stream: false,
    }
}

/// Shorthand for a streaming row.
const fn st(
    group: &'static str,
    name: &'static str,
    verb: &'static str,
    args: &'static [&'static str],
    doc: &'static str,
) -> Sub {
    Sub {
        group,
        name,
        verb,
        args,
        doc,
        stream: true,
    }
}

/// The named surface. Every row is one command on the bus.
const SUBS: &[Sub] = &[
    /* ---- tabs (browser.*, executed by background.js execZbAction) ---- */
    s("tab", "new", "browser.newTab", &[], "open an empty tab"),
    s(
        "tab",
        "close",
        "browser.closeTab",
        &[],
        "close the active tab",
    ),
    s(
        "tab",
        "others",
        "browser.closeOthers",
        &[],
        "close every other tab",
    ),
    s(
        "tab",
        "right",
        "browser.closeRight",
        &[],
        "close tabs to the right",
    ),
    s(
        "tab",
        "left",
        "browser.closeLeft",
        &[],
        "close tabs to the left",
    ),
    s(
        "tab",
        "dups",
        "browser.closeDuplicates",
        &[],
        "close duplicate URLs",
    ),
    s(
        "tab",
        "reopen",
        "browser.reopenTab",
        &[],
        "restore the last closed tab",
    ),
    s(
        "tab",
        "dup",
        "browser.duplicateTab",
        &[],
        "duplicate the active tab",
    ),
    s("tab", "pin", "browser.pinTab", &[], "toggle pinned"),
    s("tab", "unpin", "browser.unpinTab", &[], "unpin"),
    s("tab", "mute", "browser.muteTab", &[], "toggle muted"),
    s("tab", "unmute", "browser.unmuteTab", &[], "unmute"),
    s(
        "tab",
        "muteothers",
        "browser.muteOthers",
        &[],
        "mute every other tab",
    ),
    s(
        "tab",
        "discard",
        "browser.discardTab",
        &[],
        "discard (free its memory)",
    ),
    s(
        "tab",
        "next",
        "browser.nextTab",
        &[],
        "activate the next tab",
    ),
    s(
        "tab",
        "prev",
        "browser.prevTab",
        &[],
        "activate the previous tab",
    ),
    s(
        "tab",
        "first",
        "browser.firstTab",
        &[],
        "activate the first tab",
    ),
    s(
        "tab",
        "last",
        "browser.lastTab",
        &[],
        "activate the last tab",
    ),
    s(
        "tab",
        "goto",
        "browser.gotoTab",
        &["index"],
        "activate tab N (0-based)",
    ),
    s(
        "tab",
        "activate",
        "browser.activate",
        &["tabId"],
        "activate a tab by id",
    ),
    s(
        "tab",
        "moveleft",
        "browser.moveTabLeft",
        &[],
        "move one place left",
    ),
    s(
        "tab",
        "moveright",
        "browser.moveTabRight",
        &[],
        "move one place right",
    ),
    s(
        "tab",
        "movefirst",
        "browser.moveTabFirst",
        &[],
        "move to the front",
    ),
    s(
        "tab",
        "movelast",
        "browser.moveTabLast",
        &[],
        "move to the end",
    ),
    s(
        "tab",
        "detach",
        "browser.tabToNewWindow",
        &[],
        "move it to a new window",
    ),
    s(
        "tab",
        "sort",
        "browser.sortTabs",
        &[],
        "sort the strip by URL",
    ),
    s("tab", "muteall", "browser.muteAll", &[], "mute every tab"),
    s(
        "tab",
        "unmuteall",
        "browser.unmuteAll",
        &[],
        "unmute every tab",
    ),
    s("tab", "pinall", "browser.pinAll", &[], "pin every tab"),
    s(
        "tab",
        "unpinall",
        "browser.unpinAll",
        &[],
        "unpin every tab",
    ),
    s(
        "tab",
        "group",
        "browser.groupTabs",
        &[],
        "group the window's tabs",
    ),
    s("tab", "ungroup", "browser.ungroupTabs", &[], "ungroup them"),
    s(
        "tab",
        "collapse",
        "browser.collapseGroups",
        &[],
        "collapse tab groups",
    ),
    s(
        "tab",
        "expand",
        "browser.expandGroups",
        &[],
        "expand tab groups",
    ),
    /* ---- windows ---- */
    s("win", "new", "browser.newWindow", &[], "new window"),
    s(
        "win",
        "incognito",
        "browser.incognitoWindow",
        &[],
        "new incognito window",
    ),
    s(
        "win",
        "close",
        "browser.closeWindow",
        &[],
        "close the focused window",
    ),
    s("win", "min", "browser.minimizeWindow", &[], "minimize"),
    s("win", "max", "browser.maximizeWindow", &[], "maximize"),
    s("win", "full", "browser.fullscreenWindow", &[], "fullscreen"),
    s(
        "win",
        "restore",
        "browser.restoreWindow",
        &[],
        "back to normal state",
    ),
    s(
        "win",
        "next",
        "browser.nextWindow",
        &[],
        "focus the next window",
    ),
    s(
        "win",
        "prev",
        "browser.prevWindow",
        &[],
        "focus the previous window",
    ),
    s(
        "win",
        "merge",
        "browser.mergeWindows",
        &[],
        "merge every window into one",
    ),
    s(
        "win",
        "center",
        "browser.centerWindow",
        &[],
        "center on its display",
    ),
    s(
        "win",
        "display",
        "browser.moveWindowNextDisplay",
        &[],
        "move to the next display",
    ),
    /* ---- tiling ---- */
    s(
        "snap",
        "left",
        "browser.snapLeft",
        &[],
        "snap to the left half",
    ),
    s(
        "snap",
        "right",
        "browser.snapRight",
        &[],
        "snap to the right half",
    ),
    s(
        "snap",
        "top",
        "browser.snapTop",
        &[],
        "snap to the top half",
    ),
    s(
        "snap",
        "bottom",
        "browser.snapBottom",
        &[],
        "snap to the bottom half",
    ),
    s(
        "snap",
        "topleft",
        "browser.snapTopLeft",
        &[],
        "snap to the top-left quarter",
    ),
    s(
        "snap",
        "topright",
        "browser.snapTopRight",
        &[],
        "snap to the top-right quarter",
    ),
    s(
        "snap",
        "bottomleft",
        "browser.snapBottomLeft",
        &[],
        "snap to the bottom-left quarter",
    ),
    s(
        "snap",
        "bottomright",
        "browser.snapBottomRight",
        &[],
        "snap to the bottom-right quarter",
    ),
    /* ---- navigation + zoom ---- */
    s("nav", "back", "browser.goBack", &[], "go back"),
    s("nav", "forward", "browser.goForward", &[], "go forward"),
    s("nav", "reload", "browser.reload", &[], "reload"),
    s(
        "nav",
        "hard",
        "browser.reloadHard",
        &[],
        "reload, bypassing the cache",
    ),
    s(
        "nav",
        "reloadall",
        "browser.reloadAll",
        &[],
        "reload every tab in the window",
    ),
    s("nav", "home", "browser.home", &[], "go to the new-tab page"),
    s("zoom", "in", "browser.zoomIn", &[], "zoom in"),
    s("zoom", "out", "browser.zoomOut", &[], "zoom out"),
    s("zoom", "reset", "browser.zoomReset", &[], "reset zoom"),
    /* ---- browsing data ---- */
    s(
        "clear",
        "cache",
        "browser.clearCache",
        &[],
        "clear the cache",
    ),
    s(
        "clear",
        "cookies",
        "browser.clearCookies",
        &[],
        "clear cookies",
    ),
    s(
        "clear",
        "both",
        "browser.clearCacheAndCookies",
        &[],
        "clear cache + cookies",
    ),
    s(
        "clear",
        "all",
        "browser.clearAllData",
        &[],
        "clear everything",
    ),
    s(
        "clear",
        "passwords",
        "browser.clearPasswords",
        &[],
        "clear saved passwords",
    ),
    /* ---- downloads ---- */
    s("dl", "get", "browser.download", &["url"], "download a URL"),
    s(
        "dl",
        "pause",
        "browser.pauseDownload",
        &[],
        "pause downloads in progress",
    ),
    s(
        "dl",
        "resume",
        "browser.resumeDownload",
        &[],
        "resume paused downloads",
    ),
    s(
        "dl",
        "cancel",
        "browser.cancelDownload",
        &[],
        "cancel downloads",
    ),
    s(
        "dl",
        "open",
        "browser.openDownload",
        &[],
        "open the last download",
    ),
    s(
        "dl",
        "show",
        "browser.showDownload",
        &[],
        "reveal it in the file manager",
    ),
    s(
        "dl",
        "retry",
        "browser.retryDownload",
        &[],
        "retry interrupted downloads",
    ),
    s(
        "dl",
        "clear",
        "browser.clearDownloads",
        &[],
        "clear the download list",
    ),
    s(
        "dl",
        "list",
        "browser.showDownloads",
        &[],
        "open the downloads page",
    ),
    /* ---- bookmarks / reading list / history ---- */
    s(
        "bm",
        "add",
        "browser.bookmarkTab",
        &[],
        "bookmark the active tab",
    ),
    s(
        "bm",
        "folder",
        "browser.bookmarkFolder",
        &["title?"],
        "create a bookmark folder",
    ),
    s(
        "bm",
        "remove",
        "browser.removeBookmark",
        &[],
        "unbookmark the active tab",
    ),
    s(
        "read",
        "add",
        "browser.addReadingList",
        &[],
        "add the tab to the reading list",
    ),
    s(
        "read",
        "remove",
        "browser.removeReadingList",
        &[],
        "remove it",
    ),
    s(
        "hist",
        "clear",
        "browser.clearHistory",
        &[],
        "delete all history",
    ),
    s(
        "hist",
        "del",
        "browser.deleteHistoryUrl",
        &["url"],
        "delete one URL",
    ),
    s(
        "hist",
        "add",
        "browser.addHistoryUrl",
        &["url"],
        "add a URL to history",
    ),
    /* ---- extensions / apps / power ---- */
    s(
        "ext",
        "enable",
        "browser.enableExtension",
        &["id"],
        "enable an extension",
    ),
    s(
        "ext",
        "disable",
        "browser.disableExtension",
        &["id"],
        "disable it",
    ),
    s(
        "ext",
        "uninstall",
        "browser.uninstallExtension",
        &["id"],
        "uninstall it",
    ),
    s(
        "ext",
        "options",
        "browser.extensionOptions",
        &["id"],
        "open its options page",
    ),
    s(
        "ext",
        "launch",
        "browser.launchApp",
        &["id"],
        "launch a packaged app",
    ),
    s(
        "power",
        "awake",
        "browser.keepAwake",
        &[],
        "keep the system awake",
    ),
    s(
        "power",
        "display",
        "browser.keepDisplayAwake",
        &[],
        "keep the display awake",
    ),
    s(
        "power",
        "sleep",
        "browser.allowSleep",
        &[],
        "release the wake lock",
    ),
    /* ---- the live page (page.rs — reads only, never writes) ---- */
    s(
        "page",
        "url",
        "page.url",
        &[],
        "address of the rendered page",
    ),
    s("page", "title", "page.title", &[], "document title"),
    s("page", "text", "page.text", &[], "rendered body text"),
    s(
        "page",
        "links",
        "page.links",
        &[],
        "every link: text + resolved href",
    ),
    s("page", "headings", "page.headings", &[], "heading outline"),
    s(
        "page",
        "tables",
        "page.tables",
        &[],
        "every table as rows of cells",
    ),
    s(
        "page",
        "forms",
        "page.forms",
        &[],
        "form shape — never field values",
    ),
    s(
        "page",
        "meta",
        "page.meta",
        &[],
        "description, canonical, og:*, JSON-LD",
    ),
    s(
        "page",
        "selection",
        "page.selection",
        &[],
        "the current selection",
    ),
    s(
        "page",
        "extract",
        "page.extract",
        &["selector", "attr?"],
        "any selector, optionally one attribute",
    ),
    s(
        "page",
        "assert",
        "page.assert",
        &["state?", "op?", "value?"],
        "test a projection (contains, equals, count_at_least, …)",
    ),
    s(
        "page",
        "witness",
        "page.witness",
        &["state?", "op?", "value?"],
        "declare a projection as a premise of the open transaction",
    ),
    s(
        "page",
        "batch",
        "page.batch",
        &[],
        "several reads in one round trip: reads=[…]",
    ),
    s(
        "page",
        "states",
        "page_states",
        &[],
        "the catalogue, and whether a browser is attached",
    ),
    /* ---- transactions (a chain that unwinds itself) ---- */
    s("txn", "begin", "txn_begin", &["txn?"], "open a transaction"),
    s(
        "txn",
        "commit",
        "txn_commit",
        &["txn"],
        "commit it — premises re-checked first",
    ),
    s(
        "txn",
        "abort",
        "txn_abort",
        &["txn"],
        "compensate every journaled step, in reverse",
    ),
    s(
        "txn",
        "undo",
        "browser.undo",
        &["txn"],
        "replay one transaction's inverses in the browser",
    ),
    /* ---- filesystem (fsops.rs + fsx.rs) ---- */
    s(
        "fs",
        "read",
        "fs_read",
        &["path"],
        "read a file → {b64, text}",
    ),
    s("fs", "write", "fs_write", &["path", "text"], "write a file"),
    s(
        "fs",
        "append",
        "fs_append",
        &["path", "text"],
        "append to a file",
    ),
    s(
        "fs",
        "list",
        "fs_list",
        &["path"],
        "one level: {name, dir, size}",
    ),
    s(
        "fs",
        "walk",
        "fs_walk",
        &["path"],
        "recursive crawl (ext=, depth=, contains=, dirs_only=)",
    ),
    s("fs", "stat", "fs_stat", &["path"], "stat one path"),
    s("fs", "mkdir", "fs_mkdir", &["path"], "mkdir -p"),
    s(
        "fs",
        "rm",
        "fs_rm",
        &["path"],
        "remove (recursive=true for a tree)",
    ),
    s(
        "fs",
        "ls",
        "fs_list_dir",
        &["dir_path"],
        "browser listing, directories first",
    ),
    s(
        "fs",
        "subdirs",
        "fs_list_subdirs",
        &["dir_path"],
        "directories only",
    ),
    s(
        "fs",
        "info",
        "fs_get_info",
        &["path"],
        "kind, sizes, times, mode, owner, link target",
    ),
    s(
        "fs",
        "size",
        "fs_folder_size",
        &["folder_path"],
        "recursive {bytes, files}",
    ),
    s(
        "fs",
        "disk",
        "fs_disk_usage",
        &["path"],
        "the mount holding a path",
    ),
    s(
        "fs",
        "xattrs",
        "fs_xattrs",
        &["path"],
        "extended attributes",
    ),
    s(
        "fs",
        "git",
        "fs_git_status",
        &["dir_path"],
        "porcelain status codes",
    ),
    s("fs", "hash", "fs_hash", &["path"], "streamed SHA-256"),
    s(
        "fs",
        "grep",
        "fs_grep",
        &["root", "needle"],
        "search a tree (case_insensitive=, max_results=)",
    ),
    s(
        "fs",
        "dups",
        "fs_find_duplicates",
        &["dir"],
        "duplicate files by content",
    ),
    s(
        "fs",
        "compare",
        "fs_compare_dirs",
        &["dir_a", "dir_b"],
        "onlyInA / onlyInB / different",
    ),
    s(
        "fs",
        "diff",
        "fs_diff",
        &["path_a", "path_b"],
        "unified text diff",
    ),
    s(
        "fs",
        "zip",
        "fs_compress",
        &["archive_path"],
        "write a zip: paths=[…]",
    ),
    s(
        "fs",
        "unzip",
        "fs_extract",
        &["archive_path", "dest_dir"],
        "extract zip/tar/tgz",
    ),
    s(
        "fs",
        "b64",
        "fs_read_file_base64",
        &["file_path"],
        "whole file as base64",
    ),
    s(
        "fs",
        "head",
        "fs_read_head",
        &["file_path"],
        "head as text (max_bytes=)",
    ),
    s(
        "fs",
        "headbytes",
        "fs_read_head_bytes",
        &["file_path"],
        "head as raw bytes",
    ),
    s(
        "fs",
        "newdir",
        "fs_create_dir",
        &["dir_path"],
        "create a directory (refuses an existing one)",
    ),
    s(
        "fs",
        "newfile",
        "fs_create_file",
        &["file_path"],
        "create a file",
    ),
    s(
        "fs",
        "copy",
        "fs_copy_path",
        &["src", "dest"],
        "copy a file or tree",
    ),
    s(
        "fs",
        "dup",
        "fs_duplicate",
        &["path"],
        "make the next free ` copy` sibling",
    ),
    s(
        "fs",
        "rename",
        "fs_rename_file",
        &["old_path", "new_path"],
        "rename / move",
    ),
    s(
        "fs",
        "del",
        "fs_delete_file",
        &["file_path"],
        "delete a file or tree",
    ),
    s(
        "fs",
        "trash",
        "fs_move_to_trash",
        &["file_path"],
        "recoverable delete",
    ),
    s(
        "fs",
        "shred",
        "fs_secure_delete",
        &["file_path"],
        "zero the bytes, then unlink",
    ),
    s(
        "fs",
        "touch",
        "fs_touch",
        &["file_path"],
        "create / bump atime + mtime",
    ),
    s(
        "fs",
        "chmod",
        "fs_chmod",
        &["path", "mode_octal"],
        "set permission bits",
    ),
    s(
        "fs",
        "relink",
        "fs_symlink_retarget",
        &["path", "new_target"],
        "repoint a symlink",
    ),
    s("fs", "home", "fs_home_dir", &[], "what `~` expands to"),
    st(
        "fs",
        "watch",
        "fs_watch",
        &["path"],
        "stream created/modified/removed",
    ),
    st(
        "fs",
        "tail",
        "fs_tail",
        &["path"],
        "stream appended lines (survives rotation)",
    ),
    /* ---- exec, jobs, processes ---- */
    s(
        "",
        "exec",
        "exec",
        &["program", "*args"],
        "run to completion → {code, stdout, stderr}",
    ),
    s("", "ps", "ps", &["filter?"], "processes by memory"),
    s(
        "",
        "kill",
        "kill",
        &["pid", "signal?"],
        "signal a process (term default)",
    ),
    s(
        "",
        "which",
        "which",
        &["program"],
        "resolve a program on $PATH",
    ),
    s(
        "job",
        "start",
        "job_start",
        &["program", "*args"],
        "background job; notifies on completion",
    ),
    s("job", "list", "job_list", &[], "status of every job"),
    s(
        "job",
        "result",
        "job_result",
        &["id"],
        "fetch + remove one finished job",
    ),
    s("job", "poll", "job_poll", &[], "drain every finished job"),
    /* ---- machine + host state ---- */
    s(
        "host",
        "info",
        "hostinfo",
        &[],
        "os, arch, kernel, hostname, cpus, mem, LAN ip",
    ),
    s(
        "host",
        "hello",
        "hello",
        &[],
        "handshake: version + capability tags",
    ),
    s(
        "host",
        "log",
        "hostlog",
        &[],
        "recent host commands (limit=)",
    ),
    s(
        "host",
        "theme",
        "get",
        &[],
        "the shared theme: scheme, ui, palette, schemes",
    ),
    s(
        "host",
        "open",
        "open",
        &["target"],
        "open a path/URL with the OS default handler",
    ),
    s(
        "host",
        "notify",
        "notify",
        &["title", "body?"],
        "notification from the HOST (top-level `notify` is the browser's)",
    ),
    s(
        "host",
        "ping",
        "ping",
        &[],
        "liveness check — the same reply as hello",
    ),
    s("sys", "once", "sysinfo_once", &[], "one system snapshot"),
    st(
        "sys",
        "stream",
        "sysinfo_start",
        &[],
        "stream stats (interval_ms=)",
    ),
    s("sys", "stop", "sysinfo_stop", &[], "stop the stream"),
    st(
        "sys",
        "meter",
        "meter_stream",
        &["path"],
        "stream an audio meter file",
    ),
    /* ---- key/value store ---- */
    s(
        "kv",
        "get",
        "kv_get",
        &["key"],
        "read (app= namespace, default zwire)",
    ),
    s("kv", "set", "kv_set", &["key", "value"], "write"),
    s(
        "kv",
        "merge",
        "kv_merge",
        &["key", "value"],
        "shallow-merge",
    ),
    s("kv", "del", "kv_del", &["key"], "delete"),
    s("kv", "keys", "kv_keys", &[], "list keys"),
    /* ---- lifecycle hooks (stryke scripts bound to browser events) ---- */
    s("hooks", "list", "hooks_list", &[], "every hook"),
    s(
        "hooks",
        "events",
        "hooks_events",
        &[],
        "the bindable events",
    ),
    s(
        "hooks",
        "save",
        "hooks_save",
        &[],
        "create / update: hook={…}",
    ),
    s("hooks", "delete", "hooks_delete", &["id"], "delete one"),
    s(
        "hooks",
        "enable",
        "hooks_set_enabled",
        &["id", "enabled"],
        "enable / disable one",
    ),
    s(
        "hooks",
        "script",
        "hooks_get_script",
        &["id"],
        "read its script",
    ),
    s(
        "hooks",
        "path",
        "hooks_script_path",
        &["id"],
        "where that script lives",
    ),
    s(
        "hooks",
        "setscript",
        "hooks_set_script",
        &["id", "code"],
        "replace its script",
    ),
    s(
        "hooks",
        "test",
        "hooks_test_run",
        &["id"],
        "run it against a sample (sample=)",
    ),
    s(
        "hooks",
        "fire",
        "hook_fire",
        &["event"],
        "fire an event (payload=)",
    ),
    /* ---- stryke ---- */
    s(
        "stryke",
        "run",
        "stryke_run",
        &["code"],
        "run stryke code (stdin=)",
    ),
    s(
        "stryke",
        "lsp",
        "stryke_lsp_start",
        &[],
        "start the language server",
    ),
    s(
        "stryke",
        "send",
        "stryke_lsp_send",
        &[],
        "send it one LSP message",
    ),
    s("stryke", "stop", "stryke_lsp_stop", &[], "stop it"),
    /* ---- the OTHER apps on the bus (suite.rs — the client leg) ---- */
    s(
        "suite",
        "list",
        "suite_list",
        &[],
        "which suite apps are actually running",
    ),
    s(
        "suite",
        "verbs",
        "suite_verbs",
        &["app"],
        "that app's typed surface",
    ),
    s(
        "suite",
        "call",
        "suite_call",
        &["app", "verb", "**args"],
        "invoke one of its verbs",
    ),
    s(
        "suite",
        "get",
        "suite_get",
        &["app", "state"],
        "read one of its state queries",
    ),
    /* ---- pub/sub + peers ---- */
    s(
        "bus",
        "pub",
        "pub",
        &["topic", "data"],
        "fan a message out to every subscriber",
    ),
    st(
        "bus",
        "sub",
        "sub",
        &["topic"],
        "stream a topic (secs= bounds the wait)",
    ),
    s("bus", "unsub", "unsub", &["topic"], "stop receiving it"),
    s("peer", "list", "peers", &[], "linked hosts"),
    s(
        "peer",
        "connect",
        "peer_connect",
        &["addr"],
        "dial another host",
    ),
    s(
        "peer",
        "remote",
        "remote",
        &["peer"],
        "run one request on a peer: request={…}",
    ),
    /* ---- observers ---- */
    s("watch", "list", "watch_list", &[], "active observers"),
    s("watch", "stop", "watch_stop", &["id?"], "stop one"),
    /* ---- PTY ---- */
    st(
        "pty",
        "run",
        "pty_spawn",
        &["run"],
        "spawn a terminal, type one command into it, and stream its output",
    ),
    st(
        "pty",
        "spawn",
        "pty_spawn",
        &[],
        "start a terminal (shell=, cols=, rows=, id=)",
    ),
    s("pty", "write", "pty_write", &["data"], "type into it (id=)"),
    s(
        "pty",
        "resize",
        "pty_resize",
        &["cols", "rows"],
        "resize it",
    ),
    s("pty", "kill", "pty_kill", &[], "kill it"),
    /* ---- the REAL tmux server (tmuxops.rs, over tmux's own wire protocol) ---- */
    s(
        "tmux",
        "status",
        "tmux_status",
        &[],
        "is a server running, and what would start one",
    ),
    s(
        "tmux",
        "tree",
        "tmux_tree",
        &[],
        "sessions → windows → panes",
    ),
    s("tmux", "sessions", "tmux_sessions", &[], "session summary"),
    s("tmux", "panes", "tmux_panes", &[], "every pane"),
    s(
        "tmux",
        "search",
        "tmux_search",
        &[],
        "the searchable pane corpus",
    ),
    s(
        "tmux",
        "capture",
        "tmux_capture",
        &["pane"],
        "a pane's visible text",
    ),
    s(
        "tmux",
        "focus",
        "tmux_focus",
        &["session", "window?", "pane?"],
        "focus a session/window/pane",
    ),
    s(
        "tmux",
        "send",
        "tmux_send",
        &["text", "*panes"],
        "send keys to panes (enter=false to hold)",
    ),
    s(
        "tmux",
        "run",
        "tmux_run",
        &["*args"],
        "fire-and-forget tmux command line",
    ),
    s(
        "tmux",
        "command",
        "tmux_command",
        &["*args"],
        "run one and wait: {code, stdout, stderr}",
    ),
    s(
        "tmux",
        "sync",
        "tmux_sync",
        &["window", "on?"],
        "synchronize-panes on a window",
    ),
    s(
        "tmux",
        "broadcast",
        "tmux_broadcast_list",
        &[],
        "windows with sync on",
    ),
    s(
        "tmux",
        "options",
        "tmux_options",
        &[],
        "server/session options",
    ),
    s(
        "tmux",
        "setopt",
        "tmux_set_option",
        &["scope", "name", "value"],
        "set one option",
    ),
    s("tmux", "keys", "tmux_keys", &[], "the key tables"),
    s(
        "tmux",
        "setkey",
        "tmux_set_key",
        &["table", "key", "command"],
        "bind a key",
    ),
    s(
        "tmux",
        "unbindkey",
        "tmux_unbind_key",
        &["table", "key"],
        "unbind a key",
    ),
    s("tmux", "buffers", "tmux_buffers", &[], "paste buffers"),
    s(
        "tmux",
        "buffer",
        "tmux_buffer",
        &["name"],
        "one buffer's contents",
    ),
    s(
        "tmux",
        "setbuffer",
        "tmux_set_buffer",
        &["name", "content"],
        "write a buffer",
    ),
    s(
        "tmux",
        "delbuffer",
        "tmux_delete_buffer",
        &["name"],
        "delete a buffer",
    ),
    s(
        "tmux",
        "paste",
        "tmux_paste_buffer",
        &["name", "pane"],
        "paste a buffer into a pane",
    ),
    s(
        "tmux",
        "export",
        "tmux_export_state",
        &[],
        "the whole server state",
    ),
    s(
        "tmux",
        "import",
        "tmux_import_state",
        &[],
        "restore it: state={…}",
    ),
    s("tmux", "snaps", "tmux_snap_list", &[], "saved sessions"),
    s(
        "tmux",
        "snap",
        "tmux_snap_detail",
        &["name"],
        "one saved session",
    ),
    s(
        "tmux",
        "save",
        "tmux_snap_save",
        &["name"],
        "save one (contents=true keeps pane text)",
    ),
    s(
        "tmux",
        "restore",
        "tmux_snap_restore",
        &["name"],
        "restore one (relaunch=true reruns programs)",
    ),
    s(
        "tmux",
        "rename",
        "tmux_snap_rename",
        &["name", "new_name"],
        "rename one",
    ),
    s(
        "tmux",
        "rmsnap",
        "tmux_snap_delete",
        &["name"],
        "delete one",
    ),
    s(
        "tmux",
        "hud",
        "browser.tmux",
        &["sub"],
        "the browser's tmux overlay: navigate / split / zoom",
    ),
    /* ---- odds and ends that earn a top-level name ---- */
    s(
        "",
        "screenshot",
        "browser.screenshot",
        &[],
        "capture the visible tab",
    ),
    s(
        "",
        "notify",
        "browser.notify",
        &["title", "message?"],
        "desktop notification from the browser",
    ),
    s(
        "",
        "lang",
        "browser.detectLanguage",
        &[],
        "the active tab's detected language",
    ),
];

/// The built-in colour schemes. h: `store::SCHEMES`. `custom` / `custom-N` are
/// also accepted by the host (a custom scheme carries its colours in the shared
/// palette).
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

/// The predicates `page assert` / `page witness` accept. h: `page::ASSERT_OPS`.
const ASSERT_OPS: &[&str] = &[
    "contains",
    "not_contains",
    "equals",
    "empty",
    "nonempty",
    "count_at_least",
    "count_at_most",
];

/// Commands handled in code rather than by a [`SUBS`] row, because each does
/// something the generic path cannot: several requests, a different reply shape,
/// or a loop over its arguments.
const SPECIAL: &[(&str, &str)] = &[
    (
        "status",
        "host + browser state (the default with no arguments)",
    ),
    ("scheme", "print or set the colour scheme"),
    ("schemes", "list the colour schemes"),
    ("light", "switch the HUD to its light variant"),
    ("dark", "switch the HUD to its dark variant"),
    ("toggle", "flip light/dark"),
    ("open", "open URLs in new tabs"),
    ("goto", "navigate the active tab to a URL"),
    ("back", "go back"),
    ("forward", "go forward"),
    (
        "reload",
        "reload the active tab (`reload hard` bypasses the cache)",
    ),
    ("clip", "read or write the clipboard"),
    (
        "verbs",
        "the automation surface, with each verb's reversibility class",
    ),
    ("call", "call any verb: zwire call VERB k=v …"),
    ("raw", "send one raw host command (JSON)"),
    ("audit", "surface verbs this plugin does not name"),
    ("help", "this text, or one group's commands"),
];

/* --------------------------------------------------------------- transport */

/// Where zwire-host's NDJSON daemon listens. h: `zwire_host::default_socket` —
/// `$ZWIRE_HOST_SOCK` wins everywhere; otherwise a *runtime* dir, never the
/// persistent state dir.
fn socket_path(host: &Host) -> PathBuf {
    if let Some(p) = cfg(host, "ZWIRE_HOST_SOCK") {
        return PathBuf::from(p);
    }
    if let Some(rt) = cfg(host, "XDG_RUNTIME_DIR") {
        return PathBuf::from(rt).join("zwire-host.sock");
    }
    if let Some(tmp) = cfg(host, "TMPDIR") {
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

/// How long a streaming command prints frames before returning. `secs=` on the
/// command line wins, then `$ZWIRE_STREAM_SECS`, then 30.
///
/// Bounded on purpose: a builtin runs in the shell's own process, so an
/// unbounded read would hand the terminal to a loop the user can only leave by
/// killing the shell.
fn stream_secs(host: &Host, args: &Value) -> u64 {
    args.get("secs")
        .and_then(Value::as_u64)
        .or_else(|| cfg(host, "ZWIRE_STREAM_SECS").and_then(|s| s.parse().ok()))
        .filter(|&n| n > 0)
        .unwrap_or(30)
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
    let stream = open_socket(sock, msg, ms)?;
    read_reply(BufReader::new(stream).lines().map_while(Result::ok), id)
}

/// Connect, write the request line, and hand back the stream for reading.
#[cfg(unix)]
fn open_socket(
    sock: &std::path::Path,
    msg: &Value,
    ms: u64,
) -> Result<std::os::unix::net::UnixStream, String> {
    use std::os::unix::net::UnixStream;
    let stream =
        UnixStream::connect(sock).map_err(|e| format!("connect {}: {e}", sock.display()))?;
    let wait = Duration::from_millis(ms);
    let _ = stream.set_read_timeout(Some(wait));
    let _ = stream.set_write_timeout(Some(wait));
    send_on(&stream, msg)?;
    Ok(stream)
}

/// Write one more message on an open socket — what a multi-frame conversation
/// (`pty run`: spawn, then type) needs.
#[cfg(unix)]
fn send_on(stream: &std::os::unix::net::UnixStream, msg: &Value) -> Result<(), String> {
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    let mut line = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    line.push(b'\n');
    w.write_all(&line).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
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

/// Run a streaming command on ONE connection: send every message in `msgs`, then
/// print each frame that comes back, one compact JSON object per line, until
/// `secs` elapses or the host hangs up.
///
/// Several messages, one connection, on purpose. A stream, a subscription and a
/// PTY all belong to the connection that asked for them (h: `zbus::serve_conn`
/// gives each bus connection its own `Session`, dropped when it hangs up), so
/// `pty run` — spawn, then type into it, then read the output — is only possible
/// while the socket stays open. A second `zwire` invocation is a second session
/// and would find no terminal there.
///
/// Any endpoint serves this: the bus wraps a stream frame as
/// `{"t":"event","value":…}` and its reply as `{"t":"reply",…,"value":…}`, both
/// of which are unwrapped here so the output is the same on every transport.
fn stream(host: &Host, msgs: Vec<Value>, secs: u64) -> c_int {
    #[cfg(unix)]
    {
        // Streams take the NDJSON daemon first when there is one. Every endpoint
        // serves them on a current host, but the browser-attached process is
        // whatever binary the app bundles, which may be older than the bus's
        // session rework — the daemon is the one the user started deliberately.
        let mut order = vec![(socket_path(host), false)];
        order.extend(
            endpoints(host)
                .into_iter()
                .filter(|(p, _)| *p != socket_path(host)),
        );
        let Some((path, is_bus)) = order.into_iter().find(|(p, _)| p.exists()) else {
            return fail("no zwire-host endpoint: is the browser running?");
        };
        let framed: Vec<Value> = msgs
            .into_iter()
            .map(|mut m| {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                if let Some(o) = m.as_object_mut() {
                    o.insert("id".into(), json!(id));
                }
                if is_bus {
                    bus_frame(&m, id)
                } else {
                    m
                }
            })
            .collect();
        let Some((first, rest)) = framed.split_first() else {
            return 0;
        };
        // A tick short enough to notice the deadline, long enough not to spin.
        let sock = match open_socket(&path, first, 1000) {
            Ok(s) => s,
            Err(e) => return fail(&e),
        };
        for m in rest {
            if let Err(e) = send_on(&sock, m) {
                return fail(&e);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(secs);
        let mut lines = BufReader::new(sock).lines();
        while Instant::now() < deadline {
            match lines.next() {
                Some(Ok(line)) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let frame = match serde_json::from_str::<Value>(line) {
                        Ok(v) => unwrap_frame(v),
                        Err(_) => continue,
                    };
                    // Terminal output is bytes that happen to have crossed a JSON
                    // pipe: printing the base64 would be printing the envelope.
                    if frame["ev"] == json!("output") {
                        if let Some(text) = b64_text(&frame) {
                            host.print(&text);
                            continue;
                        }
                    }
                    host.print(&format!("{frame}\n"));
                }
                // A read timeout ends the iterator; the deadline decides whether
                // that is "quiet" or "done".
                Some(Err(_)) | None => continue,
            }
        }
        0
    }
    #[cfg(not(unix))]
    {
        let _ = (host, msgs, secs);
        fail("streaming is not supported on this platform")
    }
}

/// The `b64` field of a frame, decoded, when it is valid UTF-8 — PTY output is
/// the shell's own bytes and belongs on the terminal as text.
fn b64_text(frame: &Value) -> Option<String> {
    use base64::Engine;
    let b64 = frame.get("b64")?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    String::from_utf8(bytes).ok()
}

/// A bus frame's payload, or the message itself on the daemon — so a stream
/// prints the same lines whichever endpoint served it.
fn unwrap_frame(v: Value) -> Value {
    match v.get("t").and_then(Value::as_str) {
        Some("event") | Some("reply") => v.get("value").cloned().unwrap_or(v),
        _ => v,
    }
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

/// Run one request and report it: the named field (or the whole reply).
fn run(host: &Host, msg: Value, field: Option<&str>) -> c_int {
    match request(host, msg) {
        Err(e) => fail(&e),
        Ok(v) => {
            if let Some(e) = reply_err(&v) {
                return fail(&e);
            }
            match field {
                Some(f) => show(host, v.get(f).unwrap_or(&Value::Null)),
                None => show(host, &present(v)),
            }
            0
        }
    }
}

/// A reply as a shell wants to read it.
///
/// Drops the envelope the protocol needs and the reader does not: `id` is this
/// request's correlation number, and `ok` was already checked — printing either
/// makes every answer something to pipe through `jq` before it is usable. A
/// reply carrying exactly one field after that IS its answer (`which` → path,
/// `kv_get` → value, `fs_home_dir` → data), so it is unwrapped; anything with
/// more than one field is printed whole, because choosing among them would be
/// guessing.
fn present(v: Value) -> Value {
    let Some(o) = v.as_object() else { return v };
    let rest: Vec<(&String, &Value)> = o
        .iter()
        .filter(|(k, _)| k.as_str() != "ok" && k.as_str() != "id")
        .collect();
    match rest.as_slice() {
        [(_, only)] => (*only).clone(),
        _ => {
            let mut m = o.clone();
            m.remove("id");
            m.remove("ok");
            Value::Object(m)
        }
    }
}

/* --------------------------------------------------------------- arguments */

/// `k=v` → a JSON pair. The value parses as JSON when it can (`3`, `true`,
/// `[1,2]`, `"x"`), so `zwire call browser.gotoTab index=3` sends a number, and
/// stays a plain string otherwise — a URL is not valid JSON and must not be
/// mangled into one.
fn kv(arg: &str) -> Option<(String, Value)> {
    let (k, v) = arg.split_once('=')?;
    if k.is_empty() {
        return None;
    }
    Some((k.to_string(), scalar(v)))
}

/// One command-line word as a JSON value: parsed when it is valid JSON, a string
/// otherwise. `3` is a number, `true` a bool, `https://…` a string.
fn scalar(word: &str) -> Value {
    serde_json::from_str::<Value>(word).unwrap_or_else(|_| json!(word))
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

/// Bind a row's positional arguments, then merge whatever `k=v` words are left.
///
/// Positionals are taken from the FRONT, in order, before anything is treated as
/// `k=v` — otherwise a value that happens to contain `=` (a URL with a query
/// string, `fs write x 'a=b'`) would be read as a key.
fn bind(spec: &[&str], rest: &[String]) -> Result<Value, String> {
    let mut o = serde_json::Map::new();
    let mut i = 0;
    for name in spec {
        if let Some(rest_name) = name.strip_prefix("**") {
            // Everything left is the CALLED app's arguments, nested so they
            // cannot collide with this command's own fields.
            let inner = kv_args(&rest[i..])?;
            o.insert(rest_name.to_string(), inner);
            i = rest.len();
            continue;
        }
        if let Some(list_name) = name.strip_prefix('*') {
            let items: Vec<Value> = rest[i..].iter().map(|w| json!(w)).collect();
            o.insert(list_name.to_string(), Value::Array(items));
            i = rest.len();
            continue;
        }
        let (key, optional) = match name.strip_suffix('?') {
            Some(k) => (k, true),
            None => (*name, false),
        };
        match rest.get(i) {
            Some(w) => {
                o.insert(key.to_string(), scalar(w));
                i += 1;
            }
            None if optional => {}
            None => return Err(format!("missing argument: {key}")),
        }
    }
    for a in &rest[i.min(rest.len())..] {
        let (k, v) = kv(a).ok_or_else(|| format!("not a k=v argument: {a}"))?;
        o.insert(k, v);
    }
    Ok(Value::Object(o))
}

/* -------------------------------------------------------------- dispatching */

/// The row for `zwire <group> <name>`, if there is one.
fn find(group: &str, name: &str) -> Option<&'static Sub> {
    SUBS.iter().find(|s| s.group == group && s.name == name)
}

/// Every group name, in table order.
fn groups() -> Vec<&'static str> {
    let mut seen: Vec<&str> = Vec::new();
    for s in SUBS {
        if !s.group.is_empty() && !seen.contains(&s.group) {
            seen.push(s.group);
        }
    }
    seen
}

/// The names inside one group.
fn names(group: &str) -> Vec<&'static str> {
    SUBS.iter()
        .filter(|s| s.group == group)
        .map(|s| s.name)
        .collect()
}

/// Send one row.
///
/// The three verb families have three different reply shapes and this is the one
/// place that knows it: a `browser.*` verb answers with a delivery receipt (the
/// browser result never comes back over the native port), a `page.*` verb nests
/// its answer under `result.value`, and a host command answers as itself.
fn dispatch(host: &Host, sub: &Sub, rest: &[String]) -> c_int {
    let args = match bind(sub.args, rest) {
        Ok(a) => a,
        Err(e) => return fail(&format!("{} {}: {e}", sub.group, sub.name)),
    };
    if sub.stream {
        let secs = stream_secs(host, &args);
        let mut msg = args;
        let mut extra = Vec::new();
        if let Some(o) = msg.as_object_mut() {
            o.remove("secs");
            o.insert("cmd".into(), json!(sub.verb));
            // `pty run CMD` is spawn-then-type on ONE connection, because the
            // terminal belongs to that connection's session and a second
            // invocation would find nothing to type into.
            if let Some(cmd) = o.remove("run").map(|v| match v {
                Value::String(s) => s,
                other => other.to_string(),
            }) {
                extra.push(json!({ "cmd": "pty_write", "data": format!("{cmd}\n") }));
            }
        }
        let mut msgs = vec![msg];
        msgs.extend(extra);
        return stream(host, msgs, secs);
    }
    if sub.verb.starts_with("browser.") {
        return call_verb(host, sub.verb, args, true);
    }
    if sub.verb.starts_with("page.") {
        return page_read(host, sub.verb, args);
    }
    let mut msg = args;
    if let Some(o) = msg.as_object_mut() {
        o.insert("cmd".into(), json!(sub.verb));
    }
    run(host, msg, None)
}

/// Call one automation verb. h: `session::handle_cmd` `"call"` →
/// `zbus::call_verb`, which forwards a `browser.*` verb to the HUD worker.
fn call_verb(host: &Host, verb: &str, args: Value, quiet: bool) -> c_int {
    // `txn` rides at the TOP level of the frame, not inside `args` — both the
    // `call` host command and the bus's `call` frame read it there (h:
    // `session::handle_cmd` `msg["txn"]`, `zbus::serve_conn`). A user types it as
    // one more `k=v`, so lift it, or the step would run outside the transaction
    // it named and never be journaled.
    let txn = args.get("txn").cloned();
    let mut msg = json!({ "cmd": "call", "verb": verb, "args": args });
    if let (Some(t), Some(o)) = (txn, msg.as_object_mut()) {
        o.insert("txn".into(), t);
    }
    match request(host, msg) {
        Err(e) => fail(&e),
        Ok(v) => {
            if let Some(e) = reply_err(&v) {
                return fail(&e);
            }
            let result = v.get("result").cloned().unwrap_or(Value::Null);
            // A `browser.*` reply is a delivery receipt, not a browser result:
            // printing it on every `zwire tab new` would be noise. Structured
            // results (a host command, a page read) still print.
            let receipt = result.get("action").is_some() && result.get("queued").is_some();
            if !quiet && !receipt {
                show(host, &result);
            }
            if receipt && result.get("delivered") == Some(&json!(0)) {
                // The action was journaled but no browser subscriber saw it —
                // reporting success there would be a lie.
                return fail("no zwire browser attached");
            }
            0
        }
    }
}

/// A `page.*` projection.
///
/// Sent as a `call` on the projection itself, never as `page_get`: the daemon
/// answers a page read on its own thread (h: `session::spawn_page_reply`), and
/// the bus's in-process dispatcher reads only the FIRST line a `Session` wrote —
/// so `page_get` over the bus comes back "no reply from host session" while the
/// answer is still in flight. `page.*` as a verb routes straight to
/// `page::request` on both transports.
fn page_read(host: &Host, verb: &str, args: Value) -> c_int {
    match request(host, json!({ "cmd": "call", "verb": verb, "args": args })) {
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
            match r.get("value") {
                Some(val) => show(host, val),
                // `assert` / `witness` answer with a verdict, not a value.
                None => show(host, &present(r.clone())),
            }
            0
        }
    }
}

/* ------------------------------------------------------ the special commands */

fn help(host: &Host, rest: &[String]) -> c_int {
    // `help GROUP` — one group, with what each command does.
    if let Some(g) = rest.first() {
        let rows: Vec<&Sub> = SUBS.iter().filter(|s| s.group == g.as_str()).collect();
        if rows.is_empty() {
            return fail(&format!("no such group: {g}"));
        }
        let mut out = String::new();
        for r in rows {
            let args = r.args.join(" ");
            out.push_str(&format!("  {:<12}{:<28}{}\n", r.name, args, r.doc));
        }
        host.print(&out);
        return 0;
    }
    let mut s = String::from("usage: zwire <command> [args]\n\n");
    for (name, doc) in SPECIAL {
        s.push_str(&format!("  {name:<12}{doc}\n"));
    }
    for r in SUBS.iter().filter(|s| s.group.is_empty()) {
        s.push_str(&format!("  {:<12}{}\n", r.name, r.doc));
    }
    s.push_str("\ngroups (zwire help GROUP):\n");
    for g in groups() {
        s.push_str(&format!("  {:<12}{}\n", g, names(g).join(" ")));
    }
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
    let f = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("-").to_string();
    let th = theme(host).unwrap_or(Value::Null);
    let light = th
        .get("ui")
        .and_then(|u| u.get("light"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let attached = bus_path(host, "zwire-page").exists();
    host.print(&format!(
        "host      {} {}\nscheme    {} ({})\nbrowser   {}\nendpoint  {}\n",
        f(&hello, "host"),
        f(&hello, "version"),
        f(&th, "scheme"),
        if light { "light" } else { "dark" },
        if attached { "attached" } else { "not attached" },
        endpoint_path(host).display(),
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
/// verb), so the retry is only meaningful on the daemon socket.
fn theme_write(host: &Host, fields: Value, field: &str) -> c_int {
    let mut msg = fields.clone();
    if let Some(o) = msg.as_object_mut() {
        o.insert("cmd".into(), json!("theme"));
    }
    match request(host, msg) {
        Err(e) => fail(&e),
        Ok(v) => {
            if v.get("err").and_then(Value::as_str) == Some("unknown_cmd") {
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

/// The published automation surface, as `{id: rev}` rows.
fn surface(host: &Host) -> Result<Vec<Value>, String> {
    let v = request(host, json!({ "cmd": "verbs" }))?;
    let list = v
        .get("verbs")
        .or_else(|| v.get("result").and_then(|r| r.get("verbs")))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if list.is_empty() {
        return Err("no automation surface in the reply".into());
    }
    Ok(list)
}

/// `zwire verbs [PATTERN]` — one `id rev` per line. h: `zbus::surface`.
fn verbs(host: &Host, rest: &[String]) -> c_int {
    let filter = rest.first().map(|s| s.to_ascii_lowercase());
    match surface(host) {
        Err(e) => fail(&e),
        Ok(list) => {
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

/// `zwire audit` — every verb the RUNNING host publishes that no [`SUBS`] row
/// names, and every row whose verb the host does not publish.
///
/// The point is that this table cannot quietly fall behind: the answer comes
/// from the host that is actually running, not from a second copy of its list
/// kept here. Exits non-zero when either side has something the other does not.
fn audit(host: &Host) -> c_int {
    let list = match surface(host) {
        Ok(l) => l,
        Err(e) => return fail(&e),
    };
    let published: Vec<String> = list
        .iter()
        .filter_map(|e| e.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let mut out = String::new();
    let mut bad = false;

    for id in &published {
        // `page_reply` / `page_serve` are the browser's own half of the page
        // protocol — a shell has nothing to answer with, so they are excluded
        // deliberately rather than missing.
        if matches!(id.as_str(), "page_reply" | "page_serve" | "page_get") {
            continue;
        }
        if !SUBS.iter().any(|s| s.verb == id) && !named_by_special(id) {
            out.push_str(&format!("unnamed   {id}\n"));
            bad = true;
        }
    }
    for s in SUBS {
        if !published.iter().any(|p| p == s.verb) {
            out.push_str(&format!("stale     {} {} → {}\n", s.group, s.name, s.verb));
            bad = true;
        }
    }
    if out.is_empty() {
        host.print(&format!(
            "{} verbs published, all reachable by name\n",
            published.len()
        ));
        return 0;
    }
    host.print(&out);
    c_int::from(bad)
}

/// Verbs a special command covers, so `audit` does not report them unnamed.
fn named_by_special(verb: &str) -> bool {
    matches!(
        verb,
        "hello"
            | "get"
            | "theme"
            | "verbs"
            | "clipboard_get"
            | "clipboard_set"
            | "browser.open"
            | "browser.openTab"
            | "browser.goBack"
            | "browser.goForward"
            | "browser.reload"
            | "browser.reloadHard"
    )
}

/* -------------------------------------------------------------- the builtin */

fn zwire(host: &Host, args: &Args) -> c_int {
    let rest = args.rest();
    let Some(cmd) = rest.first().map(|s| s.as_str()) else {
        return status(host);
    };
    let tail = &rest[1..];
    match cmd {
        "help" | "-h" | "--help" => help(host, tail),
        "status" => status(host),
        "audit" => audit(host),

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

        // A top-level row, then a group.
        other => {
            if let Some(sub) = find("", other) {
                return dispatch(host, sub, tail);
            }
            if !names(other).is_empty() {
                let Some(name) = tail.first() else {
                    host.print(&format!("{}\n", names(other).join(" ")));
                    return 0;
                };
                return match find(other, name) {
                    Some(sub) => dispatch(host, sub, &tail[1..]),
                    None => fail(&format!("unknown {other} command: {name}")),
                };
            }
            fail(&format!("unknown command: {other}"))
        }
    }
}

/* ------------------------------------------------------------- completions */

/// Native completion. The word list is `[CURRENT, word0, word1, …]` — see
/// PLUGINS.md — so `words[1]` is the first argument and `current` says which
/// word is being completed.
fn zwire_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();
    let current: usize = a.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let words = &a[1..];
    let prefix = current
        .checked_sub(1)
        .and_then(|i| words.get(i))
        .map(String::as_str)
        .unwrap_or("");
    let first = words.get(1).map(String::as_str).unwrap_or("");
    let second = words.get(2).map(String::as_str).unwrap_or("");

    let emit = |cands: Vec<&str>| {
        for c in cands {
            if c.starts_with(prefix) {
                host.add_match(c);
            }
        }
    };

    // Word 2 is the command or group; later words complete per command.
    if current <= 2 {
        let mut c: Vec<&str> = SPECIAL.iter().map(|(k, _)| *k).collect();
        c.extend(names(""));
        c.extend(groups());
        emit(c);
        return 0;
    }
    if current == 3 {
        match first {
            "scheme" => emit(SCHEMES.to_vec()),
            "help" => emit(groups()),
            "reload" => emit(vec!["hard"]),
            // The live surface, asked of the running host — so a verb added to
            // zwire-host completes here the day it ships, with no table to update.
            "call" | "verbs" => emit_surface(host, prefix),
            g => emit(names(g)),
        }
        return 0;
    }
    // Word 4+: the arguments of a known row.
    match (first, second) {
        ("page", "assert") | ("page", "witness") => {
            if current == 4 {
                emit(
                    SUBS.iter()
                        .filter(|s| s.group == "page" && s.verb.starts_with("page."))
                        .map(|s| s.verb)
                        .collect(),
                );
            } else if current == 5 {
                emit(ASSERT_OPS.to_vec());
            }
        }
        ("suite", "verbs") | ("suite", "call") | ("suite", "get") => {
            if current == 4 {
                emit_apps(host, prefix);
            }
        }
        ("tmux", "hud") => emit(vec!["navigate", "split", "zoom"]),
        _ => {}
    }
    0
}

/// Candidates from the running host's surface.
fn emit_surface(host: &Host, prefix: &str) {
    if let Ok(list) = surface(host) {
        for e in &list {
            if let Some(id) = e.get("id").and_then(Value::as_str) {
                if id.starts_with(prefix) {
                    host.add_match(id);
                }
            }
        }
    }
}

/// Candidates from the suite apps that are actually running. h: `suite_list`
/// proves each one by dialing it, so this never offers a dead socket.
fn emit_apps(host: &Host, prefix: &str) {
    let Ok(v) = request(host, json!({ "cmd": "suite_list" })) else {
        return;
    };
    let apps = v
        .get("apps")
        .or_else(|| v.get("result").and_then(|r| r.get("apps")))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for a in &apps {
        let name = a.as_str().or_else(|| a.get("app").and_then(Value::as_str));
        if let Some(n) = name {
            if n.starts_with(prefix) {
                host.add_match(n);
            }
        }
    }
}

declare_plugin! {
    name: "zwire",
    version: "0.3.1",
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
    fn positionals_bind_before_anything_is_read_as_a_key() {
        let spec = &["path", "text"];
        let rest = words(&["/tmp/x", "a=b", "mode=644"]);
        let got = bind(spec, &rest).unwrap();
        // `a=b` is the file's CONTENT, not a key: it was still positional.
        assert_eq!(got["path"], json!("/tmp/x"));
        assert_eq!(got["text"], json!("a=b"));
        // Only what is left over is `k=v`.
        assert_eq!(got["mode"], json!(644));
    }

    #[test]
    fn optional_positionals_may_be_absent_and_required_ones_may_not() {
        assert_eq!(
            bind(&["pid", "signal?"], &words(&["991"])).unwrap(),
            json!({"pid": 991})
        );
        assert_eq!(
            bind(&["pid", "signal?"], &words(&["991", "kill"])).unwrap(),
            json!({"pid": 991, "signal": "kill"})
        );
        assert!(bind(&["pid"], &[])
            .unwrap_err()
            .contains("missing argument"));
    }

    #[test]
    fn star_collects_a_list_and_double_star_a_nested_object() {
        let got = bind(&["program", "*args"], &words(&["git", "status", "-s"])).unwrap();
        assert_eq!(got, json!({"program": "git", "args": ["status", "-s"]}));
        // A list keeps its words as STRINGS: `git log -n 5` must not send 5 as a
        // number to a program that takes argv.
        let n = bind(&["program", "*args"], &words(&["git", "-n", "5"])).unwrap();
        assert_eq!(n["args"], json!(["-n", "5"]));

        let call = bind(
            &["app", "verb", "**args"],
            &words(&["zcite", "item.add", "title=X", "year=1999"]),
        )
        .unwrap();
        assert_eq!(
            call,
            json!({"app": "zcite", "verb": "item.add", "args": {"title": "X", "year": 1999}})
        );
    }

    #[test]
    fn every_row_is_addressable_and_unique() {
        for s in SUBS {
            assert!(!s.name.is_empty() && !s.verb.is_empty(), "empty row");
            assert!(!s.doc.is_empty(), "{} {} has no doc", s.group, s.name);
            assert_eq!(
                find(s.group, s.name).map(|r| r.verb),
                Some(s.verb),
                "{} {} is shadowed by an earlier row",
                s.group,
                s.name
            );
        }
        // A group name must not collide with a top-level command, or the
        // dispatcher would never reach the group.
        for g in groups() {
            assert!(find("", g).is_none(), "group {g} shadows a top-level row");
            assert!(
                !SPECIAL.iter().any(|(k, _)| *k == g),
                "group {g} shadows a special command"
            );
        }
        for s in SUBS.iter().filter(|s| s.group.is_empty()) {
            assert!(
                !SPECIAL.iter().any(|(k, _)| *k == s.name),
                "{} is both a row and a special command",
                s.name
            );
        }
    }

    #[test]
    fn arg_specs_are_well_formed() {
        for s in SUBS {
            let mut seen_rest = false;
            for a in s.args {
                assert!(!seen_rest, "{} {}: nothing may follow {a}", s.group, s.name);
                if a.starts_with('*') {
                    seen_rest = true;
                }
                let bare = a.trim_start_matches('*').trim_end_matches('?');
                assert!(
                    !bare.is_empty() && bare.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                    "{} {}: odd argument name {a}",
                    s.group,
                    s.name
                );
            }
        }
    }

    /// The table's verbs must be spelled the way the host publishes them: a typo
    /// in a `browser.*` action is silently ignored by the service worker, and a
    /// typo in a host command comes back as `unknown_cmd` at the worst moment.
    #[test]
    fn verb_names_match_the_hosts_spelling_conventions() {
        for s in SUBS {
            if let Some(action) = s.verb.strip_prefix("browser.") {
                assert!(
                    action.chars().all(|c| c.is_ascii_alphanumeric())
                        && action.starts_with(|c: char| c.is_ascii_lowercase()),
                    "{} is not a camelCase browser action",
                    s.verb
                );
            } else if let Some(state) = s.verb.strip_prefix("page.") {
                assert!(
                    state.chars().all(|c| c.is_ascii_lowercase()),
                    "{} is not a page projection",
                    s.verb
                );
            } else {
                assert!(
                    s.verb
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit()),
                    "{} is not a host command name",
                    s.verb
                );
            }
        }
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
        let r = from_bus_reply(
            &json!({"cmd": "call", "verb": "page.url"}),
            json!({"t": "reply", "ok": true, "value": {"ok": true, "value": "u"}}),
        );
        assert_eq!(r, json!({"ok": true, "result": {"ok": true, "value": "u"}}));
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
        let only_events = [r#"{"ev":"pub"}"#].iter().map(|s| s.to_string());
        assert!(read_reply(only_events, 1).is_err());
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

    /// Every family the bus publishes is represented, so "everything from the
    /// bus" is checkable here and not only against a running host (`zwire audit`
    /// is the live half of the same claim).
    #[test]
    fn the_table_covers_every_command_family() {
        for prefix in [
            "browser.", "page.", "fs_", "tmux_", "kv_", "hooks_", "job_", "suite_", "pty_",
            "sysinfo", "txn_", "stryke", "peer", "watch_",
        ] {
            assert!(
                SUBS.iter().any(|s| s.verb.starts_with(prefix)),
                "no command reaches the {prefix}* family"
            );
        }
    }

    fn words(w: &[&str]) -> Vec<String> {
        w.iter().map(|s| s.to_string()).collect()
    }
}
