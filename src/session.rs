use crate::elevation::{Gate, Output};
use crate::history::{self, Pattern, Query};
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct View {
    pub text: String,
    pub results: Value,
    pub summary: String,
    pub generation: u64,
    pub error: String,
}

pub struct Prompt {
    pub message: String,
    pub confirmation: bool,
    pub reply: mpsc::Sender<Option<String>>,
    pub credential: Option<String>,
}

struct Shared {
    stop: AtomicBool,
    connected: AtomicBool,
    revision: AtomicU64,
    received_bytes: AtomicU64,
    encoding_errors: AtomicU64,
    trap: Mutex<crate::trap::Trap>,
    generation: AtomicU64,
    query: Mutex<Query>,
    view: Mutex<Option<View>>,
    status: Mutex<String>,
    diagnostics: Mutex<String>,
    child: Mutex<Option<Child>>,
    raw: Mutex<RawStream>,
}

#[derive(Default)]
struct RawStream {
    path: Option<PathBuf>,
    bytes: u64,
    decoder: Option<crate::encoding::Decoder>,
    finished: bool,
    limit: u64,
    encoding: String,
    resets: Vec<u64>,
}

pub struct Session {
    shared: Arc<Shared>,
    pub path: PathBuf,
    pub prompts: mpsc::Receiver<Prompt>,
    connection: Option<std::rc::Rc<crate::connections::Managed>>,
    own_transport: Option<Arc<crate::ssh::Transport>>,
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path)
}

fn create_directory() -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let base = crate::workspace::cache_directory()?;
    fs::create_dir_all(&base)?;
    for _ in 0..100 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = base.join(format!(
            "{now}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("Cannot create archive directory"))
}

pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone)]
pub struct SshOptions {
    pub source: String,
    pub host: String,
    pub user: String,
    pub port: u16,
    pub key: String,
    pub path: String,
    pub initial: u32,
    pub elevation: u32,
    pub run_user: String,
}

fn remote_command(options: &SshOptions, marker: &str) -> io::Result<String> {
    let tail = crate::source::LogSource::parse(&options.source)?
        .command(&options.path, options.initial)?;
    if options.elevation == 0 {
        return Ok(tail);
    }
    if options.elevation > 2
        || options.run_user.is_empty()
        || options.run_user.starts_with('-')
        || !options
            .run_user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.$".contains(c))
    {
        return Err(io::Error::other("Check escalation method / run-as user"));
    }
    // Require stty to succeed BEFORE any password is sent. Disable CRLF translation
    // and terminal echo, including when sudo/su restores the previous terminal state.
    let inner = format!(
        "stty -echo -onlcr || exit 1; printf '%s' {}; {tail}",
        quote(marker)
    );
    let elevated = if options.elevation == 1 {
        format!(
            "exec sudo -p 'TAILER-SUDO-PASSWORD:' -u {} -- /bin/sh -c {}",
            quote(&options.run_user),
            quote(&inner)
        )
    } else {
        // GNU util-linux/BSD/macOS accept shell arguments after the target user.
        // Use a login su session; command is explicitly run with POSIX sh.
        format!(
            "exec su - {} -c {}",
            quote(&options.run_user),
            quote(&format!("exec /bin/sh -c {}", quote(&inner)))
        )
    };
    Ok(format!(
        "stty -echo -onlcr || exit 1; export LC_ALL=C; {elevated}"
    ))
}

impl Session {
    fn start_ssh(
        options: SshOptions,
        limit: u64,
        transport: Arc<crate::ssh::Transport>,
        encoding: &str,
        auth_prompts: Option<mpsc::Receiver<Prompt>>,
    ) -> io::Result<Self> {
        crate::ssh::validate(&options)?;
        let marker = format!(
            "\x1eTAILER-{}-{}\x1f",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let command = remote_command(&options, &marker)?;
        let decoder = crate::encoding::Decoder::new(encoding)?;
        let directory = create_directory()?;
        let path = directory.join("received.log");
        let mut archive = private_file(&path)?;
        let shared = new_shared(None);
        initialize_raw(&shared, &path, decoder, limit, encoding)?;
        let (tx, input) = mpsc::sync_channel(8);
        let (public_tx, prompts) = mpsc::sync_channel(8);
        let state = shared.clone();
        let identity =
            serde_json::json!([options.host, options.port, options.user, options.run_user])
                .to_string();
        thread::spawn(move || crate::credentials::broker(identity, input, public_tx, &state.stop));
        let own_transport = auth_prompts.as_ref().map(|_| transport.clone());
        if let Some(auth_prompts) = auth_prompts {
            let tx = tx.clone();
            let state = shared.clone();
            thread::spawn(move || {
                while !state.stop.load(Ordering::Relaxed) {
                    match auth_prompts.recv_timeout(Duration::from_millis(100)) {
                        Ok(prompt) => {
                            if tx.send(prompt).is_err() {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
            });
        }
        let state = shared.clone();
        crate::ssh::runtime().spawn(async move {
            let mut gate = (options.elevation != 0).then(|| Gate::new(&marker));
            let result = async {
                while !transport.is_ready() {
                    if transport.stop.load(Ordering::Relaxed) {
                        return Err(io::Error::other(transport.status()));
                    }
                    if state.stop.load(Ordering::Relaxed) {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                let mut channel = transport.channel().await?;
                let mut pending = std::collections::VecDeque::new();
                let result: io::Result<()> = async {
                    if options.elevation != 0 {
                        channel
                            .request_pty(true, "xterm", 80, 24, 0, 0, &[(russh::Pty::ECHO, 0)])
                            .await
                            .map_err(crate::ssh::error)?;
                        expect_channel_success(&mut channel, &mut pending, &state.stop, "PTY")
                            .await?;
                    }
                    channel
                        .exec(true, command)
                        .await
                        .map_err(crate::ssh::error)?;
                    expect_channel_success(&mut channel, &mut pending, &state.stop, "exec").await?;
                    state.connected.store(options.elevation == 0, Ordering::Relaxed);
                    let mut exit = None;
                    loop {
                        let message = if let Some(message) = pending.pop_front() {
                            Some(message)
                        } else {
                            tokio::select! {
                                _ = crate::ssh::wait_stopped(&state.stop) => return Ok(()),
                                message = channel.wait() => message,
                            }
                        };
                        match message {
                            Some(russh::ChannelMsg::Data { data })
                            | Some(russh::ChannelMsg::ExtendedData { data, .. }) => {
                                let data = if let Some(gate) = &mut gate {
                                    match gate.push(&data)? {
                                        Output::Waiting => continue,
                                        Output::Password => {
                                            let answer = crate::ssh::prompt(
                                                &tx,
                                                &state.stop,
                                                format!(
                                                    "Privilege escalation → {}\nPassword:",
                                                    options.run_user
                                                ),
                                                false,
                                            )
                                            .await?;
                                            if answer.contains(['\n', '\r']) {
                                                return Err(io::Error::other("Invalid authentication input"));
                                            }
                                            channel
                                                .data_bytes(format!("{answer}\n").into_bytes())
                                                .await
                                                .map_err(crate::ssh::error)?;
                                            continue;
                                        }
                                        Output::Log(data) => {
                                            state.connected.store(true, Ordering::Relaxed);
                                            data
                                        },
                                    }
                                } else {
                                    data.to_vec()
                                };
                                receive_raw(&state, &mut archive, &data, false)?;
                            }
                            Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                                exit = Some(exit_status)
                            }
                            Some(russh::ChannelMsg::ExitSignal { .. }) => {
                                return Err(io::Error::other(
                                    "Remote command terminated by a signal",
                                ));
                            }
                            Some(russh::ChannelMsg::Close) | None => break,
                            _ => {}
                        }
                    }
                    if gate.as_ref().is_some_and(|gate| !gate.ready) {
                        return Err(io::Error::other(
                            "Privilege escalation failed or ended. Authentication output was not saved.",
                        ));
                    }
                    receive_raw(&state, &mut archive, &[], true)?;
                    *state.status.lock().unwrap() =
                        format!("Connection ended ({exit:?}) · Saved history remains searchable");
                    Ok(())
                }
                .await;
                let _ = channel.close().await;
                result
            }
            .await;
            state.connected.store(false, Ordering::Relaxed);
            if let Err(error) = result {
                *state.status.lock().unwrap() = format!("Connection ended: {error}");
            }
            state.revision.fetch_add(1, Ordering::Relaxed);
        });
        spawn_history_worker(&shared, &path);
        Ok(Self {
            shared,
            path,
            prompts,
            connection: None,
            own_transport,
        })
    }
    #[cfg(test)]
    pub fn shared_ssh(
        options: SshOptions,
        limit: u64,
        connection: std::rc::Rc<crate::connections::Managed>,
    ) -> io::Result<Self> {
        Self::shared_ssh_encoded(options, limit, connection, "UTF-8")
    }

    pub fn shared_ssh_encoded(
        options: SshOptions,
        limit: u64,
        connection: std::rc::Rc<crate::connections::Managed>,
        encoding: &str,
    ) -> io::Result<Self> {
        let mut session =
            Self::start_ssh(options, limit, connection.transport.clone(), encoding, None)?;
        session.connection = Some(connection);
        Ok(session)
    }

    #[cfg(test)]
    pub fn local(path: String, initial: u32, limit: u64) -> io::Result<Self> {
        Self::local_encoded(path, initial, limit, "UTF-8")
    }

    pub fn local_encoded(
        path: String,
        initial: u32,
        limit: u64,
        encoding: &str,
    ) -> io::Result<Self> {
        let decoder = crate::encoding::Decoder::new(encoding)?;
        let mut follower = crate::tail::FileFollower::open(path.into(), initial, encoding)?;
        let dir = create_directory()?;
        let path = dir.join("received.log");
        let mut archive = private_file(&path)?;
        let shared = new_shared(None);
        initialize_raw(&shared, &path, decoder, limit, encoding)?;
        let (_, prompts) = mpsc::channel();
        shared.connected.store(true, Ordering::Relaxed);
        let state = shared.clone();
        thread::spawn(move || {
            let mut buffer = [0; 64 * 1024];
            let result = (|| -> io::Result<()> {
                while !state.stop.load(Ordering::Relaxed) {
                    let (count, reset) = follower.poll(&mut buffer)?;
                    if reset {
                        // Finish the previous file's incomplete sequence, then reset the decoder.
                        reset_raw_decoder(&state, &mut archive)?;
                        // Do not match a literal across two distinct files.
                        let mut trap = state.trap.lock().unwrap();
                        trap.reset();
                    }
                    if count == 0 {
                        thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    receive_raw(&state, &mut archive, &buffer[..count], false)?;
                }
                Ok(())
            })();
            state.connected.store(false, Ordering::Relaxed);
            if let Err(error) = result {
                *state.status.lock().unwrap() = error.to_string();
                state.revision.fetch_add(1, Ordering::Relaxed);
            }
        });
        spawn_history_worker(&shared, &path);
        Ok(Self {
            shared,
            path,
            prompts,
            connection: None,
            own_transport: None,
        })
    }

    pub fn ssh(options: SshOptions, limit: u64) -> io::Result<Self> {
        remote_command(&options, "validate")?;
        let (transport, prompts) = crate::ssh::Transport::start(&options)?;
        let result = Self::start_ssh(options, limit, transport.clone(), "UTF-8", Some(prompts));
        if result.is_err() {
            transport.stop();
        }
        result
    }

    pub fn local_command(script: String, limit: u64, encoding: &str) -> io::Result<Self> {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(crate::source::LogSource::Custom.command(&script, 0)?);
        Self::start_encoded(command, limit, None, encoding)
    }

    #[cfg(test)]
    pub(crate) fn start(
        command: Command,
        limit: u64,
        elevation: Option<(String, String)>,
    ) -> io::Result<Self> {
        Self::start_encoded(command, limit, elevation, "UTF-8")
    }

    fn start_encoded(
        mut command: Command,
        limit: u64,
        elevation: Option<(String, String)>,
        encoding: &str,
    ) -> io::Result<Self> {
        let decoder = crate::encoding::Decoder::new(encoding)?;
        let dir = create_directory()?;
        let path = dir.join("received.log");
        let mut archive = private_file(&path)?;
        let (elevation_tx, prompt_input) = mpsc::sync_channel(8);
        let (public_tx, prompts) = mpsc::sync_channel(8);
        let connection = format!("{:?}", command.get_args().collect::<Vec<_>>());
        let mut child = command
            .stdin(if elevation.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let mut stdin = child.stdin.take();
        let shared = new_shared(Some(child));
        initialize_raw(&shared, &path, decoder, limit, encoding)?;
        let state = shared.clone();
        thread::spawn(move || {
            crate::credentials::broker(connection, prompt_input, public_tx, &state.stop)
        });
        let state = shared.clone();
        let stderr_worker = thread::spawn(move || {
            let _ = history::read_lines(stderr, |line| {
                if state.stop.load(Ordering::Relaxed) {
                    return Err(io::Error::other("Stopped"));
                }
                let mut diagnostics = state.diagnostics.lock().unwrap();
                diagnostics.push_str(&line);
                diagnostics.push('\n');
                history::trim_text(&mut diagnostics);
                *state.status.lock().unwrap() = line;
                Ok(())
            });
        });
        let state = shared.clone();
        thread::spawn(move || {
            let mut buf = [0; 64 * 1024];
            let mut gate = elevation.as_ref().map(|(marker, _)| Gate::new(marker));
            let result = (|| -> io::Result<()> {
                while !state.stop.load(Ordering::Relaxed) {
                    let count = stdout.read(&mut buf)?;
                    if count == 0 && gate.as_ref().is_some_and(|gate| !gate.ready) {
                        return Err(io::Error::other(
                            "Privilege escalation failed / ended. Check the user, sudo/su settings and user shell. Authentication output was not saved.",
                        ));
                    }
                    let data = if let Some(gate) = &mut gate {
                        match gate.push(&buf[..count])? {
                            Output::Waiting => continue,
                            Output::Password => {
                                let answer = request_secret(
                                    &elevation_tx,
                                    &state.stop,
                                    &elevation.as_ref().unwrap().1,
                                )?;
                                let mut answer = answer.into_bytes();
                                let input = stdin.as_mut().ok_or_else(|| {
                                    io::Error::other("Escalation input is unavailable")
                                })?;
                                let write = input.write_all(&answer);
                                answer.fill(0);
                                write?;
                                input.write_all(b"\n")?;
                                input.flush()?;
                                continue;
                            }
                            Output::Log(data) => data,
                        }
                    } else {
                        buf[..count].to_vec()
                    };
                    receive_raw(&state, &mut archive, &data, count == 0)?;
                    if count == 0 {
                        break;
                    }
                }
                Ok(())
            })();
            // Keep the child reachable by stop() while waiting. An SSH master
            // may close stdout but continue running indefinitely with -N.
            let exit = loop {
                let mut guard = state.child.lock().unwrap();
                let Some(child) = guard.as_mut() else {
                    break None;
                };
                if result.is_err() || state.stop.load(Ordering::Relaxed) {
                    let _ = child.kill();
                }
                match child.try_wait() {
                    Ok(Some(exit)) => {
                        guard.take();
                        break Some(exit);
                    }
                    Err(_) => {
                        guard.take();
                        break None;
                    }
                    Ok(None) => {}
                }
                drop(guard);
                thread::sleep(Duration::from_millis(20));
            };
            state.connected.store(false, Ordering::Relaxed);
            if let Some(exit) = exit {
                let _ = stderr_worker.join();
                let diagnostics = state.diagnostics.lock().unwrap().clone();
                let mut status = state.status.lock().unwrap();
                if let Err(e) = result {
                    *status = format!("{e}\n{diagnostics}");
                } else if !state.stop.load(Ordering::Relaxed) {
                    *status = format!(
                        "Connection ended ({exit}) · Saved history remains searchable\n{diagnostics}"
                    );
                }
            }
            state.revision.fetch_add(1, Ordering::Relaxed);
        });
        spawn_history_worker(&shared, &path);
        Ok(Self {
            shared,
            path,
            prompts,
            connection: None,
            own_transport: None,
        })
    }

    pub fn query(&self, mut query: Query) -> u64 {
        let generation = self.shared.generation.fetch_add(1, Ordering::Relaxed) + 1;
        query.generation = generation;
        *self.shared.query.lock().unwrap() = query;
        self.shared.revision.fetch_add(1, Ordering::Relaxed);
        generation
    }
    pub fn view(&self) -> Option<View> {
        self.shared.view.lock().unwrap().take()
    }
    pub fn received_bytes(&self) -> u64 {
        self.shared.received_bytes.load(Ordering::Relaxed)
    }
    pub fn is_connected(&self) -> bool {
        self.shared.connected.load(Ordering::Relaxed) && !self.shared.stop.load(Ordering::Relaxed)
    }
    pub fn change_encoding(&self, encoding: &str) -> io::Result<()> {
        let mut raw = self.shared.raw.lock().unwrap();
        let mut decoder = crate::encoding::Decoder::new(encoding)?;
        let mut output = private_file(&self.path.with_extension("redecode"))?;
        let result = (|| -> io::Result<()> {
            let mut written = 0u64;
            if let Some(path) = &raw.path {
                let mut input = File::open(path)?;
                let mut buffer = [0; 64 * 1024];
                let mut position = 0;
                for (segment, boundary) in raw
                    .resets
                    .iter()
                    .copied()
                    .chain(std::iter::once(raw.bytes))
                    .enumerate()
                {
                    while position < boundary {
                        let size = (boundary - position).min(buffer.len() as u64) as usize;
                        let count = input.read(&mut buffer[..size])?;
                        if count == 0 {
                            return Err(io::Error::other("Raw archive incomplete"));
                        }
                        position += count as u64;
                        let text = decoder.decode(&buffer[..count], false)?;
                        write_redecoded(&mut output, &text, &mut written, raw.limit)?;
                    }
                    if segment < raw.resets.len() {
                        let text = decoder.decode(&[], true)?;
                        write_redecoded(&mut output, &text, &mut written, raw.limit)?;
                        decoder = crate::encoding::Decoder::new(encoding)?;
                    }
                }
            }
            if raw.finished {
                let text = decoder.decode(&[], true)?;
                write_redecoded(&mut output, &text, &mut written, raw.limit)?;
            }
            output.flush()?;
            fs::copy(self.path.with_extension("redecode"), &self.path)?;
            self.shared.received_bytes.store(written, Ordering::Relaxed);
            self.shared
                .encoding_errors
                .store(decoder.errors, Ordering::Relaxed);
            raw.decoder = Some(decoder);
            raw.encoding = encoding.to_owned();
            // Reinterpretation is not new reception and must not fire trap alerts.
            let mut trap = self.shared.trap.lock().unwrap();
            trap.reset();
            self.shared.revision.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })();
        drop(output);
        let _ = fs::remove_file(self.path.with_extension("redecode"));
        result
    }
    #[cfg(test)]
    pub fn set_trap(&self, text: String, ignore_case: bool) {
        self.shared
            .trap
            .lock()
            .unwrap()
            .configure(text, ignore_case);
    }
    pub fn set_trap_rules(&self, rules: Vec<crate::trap::CompiledRule>) {
        self.shared.trap.lock().unwrap().configure_rules(rules);
    }
    pub fn trap_hits(&self) -> u64 {
        self.shared.trap.lock().unwrap().hits
    }

    pub fn status(&self) -> String {
        let status = compact_status(&self.shared.status.lock().unwrap());
        let errors = self.shared.encoding_errors.load(Ordering::Relaxed);
        if errors == 0 {
            status
        } else {
            format!(
                "{status}\nEncoding conversion: replaced {errors} malformed / incomplete sequences"
            )
        }
    }
    pub fn stop(&self) {
        self.shared.connected.store(false, Ordering::Relaxed);
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(transport) = &self.own_transport {
            transport.stop();
        }
        if let Some(child) = self.shared.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

fn compact_status(status: &str) -> String {
    status
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn initialize_raw(
    state: &Shared,
    path: &Path,
    decoder: crate::encoding::Decoder,
    limit: u64,
    encoding: &str,
) -> io::Result<()> {
    let raw_path = path.with_extension("raw");
    private_file(&raw_path)?;
    *state.raw.lock().unwrap() = RawStream {
        path: Some(raw_path),
        decoder: Some(decoder),
        limit,
        encoding: encoding.to_owned(),
        ..RawStream::default()
    };
    Ok(())
}

fn reset_raw_decoder(state: &Shared, archive: &mut File) -> io::Result<()> {
    let mut raw = state.raw.lock().unwrap();
    let text = raw.decoder.as_mut().unwrap().decode(&[], true)?;
    let mut bytes = archive.seek(SeekFrom::End(0))?;
    retain_received(state, archive, &mut bytes, raw.limit, &text)?;
    let offset = raw.bytes;
    raw.resets.push(offset);
    raw.decoder = Some(crate::encoding::Decoder::new(&raw.encoding)?);
    Ok(())
}

fn write_redecoded(output: &mut File, text: &str, written: &mut u64, limit: u64) -> io::Result<()> {
    if text.len() as u64 > limit.saturating_sub(*written) {
        return Err(io::Error::other(
            "Storage limit reached. Collection stopped; history is retained.",
        ));
    }
    output.write_all(text.as_bytes())?;
    *written += text.len() as u64;
    Ok(())
}

fn receive_raw(state: &Shared, archive: &mut File, input: &[u8], finished: bool) -> io::Result<()> {
    let mut raw = state.raw.lock().unwrap();
    let retained = input
        .len()
        .min(raw.limit.saturating_mul(4).saturating_sub(raw.bytes) as usize);
    let truncated = retained < input.len();
    let input = &input[..retained];
    if let Some(path) = &raw.path {
        OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(input)?;
    }
    raw.bytes += input.len() as u64;
    let decoder = raw
        .decoder
        .as_mut()
        .ok_or_else(|| io::Error::other("Decoder unavailable"))?;
    let text = decoder.decode(input, finished)?;
    state
        .encoding_errors
        .store(decoder.errors, Ordering::Relaxed);
    raw.finished = finished;
    let mut bytes = archive.seek(SeekFrom::End(0))?;
    retain_received(state, archive, &mut bytes, raw.limit, &text)?;
    if truncated {
        return Err(io::Error::other(
            "Storage limit reached. Collection stopped; history is retained.",
        ));
    }
    Ok(())
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn expect_channel_success(
    channel: &mut russh::Channel<russh::client::Msg>,
    pending: &mut std::collections::VecDeque<russh::ChannelMsg>,
    stop: &AtomicBool,
    request: &str,
) -> io::Result<()> {
    let deadline = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(deadline);
    loop {
        let message = tokio::select! {
            _ = &mut deadline => return Err(io::Error::other(format!("SSH channel request timed out: {request}"))),
            _ = crate::ssh::wait_stopped(stop) => return Err(io::Error::other("SSH connection stopped")),
            message = channel.wait() => message,
        };
        if channel_request_response(message, pending, request)? {
            return Ok(());
        }
    }
}

fn channel_request_response(
    message: Option<russh::ChannelMsg>,
    pending: &mut std::collections::VecDeque<russh::ChannelMsg>,
    request: &str,
) -> io::Result<bool> {
    match message {
        Some(russh::ChannelMsg::Success) => Ok(true),
        Some(russh::ChannelMsg::Failure) => Err(io::Error::other(format!(
            "SSH channel request was rejected: {request}"
        ))),
        Some(russh::ChannelMsg::Close) | None => Err(io::Error::other(format!(
            "Connection closed before the SSH channel request reply: {request}"
        ))),
        Some(russh::ChannelMsg::WindowAdjusted { .. }) => Ok(false),
        Some(message) => {
            // Bound pre-response output even if the peer never acknowledges.
            if pending.len() >= 64 {
                return Err(io::Error::other(
                    "Too much output before the SSH channel reply",
                ));
            }
            pending.push_back(message);
            Ok(false)
        }
    }
}

fn new_shared(child: Option<Child>) -> Arc<Shared> {
    Arc::new(Shared {
        stop: AtomicBool::new(false),
        connected: AtomicBool::new(child.is_some()),
        revision: AtomicU64::new(1),
        generation: AtomicU64::new(0),
        query: Mutex::new(Query::default()),
        received_bytes: AtomicU64::new(0),
        encoding_errors: AtomicU64::new(0),
        trap: Mutex::new(crate::trap::Trap::default()),
        view: Mutex::new(None),
        status: Mutex::new("Starting connection / following".into()),
        diagnostics: Mutex::new(String::new()),
        child: Mutex::new(child),
        raw: Mutex::new(RawStream::default()),
    })
}

fn retain_received(
    state: &Shared,
    archive: &mut File,
    bytes: &mut u64,
    limit: u64,
    data: &str,
) -> io::Result<()> {
    let mut retained = data.len().min(limit.saturating_sub(*bytes) as usize);
    while !data.is_char_boundary(retained) {
        retained -= 1;
    }
    archive.write_all(&data.as_bytes()[..retained])?;
    state
        .trap
        .lock()
        .unwrap()
        .receive(&data.as_bytes()[..retained]);
    *bytes += retained as u64;
    state
        .received_bytes
        .fetch_add(retained as u64, Ordering::Relaxed);
    state.revision.fetch_add(1, Ordering::Relaxed);
    if retained < data.len() {
        return Err(io::Error::other(
            "Storage limit reached. Collection stopped; history is retained.",
        ));
    }
    *state.status.lock().unwrap() = format!("Following · Saved {} KiB", *bytes / 1024);
    Ok(())
}

fn spawn_history_worker(shared: &Arc<Shared>, path: &Path) {
    let state = shared.clone();
    let history_path = path.to_owned();
    thread::spawn(move || {
        let mut last = 0;
        while !state.stop.load(Ordering::Relaxed) {
            let revision = state.revision.load(Ordering::Relaxed);
            if last != revision {
                let query = state.query.lock().unwrap().clone();
                let view = make_view(&history_path, &query, &state);
                if state.generation.load(Ordering::Relaxed) == query.generation {
                    *state.view.lock().unwrap() = Some(view);
                }
                last = revision;
            }
            thread::sleep(Duration::from_millis(250));
        }
    });
}

fn make_view(path: &Path, query: &Query, state: &Shared) -> View {
    let _raw = state.raw.lock().unwrap();
    let result = (|| -> Result<View, String> {
        let mut view = View {
            generation: query.generation,
            results: json!([]),
            ..View::default()
        };
        if let Some(target) = query.context {
            view.text = history::context(path, target).map_err(|e| e.to_string())?;
            view.summary = format!("Context around history line {target} (without filtering)");
        } else {
            let matched = history::scan(
                path,
                &query.filter,
                &state.stop,
                &state.generation,
                query.generation,
            )?;
            for (_, line) in &matched.lines {
                view.text.push_str(line);
                view.text.push('\n');
            }
            history::trim_text(&mut view.text);
            view.summary = format!(
                "Filter matched {} lines · Showing up to the last 2,000 lines / 256KiB",
                matched.count
            );
        }
        if !query.search.text.is_empty() {
            let matches = history::scan(
                path,
                &query.search,
                &state.stop,
                &state.generation,
                query.generation,
            )?;
            view.summary.push_str(&format!(
                " · History search: {} matches (last 2,000 results maximum)",
                matches.count
            ));
            view.results = Value::Array(
                matches
                    .lines
                    .into_iter()
                    .map(|(number, text)| json!({"line":number, "text":text}))
                    .collect(),
            );
        }
        Ok(view)
    })();
    result.unwrap_or_else(|error| View {
        generation: query.generation,
        error,
        ..View::default()
    })
}

fn request_secret(
    tx: &mpsc::SyncSender<Prompt>,
    stop: &AtomicBool,
    message: &str,
) -> io::Result<String> {
    let (reply, receiver) = mpsc::channel();
    tx.try_send(Prompt {
        message: message.into(),
        confirmation: false,
        reply,
        credential: None,
    })
    .map_err(|_| io::Error::other("Too many authentication requests"))?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(io::Error::other("Privilege escalation stopped"));
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Some(answer)) if !answer.contains(['\n', '\r', '\0']) => return Ok(answer),
            Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("Privilege escalation cancelled"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

pub fn pattern(text: String, regex: bool, ignore_case: bool, invert: bool) -> Pattern {
    Pattern {
        text,
        regex,
        ignore_case,
        invert,
        numeric: Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn status_has_no_empty_diagnostic_lines() {
        assert_eq!(
            compact_status("Connection ended (exit status: 127)\n\r\nssh_srv: Connected\n"),
            "Connection ended (exit status: 127)\nssh_srv: Connected"
        );
    }
    #[test]
    fn command_encoding_change_replays_raw_without_running_again() {
        let session = Session::local_command(
            "printf '\\223\\372\\226\\173\\214\\352\\n'; sleep 1; printf '\\223\\372\\n'".into(),
            4096,
            "UTF-8",
        )
        .unwrap();
        thread::sleep(Duration::from_millis(300));
        assert!(fs::read_to_string(&session.path).unwrap().contains('�'));
        session.change_encoding("CP932").unwrap();
        wait_for_archive(&session, "日本語\n");
        thread::sleep(Duration::from_millis(1200));
        wait_for_archive(&session, "日本語\n日\n");
        assert!(!session.is_connected());
        session.change_encoding("UTF-8").unwrap();
        assert!(fs::read_to_string(&session.path).unwrap().contains('�'));
        session.change_encoding("CP932").unwrap();
        assert_eq!(fs::read_to_string(&session.path).unwrap(), "日本語\n日\n");
    }

    #[test]
    fn connection_state_tracks_local_follow_and_stop() {
        let dir = create_directory().unwrap();
        let path = dir.join("input.log");
        fs::write(&path, "initial\n").unwrap();
        let session =
            Session::local_encoded(path.display().to_string(), 10, 1024, "UTF-8").unwrap();
        assert!(session.is_connected());
        session.stop();
        assert!(!session.is_connected());
    }

    #[test]
    fn channel_request_waits_past_window_updates_and_preserves_early_output() {
        use russh::ChannelMsg;
        let mut pending = std::collections::VecDeque::new();
        assert!(
            !channel_request_response(
                Some(ChannelMsg::WindowAdjusted {
                    new_size: 2_097_152
                }),
                &mut pending,
                "exec"
            )
            .unwrap()
        );
        assert!(pending.is_empty());
        assert!(
            !channel_request_response(
                Some(ChannelMsg::Data {
                    data: b"early output".to_vec().into()
                }),
                &mut pending,
                "exec"
            )
            .unwrap()
        );
        assert!(
            !channel_request_response(
                Some(ChannelMsg::ExtendedData {
                    ext: 1,
                    data: b"early stderr".to_vec().into()
                }),
                &mut pending,
                "exec"
            )
            .unwrap()
        );
        assert!(channel_request_response(Some(ChannelMsg::Success), &mut pending, "exec").unwrap());
        assert!(
            matches!(pending.pop_front(), Some(ChannelMsg::Data { data }) if data.as_ref() == b"early output")
        );
        assert!(
            matches!(pending.pop_front(), Some(ChannelMsg::ExtendedData { data, .. }) if data.as_ref() == b"early stderr")
        );
    }

    #[test]
    fn channel_request_reports_rejection_and_disconnect_separately() {
        let mut pending = std::collections::VecDeque::new();
        assert_eq!(
            channel_request_response(Some(russh::ChannelMsg::Failure), &mut pending, "PTY")
                .unwrap_err()
                .to_string(),
            "SSH channel request was rejected: PTY"
        );
        for message in [None, Some(russh::ChannelMsg::Close)] {
            assert_eq!(
                channel_request_response(message, &mut pending, "exec")
                    .unwrap_err()
                    .to_string(),
                "Connection closed before the SSH channel request reply: exec"
            );
        }
    }
    fn wait_for_archive(session: &Session, expected: &str) {
        for _ in 0..150 {
            if fs::read_to_string(&session.path).unwrap() == expected {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read_to_string(&session.path).unwrap(), expected);
    }

    #[test]
    fn native_local_follow_preserves_history_across_rotation_and_truncation() {
        let directory = create_directory().unwrap();
        let source = directory.join("native.log");
        fs::write(&source, "old\ninitial\n").unwrap();
        let session = Session::local(source.display().to_string(), 1, 4096).unwrap();
        assert!(session.shared.child.lock().unwrap().is_none());
        wait_for_archive(&session, "initial\n");
        OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"append\n")
            .unwrap();
        wait_for_archive(&session, "initial\nappend\n");
        fs::write(&source, "new\n").unwrap();
        wait_for_archive(&session, "initial\nappend\nnew\n");
        fs::rename(&source, directory.join("native.old")).unwrap();
        thread::sleep(Duration::from_millis(150));
        fs::write(&source, "replacement\n").unwrap();
        wait_for_archive(&session, "initial\nappend\nnew\nreplacement\n");
        let archive = session.path.clone();
        drop(session);
        thread::sleep(Duration::from_millis(300));
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn native_local_decodes_split_input_and_enforces_capacity() {
        let directory = create_directory().unwrap();
        let source = directory.join("encoded.log");
        fs::write(&source, b"old\n").unwrap();
        let session = Session::local_encoded(source.display().to_string(), 0, 4, "CP932").unwrap();
        let mut output = OpenOptions::new().append(true).open(&source).unwrap();
        output.write_all(&[0x93]).unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(session.received_bytes(), 0);
        output.write_all(&[0xfa, b'\n']).unwrap();
        wait_for_archive(&session, "日\n");
        output.write_all(b"overflow\n").unwrap();
        for _ in 0..150 {
            if session.status().contains("Storage limit") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.status().contains("Storage limit"));
        assert_eq!(session.received_bytes(), 4);
        wait_for_archive(&session, "日\n");
        let archive = session.path.clone();
        drop(session);
        drop(output);
        thread::sleep(Duration::from_millis(300));
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn trap_detects_new_receives_independently_of_filter_and_configuration() {
        let directory = create_directory().unwrap();
        let source = directory.join("trap.log");
        fs::write(&source, "ERROR old\n").unwrap();
        let session = Session::local(source.display().to_string(), 0, 1024).unwrap();
        // Let tail establish its initial end-of-file position.
        thread::sleep(Duration::from_millis(200));
        session.set_trap("ERROR".into(), true);
        session.query(Query {
            filter: pattern("not-visible".into(), false, false, false),
            ..Query::default()
        });
        assert_eq!(session.trap_hits(), 0);
        OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"error new\n")
            .unwrap();
        for _ in 0..100 {
            if session.trap_hits() > 0 {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(session.trap_hits(), 1);
        session.set_trap("ERROR".into(), false);
        assert_eq!(session.trap_hits(), 1);
        let archive = session.path.clone();
        drop(session);
        // Reader is stopped asynchronously; preserve the fixture until reaped.
        thread::sleep(Duration::from_millis(100));
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn docker_source_archives_stdout_and_stderr_with_expected_arguments() {
        let directory = create_directory().unwrap();
        let executable = directory.join("docker");
        fs::write(&executable, "#!/bin/sh\n[ \"$*\" = 'logs --tail 20 --follow --timestamps web-1' ] || exit 9\nprintf 'container-out\\n'\nprintf 'container-error\\n' >&2\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(
                crate::source::LogSource::Docker
                    .command("web-1", 20)
                    .unwrap(),
            )
            .env("PATH", &directory);
        let session = Session::start(command, 1024, None).unwrap();
        for _ in 0..100 {
            if session.status().contains("Connection ended") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(&session.path).unwrap(),
            "container-out\ncontainer-error\n"
        );
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn custom_script_archives_pipeline_multiline_and_stderr() {
        let script = "value='quoted value'\nprintf '%s\\n' \"$value\" | tr 'a-z' 'A-Z'\nprintf 'diagnostic\\n' >&2";
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(
            crate::source::LogSource::Custom
                .command(script, 50)
                .unwrap(),
        );
        let session = Session::start(command, 1024, None).unwrap();
        for _ in 0..150 {
            if session.status().contains("Connection ended") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(&session.path).unwrap(),
            "QUOTED VALUE\ndiagnostic\n"
        );
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
    }

    #[test]
    fn custom_script_uses_existing_elevation_wrappers() {
        let mut options = SshOptions {
            source: "custom".into(),
            host: "server".into(),
            user: "".into(),
            port: 0,
            key: "".into(),
            path: "printf '%s' \"$HOME\" | cat\necho error >&2".into(),
            initial: 0,
            elevation: 0,
            run_user: "root".into(),
        };
        assert_eq!(
            remote_command(&options, "MARKER").unwrap(),
            crate::source::LogSource::Custom
                .command(&options.path, 0)
                .unwrap()
        );
        for (mode, wrapper) in [(1, "sudo -p"), (2, "su -")] {
            options.elevation = mode;
            let command = remote_command(&options, "MARKER").unwrap();
            assert!(command.contains(wrapper));
            assert!(command.contains("MARKER"));
            let inner = format!(
                "stty -echo -onlcr || exit 1; printf '%s' {}; {}",
                quote("MARKER"),
                crate::source::LogSource::Custom
                    .command(&options.path, 0)
                    .unwrap()
            );
            let quoted_inner = if mode == 1 {
                quote(&inner)
            } else {
                quote(&format!("exec /bin/sh -c {}", quote(&inner)))
            };
            assert!(command.contains(&quoted_inner));
        }
    }

    #[test]
    fn local_command_captures_both_streams_with_pipelines_and_encoding() {
        let session = Session::local_command(
            "printf '\\223\\372\\226\\173\\214\\352\\n' | cat; printf 'stderr\\n' >&2".into(),
            4096,
            "CP932",
        )
        .unwrap();
        for _ in 0..150 {
            if session.status().contains("Connection ended") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(&session.path).unwrap(),
            "日本語\nstderr\n"
        );
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        assert!(Session::local_command(" ".into(), 4096, "UTF-8").is_err());
    }

    #[test]
    fn stopping_local_command_terminates_foreground_process() {
        let session = Session::local_command("exec sleep 30".into(), 4096, "UTF-8").unwrap();
        session.stop();
        for _ in 0..150 {
            if session.shared.child.lock().unwrap().is_none() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.shared.child.lock().unwrap().is_none());
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
    }

    #[test]
    fn cp932_stream_is_saved_searched_and_trapped_as_utf8() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 0.1; printf '\\223'; sleep 0.1; printf '\\372\\226\\173\\214\\352\\n'; printf '\\223'; sleep 0.1"]);
        let session = Session::start_encoded(command, 4096, None, "CP932").unwrap();
        session.set_trap("日本語".into(), false);
        for _ in 0..150 {
            if session.status().contains("Connection ended") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(
            session
                .status()
                .contains("replaced 1 malformed / incomplete sequences"),
            "{}",
            session.status()
        );
        assert_eq!(fs::read_to_string(&session.path).unwrap(), "日本語\n�");
        assert_eq!(session.trap_hits(), 1);
        let matches = history::scan(
            &session.path,
            &pattern("日本語".into(), false, false, false),
            &AtomicBool::new(false),
            &AtomicU64::new(0),
            0,
        )
        .unwrap();
        assert_eq!(matches.count, 1);
        assert_eq!(matches.lines[0].1, "日本語");
        assert!(
            history::context(&session.path, 1)
                .unwrap()
                .contains("日本語")
        );
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
    }

    #[test]
    fn conversion_capacity_stops_at_utf8_character_boundary() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "printf '\\223\\372\\226\\173\\214\\352\\n'"]);
        let session = Session::start_encoded(command, 5, None, "CP932").unwrap();
        for _ in 0..150 {
            if session.status().contains("Storage limit") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.status().contains("Storage limit"));
        assert_eq!(fs::read_to_string(&session.path).unwrap(), "日");
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
    }

    #[test]
    fn elevation_markers_are_removed_before_cp932_conversion() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf 'banner READY\\223\\372\\226\\173\\214\\352\\n'",
        ]);
        let session = Session::start_encoded(
            command,
            4096,
            Some(("READY".into(), "sudo".into())),
            "CP932",
        )
        .unwrap();
        for _ in 0..150 {
            if session.status().contains("Connection ended") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fs::read_to_string(&session.path).unwrap(), "日本語\n");
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
    }

    #[test]
    fn docker_source_uses_same_sudo_and_su_wrappers() {
        let mut options = SshOptions {
            source: "docker".into(),
            host: "server".into(),
            user: String::new(),
            port: 0,
            key: String::new(),
            path: "web-1".into(),
            initial: 50,
            elevation: 1,
            run_user: "root".into(),
        };
        let command = remote_command(&options, "MARKER").unwrap();
        assert!(command.contains("sudo -p"));
        assert!(command.contains("docker logs --tail 50 --follow --timestamps"));
        assert!(command.contains("2>&1"));
        options.elevation = 2;
        assert!(
            remote_command(&options, "MARKER")
                .unwrap()
                .contains("exec su -")
        );
    }
    #[test]
    fn ssh_diagnostics_survive_elevation_failure() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("printf 'Permission denied (password).\\n' >&2; exit 255");
        let session = Session::start(command, 1024, Some(("READY".into(), "sudo".into()))).unwrap();
        for _ in 0..100 {
            if session.status().contains("Privilege escalation failed") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.status().contains("Permission denied (password)."));
        let path = session.path.clone();
        drop(session);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
    #[test]
    fn elevation_password_is_not_archived() {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg("printf 'banner\\nPassword:'; IFS= read -r secret; test \"$secret\" = 'test-secret' || exit 1; printf 'READYprivileged-log\\n'");
        let session =
            Session::start(command, 1024, Some(("READY".into(), "sudo test".into()))).unwrap();
        let prompt = session
            .prompts
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        assert_eq!(prompt.message, "sudo test");
        assert!(fs::read(&session.path).unwrap().is_empty());
        prompt.reply.send(Some("test-secret".into())).unwrap();
        for _ in 0..100 {
            if session.status().contains("Connection ended") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            fs::read_to_string(&session.path).unwrap(),
            "privileged-log\n"
        );
        let path = session.path.clone();
        drop(session);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn elevation_cancel_keeps_archive_empty() {
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg("printf 'Password:'; read secret; printf 'READYshould-not-be-saved'");
        let session =
            Session::start(command, 1024, Some(("READY".into(), "su test".into()))).unwrap();
        session
            .prompts
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .reply
            .send(None)
            .unwrap();
        for _ in 0..100 {
            if session.status().contains("cancelled") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.status().contains("cancelled"));
        assert!(fs::read(&session.path).unwrap().is_empty());
        let path = session.path.clone();
        drop(session);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn elevation_command_quotes_nested_paths_and_selects_tty() {
        let mut options = SshOptions {
            source: "file".into(),
            host: "server".into(),
            user: String::new(),
            port: 0,
            key: String::new(),
            path: "/tmp/a'b;$(unsafe)".into(),
            initial: 50,
            elevation: 1,
            run_user: "root".into(),
        };
        let command = remote_command(&options, "MARKER").unwrap();
        assert!(command.contains("sudo -p"));
        options.elevation = 2;
        assert!(
            remote_command(&options, "MARKER")
                .unwrap()
                .contains("exec su - 'root' -c")
        );
        options.run_user = "root;echo unsafe".into();
        assert!(remote_command(&options, "MARKER").is_err());
    }
    #[test]
    fn remote_path_is_shell_quoted() {
        assert_eq!(quote("/tmp/a'b;$(x)"), "'/tmp/a'\\''b;$(x)'");
        let options = SshOptions {
            source: "file".into(),
            host: "-bad".into(),
            user: String::new(),
            port: 0,
            key: String::new(),
            path: "/tmp/log".into(),
            initial: 50,
            elevation: 0,
            run_user: "root".into(),
        };
        assert!(crate::ssh::validate(&options).is_err());
    }
    #[test]
    fn session_preserves_old_history_filters_searches_and_stops() {
        let dir = create_directory().unwrap();
        let source = dir.join("source.log");
        fs::write(&source, "before\nERROR old\ninfo\n").unwrap();
        let session = Session::local(source.display().to_string(), 2, 1024 * 1024).unwrap();
        fn await_view(
            session: &Session,
            generation: u64,
            predicate: impl Fn(&View) -> bool,
        ) -> View {
            for _ in 0..100 {
                if let Some(view) = session.view()
                    && view.generation == generation
                    && predicate(&view)
                {
                    return view;
                }
                thread::sleep(Duration::from_millis(50));
            }
            panic!("view timeout: {}", session.status());
        }
        await_view(&session, 0, |view| view.text.contains("ERROR old"));
        OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all("new\n".repeat(2500).as_bytes())
            .unwrap();
        await_view(&session, 0, |view| {
            view.text.contains("new") && !view.text.contains("ERROR old")
        });
        let generation = session.query(Query {
            filter: pattern("error".into(), false, true, false),
            search: pattern("ERROR".into(), true, false, false),
            ..Query::default()
        });
        let received = session.received_bytes();
        let view = await_view(&session, generation, |view| {
            view.results.as_array().is_some_and(|a| !a.is_empty())
        });
        assert!(view.text.contains("ERROR old"));
        assert_eq!(session.received_bytes(), received);
        assert!(received > 0);
        assert_eq!(view.results[0]["line"], 1);
        assert!(
            !fs::read_to_string(&session.path)
                .unwrap()
                .contains("before")
        );
        let generation = session.query(Query {
            filter: pattern("ERROR".into(), false, false, true),
            ..Query::default()
        });
        let view = await_view(&session, generation, |view| view.text.contains("new"));
        assert!(!view.text.contains("ERROR"));
        let generation = session.query(Query {
            context: Some(1),
            ..Query::default()
        });
        assert!(
            await_view(&session, generation, |view| view
                .text
                .contains("1: ERROR old"))
            .text
            .contains("2: info")
        );
        let generation = session.query(Query {
            filter: pattern("[".into(), true, false, false),
            ..Query::default()
        });
        assert!(
            !await_view(&session, generation, |view| !view.error.is_empty())
                .error
                .is_empty()
        );
        assert!(session.shared.child.lock().unwrap().is_none());
        let archive = session.path.clone();
        session.stop();
        thread::sleep(Duration::from_millis(150));
        let received = session.received_bytes();
        OpenOptions::new()
            .append(true)
            .open(&source)
            .unwrap()
            .write_all(b"after-stop\n")
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        assert_eq!(session.received_bytes(), received);
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn capacity_limit_stops_without_discarding_saved_history() {
        let dir = create_directory().unwrap();
        let source = dir.join("source.log");
        fs::write(&source, b"1234567890\n").unwrap();
        let session = Session::local(source.display().to_string(), 50, 5).unwrap();
        for _ in 0..100 {
            if session.status().contains("Storage limit") {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(session.status().contains("Storage limit"));
        assert_eq!(fs::read(&session.path).unwrap(), b"12345");
        let archive = session.path.clone();
        drop(session);
        fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }
}
