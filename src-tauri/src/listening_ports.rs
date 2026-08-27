// listening_ports — hover-pulled tooltip metadata (v1.0 Phase 15 / #42).
//
// Deep module: a tiny pure surface answering "which TCP ports does THIS
// panel's process tree listen on?" hiding three OS-specific enumeration
// mechanisms (Linux /proc tables + fd scan, macOS lsof, Windows netstat +
// CIM parent map). Queried ONLY on sidebar-tab hover by the frontend —
// never on a timer, zero background cost while unused.
//
// Assumptions encoded by these tests:
//  - Input shapes: listeners is (port, pid) pairs from any per-OS source;
//    parents is (pid, parent_pid) edges covering the live processes
//    (gracefully incomplete — unreachable ancestors just end the walk).
//  - A listener BELONGS to a tab when its owning pid is the tab's shell
//    (root), or any descendant of it — direct children AND grandchildren
//    count (a dev server started via npm/virtualenv is usually deeper).
//  - A pid listening on several sockets, or one port bound on both IPv4 and
//    IPv6 (or several interface addresses), yields that port ONCE.
//  - Output: ascending sorted Vec<u16>, deduplicated by port number
//    (issue #42: dedupe by port, never distinguish v4-vs-v6).
//  - Ports stay separated per tab: matching happens against ONE root at a
//    time, so panel A's server can never leak into panel B's list.
//  - NOT tested here: the per-OS fixtures live with their parsers below;
//    PtyService child-pid lookup and the Tauri command wiring live in
//    lib.rs; frontend hover/tooltip behavior lives in Vitest (src/tabPorts).
//
// The empty answer (root gone / nothing listens) is an EMPTY vector — the
// UI renders the explicit "No listening ports" state so silence is never
// ambiguous.

use std::collections::{HashMap, HashSet};

/// Ports owned by `root`'s process tree among `listeners`, ascending,
/// deduplicated. One call answers one tab.
pub fn ports_for_root(root: u32, listeners: &[(u16, u32)], parents: &[(u32, u32)]) -> Vec<u16> {
    let tree_pids = descendants_of(root, parents);
    let mut ports: Vec<u16> = listeners
        .iter()
        .filter(|(_, pid)| tree_pids.contains(pid))
        .map(|(port, _)| *port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// One TAB's answer: the union of `ports_for_root` over every panel shell in
/// that tab (`roots` — shells whose pid is gone just contribute nothing),
/// sorted and deduplicated as a whole.
pub fn aggregate_ports(listeners: &[(u16, u32)], parents: &[(u32, u32)], roots: &[u32]) -> Vec<u16> {
    let mut ports: Vec<u16> = roots
        .iter()
        .flat_map(|&root| ports_for_root(root, listeners, parents))
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Every descendant of `root` (children, grandchildren, …), EXCLUDING root
/// itself unless some edge reaches back to it — ownership comes from the
/// tree shape alone.
pub fn descendants_of(root: u32, parents: &[(u32, u32)]) -> HashSet<u32> {
    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(pid, ppid) in parents {
        children_of.entry(ppid).or_default().push(pid);
    }
    let mut found = HashSet::new();
    let mut queue = vec![root];
    while let Some(p) = queue.pop() {
        if let Some(children) = children_of.get(&p) {
            for &c in children {
                // Mark before enqueuing so cycles cannot loop forever.
                if found.insert(c) {
                    queue.push(c);
                }
            }
        }
    }
    found
}

/// Linux step 1: LISTEN rows of a `/proc/net/tcp` or `/proc/net/tcp6`
/// table as (inode, local port). Both tables share one line format; state
/// `0A` is LISTEN. Ports and inodes are hex (the kernel prints neither in
/// decimal).
pub fn parse_linux_listening(table: &str) -> Vec<(u64, u16)> {
    table
        .lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 10 || !cols[0].ends_with(':') {
                return None; // header / blank
            }
            if cols[3] != "0A" {
                return None; // not LISTEN
            }
            let port = u16::from_str_radix(cols[1].rsplit(':').next()?, 16).ok()?;
            let inode = cols[9].parse::<u64>().ok()?;
            Some((inode, port))
        })
        .collect()
}

/// Linux step 2: the target string of a `/proc/<pid>/fd/<n>` symlink.
/// Only `socket:[<decimal inode>]` names a socket — everything else is not
/// a candidate owner and yields None.
pub fn fd_inode(link_target: &str) -> Option<u64> {
    link_target
        .strip_prefix("socket:[")?
        .strip_suffix(']')?
        .parse()
        .ok()
}

/// macOS LISTEN table, straight from `lsof -nP -iTCP -sTCP:LISTEN`: one row
/// per listening socket as (port, pid). The NAME column's trailing port (the
/// text after the LAST colon) is the listen port; wildcards arrive as
/// `*:5173`, `[::]:3000`, or `localhost:8000`. Non-LISTEN rows and anything
/// without a numeric pid/port are skipped.
pub fn parse_lsof_listen(output: &str) -> Vec<(u16, u32)> {
    output
        .lines()
        .filter(|l| l.contains(" TCP ") && l.contains("(LISTEN)"))
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let pid = cols.get(1)?.parse::<u32>().ok()?;
            let name_start = line.find(" TCP ")? + 4;
            let name = line[name_start..]
                .trim_end()
                .trim_end_matches("(LISTEN)")
                .trim_end();
            let port = name.rsplit(':').next()?.parse::<u16>().ok()?;
            Some((port, pid))
        })
        .collect()
}

/// Windows LISTEN table from `netstat -ano -p tcp`: TCP rows in state
/// LISTENING become (port, pid). The local address's port sits after the
/// LAST colon (`0.0.0.0:8000`, `[::]:8080`), which also survives IPv6.
pub fn parse_netstat_listen(output: &str) -> Vec<(u16, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() < 5 || cols[0] != "TCP" || cols[3] != "LISTENING" {
                return None;
            }
            let port = cols[1].rsplit(':').next()?.parse::<u16>().ok()?;
            let pid = cols[4].parse::<u32>().ok()?;
            Some((port, pid))
        })
        .collect()
}

/// Windows process-tree source: two-column CSV as produced by PowerShell's
/// `Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId |
/// ConvertTo-Csv -NoTypeInformation`. Returns plain (pid, ppid) edges; rows
/// that do not hold two integers (header, blank) are skipped.
pub fn parse_win_parents(csv: &str) -> Vec<(u32, u32)> {
    csv.lines()
        .skip_while(|l| !l.contains("ParentProcessId")) // drop the header row
        .filter_map(|line| {
            let nums: Option<Vec<u32>> = line
                .split(',')
                .map(|c| c.trim_matches('"').parse::<u32>())
                .collect::<Result<_, _>>()
                .ok();
            match nums? {
                pair if pair.len() == 2 => Some((pair[0], pair[1])),
                _ => None,
            }
        })
        .collect()
}

/// Unix process-tree source: `ps -axo pid=,ppid=` output (the `=` form
/// suppresses the header; macOS lacks `/proc`, Linux uses it anyway so one
/// parser covers both). Plain numeric rows become (pid, ppid) edges; junk is
/// skipped — a corrupted row must never panic the metadata query.
pub fn parse_ps_parents(output: &str) -> Vec<(u32, u32)> {
    output
        .lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let pid = cols.next()?.parse::<u32>().ok()?;
            let ppid = cols.next()?.parse::<u32>().ok()?;
            Some((pid, ppid))
        })
        .collect()
}

// --- OS boundaries -----------------------------------------------------------
//
// Thin cfg-split shims over the parsers above, mirroring pty_service's
// process_cwd/process_name split. Deliberately NOT unit-tested — they touch
// the live system; #42 verifies these per platform via HITL spot-checks.
// Failures resolve to an EMPTY answer (never an error): a tooltip may be
// quiet, but the metadata query must not fail the sidebar.

#[cfg(target_os = "linux")]
pub fn listening_sockets() -> Vec<(u16, u32)> {
    // Kernel tables give inode→port; ownership needs a /proc/<pid>/fd scan
    // mapping each socket inode back to its owning pid. Two small reads per
    // table + one dir walk, all instant and offline.
    let mut tables = String::new();
    for name in ["/proc/net/tcp", "/proc/net/tcp6"] {
        if let Ok(t) = std::fs::read_to_string(name) {
            tables.push_str(&t);
        }
    }
    let rows = parse_linux_listening(&tables);
    if rows.is_empty() {
        return Vec::new();
    }
    let ports_by_inode: HashMap<u64, u16> = rows.into_iter().collect();

    let mut out = Vec::new();
    let Ok(proc_dir) = std::fs::read_dir("/proc") else {
        return out;
    };
    for entry in proc_dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path()) {
                if let Some(inode) = fd_inode(&target.to_string_lossy()) {
                    if let Some(&port) = ports_by_inode.get(&inode) {
                        out.push((port, pid));
                    }
                }
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
pub fn listening_sockets() -> Vec<(u16, u32)> {
    run_tool("lsof", &["-nP", "-iTCP", "-sTCP:LISTEN"])
        .map(|out| parse_lsof_listen(&out))
        .unwrap_or_default()
}

#[cfg(windows)]
pub fn listening_sockets() -> Vec<(u16, u32)> {
    run_tool("netstat", ["-ano", "-p", "tcp"])
        .map(|out| parse_netstat_listen(&out))
        .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn parent_edges() -> Vec<(u32, u32)> {
    run_tool("ps", &["-axo", "pid=,ppid="])
        .map(|out| parse_ps_parents(&out))
        .unwrap_or_default()
}

#[cfg(windows)]
pub fn parent_edges() -> Vec<(u32, u32)> {
    const PS: &str = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId | ConvertTo-Csv -NoTypeInformation";
    run_tool("powershell", ["-NoProfile", "-Command", PS])
        .map(|out| parse_win_parents(&out))
        .unwrap_or_default()
}

/// One bounded, output-captured tool run; any failure (missing binary,
/// non-zero exit, weird bytes) is just None — the caller answers empty.
fn run_tool(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-B1 (AC1 — the tracer bullet: a GRANDCHILD's dev server counts):
    //   Input:  shell=100 → 200 (npm) → 300 (node server), node holds :8000
    //   Output: [8000] — depth must not disqualify; that is the whole point
    //           of walking the tree instead of checking direct children.
    #[test]
    fn grandchild_listener_counts_for_its_tab() {
        let parents = [(200u32, 100u32), (300, 200)];
        let listeners = [(8000u16, 300u32)];
        assert_eq!(ports_for_root(100, &listeners, &parents), vec![8000]);
    }

    // T-B2 (AC1 — unrelated PIDs never leak into a tab):
    //   Input:  same tree, plus a system daemon 1→500 holding :5432 and a
    //           DIFFERENT root 999's own server on :9000
    //   Output: [] for root 100 — the two servers belong elsewhere.
    #[test]
    fn unrelated_processes_are_excluded() {
        let parents = [(200u32, 100u32), (300, 200), (500, 1), (901, 999)];
        let listeners = [(5432u16, 500u32), (9000u16, 901u32)];
        assert_eq!(ports_for_root(100, &listeners, &parents), Vec::<u16>::new());
    }

    // T-B3 (AC1 + assumptions — SEPARATION between tabs): the same fixture
    //   asked for two different roots returns each tab ONLY its own tree's
    //   ports; matching is per-call against one root, so aggregation can
    //   never cross wires panels.
    #[test]
    fn two_roots_get_separated_port_lists() {
        let parents = [(200u32, 100u32), (300, 200), (400, 999)];
        let listeners = [(8000u16, 300u32), (3000u16, 400u32)];
        assert_eq!(ports_for_root(100, &listeners, &parents), vec![8000]);
        assert_eq!(ports_for_root(999, &listeners, &parents), vec![3000]);
    }

    // T-B4 (assumptions — dedupe by PORT NUMBER, sort ascending): one pid
    //   bound to 5173 on several sockets (IPv4 + IPv6), input arrives
    //   unsorted → output lists 5173 once, ordered.
    #[test]
    fn duplicate_socket_entries_collapse_to_one_port_sorted() {
        let parents = [(200u32, 100u32)];
        let listeners = [(5173u16, 200u32), (3000u16, 200u32), (5173u16, 200u32)];
        assert_eq!(ports_for_root(100, &listeners, &parents), vec![3000, 5173]);
    }

    // --- Linux: /proc/net/tcp{,6} + fd links --------------------------------
    //
    // T-L1 (AC5 — Linux socket TABLE parsing): the kernel's tcp table lists
    //   every socket; only state `0A` is LISTEN. Ports and inodes are HEX.
    //   Input: a pasted /proc/net/tcp fragment with one real listener on
    //          0.0.0.0:8000 (1F40) inode 12345, an ESTABLISHED row, and a
    //          listening IPv6 row from the tcp6 table (same format).
    //   Output: the two LISTEN rows' (inode, port); nothing else.
    #[test]
    fn linux_table_yields_only_listening_sockets() {
        let v4 = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
                  \x20\x20\x20 0: 00000000:1F40 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 ffff8\n\
                  \x20\x20\x20 1: 0100007F:D431 8EF0A8C0:01BB 06 00000000:00000000 01:000000D2 00000000  1000        0 99 4 ffff9\n";
        let v6 = "  sl  local_address remote_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode\n\
                  \x20\x20\x20 0: 00000000000000000000000000000000:143C 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 67890 1 ffffa\n";
        let mut got = parse_linux_listening(v4);
        got.extend(parse_linux_listening(v6));
        got.sort();
        assert_eq!(got, vec![(12345u64, 8000u16), (67890, 0x143C)]);
    }

    // T-L3 (AC5 — Linux OWNERSHIP step): `/proc/<pid>/fd/<n>` symlinks a
    //   listening socket to `socket:[<inode>]`; anything else (pipes, cwd
    //   dirs, anon inodes) must refuse rather than guess.
    #[test]
    fn fd_link_parses_socket_inode_only() {
        assert_eq!(fd_inode("socket:[12345]"), Some(12345));
        assert_eq!(fd_inode("socket:[0]"), Some(0));
        assert_eq!(fd_inode("pipe:[777]"), None);
        assert_eq!(fd_inode("anon_inode:[eventpoll]"), None);
        assert_eq!(fd_inode("/home/user/project"), None);
        assert_eq!(fd_inode(""), None);
    }

    // --- macOS: `lsof -nP -iTCP -sTCP:LISTEN` -------------------------------
    //
    // T-M1 (AC5 — macOS LISTEN parsing): real-world shapes — a bare-Vite
    //   node on 0.0.0.0 (`*:5173`), an IPv6 wildcard (`[::]:3000`), a
    //   loopback bind (`localhost:8000`)… and one ESTABLISHED line that must
    //   be dropped even if it sneaks into the output.
    #[test]
    fn macos_lsof_listens_parse_to_port_pid() {
        let out = "\
COMMAND     PID       USER   FD   TYPE             DEVICE SIZE/OFF NODE NAME
node      12345      adam   23u  IPv4  0x9f31a2b3c4d5e6f7      0t0  TCP *:5173 (LISTEN)
node      12345      adam   24u  IPv6  0x9f31a2b3c4d5e6f8      0t0  TCP [::]:3000 (LISTEN)
python3    6789      adam   12u  IPv4  0x9f31a2b3c4d5e6f9      0t0  TCP localhost:8000 (LISTEN)
node      12345      adam   25u  IPv4  0x9f31a2b3c4d5e6fa      0t0  TCP 192.168.1.10:49152->93.184.216.34:443 (ESTABLISHED)
";
        let mut got = parse_lsof_listen(out);
        got.sort();
        assert_eq!(
            got,
            vec![(3000u16, 12345u32), (5173, 12345), (8000, 6789)]
        );
    }

    // T-M2 (AC5 — garbage tolerance): truncated/pid-less lines, and prose,
    //   never become bogus listeners.
    #[test]
    fn macos_lsof_ignores_unparseable_lines() {
        assert_eq!(parse_lsof_listen("n/a root 0u IPv4 TCP *:22 (LISTEN)"), Vec::new());
        assert_eq!(parse_lsof_listen("garbage"), Vec::new());
        assert_eq!(parse_lsof_listen(""), Vec::new());
    }

    // --- Windows: `netstat -ano -p tcp` + PowerShell CIM parent map ---------
    //
    // T-W1 (AC5 — Windows LISTEN parsing): netstat's own table shape, with
    //   `[::]` and `0.0.0.0` wildcards; ESTABLISHED/TIME_WAIT rows and the
    //   header must not survive.
    #[test]
    fn windows_netstat_listens_parse_to_port_pid() {
        let out = "\
\n  Active Connections

  Proto  Local Address          Foreign Address        State           PID
  TCP    0.0.0.0:8000           0.0.0.0:0              LISTENING       4128
  TCP    [::]:8080              [::]:0                 LISTENING       2222
  TCP    127.0.0.1:139          0.0.0.0:0              ESTABLISHED     1000
  TCP    192.168.1.10:49152     93.184.216.34:443      TIME_WAIT       0
";
        let mut got = parse_netstat_listen(out);
        got.sort();
        assert_eq!(got, vec![(8000u16, 4128u32), (8080, 2222)]);
    }

    // T-W2 (AC5 — Windows PROCESS TREE source): PowerShell's
    //   `Get-CimInstance Win32_Process | Select ProcessId,ParentProcessId |
    //   ConvertTo-Csv -NoTypeInformation` CSV — quoted numbers with a
    //   header row become plain pid→ppid edges.
    #[test]
    fn windows_cim_csv_parses_parent_edges() {
        let csv = "\"ProcessId\",\"ParentProcessId\"\r\n\"4\",\"0\"\r\n\"12345\",\"500\"\r\n";
        assert_eq!(
            parse_win_parents(csv),
            vec![(4u32, 0u32), (12345, 500)]
        );
        // Headerless / malformed input is tolerated (row skipped, not panic).
        assert_eq!(parse_win_parents("\"x\",\"y\"\r\n\r\n"), Vec::new());
    }

    // --- Unix (Linux + macOS): `ps -axo pid=,ppid=` -------------------------
    //
    // T-U1 (AC5 — Unix PROCESS TREE source): right-aligned numeric columns,
    //   no header (the `=` form suppresses it); edges (pid, ppid); junk
    //   lines skipped rather than panicking.
    #[test]
    fn unix_ps_output_parses_parent_edges() {
        let out = "      1       0\n\
                   \x20\x20 200     100\n\
                   \x20 12345     200\n";
        assert_eq!(parse_ps_parents(out), vec![(1u32, 0u32), (200, 100), (12345, 200)]);
        assert_eq!(parse_ps_parents(""), Vec::new());
        assert_eq!(parse_ps_parents("garbage line here\n"), Vec::new());
    }

    // T-A1 (#42 "aggregates per tab"): one TAB may hold several panels;
    //   their trees are UNIONED, then sorted/deduped ONCE — a port bound by
    //   two panels appears a single time.
    #[test]
    fn multi_panel_tabs_union_dedupe_and_sort() {
        let parents = [(200u32, 100u32), (300, 200), (400, 101)];
        let listeners = [(5173u16, 300u32), (8000u16, 400u32), (5173u16, 200u32)];
        assert_eq!(
            aggregate_ports(&listeners, &parents, &[100, 101]),
            vec![5173, 8000]
        );
    }
}
