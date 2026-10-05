use crate::{LogBackend, session::SshOptions};
use qtbridge::qobject;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::HashMap,
    io,
    rc::{Rc, Weak},
};

thread_local! {
    static ACTIVE: RefCell<HashMap<String, Weak<Managed>>> = RefCell::new(HashMap::new());
}

pub struct Managed {
    pub transport: std::sync::Arc<crate::ssh::Transport>,
    pub prompts: std::sync::mpsc::Receiver<crate::session::Prompt>,
    pub name: String,
}

impl Drop for Managed {
    fn drop(&mut self) {
        self.transport.stop();
    }
}

pub fn acquire(profile: &Value, options: &SshOptions) -> io::Result<Rc<Managed>> {
    let id = profile["id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::other("Connection not found"))?;
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        active.retain(|_, weak| weak.strong_count() > 0);
        if let Some(connection) = active.get(id).and_then(Weak::upgrade) {
            return Ok(connection);
        }
        let (transport, prompts) = crate::ssh::Transport::start(options)?;
        let connection = Rc::new(Managed {
            transport,
            prompts,
            name: profile["name"].as_str().unwrap_or(id).into(),
        });
        active.insert(id.into(), Rc::downgrade(&connection));
        Ok(connection)
    })
}

#[derive(Default)]
pub struct ConnectionManager {
    auth: LogBackend,
    current: Option<Weak<Managed>>,
    auth_prompt: String,
    auth_confirmation: bool,
    auth_storable: bool,
    credential_status: String,
    status: String,
    statuses: Value,
}

#[qobject]
impl ConnectionManager {
    qproperty!(
        "authPrompt",
        Read = translated_auth_prompt,
        Notify = changed
    );
    qproperty!(
        "authConfirmation",
        Member = auth_confirmation,
        Notify = changed
    );
    qproperty!("authStorable", Member = auth_storable, Notify = changed);
    qproperty!(
        "credentialStatus",
        Read = translated_credential_status,
        Notify = changed
    );
    qproperty!("status", Read = translated_status, Notify = changed);
    qproperty!("statuses", Member = statuses, Notify = changed);
    fn translated_status(&self) -> String {
        crate::i18n::text(&self.status)
    }
    fn translated_auth_prompt(&self) -> String {
        crate::i18n::text(&self.auth_prompt)
    }
    fn translated_credential_status(&self) -> String {
        crate::i18n::text(&self.credential_status)
    }
    #[qsignal]
    fn changed(&mut self);
    #[qslot]
    fn poll(&mut self) {
        self.auth.poll();
        if self.current.as_ref().is_some_and(|w| w.upgrade().is_none()) {
            self.auth.stop();
            self.current = None;
        }
        let connections = ACTIVE.with(|active| {
            active
                .borrow()
                .values()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>()
        });
        self.status = connections
            .iter()
            .map(|c| {
                format!(
                    "{}: {}",
                    c.name,
                    if c.transport.is_ready() {
                        "Connected"
                    } else {
                        "Waiting / disconnected"
                    }
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        self.statuses = ACTIVE.with(|active| {
            Value::Object(
                active
                    .borrow()
                    .iter()
                    .filter_map(|(id, weak)| {
                        let connection = weak.upgrade()?;
                        let state = if connection.transport.is_ready() {
                            "Connected"
                        } else {
                            "Waiting / disconnected"
                        };
                        Some((
                            id.clone(),
                            Value::String(format!(
                                "{}: {}",
                                connection.name,
                                crate::i18n::text(state)
                            )),
                        ))
                    })
                    .collect(),
            )
        });
        if self.auth.prompt.is_none() {
            for connection in connections {
                if let Ok(prompt) = connection.prompts.try_recv() {
                    self.auth.auth_prompt = format!("{}\n{}", connection.name, prompt.message);
                    self.auth.auth_confirmation = prompt.confirmation;
                    self.auth.auth_storable = prompt.credential.is_some();
                    self.auth.prompt = Some(prompt);
                    self.current = Some(Rc::downgrade(&connection));
                    break;
                }
            }
        }
        self.auth_prompt = self.auth.auth_prompt.clone();
        self.auth_confirmation = self.auth.auth_confirmation;
        self.auth_storable = self.auth.auth_storable;
        self.credential_status = self.auth.credential_status.clone();
        self.changed();
    }
    #[qslot]
    fn answer_auth(&mut self, answer: String, accepted: bool, remember: bool) {
        self.auth.answer_auth(answer, accepted, remember);
        self.auth_prompt.clear();
        self.changed();
    }
    #[qslot]
    fn forget_credential(&mut self) {
        self.auth.forget_credential();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_shares_transport_and_last_reference_stops_it() {
        let options = SshOptions {
            source: "file".into(),
            host: "127.0.0.1".into(),
            port: 1,
            user: "test".into(),
            key: String::new(),
            path: "/log".into(),
            initial: 0,
            elevation: 0,
            run_user: "root".into(),
        };
        let profile = serde_json::json!({"id":"registry-native-test", "name":"Test"});
        let first = acquire(&profile, &options).unwrap();
        let second = acquire(&profile, &options).unwrap();
        assert!(Rc::ptr_eq(&first, &second));
        let transport = first.transport.clone();
        drop(first);
        assert_eq!(Rc::strong_count(&second), 1);
        drop(second);
        assert!(transport.stop.load(std::sync::atomic::Ordering::Relaxed));
    }
}
