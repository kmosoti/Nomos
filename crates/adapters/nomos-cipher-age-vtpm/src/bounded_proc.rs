//! Bounded child process runner enforcing timeouts and stdout/stderr byte caps.

use std::io::{Read as _, Write as _};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nomos_cipher::{CipherError, CipherResult};

/// Default wall-clock execution deadline enforced per remote SSH invocation.
pub const DEFAULT_SSH_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum stdout or stderr bytes captured from a child transport process (256 KiB).
pub const MAX_SSH_OUTPUT_BYTES: usize = 256 * 1024;

/// Bounded output captured from a child process.
#[derive(Clone, Debug)]
pub struct BoundedCommandOutput {
    /// Exit status returned by the child process.
    pub status: ExitStatus,
    /// Captured stdout bytes (up to [`MAX_SSH_OUTPUT_BYTES`]).
    pub stdout: Vec<u8>,
    /// Captured stderr bytes (up to [`MAX_SSH_OUTPUT_BYTES`]).
    pub stderr: Vec<u8>,
}

/// Runs `cmd` with optional `stdin_bytes`, enforcing `timeout` and `max_output_bytes`.
///
/// Kills and reaps the child process on timeout or pipe output overflow.
pub fn run_command_bounded(
    mut cmd: Command,
    stdin_bytes: Option<&[u8]>,
    label: &str,
    timeout: Duration,
    max_output_bytes: usize,
) -> CipherResult<BoundedCommandOutput> {
    cmd.stdin(if stdin_bytes.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| CipherError::Transport(format!("{label} spawn: {e}")))?;

    let overflow = Arc::new(AtomicBool::new(false));
    let stdout_handle =
        spawn_bounded_reader(child.stdout.take(), max_output_bytes, Arc::clone(&overflow));
    let stderr_handle =
        spawn_bounded_reader(child.stderr.take(), max_output_bytes, Arc::clone(&overflow));

    if let Some(payload) = stdin_bytes
        && let Some(mut stdin) = child.stdin.take()
    {
        let _ = stdin.write_all(payload);
    }

    let deadline = Instant::now() + timeout;
    let status = loop {
        if overflow.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let _ = stderr_handle.join();
            return Err(CipherError::Transport(format!(
                "{label} output exceeded {max_output_bytes} byte limit"
            )));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_handle.join();
                    let _ = stderr_handle.join();
                    return Err(CipherError::Transport(format!(
                        "{label} timed out after {}ms",
                        timeout.as_millis()
                    )));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(CipherError::Transport(format!("{label} wait: {e}")));
            }
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    if overflow.load(Ordering::Relaxed)
        || stdout.len() > max_output_bytes
        || stderr.len() > max_output_bytes
    {
        return Err(CipherError::Transport(format!(
            "{label} output exceeded {max_output_bytes} byte limit"
        )));
    }
    Ok(BoundedCommandOutput {
        status,
        stdout,
        stderr,
    })
}

fn spawn_bounded_reader<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
    max_bytes: usize,
    overflow: Arc<AtomicBool>,
) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let Some(reader) = pipe else {
            return Vec::new();
        };
        let mut buf = Vec::new();
        if reader
            .take((max_bytes as u64).saturating_add(1))
            .read_to_end(&mut buf)
            .is_ok()
            && buf.len() > max_bytes
        {
            overflow.store(true, Ordering::Relaxed);
            buf.truncate(max_bytes);
        }
        buf
    })
}
