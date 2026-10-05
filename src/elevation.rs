use std::io;

/// Authentication output is quarantined until the privileged command signals readiness.
pub struct Gate {
    marker: Vec<u8>,
    pending: Vec<u8>,
    pub ready: bool,
}

pub enum Output {
    Waiting,
    Password,
    Log(Vec<u8>),
}

impl Gate {
    pub fn new(marker: &str) -> Self {
        Self {
            marker: marker.as_bytes().to_vec(),
            pending: Vec::new(),
            ready: false,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> io::Result<Output> {
        if self.ready {
            return Ok(Output::Log(bytes.to_vec()));
        }
        self.pending.extend_from_slice(bytes);
        if let Some(position) = self
            .pending
            .windows(self.marker.len())
            .position(|w| w == self.marker)
        {
            self.ready = true;
            let log = self.pending.split_off(position + self.marker.len());
            self.pending.clear();
            return Ok(Output::Log(log));
        }
        let text = String::from_utf8_lossy(&self.pending).to_ascii_lowercase();
        if text.contains("tailer-sudo-password:") || text.contains("password:") {
            self.pending.clear();
            return Ok(Output::Password);
        }
        if self.pending.len() > 16 * 1024 {
            return Err(io::Error::other(
                "Escalation startup output is too long. Cannot start log collection.",
            ));
        }
        Ok(Output::Waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_prompts_and_marker_are_never_logs() {
        let mut gate = Gate::new("\x1eunique\x1f");
        assert!(matches!(
            gate.push(b"banner\nPass").unwrap(),
            Output::Waiting
        ));
        assert!(matches!(gate.push(b"word:").unwrap(), Output::Password));
        assert!(matches!(
            gate.push(b"failure\nPassword:").unwrap(),
            Output::Password
        ));
        assert!(matches!(gate.push(b"\n\x1euni").unwrap(), Output::Waiting));
        let Output::Log(bytes) = gate.push(b"que\x1ffirst\n").unwrap() else {
            panic!()
        };
        assert_eq!(bytes, b"first\n");
        let Output::Log(bytes) = gate.push(b"Password: legitimate log\n").unwrap() else {
            panic!()
        };
        assert_eq!(bytes, b"Password: legitimate log\n");
    }
    #[test]
    fn readiness_without_password_and_output_bound() {
        let mut gate = Gate::new("READY");
        assert!(matches!(gate.push(b"READY").unwrap(), Output::Log(_)));
        assert!(Gate::new("READY").push(&vec![b'x'; 17000]).is_err());
    }
}
