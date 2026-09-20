//! The per-user local socket transport — one API over two backends:
//!
//! - Unix (macOS/Linux): a UDS at `<config_dir>/storestation.sock`, permissions
//!   0600, created/removed through the filesystem (stale files are the
//!   daemon's to clean).
//! - Windows: a named pipe `\\.\pipe\umux-storestation-<hash>` named after the
//!   config dir, so `UMUX_CONFIG_DIR` isolates test instances without any
//!   filesystem artifacts to clean (the kernel object dies with the process).
//!
//! Both sides expose blocking `Read`/`Write` streams — the protocol layer
//! above is transport-agnostic. The Windows backend wraps tokio named pipes
//! in a small per-object runtime; on Unix everything is pure `std`.

use std::io;
use std::path::Path;
use std::time::Duration;

use crate::socketpath;

/// Client-side connect budget (protocol design doc: connect 3 s).
#[cfg(windows)]
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Request/read budget (protocol design doc: request 10 s) — unix stream
/// timeouts; the blocking wrappers bound their waits per call on Windows.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// One accept tick — the serve loop polls its stop conditions between ticks.
pub const ACCEPT_TICK: Duration = Duration::from_millis(25);

/// Applied to every stream before protocol I/O so a dead peer can never pin
/// a thread forever. Unix gets real socket timeouts; Windows streams are
/// already bounded by their `block_on` wrappers.
pub trait StreamTimeouts {
    fn apply_timeouts(&self) {}
}

/// The split halves of one connection (phase 2): the reader thread owns
/// `read` (frames from the client), the writer thread owns `write` (request
/// responses, pushed session data frames, events — one serialized write
/// path so partial frames can never interleave).
pub struct StreamHalves {
    pub read: Box<dyn std::io::Read + Send>,
    pub write: Box<dyn std::io::Write + Send>,
}

// --- Unix: UDS, pure std ----------------------------------------------------

#[cfg(unix)]
pub type Stream = std::os::unix::net::UnixStream;
#[cfg(unix)]
pub type ClientStream = std::os::unix::net::UnixStream;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    pub struct Listener(UnixListener);

    impl Listener {
        /// Bind the UDS. Callers must have removed any stale socket file
        /// first (server::prepare owns that decision).
        pub fn bind(config_dir: &Path) -> io::Result<Listener> {
            let listener = UnixListener::bind(socketpath::socket_path(config_dir))?;
            // 0600: only the owning user talks to their own daemon.
            let _ = std::fs::set_permissions(
                socketpath::socket_path(config_dir),
                std::fs::Permissions::from_mode(0o600),
            );
            listener.set_nonblocking(true)?;
            Ok(Listener(listener))
        }

        /// ONE non-blocking accept attempt. `Ok(None)` = nothing waiting
        /// (or `stop` already asked) — the serve loop sleeps a tick and
        /// re-checks its stop conditions.
        pub fn next_client(
            &mut self,
            stop: &dyn Fn() -> bool,
        ) -> io::Result<Option<super::Stream>> {
            if stop() {
                return Ok(None);
            }
            match self.0.accept() {
                Ok((stream, _)) => {
                    // BSD/macOS accepted sockets INHERIT the listener's
                    // O_NONBLOCK (Linux clears it) — the protocol loop needs
                    // blocking reads, so clear it explicitly.
                    let _ = stream.set_nonblocking(false);
                    Ok(Some(stream))
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        }
    }

    pub fn connect(config_dir: &Path) -> io::Result<ClientStream> {
        UnixStream::connect(socketpath::socket_path(config_dir))
    }

    pub fn endpoint_display(config_dir: &Path) -> String {
        socketpath::socket_path(config_dir)
            .display()
            .to_string()
    }

    /// Remove a leftover socket FILE (a crash's orphan). Part of every
    /// daemon start and every clean stop.
    pub fn remove_socket_file(config_dir: &Path) {
        let _ = std::fs::remove_file(socketpath::socket_path(config_dir));
    }

    /// Split one accepted connection into read/write halves. `try_clone`
    /// duplicates the fd — both halves are the same socket.
    pub fn split_stream(stream: super::Stream) -> io::Result<super::StreamHalves> {
        let write = stream.try_clone()?;
        Ok(super::StreamHalves {
            read: Box::new(stream),
            write: Box::new(write),
        })
    }

    /// Split a client-side stream the same way (the desktop daemon-client
    /// driver's persistent connection, phase 4).
    pub fn split_client(stream: ClientStream) -> io::Result<super::StreamHalves> {
        let write = stream.try_clone()?;
        Ok(super::StreamHalves {
            read: Box::new(stream),
            write: Box::new(write),
        })
    }
}

// --- Windows: named pipe via tokio ------------------------------------------

#[cfg(windows)]
mod imp {
    use super::*;
    use std::time::Instant;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, ServerOptions};

    /// ERROR_PIPE_BUSY — every other pipe instance is busy; retrying is the
    /// documented client behavior for local RPC pipes.
    const ERROR_PIPE_BUSY: i32 = 231;

    fn new_runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio current-thread runtime")
    }

    pub struct Listener {
        rt: tokio::runtime::Runtime,
        name: String,
        server: Option<NamedPipeServer>,
    }

    impl Listener {
        pub fn bind(config_dir: &Path) -> io::Result<Listener> {
            let rt = new_runtime();
            let name = socketpath::pipe_name(config_dir);
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&name)?;
            Ok(Listener {
                rt,
                name,
                server: Some(server),
            })
        }

        /// Wait up to one tick for a client; `Ok(None)` = nothing arrived
        /// (or `stop` asked). A connected instance is handed out and a
        /// fresh one is created for the next client.
        pub fn next_client(
            &mut self,
            stop: &dyn Fn() -> bool,
        ) -> io::Result<Option<super::Stream>> {
            if stop() {
                return Ok(None);
            }
            let mut server = self.server.as_mut().expect("listener is bound");
            match self.rt.block_on(async {
                tokio::time::timeout(ACCEPT_TICK, server.connect()).await
            }) {
                Ok(Ok(())) => {
                    let connected = self.server.take().expect("just used");
                    self.server = Some(ServerOptions::new().create(&self.name)?);
                    Ok(Some(super::Stream::new(connected)))
                }
                Ok(Err(e)) => Err(e),
                Err(_elapsed) => Ok(None),
            }
        }
    }

    /// Server side of one accepted connection — owns its tiny runtime, so a
    /// handler thread needs nothing from the accept loop.
    pub struct Stream {
        rt: tokio::runtime::Runtime,
        pipe: NamedPipeServer,
    }

    impl Stream {
        fn new(pipe: NamedPipeServer) -> Stream {
            Stream {
                rt: new_runtime(),
                pipe,
            }
        }
    }

    impl io::Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.rt.block_on(async { self.pipe.read(buf).await })
        }
    }

    impl io::Write for Stream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.rt.block_on(async { self.pipe.write(buf).await })
        }
        fn flush(&mut self) -> io::Result<()> {
            self.rt.block_on(async { self.pipe.flush().await })
        }
    }

    /// Client side — what the CLI (and later the desktop driver) holds.
    pub struct ClientStream {
        rt: tokio::runtime::Runtime,
        pipe: NamedPipeClient,
    }

    impl io::Read for ClientStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.rt.block_on(async { self.pipe.read(buf).await })
        }
    }

    impl io::Write for ClientStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.rt.block_on(async { self.pipe.write(buf).await })
        }
        fn flush(&mut self) -> io::Result<()> {
            self.rt.block_on(async { self.pipe.flush().await })
        }
    }

    /// Open the pipe; ENOENT answers instantly (offline check), a busy
    /// server is retried within the connect budget.
    pub fn connect(config_dir: &Path) -> io::Result<ClientStream> {
        let name = socketpath::pipe_name(config_dir);
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match ClientOptions::new().open(&name) {
                Ok(pipe) => {
                    return Ok(ClientStream {
                        rt: new_runtime(),
                        pipe,
                    })
                }
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out connecting to the umux-storestation pipe",
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn endpoint_display(config_dir: &Path) -> String {
        socketpath::pipe_name(config_dir)
    }

    /// The pipe is a kernel object — there is no stale file to remove.
    pub fn remove_socket_file(_config_dir: &Path) {}

    /// Split one accepted connection into read/write halves. tokio's
    /// `into_split` hands out owned halves; each gets its own tiny
    /// current-thread runtime so the reader thread and the writer thread
    /// never contend for one runtime.
    pub fn split_stream(stream: super::Stream) -> io::Result<super::StreamHalves> {
        let Stream { rt: _, pipe } = stream;
        let (read, write) = pipe.into_split();
        Ok(super::StreamHalves {
            read: Box::new(ReadHalf {
                rt: new_runtime(),
                half: read,
            }),
            write: Box::new(WriteHalf {
                rt: new_runtime(),
                half: write,
            }),
        })
    }

    /// Split a client-side stream the same way (the desktop daemon-client
    /// driver's persistent connection, phase 4).
    pub fn split_client(stream: ClientStream) -> io::Result<super::StreamHalves> {
        let ClientStream { rt: _, pipe } = stream;
        let (read, write) = pipe.into_split();
        Ok(super::StreamHalves {
            read: Box::new(ReadHalf {
                rt: new_runtime(),
                half: read,
            }),
            write: Box::new(WriteHalf {
                rt: new_runtime(),
                half: write,
            }),
        })
    }

    /// The read half of a split stream — owned by the connection's reader.
    struct ReadHalf {
        rt: tokio::runtime::Runtime,
        half: tokio::net::windows::named_pipe::OwnedReadHalf,
    }

    impl io::Read for ReadHalf {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.rt.block_on(async { self.half.read(buf).await })
        }
    }

    /// The write half of a split stream — owned by the connection's writer.
    struct WriteHalf {
        rt: tokio::runtime::Runtime,
        half: tokio::net::windows::named_pipe::OwnedWriteHalf,
    }

    impl io::Write for WriteHalf {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.rt.block_on(async { self.half.write(buf).await })
        }
        fn flush(&mut self) -> io::Result<()> {
            self.rt.block_on(async { self.half.flush().await })
        }
    }
}

#[cfg(unix)]
impl StreamTimeouts for std::os::unix::net::UnixStream {
    fn apply_timeouts(&self) {
        let _ = self.set_read_timeout(Some(REQUEST_TIMEOUT));
        let _ = self.set_write_timeout(Some(REQUEST_TIMEOUT));
    }
}

#[cfg(windows)]
impl StreamTimeouts for imp::Stream {}
#[cfg(windows)]
impl StreamTimeouts for imp::ClientStream {}

// On unix Stream/ClientStream are the type aliases at the top of this
// module; on windows they are the structs inside imp — re-exported here.
#[cfg(unix)]
pub use imp::{connect, endpoint_display, remove_socket_file, split_client, split_stream, Listener};
#[cfg(windows)]
pub use imp::{
    connect, endpoint_display, remove_socket_file, split_client, split_stream, ClientStream,
    Listener, Stream,
};
