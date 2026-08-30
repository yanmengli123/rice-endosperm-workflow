//! Stopping a runtime must reclaim the worker's whole process tree.
//!
//! An interpreter is often a launcher rather than the process doing the work:
//! on Windows `Rscript.exe` re-launches `bin\x64\Rscript.exe` through
//! `cmd.exe`, so killing the direct child used to leave the real R worker
//! running with the inherited stderr pipe still open (#941). This fixture
//! re-executes itself as a two-level tree to reproduce that shape on every
//! host, which a shell one-liner could not do portably.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use wisp_runtime::{KernelClient, RuntimeKernel};

const WORKER_ARG: &str = "--fake-worker";
const GRANDCHILD_ARG: &str = "--fake-grandchild";
const EXIT_ON_EOF_ARG: &str = "--exit-on-eof";
const READY_FRAME: &str =
    r#"{"type":"ready","protocol":1,"language":"python","pid":1,"version":"test"}"#;

fn main() -> ExitCode {
    let args = std::env::args().collect::<Vec<_>>();
    match args.get(1).map(String::as_str) {
        Some(WORKER_ARG) => fake_worker(
            args.get(2).map(PathBuf::from),
            args.get(3).is_some_and(|arg| arg == EXIT_ON_EOF_ARG),
        ),
        Some(GRANDCHILD_ARG) => fake_grandchild(args.get(2).map(PathBuf::from)),
        _ => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");
            match runtime.block_on(run_regressions()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("runtime worker process-tree regression failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

async fn run_regressions() -> Result<(), String> {
    stopping_a_stuck_worker_reclaims_its_descendants().await?;
    a_graceful_worker_exit_still_reclaims_its_descendants().await?;
    dropping_the_client_reclaims_the_tree().await
}

/// A worker that ignores stdin EOF has to be killed, and the kill must reach
/// the grandchild holding the inherited stderr pipe.
async fn stopping_a_stuck_worker_reclaims_its_descendants() -> Result<(), String> {
    let fixture = launch_fixture(false).await?;
    let mut client = fixture.client;
    tokio::time::timeout(Duration::from_secs(15), client.shutdown())
        .await
        .map_err(|_| "shutdown did not finish within 15s".to_string())?
        .map_err(|error| format!("shutdown failed: {error}"))?;
    assert_fixture_stopped(fixture.root, fixture.worker, fixture.grandchild).await
}

/// Exiting on stdin EOF is the fast path, but a background process the worker
/// started would otherwise outlive the runtime it belongs to.
async fn a_graceful_worker_exit_still_reclaims_its_descendants() -> Result<(), String> {
    let fixture = launch_fixture(true).await?;
    let mut client = fixture.client;
    tokio::time::timeout(Duration::from_secs(15), client.shutdown())
        .await
        .map_err(|_| "graceful shutdown did not finish within 15s".to_string())?
        .map_err(|error| format!("graceful shutdown failed: {error}"))?;
    assert_fixture_stopped(fixture.root, fixture.worker, fixture.grandchild).await
}

/// A dropped client (a panic, or a launch abandoned mid-handshake) must not
/// leave an interpreter tree behind either.
async fn dropping_the_client_reclaims_the_tree() -> Result<(), String> {
    let fixture = launch_fixture(false).await?;
    drop(fixture.client);
    assert_fixture_stopped(fixture.root, fixture.worker, fixture.grandchild).await
}

fn fake_worker(root: Option<PathBuf>, exit_on_eof: bool) -> ExitCode {
    let Some(root) = root else {
        return ExitCode::FAILURE;
    };
    if write_pid(&root.join("worker.pid")).is_err() {
        return ExitCode::FAILURE;
    }
    let Ok(executable) = std::env::current_exe() else {
        return ExitCode::FAILURE;
    };
    // stderr is inherited on purpose: that is what keeps the host waiting for
    // an EOF the direct child can no longer deliver.
    let Ok(_grandchild) = std::process::Command::new(executable)
        .arg(GRANDCHILD_ARG)
        .arg(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
    else {
        return ExitCode::FAILURE;
    };

    println!("{READY_FRAME}");
    if exit_on_eof {
        let mut line = String::new();
        while matches!(std::io::stdin().read_line(&mut line), Ok(read) if read > 0) {
            line.clear();
        }
        return ExitCode::SUCCESS;
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn fake_grandchild(root: Option<PathBuf>) -> ExitCode {
    let Some(root) = root else {
        return ExitCode::FAILURE;
    };
    if write_pid(&root.join("grandchild.pid")).is_err() {
        return ExitCode::FAILURE;
    }
    loop {
        std::thread::sleep(Duration::from_secs(60));
    }
}

struct Fixture {
    root: PathBuf,
    client: KernelClient,
    worker: u32,
    grandchild: u32,
}

async fn launch_fixture(exit_on_eof: bool) -> Result<Fixture, String> {
    let root = unique_temp_dir()?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut args = vec![OsString::from(WORKER_ARG), root.clone().into_os_string()];
    if exit_on_eof {
        args.push(OsString::from(EXIT_ON_EOF_ARG));
    }
    let client = KernelClient::spawn_command(&executable, &args, &[], None, "python")
        .await
        .map_err(|error| error.to_string())?;
    let worker = wait_for_pid(&root.join("worker.pid")).await?;
    let grandchild = wait_for_pid(&root.join("grandchild.pid")).await?;
    Ok(Fixture {
        root,
        client,
        worker,
        grandchild,
    })
}

async fn assert_fixture_stopped(root: PathBuf, worker: u32, grandchild: u32) -> Result<(), String> {
    let worker_stopped = wait_until_stopped(worker, Duration::from_secs(3)).await;
    let grandchild_stopped = wait_until_stopped(grandchild, Duration::from_secs(3)).await;
    if !worker_stopped {
        terminate_exact(worker);
    }
    if !grandchild_stopped {
        terminate_exact(grandchild);
    }
    let _ = std::fs::remove_dir_all(&root);

    if !worker_stopped || !grandchild_stopped {
        return Err(format!(
            "stopping the runtime left processes alive: worker_alive={}, grandchild_alive={}",
            !worker_stopped, !grandchild_stopped
        ));
    }
    Ok(())
}

fn write_pid(path: &Path) -> std::io::Result<()> {
    std::fs::write(path, std::process::id().to_string())
}

fn unique_temp_dir() -> Result<PathBuf, String> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("wisp-runtime-process-tree-{nonce}"));
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    Ok(root)
}

async fn wait_for_pid(path: &Path) -> Result<u32, String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(value) = std::fs::read_to_string(path) {
            if let Ok(pid) = value.trim().parse::<u32>() {
                return Ok(pid);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("timed out waiting for {}", path.display()));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn wait_until_stopped(pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if !process_exists(pid) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn terminate_exact(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
    };
    unsafe {
        let process = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if process.is_null() {
            return false;
        }
        let status = WaitForSingleObject(process, 0);
        CloseHandle(process);
        status == WAIT_TIMEOUT
    }
}

#[cfg(windows)]
fn terminate_exact(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}
