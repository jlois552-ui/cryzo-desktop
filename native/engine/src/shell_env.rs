//! Apps opened from the macOS Dock or a Linux app menu start with a bare environment, so `npm`,
//! `git` and tools installed with Homebrew or nvm wouldn't be found. Like upstream's desktop app,
//! Cryzo takes the user's login-shell environment once at startup. Windows needs nothing.

/// Adopts the login shell's environment. Call it first thing in `main`, before any thread starts.
pub fn adopt() {
    #[cfg(unix)]
    {
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        // Nushell's `-c` doesn't run POSIX `env -0`.
        if std::path::Path::new(&shell)
            .file_name()
            .is_some_and(|name| name == "nu")
        {
            return;
        }
        let Some(environment) = ["-ilc", "-lc"]
            .into_iter()
            .find_map(|mode| probe(&shell, mode))
        else {
            return;
        };
        for (key, value) in environment {
            // SAFETY: called at the start of `main`, before any other thread exists.
            unsafe { std::env::set_var(key, value) };
        }
    }
}

/// `env -0` from the shell in `mode`, or nothing when it fails or takes over 5 seconds (an
/// interactive shell config can hang waiting for input).
#[cfg(unix)]
fn probe(shell: &str, mode: &str) -> Option<Vec<(String, String)>> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let mut child = Command::new(shell)
        .args([mode, "env -0"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut output).map(|_| output)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let environment = parse(&reader.join().ok()?.ok()?);
    (!environment.is_empty()).then_some(environment)
}

/// `KEY=value` entries separated by NUL, as `env -0` prints them.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse(output: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(output)
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn env_output_becomes_variables() {
        assert_eq!(
            parse(b"PATH=/opt/homebrew/bin:/usr/bin\0MULTI=a\nb=c\0=skipped\0\0"),
            vec![
                ("PATH".into(), "/opt/homebrew/bin:/usr/bin".into()),
                ("MULTI".into(), "a\nb=c".into()),
            ]
        );
    }
}
