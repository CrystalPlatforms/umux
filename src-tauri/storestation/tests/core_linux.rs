//! Linux Core uses a real D-Bus transport and a private fake logind.
//! The returned Unix socket fd models logind's inhibitor pipe: EOF proves
//! every copy was closed. No desktop session, root, or host power changes.
#![cfg(target_os = "linux")]

use std::ffi::CString;
use std::io::{BufRead, BufReader, Read};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};

use dbus::blocking::Connection;
use dbus::channel::{Channel, MatchingReceiver, Sender};
use dbus::message::MatchRule;
use serde_json::{json, Value};
use umux_storestation::client::Client;

const WAIT: Duration = Duration::from_secs(5);

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Logind {
    _bus: Process,
    address: String,
    inhibitors: mpsc::Receiver<UnixStream>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Logind {
    fn start(reply: &'static str) -> Self {
        let mut bus = Process(
            Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .expect("dbus-daemon must be installed"),
        );
        let mut address = String::new();
        BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let address = address.trim().to_owned();
        let thread_address = address.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (fd_tx, fd_rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut channel = Channel::open_private(&thread_address).unwrap();
            channel.register().unwrap();
            let conn = Connection::from(channel);
            conn.request_name("org.freedesktop.login1", false, true, false)
                .unwrap();
            conn.start_receive(
                MatchRule::new_method_call(),
                Box::new(move |msg, conn| {
                    assert_eq!(msg.path().unwrap().to_string(), "/org/freedesktop/login1");
                    assert_eq!(
                        msg.interface().unwrap().to_string(),
                        "org.freedesktop.login1.Manager"
                    );
                    assert_eq!(msg.member().unwrap().to_string(), "Inhibit");
                    let (what, who, why, mode): (String, String, String, String) =
                        msg.read4().unwrap();
                    // Only sleep: no idle/display inhibition or lid-switch interception.
                    assert_eq!(what, "sleep");
                    assert_eq!(who, "umux Core");
                    assert!(!why.is_empty());
                    assert_eq!(mode, "block");
                    let response = match reply {
                        "deny" => msg.error(
                            &"org.freedesktop.DBus.Error.AccessDenied".into(),
                            &CString::new("inhibit denied by test policy").unwrap(),
                        ),
                        "timeout" => return true,
                        "malformed" => msg.method_return().append1("not a file descriptor"),
                        _ => {
                            let (reader, writer) = UnixStream::pair().unwrap();
                            fd_tx.send(reader).unwrap();
                            let fd: OwnedFd = writer.into();
                            msg.method_return().append1(fd)
                        }
                    };
                    conn.send(response).unwrap();
                    true
                }),
            );
            ready_tx.send(()).unwrap();
            while !thread_stop.load(Ordering::SeqCst) {
                conn.process(Duration::from_millis(20)).unwrap();
            }
        });
        ready_rx.recv_timeout(WAIT).unwrap();
        Self {
            _bus: bus,
            address,
            inhibitors: fd_rx,
            stop,
            thread: Some(thread),
        }
    }

    fn inhibitor(&self) -> UnixStream {
        self.inhibitors
            .recv_timeout(WAIT)
            .expect("logind received Inhibit")
    }
}

impl Drop for Logind {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn start_daemon(dir: &Path, address: &str) -> (Process, Client) {
    let mut daemon = Process(
        Command::new(env!("CARGO_BIN_EXE_umux-storestation"))
            .arg("run")
            .env("UMUX_CONFIG_DIR", dir)
            .env("DBUS_SYSTEM_BUS_ADDRESS", address)
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(client) = Client::connect(dir, "cli", "1.7.5") {
            return (daemon, client);
        }
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "daemon exited early"
        );
        assert!(Instant::now() < deadline, "daemon did not become ready");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn set(client: &mut Client, enabled: bool) -> Value {
    client
        .call("core.set", json!({"enabled": enabled}))
        .unwrap()
}

fn status(client: &mut Client) -> Value {
    client.call("storestation.status", json!({})).unwrap()["core"].clone()
}

fn assert_held(view: &Value) {
    assert_eq!(view.as_object().unwrap().len(), 3, "Phase 1 schema");
    assert_eq!(view["enabled"], true);
    assert_eq!(view["held"], true, "{view}");
    let instruction = view["instruction"].as_str().expect("Linux lid caveat");
    assert!(instruction.contains("lid"), "{instruction}");
    assert!(instruction.contains("logind"), "{instruction}");
}

fn assert_open(fd: &mut UnixStream) {
    fd.set_nonblocking(true).unwrap();
    assert_eq!(
        fd.read(&mut [0]).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    fd.set_nonblocking(false).unwrap();
}

fn assert_released(mut fd: UnixStream) {
    fd.set_read_timeout(Some(WAIT)).unwrap();
    assert_eq!(
        fd.read(&mut [0])
            .expect("inhibitor released before timeout"),
        0
    );
}

fn stop_daemon(daemon: &mut Process, client: &mut Client) {
    client.call("storestation.shutdown", json!({})).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(exit) = daemon.0.try_wait().unwrap() {
            assert!(exit.success());
            return;
        }
        assert!(Instant::now() < deadline, "graceful shutdown hung");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn linux_toggle_holds_fd_reports_lid_and_releases_on_off() {
    let logind = Logind::start("ok");
    let dir = tempfile::tempdir().unwrap();
    let (_daemon, mut client) = start_daemon(dir.path(), &logind.address);
    assert_eq!(
        status(&mut client),
        json!({"enabled": false, "held": false, "instruction": null})
    );
    let on = set(&mut client, true);
    assert_held(&on);
    assert_eq!(status(&mut client), on);
    let mut fd = logind.inhibitor();
    assert_open(&mut fd);
    assert_eq!(
        set(&mut client, false),
        json!({"enabled": false, "held": false, "instruction": null})
    );
    assert_released(fd);
}

#[test]
fn linux_shutdown_releases_and_restart_restores_without_client_call() {
    let logind = Logind::start("ok");
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, mut client) = start_daemon(dir.path(), &logind.address);
    assert_held(&set(&mut client, true));
    let fd = logind.inhibitor();
    stop_daemon(&mut daemon, &mut client);
    assert_released(fd);
    let (mut restarted, mut client) = start_daemon(dir.path(), &logind.address);
    assert_held(&status(&mut client));
    let mut fd = logind.inhibitor();
    assert_open(&mut fd);
    stop_daemon(&mut restarted, &mut client);
    assert_released(fd);
}

#[test]
fn linux_sigkill_releases_the_inhibitor() {
    let logind = Logind::start("ok");
    let dir = tempfile::tempdir().unwrap();
    let (mut daemon, mut client) = start_daemon(dir.path(), &logind.address);
    assert_held(&set(&mut client, true));
    let mut fd = logind.inhibitor();
    assert_open(&mut fd);
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    assert_released(fd);
}

#[test]
fn linux_refusal_and_malformed_reply_never_claim_a_hold() {
    for reply in ["deny", "malformed"] {
        let logind = Logind::start(reply);
        let dir = tempfile::tempdir().unwrap();
        let (_daemon, mut client) = start_daemon(dir.path(), &logind.address);
        let view = set(&mut client, true);
        assert_eq!(view["enabled"], true);
        assert_eq!(view["held"], false);
        let instruction = view["instruction"].as_str().unwrap();
        assert!(instruction.contains("logind"), "{instruction}");
        if reply == "deny" {
            assert!(
                instruction.contains("inhibit denied by test policy"),
                "{instruction}"
            );
        }
        assert_eq!(status(&mut client), view);
        assert_eq!(set(&mut client, false)["instruction"], Value::Null);
    }
}

#[test]
fn linux_missing_system_bus_is_an_honest_nonheld_state() {
    let dir = tempfile::tempdir().unwrap();
    let address = format!("unix:path={}/absent-bus", dir.path().display());
    let (_daemon, mut client) = start_daemon(dir.path(), &address);
    let view = set(&mut client, true);
    assert_eq!(view["enabled"], true);
    assert_eq!(view["held"], false);
    assert!(view["instruction"].as_str().unwrap().contains("system bus"));
}

#[test]
fn linux_unresponsive_logind_times_out_without_hanging_core() {
    let logind = Logind::start("timeout");
    let dir = tempfile::tempdir().unwrap();
    let (_daemon, mut client) = start_daemon(dir.path(), &logind.address);
    let started = Instant::now();
    let view = set(&mut client, true);
    assert!(started.elapsed() < WAIT, "Core must not hang on logind");
    assert_eq!(view["enabled"], true);
    assert_eq!(view["held"], false);
    assert!(view["instruction"].as_str().unwrap().contains("logind"));
    assert_eq!(set(&mut client, false)["instruction"], Value::Null);
}
