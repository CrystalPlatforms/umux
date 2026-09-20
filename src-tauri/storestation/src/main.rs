//! `umux-storestation` — the headless umux Storestation daemon (#83, v1.7.0 phase 1).
//!
//! Two commands, one vocabulary with the `umux` CLI (flags, exit codes and
//! error codes are identical by design):
//! - `run`  — serve the per-user socket in the foreground until stopped
//!            (`umux-storestation stop`, the `storestation.shutdown` op, or Ctrl+C).
//!            A second run against a live instance exits 4.
//! - `stop` — graceful shutdown; idempotent (offline exits 0).
//!
//! Exit codes (catalog): 0 ok/offline · 2 usage (clap) · 3 storestation unreachable
//! (future live commands) · 4 already running · 5 internal.

use clap::{CommandFactory, Parser};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use umux_storestation::client::{Client, ConnectError};
use umux_storestation::protocol::{codes, ErrorObj};
use umux_storestation::{server, transport};

#[derive(Parser)]
#[command(
    name = "umux-storestation",
    version,
    about = "umux Storestation — the headless umux daemon (serves terminal sessions over a local socket)",
    after_help = umux_storestation::protocol::EXIT_CODE_HELP,
)]
struct StorestationCli {
    /// Use this config directory (precedence: flag > UMUX_CONFIG_DIR > default)
    #[arg(long, global = true, value_name = "DIR")]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Serve the per-user socket in the foreground until stopped
    Run,
    /// Gracefully stop a running daemon (idempotent — offline is fine)
    Stop {
        /// Print the machine-readable result object
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    // Bare `umux-storestation` is the "what can this do?" entry, same as `umux`.
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        let _ = StorestationCli::command().print_help();
        return;
    }
    let cli = StorestationCli::parse();

    // Flag > env > default: writing the resolved value into UMUX_CONFIG_DIR
    // keeps every path (store, socket, pid) on store_core's single resolver.
    if let Some(dir) = &cli.config_dir {
        std::env::set_var("UMUX_CONFIG_DIR", dir.as_os_str());
    }

    let code = match cli.command {
        Some(Command::Run) => cmd_run(),
        Some(Command::Stop { json }) => cmd_stop(json),
        None => 0,
    };
    std::process::exit(code);
}

fn cmd_run() -> i32 {
    let dir = store_core::paths::config_dir();
    let prepared = match server::prepare(&dir) {
        Ok(prepared) => prepared,
        Err(server::PrepareError::AlreadyRunning { pid }) => {
            let message = match pid {
                Some(pid) => format!("umux Storestation is already running (pid {pid})."),
                None => "umux Storestation is already running.".into(),
            };
            print_error(&ErrorObj::new(
                codes::STORESTATION_ALREADY_RUNNING,
                message,
                vec!["stop it with: umux-storestation stop".into()],
            ));
            return 4;
        }
        Err(server::PrepareError::Io(e)) => {
            print_error(&ErrorObj::new(
                codes::IO_ERROR,
                format!("could not start umux-storestation: {e}"),
                vec![],
            ));
            return 5;
        }
    };

    println!(
        "umux-storestation {} listening on {} (pid {})",
        env!("CARGO_PKG_VERSION"),
        transport::endpoint_display(&dir),
        std::process::id()
    );

    // Ctrl+C joins the same shutdown path as `stop`: set the flag, the serve
    // loop breaks on its next tick, cleanup runs, exit 0.
    let flag = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&flag);
    if let Err(e) = ctrlc::set_handler(move || {
        handler_flag.store(true, Ordering::SeqCst);
    }) {
        eprintln!("umux-storestation: could not install the Ctrl+C handler ({e}) — stop still works");
    }
    server::serve(prepared, move || flag.load(Ordering::SeqCst));
    0
}

fn cmd_stop(json: bool) -> i32 {
    let dir = store_core::paths::config_dir();
    match Client::connect(&dir, "cli", env!("CARGO_PKG_VERSION")) {
        Ok(mut client) => match client.call("storestation.shutdown", serde_json::json!({})) {
            Ok(_) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "stopped": true })).unwrap());
                } else {
                    println!("umux Storestation stopped.");
                }
                0
            }
            Err(err) => {
                print_error(&err);
                5
            }
        },
        Err(ConnectError::NotRunning { stale }) => {
            // Idempotent: offline is not a failure. Stale leftovers are
            // cleaned on the way out, same as a daemon start would.
            if stale {
                server::cleanup(&dir);
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "stopped": false })).unwrap()
                );
            } else {
                println!("umux Storestation is not running — nothing to stop.");
            }
            0
        }
        Err(other) => {
            print_error(&other.to_error_obj());
            5
        }
    }
}

/// One-line machine-readable error object on stderr (the CLI convention).
fn print_error(err: &ErrorObj) {
    eprintln!("{}", serde_json::to_string(err).expect("error objects serialize"));
}
