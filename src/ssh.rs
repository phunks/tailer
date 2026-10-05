use crate::session::{Prompt, SshOptions};
use russh::{client, keys};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("SSH runtime")
    })
}

pub fn error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

pub struct Transport {
    handle: tokio::sync::Mutex<Option<client::Handle<Handler>>>,
    pub stop: AtomicBool,
    ready: AtomicBool,
    status: Mutex<String>,
}

impl Transport {
    pub fn start(options: &SshOptions) -> io::Result<(Arc<Self>, mpsc::Receiver<Prompt>)> {
        Self::start_with_home(options, home()?)
    }

    fn start_with_home(
        options: &SshOptions,
        home: PathBuf,
    ) -> io::Result<(Arc<Self>, mpsc::Receiver<Prompt>)> {
        validate(options)?;
        let options = options.clone();
        let transport = Arc::new(Self {
            handle: tokio::sync::Mutex::new(None),
            stop: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            status: Mutex::new("Starting connection / following".into()),
        });
        let (tx, input) = mpsc::sync_channel(8);
        let (output, prompts) = mpsc::sync_channel(8);
        let state = transport.clone();
        let identity =
            serde_json::json!([options.host, options.port, options.user, options.key]).to_string();
        std::thread::spawn(move || {
            crate::credentials::broker(identity, input, output, &state.stop)
        });
        let state = transport.clone();
        runtime().spawn(async move {
            let result = tokio::select! {
                result = connect(&options, tx, state.clone(), home) => result,
                _ = wait_stopped(&state.stop) => Err(error("SSH connection stopped")),
            };
            match result {
                Ok(handle) => {
                    *state.handle.lock().await = Some(handle);
                    state.ready.store(true, Ordering::Release);
                    *state.status.lock().unwrap() = "Connected".into();
                    loop {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        let mut guard = state.handle.lock().await;
                        let Some(handle) = guard.as_mut() else { break };
                        if state.stop.load(Ordering::Relaxed) || handle.is_closed() {
                            let _ = handle
                                .disconnect(russh::Disconnect::ByApplication, "closed", "en")
                                .await;
                            guard.take();
                            break;
                        }
                    }
                    state.ready.store(false, Ordering::Release);
                    state.stop.store(true, Ordering::Relaxed);
                    *state.status.lock().unwrap() =
                        "Connection ended · Saved history remains searchable".into();
                }
                Err(err) => {
                    state.stop.store(true, Ordering::Relaxed);
                    *state.status.lock().unwrap() = format!("Connection ended: {err}");
                }
            }
        });
        Ok((transport, prompts))
    }
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire) && !self.stop.load(Ordering::Relaxed)
    }
    pub fn status(&self) -> String {
        self.status.lock().unwrap().clone()
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub async fn channel(&self) -> io::Result<russh::Channel<client::Msg>> {
        if !self.is_ready() {
            return Err(error("Shared SSH connection has ended"));
        }
        let guard = self.handle.lock().await;
        let handle = guard
            .as_ref()
            .ok_or_else(|| error("Shared SSH connection has ended"))?;
        tokio::time::timeout(Duration::from_secs(15), handle.channel_open_session())
            .await
            .map_err(error)?
            .map_err(error)
    }
}

pub async fn wait_stopped(stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub fn validate(options: &SshOptions) -> io::Result<()> {
    if options.host.is_empty()
        || options.host.starts_with('-')
        || options
            .host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(error("Check the host name"));
    }
    Ok(())
}

fn home() -> io::Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| error("Home directory was not found"))
}

pub async fn prompt(
    tx: &mpsc::SyncSender<Prompt>,
    stop: &AtomicBool,
    message: String,
    confirmation: bool,
) -> io::Result<String> {
    if message.len() > 16 * 1024 {
        return Err(error("Authentication prompt is too long"));
    }
    let (reply, answer) = mpsc::channel();
    tx.try_send(Prompt {
        message,
        confirmation,
        reply,
        credential: None,
    })
    .map_err(|_| error("Too many authentication requests"))?;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Err(error("Authentication stopped"));
        }
        match answer.try_recv() {
            Ok(Some(answer)) if answer.len() <= 16 * 1024 && !answer.contains('\0') => {
                return Ok(answer);
            }
            Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                return Err(error("Authentication cancelled"));
            }
            Err(mpsc::TryRecvError::Empty) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

pub struct Handler {
    host: String,
    port: u16,
    known_hosts: PathBuf,
    prompts: mpsc::SyncSender<Prompt>,
    state: Arc<Transport>,
}

impl client::Handler for Handler {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        if key.certificate().is_some() {
            return Err(error("SSH host certificates are not supported").into());
        }
        let key = key.public_key();
        match check_host(&self.host, self.port, &key, &self.known_hosts)? {
            true => Ok(true),
            false => {
                let message = format!(
                    "SSH host key: {}:{}\n{}\n{}\nVerify that the destination is trusted.",
                    self.host,
                    self.port,
                    key.algorithm(),
                    key.fingerprint(keys::HashAlg::Sha256)
                );
                prompt(&self.prompts, &self.state.stop, message, true).await?;
                // Serialize the recheck and append to avoid racing first connections.
                static SAVE: Mutex<()> = Mutex::new(());
                let _guard = SAVE.lock().unwrap();
                if !check_host(&self.host, self.port, &key, &self.known_hosts)? {
                    keys::known_hosts::learn_known_hosts_path(
                        &self.host,
                        self.port,
                        &key,
                        &self.known_hosts,
                    )
                    .map_err(error)?;
                }
                Ok(true)
            }
        }
    }
}

fn check_host(host: &str, port: u16, key: &keys::PublicKey, path: &Path) -> io::Result<bool> {
    match std::fs::metadata(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
        Ok(_) => {
            std::fs::File::open(path)?;
        }
    }
    // russh's known-hosts parser does not enforce OpenSSH marker semantics.
    // Fail closed rather than silently ignoring @revoked/@cert-authority.
    if std::fs::read_to_string(path)?
        .lines()
        .any(|line| line.trim_start().starts_with('@'))
    {
        return Err(error(
            "The @revoked / @cert-authority format in known_hosts is not supported",
        ));
    }
    keys::check_known_hosts_path(host, port, key, path).map_err(error)
}

async fn connect(
    options: &SshOptions,
    tx: mpsc::SyncSender<Prompt>,
    state: Arc<Transport>,
    home: PathBuf,
) -> io::Result<client::Handle<Handler>> {
    let user = if options.user.is_empty() {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .map_err(error)?
    } else {
        options.user.clone()
    };
    let port = if options.port == 0 { 22 } else { options.port };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 3,
        ..Default::default()
    });
    let handler = Handler {
        host: options.host.clone(),
        port,
        known_hosts: home.join(".ssh/known_hosts"),
        prompts: tx.clone(),
        state: state.clone(),
    };
    // Apply timeout to TCP only: host-key confirmation must remain interactive.
    let socket = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::net::TcpStream::connect((options.host.as_str(), port)),
    )
    .await
    .map_err(error)??;
    let mut handle = client::connect_stream(config, socket, handler)
        .await
        .map_err(error)?;
    let mut auth = handle.authenticate_none(&user).await.map_err(error)?;
    if auth.success() {
        return Ok(handle);
    }
    let paths = if options.key.is_empty() {
        ["id_ed25519", "id_ecdsa", "id_rsa"]
            .iter()
            .map(|name| home.join(".ssh").join(name))
            .filter(|p| p.is_file())
            .collect::<Vec<_>>()
    } else {
        vec![if let Some(rest) = options.key.strip_prefix("~/") {
            home.join(rest)
        } else {
            PathBuf::from(&options.key)
        }]
    };
    for path in paths {
        let key = match keys::load_secret_key(&path, None) {
            Ok(key) => key,
            Err(keys::Error::KeyIsEncrypted) => {
                let mut loaded = None;
                for _ in 0..3 {
                    let password = prompt(
                        &tx,
                        &state.stop,
                        format!("Enter passphrase for key '{}':", path.display()),
                        false,
                    )
                    .await?;
                    if let Ok(key) = keys::load_secret_key(&path, Some(&password)) {
                        loaded = Some(key);
                        break;
                    }
                }
                loaded.ok_or_else(|| error("Failed to decrypt the private key"))?
            }
            Err(err) => return Err(error(format!("Private key {}: {err}", path.display()))),
        };
        let hash = handle
            .best_supported_rsa_hash()
            .await
            .map_err(error)?
            .flatten();
        auth = handle
            .authenticate_publickey(&user, keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
            .map_err(error)?;
        if auth.success() {
            return Ok(handle);
        }
    }
    if let client::AuthResult::Failure {
        remaining_methods, ..
    } = &auth
    {
        if remaining_methods.contains(&russh::MethodKind::KeyboardInteractive) {
            let mut response = handle
                .authenticate_keyboard_interactive_start(&user, None)
                .await
                .map_err(error)?;
            for _ in 0..32 {
                match response {
                    client::KeyboardInteractiveAuthResponse::Success => return Ok(handle),
                    client::KeyboardInteractiveAuthResponse::Failure { .. } => break,
                    client::KeyboardInteractiveAuthResponse::InfoRequest {
                        name,
                        instructions,
                        prompts,
                    } => {
                        let mut answers = Vec::new();
                        for item in prompts {
                            answers.push(
                                prompt(
                                    &tx,
                                    &state.stop,
                                    format!(
                                        "keyboard-interactive\n{name}\n{instructions}\n{}",
                                        item.prompt
                                    ),
                                    false,
                                )
                                .await?,
                            );
                        }
                        response = handle
                            .authenticate_keyboard_interactive_respond(answers)
                            .await
                            .map_err(error)?;
                    }
                }
            }
        }
    }
    for _ in 0..3 {
        let password = prompt(
            &tx,
            &state.stop,
            format!("{user}@{} password:", options.host),
            false,
        )
        .await?;
        if handle
            .authenticate_password(&user, password)
            .await
            .map_err(error)?
            .success()
        {
            return Ok(handle);
        }
    }
    Err(error("SSH authentication failed"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::{Channel, ChannelId, server};
    use std::sync::atomic::AtomicU64;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "tailer-russh-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn key(seed: u8) -> keys::PrivateKey {
        keys::ssh_key::private::Ed25519Keypair::from_seed(&[seed; 32]).into()
    }
    struct TestServer {
        public_key: keys::PublicKey,
        commands: Arc<Mutex<Vec<String>>>,
        elevated: std::collections::HashMap<ChannelId, String>,
        interactive: bool,
    }
    impl server::Handler for TestServer {
        type Error = russh::Error;
        async fn auth_password(
            &mut self,
            user: &str,
            password: &str,
        ) -> Result<server::Auth, Self::Error> {
            Ok(if user == "test" && password == "secret" {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            })
        }
        async fn auth_keyboard_interactive<'a>(
            &'a mut self,
            _: &str,
            _: &str,
            response: Option<server::Response<'a>>,
        ) -> Result<server::Auth, Self::Error> {
            if !self.interactive {
                return Ok(server::Auth::reject());
            }
            if let Some(mut response) = response {
                return Ok(
                    if response
                        .next()
                        .is_some_and(|answer| answer.as_ref() == b"otp-code")
                    {
                        server::Auth::Accept
                    } else {
                        server::Auth::reject()
                    },
                );
            }
            Ok(server::Auth::Partial {
                name: "test".into(),
                instructions: "verification".into(),
                prompts: vec![("OTP:".into(), false)].into(),
            })
        }
        async fn auth_publickey(
            &mut self,
            user: &str,
            key: &keys::PublicKey,
        ) -> Result<server::Auth, Self::Error> {
            Ok(if user == "test" && *key == self.public_key {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            })
        }
        async fn channel_open_session(
            &mut self,
            _: Channel<server::Msg>,
            reply: server::ChannelOpenHandle,
            _: &mut server::Session,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }
        async fn exec_request(
            &mut self,
            channel: ChannelId,
            command: &[u8],
            session: &mut server::Session,
        ) -> Result<(), Self::Error> {
            self.commands
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(command).into_owned());
            session.channel_success(channel)?;
            if command.windows(7).any(|part| part == b"sudo -p") {
                let command = String::from_utf8_lossy(command);
                let start = command.find('\x1e').unwrap();
                let end = command[start..].find('\x1f').unwrap() + start + 1;
                self.elevated.insert(channel, command[start..end].into());
                session.data(channel, b"Password:".to_vec())?;
                return Ok(());
            }
            session.data(channel, b"remote-out\n".to_vec())?;
            session.extended_data(channel, 1, b"remote-error\n".to_vec())?;
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
            Ok(())
        }
        async fn pty_request(
            &mut self,
            channel: ChannelId,
            _: &str,
            _: u32,
            _: u32,
            _: u32,
            _: u32,
            modes: &[(russh::Pty, u32)],
            session: &mut server::Session,
        ) -> Result<(), Self::Error> {
            assert!(modes.contains(&(russh::Pty::ECHO, 0)));
            session.channel_success(channel)?;
            Ok(())
        }
        async fn data(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut server::Session,
        ) -> Result<(), Self::Error> {
            assert_eq!(data, b"sudo-secret\n");
            let marker = self.elevated.remove(&channel).unwrap();
            session.data(channel, format!("{marker}remote-out\n").into_bytes())?;
            session.extended_data(channel, 1, b"remote-error\n".to_vec())?;
            session.exit_status_request(channel, 0)?;
            session.close(channel)?;
            Ok(())
        }
    }

    fn options(port: u16) -> SshOptions {
        SshOptions {
            source: "custom".into(),
            host: "127.0.0.1".into(),
            user: "test".into(),
            port,
            key: String::new(),
            path: "echo remote".into(),
            initial: 0,
            elevation: 0,
            run_user: "root".into(),
        }
    }

    #[test]
    fn host_key_known_unknown_and_changed_are_distinguished() {
        let fixture = Fixture::new();
        let path = fixture.0.join("known_hosts");
        assert!(!check_host("localhost", 2222, key(1).public_key(), &path).unwrap());
        keys::known_hosts::learn_known_hosts_path("localhost", 2222, key(1).public_key(), &path)
            .unwrap();
        assert!(check_host("localhost", 2222, key(1).public_key(), &path).unwrap());
        assert!(check_host("localhost", 2222, key(2).public_key(), &path).is_err());
    }

    #[test]
    fn encrypted_openssh_key_loads_only_with_correct_passphrase() {
        let fixture = Fixture::new();
        let path = fixture.0.join("encrypted");
        let encrypted = key(2)
            .encrypt_with(
                keys::ssh_key::Cipher::Aes256Ctr,
                keys::ssh_key::Kdf::Bcrypt {
                    salt: vec![9; 16],
                    rounds: 1,
                },
                1234,
                "passphrase",
            )
            .unwrap();
        std::fs::write(
            &path,
            encrypted
                .to_openssh(keys::ssh_key::LineEnding::LF)
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
        assert!(matches!(
            keys::load_secret_key(&path, None),
            Err(keys::Error::KeyIsEncrypted)
        ));
        assert!(keys::load_secret_key(&path, Some("wrong")).is_err());
        assert_eq!(
            keys::load_secret_key(&path, Some("passphrase"))
                .unwrap()
                .public_key(),
            key(2).public_key()
        );
    }

    #[test]
    fn native_ssh_authenticates_and_shares_channels_without_processes() {
        for (use_key, elevation, interactive) in [
            (false, 0, false),
            (true, 0, false),
            (true, 1, false),
            (false, 0, true),
        ] {
            let fixture = Fixture::new();
            let commands = Arc::new(Mutex::new(Vec::new()));
            let (port_tx, port_rx) = mpsc::channel();
            let server_commands = commands.clone();
            let server_task = runtime().spawn(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                port_tx.send(listener.local_addr().unwrap().port()).unwrap();
                let (socket, _) = listener.accept().await.unwrap();
                let config = Arc::new(server::Config {
                    keys: vec![key(1)],
                    auth_rejection_time: Duration::ZERO,
                    auth_rejection_time_initial: Some(Duration::ZERO),
                    ..Default::default()
                });
                let session = server::run_stream(
                    config,
                    socket,
                    TestServer {
                        public_key: key(2).public_key().clone(),
                        commands: server_commands,
                        elevated: std::collections::HashMap::new(),
                        interactive,
                    },
                )
                .await
                .unwrap();
                let _ = session.await;
            });
            let mut options = options(port_rx.recv_timeout(Duration::from_secs(3)).unwrap());
            options.elevation = elevation;
            if use_key {
                let path = fixture.0.join("identity");
                std::fs::write(
                    &path,
                    key(2)
                        .to_openssh(keys::ssh_key::LineEnding::LF)
                        .unwrap()
                        .as_bytes(),
                )
                .unwrap();
                options.key = path.display().to_string();
            }
            let (transport, prompts) =
                Transport::start_with_home(&options, fixture.0.clone()).unwrap();
            let host = prompts.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(host.confirmation);
            assert!(host.message.contains("SHA256:"));
            host.reply.send(Some(String::new())).unwrap();
            if !use_key {
                let password = prompts.recv_timeout(Duration::from_secs(5)).unwrap();
                assert!(!password.confirmation);
                password
                    .reply
                    .send(Some(if interactive { "otp-code" } else { "secret" }.into()))
                    .unwrap();
            }
            for _ in 0..150 {
                if transport.is_ready() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(transport.is_ready(), "{}", transport.status());
            let managed = std::rc::Rc::new(crate::connections::Managed {
                transport: transport.clone(),
                prompts,
                name: "test".into(),
            });
            let first = crate::session::Session::shared_ssh(options.clone(), 4096, managed.clone())
                .unwrap();
            let second =
                crate::session::Session::shared_ssh(options.clone(), 4096, managed.clone())
                    .unwrap();
            for session in [&first, &second] {
                if elevation != 0 {
                    let prompt = session
                        .prompts
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                    prompt.reply.send(Some("sudo-secret".into())).unwrap();
                }
                for _ in 0..150 {
                    if session.status().contains("Connection ended") {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                assert_eq!(
                    std::fs::read_to_string(&session.path).unwrap(),
                    "remote-out\nremote-error\n",
                    "{}",
                    session.status()
                );
            }
            assert_eq!(commands.lock().unwrap().len(), 2);
            assert!(commands.lock().unwrap()[0].contains("/bin/sh"));
            let paths = [first.path.clone(), second.path.clone()];
            drop(first);
            assert!(transport.is_ready());
            transport.stop();
            let failed =
                crate::session::Session::shared_ssh(options, 4096, managed.clone()).unwrap();
            for _ in 0..150 {
                if failed.status().contains("Connection ended") {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(std::fs::read(&failed.path).unwrap().is_empty());
            assert_eq!(commands.lock().unwrap().len(), 2);
            let failed_path = failed.path.clone();
            drop(failed);
            drop(second);
            drop(managed);
            std::thread::sleep(Duration::from_millis(300));
            for path in paths.into_iter().chain([failed_path]) {
                let _ = std::fs::remove_dir_all(path.parent().unwrap());
            }
            server_task.abort();
        }
    }

    #[test]
    #[ignore = "requires a disposable OpenSSH server and TAILER_TEST_SSH_KEY / PORT / USER"]
    fn real_openssh_exec_handles_window_adjustment_before_success() {
        let fixture = Fixture::new();
        let mut options = options(
            std::env::var("TAILER_TEST_SSH_PORT")
                .unwrap()
                .parse()
                .unwrap(),
        );
        options.user = std::env::var("TAILER_TEST_SSH_USER").unwrap();
        options.key = std::env::var("TAILER_TEST_SSH_KEY").unwrap();
        options.path = "printf 'openssh-out\\n'; printf 'openssh-error\\n' >&2".into();
        let (transport, prompts) = Transport::start_with_home(&options, fixture.0.clone()).unwrap();
        prompts
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .reply
            .send(Some("yes".into()))
            .unwrap();
        for _ in 0..250 {
            if transport.is_ready() || transport.stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(transport.is_ready(), "{}", transport.status());
        let managed = std::rc::Rc::new(crate::connections::Managed {
            transport,
            prompts,
            name: "OpenSSH".into(),
        });
        let session = crate::session::Session::shared_ssh(options, 4096, managed.clone()).unwrap();
        for _ in 0..250 {
            if session.status().contains("Connection ended") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            std::fs::read_to_string(&session.path).unwrap(),
            "openssh-out\nopenssh-error\n",
            "{}",
            session.status()
        );
        let path = session.path.clone();
        drop(session);
        drop(managed);
        std::thread::sleep(Duration::from_millis(300));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn canceled_prompt_does_not_accept_host_key() {
        runtime().block_on(async {
            let (tx, rx) = mpsc::sync_channel::<Prompt>(1);
            let stop = AtomicBool::new(false);
            let responder =
                std::thread::spawn(move || rx.recv().unwrap().reply.send(None).unwrap());
            assert!(prompt(&tx, &stop, "host".into(), true).await.is_err());
            responder.join().unwrap();
        });
    }
}
