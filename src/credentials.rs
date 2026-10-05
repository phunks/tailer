use crate::session::Prompt;
use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};

pub fn account(connection: &str, message: &str, confirmation: bool) -> Option<String> {
    let lower = message.to_ascii_lowercase();
    if confirmation
        || lower.contains("otp")
        || lower.contains("verification")
        || lower.contains("token")
        || lower.contains("one-time")
        || lower.contains("keyboard-interactive")
    {
        return None;
    }
    if lower.contains("password")
        || lower.contains("passphrase")
        || lower.contains("privilege escalation")
    {
        Some(serde_json::json!([connection, message]).to_string())
    } else {
        None
    }
}

pub fn store(account: &str, password: &str) -> Result<(), String> {
    keyring::Entry::new("Tailer", account)
        .and_then(|entry| entry.set_password(password))
        .map_err(|e| e.to_string())
}

pub fn forget(account: &str) -> Result<(), String> {
    keyring::Entry::new("Tailer", account)
        .and_then(|entry| entry.delete_credential())
        .map_err(|e| e.to_string())
}

fn retrieve(account: &str) -> Option<String> {
    #[cfg(not(test))]
    {
        keyring::Entry::new("Tailer", account)
            .ok()?
            .get_password()
            .ok()
    }
    #[cfg(test)]
    {
        let _ = account;
        None
    } // Automated tests never access the user's real store.
}

pub fn broker(
    connection: String,
    input: mpsc::Receiver<Prompt>,
    output: mpsc::SyncSender<Prompt>,
    stop: &AtomicBool,
) {
    broker_with_lookup(connection, input, output, stop, retrieve);
}

fn broker_with_lookup(
    connection: String,
    input: mpsc::Receiver<Prompt>,
    output: mpsc::SyncSender<Prompt>,
    stop: &AtomicBool,
    lookup: impl Fn(&str) -> Option<String>,
) {
    let mut tried = HashSet::new();
    while !stop.load(Ordering::Relaxed) {
        let mut prompt = match input.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(prompt) => prompt,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        prompt.credential = account(&connection, &prompt.message, prompt.confirmation);
        if let Some(account) = &prompt.credential
            && tried.insert(account.clone())
            && let Some(password) = lookup(account)
        {
            let _ = prompt.reply.send(Some(password));
            continue;
        }
        if let Err(error) = output.try_send(prompt) {
            let prompt = match error {
                mpsc::TrySendError::Full(prompt) | mpsc::TrySendError::Disconnected(prompt) => {
                    prompt
                }
            };
            let _ = prompt.reply.send(None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_passwords_and_passphrases_have_store_identifiers() {
        assert!(account("host", "Password:", false).is_some());
        assert!(account("host", "Enter passphrase for key '/key':", false).is_some());
        assert!(account("host", "OTP password:", false).is_none());
        assert!(account("host", "keyboard-interactive\nPassword:", false).is_none());
        assert!(account("host", "yes/no", true).is_none());
        assert_ne!(
            account("host1", "Password:", false),
            account("host2", "Password:", false)
        );
    }

    #[test]
    fn saved_secret_is_tried_once_then_prompts_user() {
        let (tx, input) = mpsc::channel();
        let (output, rx) = mpsc::sync_channel(8);
        let worker = std::thread::spawn(move || {
            broker_with_lookup(
                "host".into(),
                input,
                output,
                &AtomicBool::new(false),
                |_| Some("saved".into()),
            )
        });
        for attempt in 0..2 {
            let (reply, answer) = mpsc::channel();
            tx.send(Prompt {
                message: "Password:".into(),
                confirmation: false,
                reply,
                credential: None,
            })
            .unwrap();
            if attempt == 1 {
                let prompt = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
                assert!(prompt.credential.is_some());
                prompt.reply.send(Some("new".into())).unwrap();
            }
            assert_eq!(
                answer
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap(),
                Some(if attempt == 0 { "saved" } else { "new" }.into())
            );
        }
        drop(tx);
        worker.join().unwrap();
    }
}
