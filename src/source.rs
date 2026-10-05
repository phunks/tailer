use crate::session::quote;
use std::io;

/// Stable identifiers are persisted, not selector indices.
pub enum LogSource {
    File,
    Docker,
    Custom,
}

impl LogSource {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "file" => Ok(Self::File),
            "docker" => Ok(Self::Docker),
            "custom" => Ok(Self::Custom),
            _ => Err(io::Error::other(format!("Unsupported log source: {value}"))),
        }
    }

    pub fn command(&self, target: &str, initial: u32) -> io::Result<String> {
        match self {
            Self::Custom => {
                if target.trim().is_empty() || target.contains('\0') {
                    return Err(io::Error::other("Enter a command (NUL is not allowed)"));
                }
                // Quote the whole script, not its words: pipelines, redirects,
                // expansions and multiple lines are interpreted remotely.
                Ok(format!("exec /bin/sh -c {} 2>&1", quote(target)))
            }
            Self::File => {
                if !target.starts_with('/') || target.contains('\0') {
                    return Err(io::Error::other("Use an absolute path for remote logs"));
                }
                Ok(format!("exec tail -n {initial} -F {}", quote(target)))
            }
            Self::Docker => {
                // Docker names and IDs contain no spaces or shell metacharacters.
                if target.is_empty()
                    || !target.as_bytes()[0].is_ascii_alphanumeric()
                    || !target
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
                {
                    return Err(io::Error::other(
                        "Check Docker container name / ID (letters, digits, _, ., -)",
                    ));
                }
                // Docker sends container stderr to its own stderr. Merge remotely,
                // leaving SSH authentication/transport diagnostics separate.
                Ok(format!(
                    "exec docker logs --tail {initial} --follow --timestamps {} 2>&1",
                    quote(target)
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_preserves_script_and_ignores_initial_lines() {
        let script = "printf '%s\\n' \"$HOME\" | cat\nprintf error >&2";
        assert_eq!(
            LogSource::Custom.command(script, 0).unwrap(),
            format!("exec /bin/sh -c {} 2>&1", quote(script))
        );
        assert_eq!(
            LogSource::Custom.command(script, 50).unwrap(),
            LogSource::Custom.command(script, 0).unwrap()
        );
        assert!(LogSource::parse("custom").is_ok());
        for script in ["", " \n ", "echo\0bad"] {
            assert!(LogSource::Custom.command(script, 0).is_err());
        }
    }
    #[test]
    fn docker_command_validates_target_and_merges_stderr() {
        assert_eq!(
            LogSource::Docker.command("web-1", 50).unwrap(),
            "exec docker logs --tail 50 --follow --timestamps 'web-1' 2>&1"
        );
        for target in ["", "-x", "a;echo x", "$(id)", "a b", "/path"] {
            assert!(LogSource::Docker.command(target, 0).is_err());
        }
        assert!(LogSource::parse("kube").is_err());
    }
    #[test]
    fn file_source_preserves_existing_validation() {
        assert_eq!(
            LogSource::File.command("/tmp/log", 20).unwrap(),
            "exec tail -n 20 -F '/tmp/log'"
        );
        assert!(LogSource::File.command("relative", 20).is_err());
    }
}
