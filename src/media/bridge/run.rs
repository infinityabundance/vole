//! Bounded foreign-tool subprocess runner — Phase V.1.3 (V.1 video programme,
//! brief §31–§34, §217).
//!
//! The foreign ingest bridge (ffprobe / ffmpeg) is **non-normative** and runs
//! only at import. Every invocation is a [`std::process::Command`] with
//! individual arguments — never a shell command string (§32). Child processes
//! are bounded (§217):
//!
//! * a wall-clock budget, after which the child is killed cleanly;
//! * stdout / stderr byte caps, after which the child is killed cleanly;
//!
//! so a hostile or pathological compressed input can never hang or flood the
//! importer. All I/O is drained concurrently (two reader threads per child),
//! the process is reaped, and the typed outcome carries stdout/stderr plus the
//! exit status. Reader threads use only `std` (no unsafe anywhere in VOLE).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::error::VoleError;

/// Bounded execution budgets for one foreign child process.
#[derive(Debug, Clone, Copy)]
pub struct ChildLimits {
    /// Maximum wall time before the child is killed.
    pub wall: Duration,
    /// Maximum stdout bytes retained (the child is killed beyond this).
    pub stdout_bytes: u64,
    /// Maximum stderr bytes retained (the child is killed beyond this).
    pub stderr_bytes: u64,
}

impl Default for ChildLimits {
    fn default() -> Self {
        ChildLimits {
            wall: Duration::from_secs(120),
            stdout_bytes: 1 << 30, // 1 GiB of pipe payload is far beyond courts
            stderr_bytes: 4 << 20, // 4 MiB of tool diagnostics
        }
    }
}

/// The outcome of a bounded child run: the drained outputs and the exit code
/// (`None` only when the process was killed by a limit).
#[derive(Debug, Clone)]
pub struct RunOutcome {
    /// Captured stdout.
    pub stdout: Vec<u8>,
    /// Captured stderr.
    pub stderr: Vec<u8>,
    /// Exit code (`None` when killed by a bound).
    pub code: Option<i32>,
}

/// Run one tool with the given arguments under the given bounds.
///
/// `program` is the resolved tool path (see [`ToolPaths::discover`]). The
/// child's stdin is null so a tool can never block waiting for terminal
/// input. Returns typed errors:
///
/// * spawn failure → [`VoleError::BridgeNotFound`];
/// * wall clock exceeded → [`VoleError::BridgeTimeout`];
/// * a byte cap exceeded → [`VoleError::BridgeOutputLimit`].
pub fn run_bounded(
    program: &Path,
    args: &[&str],
    limits: &ChildLimits,
) -> Result<RunOutcome, VoleError> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|_| VoleError::BridgeNotFound)?;

    let out = Arc::new(Mutex::new(Vec::new()));
    let err = Arc::new(Mutex::new(Vec::new()));
    let out_cap_hit = Arc::new(AtomicBool::new(false));
    let err_cap_hit = Arc::new(AtomicBool::new(false));

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out_thread = stdout.take().map(|mut pipe| {
        let buf = Arc::clone(&out);
        let cap_hit = Arc::clone(&out_cap_hit);
        let cap = limits.stdout_bytes;
        std::thread::spawn(move || {
            let mut chunk = [0u8; 1 << 16];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut b = buf.lock().expect("drain mutex");
                        if b.len() as u64 + n as u64 > cap {
                            cap_hit.store(true, Ordering::SeqCst);
                            break;
                        }
                        b.extend_from_slice(&chunk[..n]);
                    }
                    Err(_) => break,
                }
            }
        })
    });
    let err_thread = stderr.take().map(|mut pipe| {
        let buf = Arc::clone(&err);
        let cap_hit = Arc::clone(&err_cap_hit);
        let cap = limits.stderr_bytes;
        std::thread::spawn(move || {
            let mut chunk = [0u8; 1 << 16];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut b = buf.lock().expect("drain mutex");
                        if b.len() as u64 + n as u64 > cap {
                            cap_hit.store(true, Ordering::SeqCst);
                            break;
                        }
                        b.extend_from_slice(&chunk[..n]);
                    }
                    Err(_) => break,
                }
            }
        })
    });

    let deadline = Instant::now() + limits.wall;
    let mut killed = false;
    loop {
        if out_cap_hit.load(Ordering::SeqCst) || err_cap_hit.load(Ordering::SeqCst) {
            let _ = child.kill();
            killed = true;
            break;
        }
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    killed = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => {
                let _ = child.kill();
                killed = true;
                break;
            }
        }
    }
    // Reap, then join the readers so their pipes close and they see EOF.
    let _ = child.wait();
    if let Some(t) = out_thread {
        let _ = t.join();
    }
    if let Some(t) = err_thread {
        let _ = t.join();
    }

    let code = if killed {
        None
    } else {
        child.try_wait().ok().flatten().and_then(|s| s.code())
    };
    let cap_hit = out_cap_hit.load(Ordering::SeqCst) || err_cap_hit.load(Ordering::SeqCst);
    let stdout = out.lock().expect("out lock").clone();
    let stderr = err.lock().expect("err lock").clone();

    if cap_hit {
        return Err(VoleError::BridgeOutputLimit);
    }
    if killed && code.is_none() {
        return Err(VoleError::BridgeTimeout);
    }
    Ok(RunOutcome {
        stdout,
        stderr,
        code,
    })
}

/// Resolved foreign tool paths plus their version strings (recorded in every
/// import report; §38/§35 record the bridge versions and every command).
#[derive(Debug, Clone)]
pub struct ToolPaths {
    /// Absolute path of the ffprobe binary.
    pub ffprobe: std::path::PathBuf,
    /// Absolute path of the ffmpeg binary.
    pub ffmpeg: std::path::PathBuf,
    /// `ffprobe -version` first line.
    pub ffprobe_version: String,
    /// `ffmpeg -version` first line.
    pub ffmpeg_version: String,
}

impl ToolPaths {
    /// Resolve `ffmpeg`/`ffprobe`: explicit environment overrides
    /// (`VOLE_FFPROBE`, `VOLE_FFMPEG`) first, then `PATH`. Every resolved
    /// binary must answer `-version`; otherwise [`VoleError::BridgeNotFound`].
    pub fn discover() -> Result<ToolPaths, VoleError> {
        let probe_env = std::env::var_os("VOLE_FFPROBE").map(std::path::PathBuf::from);
        let ffmpeg_env = std::env::var_os("VOLE_FFMPEG").map(std::path::PathBuf::from);
        let ffprobe = match probe_env {
            Some(p) => p,
            None => {
                let path_var = std::env::var_os("PATH").ok_or(VoleError::BridgeNotFound)?;
                find_on_path(&path_var, "ffprobe").ok_or(VoleError::BridgeNotFound)?
            }
        };
        let ffmpeg = match ffmpeg_env {
            Some(p) => p,
            None => {
                let path_var = std::env::var_os("PATH").ok_or(VoleError::BridgeNotFound)?;
                find_on_path(&path_var, "ffmpeg").ok_or(VoleError::BridgeNotFound)?
            }
        };
        let ffprobe_version = tool_version(&ffprobe)?;
        let ffmpeg_version = tool_version(&ffmpeg)?;
        Ok(ToolPaths {
            ffprobe,
            ffmpeg,
            ffprobe_version,
            ffmpeg_version,
        })
    }
}

fn find_on_path(path_var: &std::ffi::OsStr, name: &str) -> Option<std::path::PathBuf> {
    for dir in std::env::split_paths(path_var) {
        let cand = dir.join(name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

fn tool_version(path: &Path) -> Result<String, VoleError> {
    let out = run_bounded(
        path,
        &["-version"],
        &ChildLimits {
            wall: Duration::from_secs(10),
            stdout_bytes: 1 << 16,
            stderr_bytes: 1 << 16,
        },
    )?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next().unwrap_or("unknown").trim().to_string();
    Ok(first)
}

/// Whether `-version` runs can resolve the tools (fast availability probe).
pub fn tools_available() -> bool {
    ToolPaths::discover().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tool_is_typed() {
        let r = run_bounded(
            Path::new("/nonexistent/ffmpeg"),
            &["-version"],
            &ChildLimits::default(),
        );
        assert_eq!(r.unwrap_err(), VoleError::BridgeNotFound);
    }

    #[test]
    fn wall_clock_bound_kills_and_is_typed() {
        // A child that outlives its wall budget is killed and reported typed.
        let r = run_bounded(
            Path::new("/bin/sh"),
            &["-c", "sleep 30"],
            &ChildLimits {
                wall: Duration::from_millis(150),
                stdout_bytes: 1 << 10,
                stderr_bytes: 1 << 10,
            },
        );
        assert_eq!(r.unwrap_err(), VoleError::BridgeTimeout);
    }

    #[test]
    fn output_cap_kills_and_is_typed() {
        let r = run_bounded(
            Path::new("/bin/sh"),
            &["-c", "head -c 100000 /dev/zero"],
            &ChildLimits {
                wall: Duration::from_secs(30),
                stdout_bytes: 1024,
                stderr_bytes: 1 << 10,
            },
        );
        assert_eq!(r.unwrap_err(), VoleError::BridgeOutputLimit);
    }
}
