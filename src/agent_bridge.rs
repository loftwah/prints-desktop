use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_PACKET: usize = 64 * 1024;
const CONNECTION_LIMIT: Duration = Duration::from_secs(2);
const REPLY_LIMIT: Duration = Duration::from_secs(4);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Grant {
    address: SocketAddrV4,
    workspace: String,
    token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Invocation {
    id: String,
    request: Value,
}

pub(crate) struct Call {
    pub request: Value,
    reply: SyncSender<Value>,
    cancelled: Arc<AtomicBool>,
}

impl Call {
    pub(crate) fn answer(self, response: Value) {
        let _ = self.reply.try_send(response);
    }
}

#[derive(Default)]
pub(crate) struct Bridge {
    active: Option<Listener>,
}

struct Listener {
    grant_file: PathBuf,
    calls: Receiver<Call>,
    stopped: Arc<AtomicBool>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = fs::remove_file(&self.grant_file);
    }
}

pub(crate) fn opaque_id() -> io::Result<String> {
    let mut entropy = [0u8; 32];
    getrandom::fill(&mut entropy).map_err(|_| io::Error::other("Random data is unavailable."))?;
    Ok(entropy.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Bridge {
    pub(crate) fn start(
        &mut self,
        directory: &Path,
        workspace: &str,
        token: &str,
        wake: rune::Waker,
    ) -> io::Result<&Path> {
        self.stop();
        let socket = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let std::net::SocketAddr::V4(address) = socket.local_addr()? else {
            return Err(io::Error::other("Couldn’t open local workspace access."));
        };
        socket.set_nonblocking(true)?;
        let path = directory.join(format!("prints-agent-{}.json", opaque_id()?));
        let grant = Grant {
            address,
            workspace: workspace.into(),
            token: token.into(),
        };
        fs::create_dir_all(directory)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        // Own the grant file as soon as creation succeeds, including write and spawn failures.
        let (send, calls) = mpsc::sync_channel(4);
        let stopped = Arc::new(AtomicBool::new(false));
        let listener = Listener {
            grant_file: path,
            calls,
            stopped: stopped.clone(),
        };
        let mut contents = serde_json::to_vec(&grant)?;
        file.write_all(&contents)?;
        contents.fill(0);
        drop(file);
        thread::Builder::new()
            .name("prints-agent".into())
            .spawn(move || {
                listen(&socket, &grant.token, &send, &stopped, wake);
            })?;
        self.active = Some(listener);
        Ok(self.grant_path().expect("workspace access started"))
    }

    pub(crate) fn stop(&mut self) {
        self.active = None;
    }

    pub(crate) fn grant_path(&self) -> Option<&Path> {
        Some(&self.active.as_ref()?.grant_file)
    }

    pub(crate) fn take(&self) -> Option<Call> {
        let calls = &self.active.as_ref()?.calls;
        while let Ok(call) = calls.try_recv() {
            if !call.cancelled.load(Ordering::Acquire) {
                return Some(call);
            }
        }
        None
    }
}

fn listen(
    listener: &TcpListener,
    token: &str,
    calls: &SyncSender<Call>,
    stopped: &AtomicBool,
    wake: rune::Waker,
) {
    while !stopped.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                let reply = receive(&mut stream, token, calls, stopped, &wake);
                let _ = send_packet(&mut stream, &reply);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::park_timeout(Duration::from_millis(20));
            }
            Ok(_) => continue,
            Err(_) => return,
        }
    }
}

fn receive(
    stream: &mut TcpStream,
    token: &str,
    calls: &SyncSender<Call>,
    stopped: &AtomicBool,
    wake: &rune::Waker,
) -> Value {
    let request = read_packet(stream, CONNECTION_LIMIT)
        .and_then(|data| serde_json::from_slice::<Value>(&data).map_err(Into::into));
    let Ok(request) = request else {
        return error("invalid_request", "Send one bounded workspace request.");
    };
    let permitted = request
        .get("bearer")
        .and_then(Value::as_str)
        .is_some_and(|supplied| same_token(token.as_bytes(), supplied.as_bytes()));
    if !permitted || stopped.load(Ordering::Acquire) {
        return error(
            "access_denied",
            "Enable access again in the Prints workspace.",
        );
    }
    let (reply, response) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    if calls
        .try_send(Call {
            request,
            reply,
            cancelled: cancelled.clone(),
        })
        .is_err()
    {
        return error(
            "unavailable",
            "The workspace is unavailable. Read its current revision before retrying.",
        );
    }
    wake();
    match response.recv_timeout(REPLY_LIMIT) {
        Ok(value) => value,
        Err(_) => {
            cancelled.store(true, Ordering::Release);
            error(
                "uncertain",
                "Read the current revision before retrying this command.",
            )
        }
    }
}

fn same_token(expected: &[u8], supplied: &[u8]) -> bool {
    expected.len() == supplied.len()
        && expected
            .iter()
            .zip(supplied)
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

fn send_packet<T: Serialize>(stream: &mut TcpStream, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_PACKET {
        return Err(io::Error::other("Workspace request or reply too large."));
    }
    stream.set_write_timeout(Some(CONNECTION_LIMIT))?;
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")
}

fn read_packet(stream: &mut TcpStream, limit: Duration) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + limit;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        stream.set_read_timeout(Some(remaining))?;
        let size = stream.read(&mut chunk)?;
        if size == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        let filled = &chunk[..size];
        let end = filled.iter().position(|byte| *byte == b'\n');
        buffer.extend_from_slice(&filled[..end.unwrap_or(size)]);
        if buffer.len() > MAX_PACKET {
            return Err(io::Error::other("Workspace request or reply too large."));
        }
        if end.is_some() {
            return Ok(buffer);
        }
    }
}

fn error(code: &str, message: &str) -> Value {
    json!({"kind": code, "message": message})
}

fn invoke(path: &Path, request: Invocation) -> Result<Value, &'static str> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| "The grant file is unavailable. Enable access in the running app.")?;
    if !metadata.is_file() || metadata.len() > 4096 {
        return Err("The grant is invalid.");
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(4097).read_to_end(&mut bytes))
        .map_err(|_| "The grant file is unavailable. Enable access in the running app.")?;
    let grant: Grant = serde_json::from_slice(&bytes).map_err(|_| "The grant is invalid.")?;
    bytes.fill(0);
    if grant.address.ip() != &Ipv4Addr::LOCALHOST {
        return Err("The grant must point to the local Prints app.");
    }
    let mut stream = TcpStream::connect_timeout(&grant.address.into(), CONNECTION_LIMIT)
        .map_err(|_| "The local workspace is unavailable. Enable access in the running app.")?;
    send_packet(
        &mut stream,
        &json!({
            "workspace": grant.workspace,
            "bearer": grant.token,
            "id": request.id,
            "request": request.request,
        }),
    )
    .map_err(|_| "The request could not be sent. Read the workspace before retrying.")?;
    let reply = read_packet(&mut stream, REPLY_LIMIT + CONNECTION_LIMIT)
        .map_err(|_| "The result is uncertain. Read the current workspace before retrying.")?;
    serde_json::from_slice(&reply).map_err(|_| "The local workspace returned an invalid reply.")
}

pub(crate) fn cli(arguments: &[OsString]) -> Option<i32> {
    if !arguments.first().is_some_and(|arg| arg == "workspace") {
        return None;
    }
    let [_, flag, grant] = arguments else {
        eprintln!("Usage: prints-desktop workspace --grant <file> < request.json");
        return Some(2);
    };
    if flag != "--grant" {
        eprintln!("Usage: prints-desktop workspace --grant <file> < request.json");
        return Some(2);
    }
    let mut request = Vec::new();
    let input = io::stdin()
        .lock()
        .take((MAX_PACKET + 1) as u64)
        .read_until(b'\n', &mut request);
    if input.is_err() || request.len() > MAX_PACKET {
        eprintln!("Workspace request exceeded the input limit.");
        return Some(2);
    }
    let Ok(request) = serde_json::from_slice::<Invocation>(&request) else {
        eprintln!("Send one JSON workspace request on standard input.");
        return Some(2);
    };
    match invoke(Path::new(grant), request) {
        Ok(reply) => {
            println!("{reply}");
            Some(0)
        }
        Err(message) => {
            // Settle remote uncertainty through the workspace revision. Never echo the token.
            println!("{}", error("unavailable_or_uncertain", message));
            Some(1)
        }
    }
}
