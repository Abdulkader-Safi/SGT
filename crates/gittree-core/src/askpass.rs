//! Credential prompts without a terminal.
//!
//! Git and ssh ask for a user name, password or passphrase by running an
//! "askpass" program with the question as its argument. The app points them
//! at its own binary, which passes the question over a local socket to the
//! running app and prints the answer. Nothing typed is stored or logged.
//!
//! The socket is a Unix socket only this user can open. Windows has none in
//! the standard library, so there it is a TCP port on the loopback address.
//! Either way the helper must also send a one-time token.

use std::io::{BufRead, BufReader, Read, Write};
#[cfg(windows)]
use std::net::{TcpListener as Listener, TcpStream as Stream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const SOCK_VAR: &str = "GITTREE_ASKPASS_SOCK";
pub const TOKEN_VAR: &str = "GITTREE_ASKPASS_TOKEN";

/// Listens for questions while a network command runs. Dropping it closes
/// the socket and removes its folder.
pub struct Server {
    dir: PathBuf,
    /// The socket's path, or `127.0.0.1:port` on Windows.
    sock: String,
    token: String,
    stop: Arc<AtomicBool>,
}

fn random_hex(bytes: usize) -> std::io::Result<String> {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(buffer.iter().map(|b| format!("{b:02x}")).collect())
}

/// Opens the socket inside `dir` and returns it with its address.
#[cfg(unix)]
fn listen(dir: &Path) -> std::io::Result<(Listener, String)> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    let sock = dir.join("s");
    let listener = Listener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    Ok((listener, sock.to_string_lossy().into_owned()))
}

#[cfg(windows)]
fn listen(_dir: &Path) -> std::io::Result<(Listener, String)> {
    let listener = Listener::bind(("127.0.0.1", 0))?;
    let address = listener.local_addr()?.to_string();
    Ok((listener, address))
}

impl Server {
    /// Starts listening. `answer` gets each question and returns what the
    /// user typed, or `None` when they cancelled. It runs on the listener's
    /// thread and may block.
    pub fn start(
        answer: impl Fn(&str) -> Option<String> + Send + 'static,
    ) -> std::io::Result<Self> {
        // A folder only this user can enter, with a name nobody can guess.
        let dir = std::env::temp_dir().join(format!("sgt-askpass-{}", random_hex(8)?));
        std::fs::create_dir(&dir)?;
        let (listener, sock) = listen(&dir)?;
        let token = random_hex(32)?;
        let stop = Arc::new(AtomicBool::new(false));

        let (expected, stopping) = (token.clone(), stop.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(&stream);
                let (mut token, mut question) = (String::new(), String::new());
                if reader.read_line(&mut token).is_err() || reader.read_line(&mut question).is_err()
                {
                    continue;
                }
                // Only the helper this app started knows the token.
                if token.trim_end() != expected {
                    continue;
                }
                if let Some(reply) = answer(question.trim_end()) {
                    let _ = stream.write_all(reply.as_bytes());
                }
            }
        });
        Ok(Self {
            dir,
            sock,
            token,
            stop,
        })
    }

    /// The environment that makes Git and ssh ask through `helper`, which
    /// is this app's own binary.
    pub fn env(&self, helper: &Path) -> Vec<(String, String)> {
        let helper = helper.to_string_lossy().into_owned();
        vec![
            ("GIT_ASKPASS".into(), helper.clone()),
            ("SSH_ASKPASS".into(), helper),
            // Without these ssh only uses askpass when it has no terminal.
            ("SSH_ASKPASS_REQUIRE".into(), "force".into()),
            ("DISPLAY".into(), ":0".into()),
            (SOCK_VAR.into(), self.sock.clone()),
            (TOKEN_VAR.into(), self.token.clone()),
        ]
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Wakes the accept loop so it sees the flag.
        let _ = Stream::connect(&self.sock);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The helper's side: sends one question and returns the answer. `None`
/// means the user cancelled, or the app is not listening.
pub fn ask(sock: &str, token: &str, question: &str) -> Option<String> {
    let mut stream = Stream::connect(sock).ok()?;
    let question = question.replace('\n', " ");
    stream
        .write_all(format!("{token}\n{question}\n").as_bytes())
        .ok()?;
    // The listener reads two lines, so it may have answered and hung up
    // already. macOS then fails this call, and the answer is still there.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream.read_to_string(&mut reply).ok()?;
    (!reply.is_empty()).then_some(reply)
}

/// True when a question asks for something secret, so the field hides it.
pub fn is_secret(question: &str) -> bool {
    let question = question.to_lowercase();
    question.contains("password") || question.contains("passphrase") || question.contains("token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_answered_only_with_the_right_token() {
        let server = Server::start(|question| {
            // The question arrives without its trailing space.
            (question == "Password for 'https://example.com':").then(|| "hunter2".to_owned())
        })
        .unwrap();
        let ask = |token: &str, question: &str| ask(&server.sock, token, question);
        assert_eq!(
            ask(&server.token, "Password for 'https://example.com': ").as_deref(),
            Some("hunter2")
        );
        // The user cancelled.
        assert_eq!(ask(&server.token, "Username: "), None);
        assert_eq!(ask("wrong", "Password for 'https://example.com': "), None);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(
                (mode(&server.dir), mode(Path::new(&server.sock))),
                (0o700, 0o600)
            );
        }
        let dir = server.dir.clone();
        drop(server);
        assert!(!dir.exists());
    }

    #[test]
    fn secrets_are_told_from_names() {
        assert!(is_secret("Password for 'https://safi@example.com': "));
        assert!(is_secret(
            "Enter passphrase for key '/Users/x/.ssh/id_ed25519': "
        ));
        assert!(!is_secret("Username for 'https://example.com': "));
    }
}
