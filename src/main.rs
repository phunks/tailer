#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod analysis;
mod analysis_presets;
mod connections;
mod credentials;
mod elevation;
mod encoding;
mod history;
mod i18n;
mod numeric;
mod session;
mod source;
mod ssh;
mod syslog;
mod tail;
mod trap;
mod workspace;

use qtbridge::qtbridge_type_lib::QString;
use qtbridge::{QApp, qobject};
use serde_json::{Value, json};
use session::{Prompt, Session, SshOptions};

pub struct LogBackend {
    analysis_worker: Option<analysis::Worker>,
    analysis_generation: u64,
    analysis_result: Value,
    analysis_busy: bool,
    pending: Option<(SshOptions, u64, std::rc::Rc<connections::Managed>)>,
    input_encoding: String,
    session: Option<Session>,
    prompt: Option<Prompt>,
    query: history::Query,
    text: String,
    status: String,
    archive_path: String,
    summary: String,
    query_error: String,
    results: Value,
    auth_prompt: String,
    auth_confirmation: bool,
    auth_storable: bool,
    credential_status: String,
    credential_feedback: Option<std::sync::mpsc::Receiver<String>>,
    received_bytes: u64,
    trap_rules: Vec<trap::CompiledRule>,
    trap_hits: u64,
    highlighted_text: String,
    display_text: String,
    syslog_badges: bool,
    selected_line: Option<usize>,
    connected: bool,
    view_revision: u64,
    view_generation: u64,
    view_active: bool,
    line_ids: Value,
}

impl Default for LogBackend {
    fn default() -> Self {
        Self {
            analysis_worker: None,
            analysis_generation: 0,
            analysis_result: json!({}),
            analysis_busy: false,
            pending: None,
            input_encoding: "UTF-8".into(),
            session: None,
            prompt: None,
            query: history::Query::default(),
            text: String::new(),
            status: String::new(),
            archive_path: String::new(),
            summary: String::new(),
            query_error: String::new(),
            results: json!([]),
            auth_prompt: String::new(),
            auth_confirmation: false,
            auth_storable: false,
            credential_status: String::new(),
            credential_feedback: None,
            received_bytes: 0,
            trap_rules: Vec::new(),
            trap_hits: 0,
            highlighted_text: String::new(),
            display_text: String::new(),
            syslog_badges: false,
            selected_line: None,
            connected: false,
            view_revision: 0,
            view_generation: 0,
            view_active: true,
            line_ids: json!([]),
        }
    }
}

#[qobject]
impl LogBackend {
    qproperty!(
        "analysisResult",
        Member = analysis_result,
        Notify = analysis_changed
    );
    qproperty!(
        "analysisBusy",
        Member = analysis_busy,
        Notify = analysis_changed
    );
    #[qsignal]
    fn analysis_changed(&mut self);

    #[qslot]
    fn analysis_preset(&self, apache: bool) -> Value {
        if apache {
            json!({"regex":analysis::APACHE_REGEX, "script":analysis::APACHE_SCRIPT})
        } else {
            json!({"regex":analysis::ACCESS_REGEX, "script":analysis::ACCESS_SCRIPT})
        }
    }

    #[qslot]
    fn analyze(
        &mut self,
        script: String,
        regex: String,
        group: String,
        metric: String,
        operation: String,
    ) {
        self.analyze_input(script, regex, group, metric, operation, json!({}));
    }

    #[qslot]
    fn analyze_input(
        &mut self,
        script: String,
        regex: String,
        group: String,
        metric: String,
        operation: String,
        settings: Value,
    ) {
        let query = match settings.get("query") {
            None => None,
            Some(Value::String(text)) => Some(text.clone()),
            _ => {
                self.analysis_worker = None;
                self.analysis_busy = false;
                self.analysis_result = json!({"error":"Query must be a string"});
                self.analysis_changed();
                return;
            }
        };
        let input = match analysis::InputConfig::from_value(&settings) {
            Ok(input) => input,
            Err(error) => {
                // Discard previous work so it cannot replace the validation error.
                self.analysis_worker = None;
                self.analysis_busy = false;
                self.analysis_result = json!({"error":error});
                self.analysis_changed();
                return;
            }
        };
        if self.analysis_worker.is_none() {
            if let Some(session) = &self.session {
                self.analysis_worker = Some(session.analysis_worker());
            } else {
                self.analysis_result = json!({"error":"No log session"});
                self.analysis_changed();
                return;
            }
        }
        self.analysis_generation =
            self.analysis_worker
                .as_ref()
                .unwrap()
                .submit(analysis::Config {
                    script,
                    regex,
                    group,
                    metric,
                    operation,
                    input,
                    query,
                });
        self.analysis_busy = true;
        self.analysis_changed();
    }

    #[qslot]
    fn stop_analysis(&mut self) {
        self.analysis_worker = None;
        self.analysis_busy = false;
        self.analysis_changed();
    }
    qproperty!("text", Member = text, Notify = changed);
    qproperty!("displayText", Member = display_text, Notify = changed);
    qproperty!(
        "highlightedText",
        Member = highlighted_text,
        Notify = changed
    );
    qproperty!("status", Read = translated_status, Notify = changed);
    qproperty!("connected", Member = connected, Notify = changed);
    qproperty!("viewRevision", Member = view_revision, Notify = changed);
    qproperty!("lineIds", Member = line_ids, Notify = changed);
    qproperty!(
        "selectedPosition",
        Read = selected_position,
        Notify = changed
    );
    qproperty!("archivePath", Member = archive_path, Notify = changed);
    qproperty!("summary", Read = translated_summary, Notify = changed);
    qproperty!(
        "queryError",
        Read = translated_query_error,
        Notify = changed
    );
    qproperty!("results", Member = results, Notify = changed);
    qproperty!(
        "authPrompt",
        Read = translated_auth_prompt,
        Notify = changed
    );
    qproperty!("authStorable", Member = auth_storable, Notify = changed);
    qproperty!(
        "credentialStatus",
        Read = translated_credential_status,
        Notify = changed
    );
    qproperty!(
        "authConfirmation",
        Member = auth_confirmation,
        Notify = changed
    );

    #[qsignal]
    fn changed(&mut self);
    #[qsignal]
    fn received(&mut self);
    #[qsignal]
    fn trapped(&mut self);

    fn translated_status(&self) -> String {
        i18n::text(&self.status)
    }
    fn selected_position(&self) -> i32 {
        trap::context_position(&self.display_text, self.selected_line)
    }
    fn translated_summary(&self) -> String {
        i18n::text(&self.summary)
    }
    fn translated_query_error(&self) -> String {
        i18n::text(&self.query_error)
    }
    fn translated_auth_prompt(&self) -> String {
        i18n::text(&self.auth_prompt)
    }
    fn translated_credential_status(&self) -> String {
        i18n::text(&self.credential_status)
    }
    #[qslot]
    fn refresh_language(&mut self) {
        self.changed();
    }

    #[qslot]
    fn set_view_active(&mut self, active: bool) {
        self.view_active = active;
        if let Some(session) = &self.session {
            session.set_view_active(active);
        }
    }

    #[qslot]
    fn set_trap(&mut self, text: String, ignore_case: bool) {
        let value = trap::sanitize_rules(&json!([{"text":text,"ignoreCase":ignore_case}]));
        self.set_trap_rules(value);
    }

    #[qslot]
    fn validate_trap_rules(&self, value: Value) -> String {
        trap::compile_rules(&value)
            .err()
            .map(|error| i18n::text(&error))
            .unwrap_or_default()
    }

    #[qslot]
    fn set_trap_rules(&mut self, value: Value) -> String {
        let rules = match trap::compile_rules(&value) {
            Ok(rules) => rules,
            Err(error) => return i18n::text(&error),
        };
        self.trap_rules = rules;
        if let Some(session) = &self.session {
            session.set_trap_rules(self.trap_rules.clone());
        }
        if self.view_active {
            self.display_text = if self.syslog_badges {
                syslog::display_text(&self.text)
            } else {
                self.text.clone()
            };
            self.highlighted_text =
                trap::highlight_syslog(&self.text, &self.trap_rules, self.syslog_badges);
        }
        self.changed();
        String::new()
    }

    #[qslot]
    fn highlight_result(&self, text: String) -> String {
        trap::highlight_syslog(&text, &self.trap_rules, self.syslog_badges)
    }

    #[qslot]
    fn set_syslog_badges(&mut self, enabled: bool) {
        if self.syslog_badges == enabled {
            return;
        }
        self.syslog_badges = enabled;
        self.query.filter.syslog_labels = enabled;
        self.query.search.syslog_labels = enabled;
        if let Some(session) = &self.session {
            session.set_syslog_labels(enabled);
        }
        self.display_text = if enabled {
            syslog::display_text(&self.text)
        } else {
            self.text.clone()
        };
        self.highlighted_text = trap::highlight_syslog(&self.text, &self.trap_rules, enabled);
        self.view_revision += 1;
        self.submit();
        self.changed();
    }

    #[qslot]
    fn open(&mut self, path: String, initial: u32, capacity_mib: u32) {
        self.install(Session::local_encoded(
            path,
            initial,
            u64::from(capacity_mib.max(1)) * 1024 * 1024,
            &self.input_encoding,
        ));
    }

    #[qslot]
    fn set_encoding(&mut self, encoding: String) {
        if self.input_encoding == encoding {
            return;
        }
        if let Some(session) = &self.session {
            if let Err(error) = session.change_encoding(&encoding) {
                self.query_error = error.to_string();
                self.changed();
                return;
            }
            self.received_bytes = session.received_bytes();
            self.query.generation = session.query(self.query.clone());
        }
        self.input_encoding = encoding;
        self.changed();
    }

    #[qslot]
    fn run_local(&mut self, command: String, capacity_mib: u32) {
        self.stop();
        self.install(Session::local_command(
            command,
            u64::from(capacity_mib.max(1)) * 1024 * 1024,
            &self.input_encoding,
        ));
    }

    #[qslot]
    fn listen_syslog(&mut self, endpoint: String, capacity_mib: u32) {
        // Release the previous socket before binding the same endpoint again.
        self.stop();
        self.input_encoding = "UTF-8".into();
        self.install(Session::syslog_udp(
            endpoint,
            u64::from(capacity_mib.max(1)) * 1024 * 1024,
        ));
    }

    #[qslot]
    #[allow(clippy::too_many_arguments)]
    fn connect_ssh(
        &mut self,
        host: String,
        user: String,
        port: u32,
        key: String,
        path: String,
        initial: u32,
        capacity_mib: u32,
        elevation: u32,
        run_user: String,
        source: String,
    ) {
        if port > u16::MAX.into() {
            self.status = "Invalid port".into();
            self.changed();
            return;
        }
        self.install(Session::ssh(
            SshOptions {
                source,
                host,
                user,
                port: port as u16,
                key,
                path,
                initial,
                elevation,
                run_user,
            },
            u64::from(capacity_mib.max(1)) * 1024 * 1024,
        ));
    }

    #[qslot]
    #[allow(clippy::too_many_arguments)]
    fn connect_profile(
        &mut self,
        profile: Value,
        path: String,
        initial: u32,
        capacity_mib: u32,
        elevation: u32,
        run_user: String,
        source: String,
    ) {
        self.stop();
        let options = SshOptions {
            host: profile["host"].as_str().unwrap_or_default().into(),
            user: profile["user"].as_str().unwrap_or_default().into(),
            port: profile["port"].as_f64().unwrap_or(0.0).clamp(0.0, 65535.0) as u16,
            key: profile["key"].as_str().unwrap_or_default().into(),
            path,
            initial,
            elevation,
            run_user,
            source,
        };
        match connections::acquire(&profile, &options) {
            Ok(connection) => {
                self.pending = Some((
                    options,
                    u64::from(capacity_mib.max(1)) * 1024 * 1024,
                    connection,
                ));
                self.status = "Waiting for shared SSH connection…".into();
            }
            Err(error) => self.status = format!("Cannot connect: {error}"),
        }
        self.changed();
    }

    fn install(&mut self, result: std::io::Result<Session>) {
        self.stop();
        self.received_bytes = 0;
        self.trap_hits = 0;
        self.selected_line = None;
        self.query.context = None;
        self.text.clear();
        self.display_text.clear();
        self.line_ids = json!([]);
        self.view_revision += 1;
        self.view_generation = 0;
        self.highlighted_text = trap::highlight_rules("", &self.trap_rules);
        self.results = json!([]);
        self.summary.clear();
        self.query_error.clear();
        self.archive_path.clear();
        self.credential_status.clear();
        match result {
            Ok(session) => {
                session.set_view_active(self.view_active);
                session.set_trap_rules(self.trap_rules.clone());
                session.set_syslog_labels(self.syslog_badges);
                self.query.generation = session.query(self.query.clone());
                self.archive_path = session.path.display().to_string();
                self.status = session.status();
                self.connected = session.is_connected();
                self.session = Some(session);
            }
            Err(error) => self.status = format!("Cannot start: {error}"),
        }
        self.changed();
    }

    #[qslot]
    fn poll(&mut self) {
        if let Some(worker) = &self.analysis_worker
            && let Some(result) = worker.take()
            && result["generation"].as_u64() == Some(self.analysis_generation)
        {
            self.analysis_result = result;
            self.analysis_busy = false;
            self.analysis_changed();
        }
        if let Some((_, _, connection)) = &self.pending {
            if connection.transport.is_ready() {
                let (options, limit, connection) = self.pending.take().unwrap();
                self.install(Session::shared_ssh_encoded(
                    options,
                    limit,
                    connection,
                    &self.input_encoding,
                ));
            } else {
                let status = connection.transport.status();
                if status.contains("Connection ended") {
                    self.pending = None;
                    self.status = status;
                    self.changed();
                }
            }
        }
        if let Some(receiver) = &self.credential_feedback
            && let Ok(message) = receiver.try_recv()
        {
            self.credential_status = message;
            self.credential_feedback = None;
            self.changed();
        }
        let Some(session) = &self.session else { return };
        let received_bytes = session.received_bytes();
        let received = received_bytes > self.received_bytes;
        self.received_bytes = received_bytes;
        let hits = session.trap_hits();
        let trapped = hits > self.trap_hits;
        self.trap_hits = hits;
        let status = session.status();
        let connected = session.is_connected();
        let mut changed = self.status != status || self.connected != connected;
        self.connected = connected;
        self.status = status;
        if self.view_active
            && let Some(view) = session.view()
            && view.generation == self.query.generation
        {
            self.query_error = view.error;
            if self.query_error.is_empty() {
                // Only an applied explicit query releases a paused display;
                // ordinary reception/status notifications keep the same revision.
                if self.view_generation != view.generation {
                    self.view_generation = view.generation;
                    self.view_revision += 1;
                }
                self.text = view.text;
                self.line_ids = json!(view.line_ids);
                self.selected_line = self.query.context;
                self.display_text = if self.syslog_badges {
                    syslog::display_text(&self.text)
                } else {
                    self.text.clone()
                };
                self.highlighted_text =
                    trap::highlight_syslog(&self.text, &self.trap_rules, self.syslog_badges);
                self.results = view.results;
                self.summary = view.summary;
            }
            changed = true;
        }
        if self.prompt.is_none()
            && let Ok(prompt) = session.prompts.try_recv()
        {
            self.auth_prompt = prompt.message.clone();
            self.auth_confirmation = prompt.confirmation;
            self.auth_storable = prompt.credential.is_some();
            self.prompt = Some(prompt);
            changed = true;
        }
        if received {
            self.received();
        }
        if trapped {
            self.trapped();
        }
        if changed {
            self.changed();
        }
    }

    #[qslot]
    fn answer_auth(&mut self, answer: String, accepted: bool, remember: bool) {
        if let Some(prompt) = self.prompt.take() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.credential_feedback = Some(rx);
            std::thread::spawn(move || {
                if accepted
                    && remember
                    && let Some(account) = &prompt.credential
                {
                    let message = match credentials::store(account, &answer) {
                        Ok(()) => "Saved in OS credential store".into(),
                        Err(e) => format!("Cannot save credentials (no plaintext fallback): {e}"),
                    };
                    let _ = tx.send(message);
                }
                let _ = prompt
                    .reply
                    .send(if accepted { Some(answer) } else { None });
            });
        }
        self.auth_prompt.clear();
        self.changed();
    }

    #[qslot]
    fn forget_credential(&mut self) {
        if let Some(prompt) = &self.prompt
            && let Some(account) = prompt.credential.clone()
        {
            let (tx, rx) = std::sync::mpsc::channel();
            self.credential_feedback = Some(rx);
            std::thread::spawn(move || {
                let message = match credentials::forget(&account) {
                    Ok(()) => "Deleted saved credentials".into(),
                    Err(e) => format!("Cannot delete credentials: {e}"),
                };
                let _ = tx.send(message);
            });
        }
    }

    #[qslot]
    fn set_filter(&mut self, text: String, regex: bool, ignore_case: bool, invert: bool) {
        self.query.filter = session::pattern(text, regex, ignore_case, invert);
        self.query.filter.syslog_labels = self.syslog_badges;
        self.query.context = None;
        self.submit();
    }

    #[qslot]
    fn set_numeric_filter(
        &mut self,
        text: String,
        regex: bool,
        ignore_case: bool,
        invert: bool,
        condition: Value,
    ) {
        self.query.filter = session::pattern(text, regex, ignore_case, invert);
        self.query.filter.numeric = condition;
        self.query.filter.syslog_labels = self.syslog_badges;
        self.query.context = None;
        self.submit();
    }

    #[qslot]
    fn search(&mut self, text: String, regex: bool, ignore_case: bool) {
        self.query.search = session::pattern(text, regex, ignore_case, false);
        self.query.search.syslog_labels = self.syslog_badges;
        self.query.context = None;
        self.submit();
    }

    #[qslot]
    fn show_context(&mut self, line: u32) {
        self.query.context = Some(line as usize);
        self.submit();
    }

    #[qslot]
    fn show_live(&mut self) {
        self.query.context = None;
        self.submit();
    }

    fn submit(&mut self) {
        if let Some(session) = &self.session {
            self.query.generation = session.query(self.query.clone());
            self.summary = "Searching / filtering all history…".into();
            self.query_error.clear();
            self.changed();
        }
    }

    #[qslot]
    fn stop(&mut self) {
        self.analysis_result = json!({});
        self.stop_analysis();
        self.connected = false;
        self.pending = None;
        if let Some(session) = self.session.take() {
            session.stop();
        }
        if let Some(prompt) = self.prompt.take() {
            let _ = prompt.reply.send(None);
        }
        self.auth_prompt.clear();
        self.changed();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let smoke_path = if args.get(1).is_some_and(|arg| arg == "--smoke-test") {
        args.get(2)
            .cloned()
            .expect("--smoke-test requires a log file path")
    } else {
        String::new()
    };
    let mut app = QApp::new();
    let i18n_test_language = args
        .iter()
        .position(|arg| arg == "--i18n-test")
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
        .unwrap_or("");
    let code = app
        .register::<LogBackend>()
        .register::<connections::ConnectionManager>()
        .register::<workspace::Workspace>()
        .register::<i18n::Translations>()
        .set_initial_property(
            "syslogTestEndpoint",
            &QString::from(
                args.iter()
                    .position(|arg| arg == "--syslog-test")
                    .and_then(|index| args.get(index + 1))
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        )
        .set_initial_property(
            "localCommandTest",
            &args.iter().any(|arg| arg == "--local-command-test"),
        )
        .set_initial_property("i18nTestLanguage", &QString::from(i18n_test_language))
        .set_initial_property("tabUiTest", &args.iter().any(|arg| arg == "--tab-ui-test"))
        .set_initial_property(
            "analysisPresetRestoreTest",
            &args.iter().any(|arg| arg == "--analysis-preset-restore"),
        )
        .set_initial_property(
            "numericTest",
            &args.iter().any(|arg| arg == "--numeric-test"),
        )
        .set_initial_property(
            "themeTestMode",
            &QString::from(
                args.iter()
                    .position(|arg| arg == "--theme-test")
                    .and_then(|index| args.get(index + 1))
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        )
        .set_initial_property(
            "bookmarkTestMode",
            &QString::from(
                args.iter()
                    .position(|arg| arg == "--bookmark-test")
                    .and_then(|index| args.get(index + 1))
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        )
        .set_initial_property("smokePath", &QString::from(smoke_path.as_str()))
        .set_initial_property(
            "analysisTestPath",
            &QString::from(
                args.iter()
                    .position(|arg| arg == "--analysis-test")
                    .and_then(|i| args.get(i + 1))
                    .map(String::as_str)
                    .unwrap_or(""),
            ),
        )
        .set_initial_property(
            "connectionUiTest",
            &args.iter().any(|arg| arg == "--connection-ui-test"),
        )
        .set_initial_property(
            "restoreTest",
            &args.iter().any(|arg| arg == "--restore-test"),
        )
        .load_qml(include_bytes!("Main.qml"))
        .run();
    drop(app);
    std::process::exit(code);
}
