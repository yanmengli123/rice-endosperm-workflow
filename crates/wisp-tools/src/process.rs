//! Spawn-time child process policy: no console window on Windows GUI builds,
//! and one OS-owned termination boundary per spawned process tree.

use std::io;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use tokio::process::{Child, Command};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

// Git for Windows can fail during DLL initialization when several windows probe
// it at once. Serialize every git.exe spawn through this lock.
static GIT_COMMAND_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn git_command_lock() -> &'static Mutex<()> {
    GIT_COMMAND_LOCK.get_or_init(|| Mutex::new(()))
}

pub struct GitCommandGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// Hold only while starting Git; do not keep this guard across `.await`.
pub fn lock_git_command() -> GitCommandGuard {
    GitCommandGuard {
        _guard: git_command_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner()),
    }
}

#[cfg_attr(not(windows), allow(unused_variables))]
pub fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

#[cfg_attr(not(windows), allow(unused_variables))]
pub fn hide_console_async(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
}

const TREE_ACTIVE: u8 = 0;
#[cfg(unix)]
const GRACEFUL_SENT: u8 = 1;
const FORCE_SENT: u8 = 2;
const TREE_DISARMED: u8 = 3;

/// One OS-owned termination boundary for a spawned wrapper and every child it
/// starts. The wrapper PID is never looked up by name.
pub struct ProcessTree {
    state: AtomicU8,
    signal_lock: std::sync::Mutex<()>,
    #[cfg(unix)]
    process_group: libc::pid_t,
    #[cfg(windows)]
    job: std::os::windows::io::OwnedHandle,
}

impl ProcessTree {
    /// Configure the child before spawn so descendants cannot escape through a
    /// race between wrapper startup and attaching the termination boundary.
    pub fn configure(command: &mut Command) {
        #[cfg(unix)]
        command.process_group(0);

        #[cfg(windows)]
        command.creation_flags(
            windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
                | windows_sys::Win32::System::Threading::CREATE_SUSPENDED,
        );
    }

    pub fn attach(child: &Child) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let process_group = child
                .id()
                .and_then(|pid| libc::pid_t::try_from(pid).ok())
                .ok_or_else(|| io::Error::other("child process has no process id"))?;
            Ok(Self {
                state: AtomicU8::new(TREE_ACTIVE),
                signal_lock: std::sync::Mutex::new(()),
                process_group,
            })
        }

        #[cfg(windows)]
        {
            Self::attach_windows(child)
        }

        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            Err(io::Error::other(
                "process-tree shutdown is unsupported on this platform",
            ))
        }
    }

    #[cfg(unix)]
    pub fn terminate_gracefully(&self) -> io::Result<()> {
        let _signal = self
            .signal_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.state.load(Ordering::SeqCst) != TREE_ACTIVE {
            return Ok(());
        }
        if self.signal_group(libc::SIGTERM)? {
            self.state.store(GRACEFUL_SENT, Ordering::SeqCst);
        } else {
            self.state.store(TREE_DISARMED, Ordering::SeqCst);
        }
        Ok(())
    }

    #[cfg(windows)]
    pub fn terminate_gracefully(&self) -> io::Result<()> {
        self.terminate_forcefully()
    }

    #[cfg(not(any(unix, windows)))]
    pub fn terminate_gracefully(&self) -> io::Result<()> {
        Ok(())
    }

    pub fn terminate_forcefully(&self) -> io::Result<()> {
        let _signal = self
            .signal_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.terminate_locked()
    }

    /// Terminate the tree only if it still holds processes, deciding and
    /// signalling under one lock. Callers that have already reaped the direct
    /// child must use this: on Unix a process-group id that just became free
    /// may already belong to an unrelated process.
    pub fn terminate_if_running(&self) -> io::Result<bool> {
        let _signal = self
            .signal_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.is_running_unlocked()? {
            self.state.store(TREE_DISARMED, Ordering::SeqCst);
            return Ok(false);
        }
        self.terminate_locked()?;
        Ok(true)
    }

    fn terminate_locked(&self) -> io::Result<()> {
        if self.state.load(Ordering::SeqCst) >= FORCE_SENT {
            return Ok(());
        }
        #[cfg(unix)]
        {
            if self.signal_group(libc::SIGKILL)? {
                self.state.store(FORCE_SENT, Ordering::SeqCst);
            } else {
                self.state.store(TREE_DISARMED, Ordering::SeqCst);
            }
            return Ok(());
        }

        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::System::JobObjects::TerminateJobObject;
            let result = unsafe { TerminateJobObject(self.job.as_raw_handle() as _, 1) };
            if result == 0 {
                let error = io::Error::last_os_error();
                if self.is_running_unlocked().unwrap_or(true) {
                    return Err(error);
                }
            }
            self.state.store(FORCE_SENT, Ordering::SeqCst);
            return Ok(());
        }

        #[cfg(not(any(unix, windows)))]
        Ok(())
    }

    pub fn is_running(&self) -> io::Result<bool> {
        let _signal = self
            .signal_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let running = self.is_running_unlocked()?;
        if !running {
            // Keep the emptiness observation and the transition that prevents
            // later numeric PGID signals inside the same critical section.
            self.state.store(TREE_DISARMED, Ordering::SeqCst);
        }
        Ok(running)
    }

    fn is_running_unlocked(&self) -> io::Result<bool> {
        if self.state.load(Ordering::SeqCst) == TREE_DISARMED {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            let result = unsafe { libc::kill(-self.process_group, 0) };
            if result == 0 {
                return Ok(true);
            }
            let error = io::Error::last_os_error();
            return match error.raw_os_error() {
                Some(libc::ESRCH) => Ok(false),
                Some(libc::EPERM) => Ok(true),
                _ => Err(error),
            };
        }

        #[cfg(windows)]
        {
            use std::mem::size_of;
            use std::os::windows::io::AsRawHandle;
            use std::ptr;
            use windows_sys::Win32::System::JobObjects::{
                JobObjectBasicAccountingInformation, QueryInformationJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            };

            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            let result = unsafe {
                QueryInformationJobObject(
                    self.job.as_raw_handle() as _,
                    JobObjectBasicAccountingInformation,
                    &mut accounting as *mut _ as _,
                    size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                    ptr::null_mut(),
                )
            };
            if result == 0 {
                return Err(io::Error::last_os_error());
            }
            return Ok(accounting.ActiveProcesses > 0);
        }

        #[cfg(not(any(unix, windows)))]
        Ok(false)
    }

    /// Stop issuing numeric process-group signals after the tree is known to be
    /// empty. This prevents a later Drop from targeting a reused Unix PGID.
    pub fn disarm(&self) {
        let _signal = self
            .signal_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.state.store(TREE_DISARMED, Ordering::SeqCst);
    }

    #[cfg(unix)]
    /// Returns whether the target process group existed when the signal was
    /// issued. ESRCH permanently disarms this numeric PGID.
    fn signal_group(&self, signal: libc::c_int) -> io::Result<bool> {
        let result = unsafe { libc::kill(-self.process_group, signal) };
        if result == 0 {
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(false)
        } else {
            Err(error)
        }
    }

    #[cfg(windows)]
    fn attach_windows(child: &Child) -> io::Result<Self> {
        use std::mem::size_of;
        use std::os::windows::io::{FromRawHandle, OwnedHandle};
        use std::ptr;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let raw_job = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if raw_job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw_job as _) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                raw_job,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            return Err(io::Error::last_os_error());
        }
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("child process has no process handle"))?;
        if unsafe { AssignProcessToJobObject(raw_job, process as _) } == 0 {
            return Err(io::Error::last_os_error());
        }
        resume_suspended_process(
            child
                .id()
                .ok_or_else(|| io::Error::other("child process has no process id"))?,
        )?;
        Ok(Self {
            state: AtomicU8::new(TREE_ACTIVE),
            signal_lock: std::sync::Mutex::new(()),
            job,
        })
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        // Drop routinely runs after the direct child has been reaped -- an
        // interpreter that exited on its own, an abandoned launch -- so this
        // must be the checked variant. Signalling a numeric process-group id
        // that is already free could reach an unrelated process.
        let _ = self.terminate_if_running();
    }
}

#[cfg(windows)]
fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    use std::mem::size_of;
    use std::os::windows::io::{FromRawHandle, OwnedHandle};
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    let raw_snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if raw_snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw_snapshot as _) };
    let mut entry = THREADENTRY32 {
        dwSize: size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut has_entry = unsafe { Thread32First(raw_snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                return Err(io::Error::last_os_error());
            }
            let resumed = unsafe { ResumeThread(thread) };
            unsafe {
                CloseHandle(thread);
            }
            if resumed == u32::MAX {
                return Err(io::Error::last_os_error());
            }
            drop(snapshot);
            return Ok(());
        }
        has_entry = unsafe { Thread32Next(raw_snapshot, &mut entry) } != 0;
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "could not find the suspended child process thread",
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn git_command_lock_is_reentrant_safe_across_threads() {
        use super::lock_git_command;
        use std::sync::Arc;
        use std::thread;

        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started_for_thread = Arc::clone(&started);
        let done_for_thread = Arc::clone(&done);
        let _guard = lock_git_command();
        let worker = thread::spawn(move || {
            started_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
            let _guard = lock_git_command();
            done_for_thread.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        while !started.load(std::sync::atomic::Ordering::SeqCst) {
            thread::yield_now();
        }
        assert!(
            !done.load(std::sync::atomic::Ordering::SeqCst),
            "git lock must serialize concurrent spawns"
        );
        drop(_guard);
        worker.join().expect("git lock worker");
        assert!(done.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[test]
    fn force_sent_still_polls_until_the_tree_is_confirmed_empty() {
        use super::*;

        let tree = ProcessTree {
            state: AtomicU8::new(FORCE_SENT),
            signal_lock: std::sync::Mutex::new(()),
            process_group: i32::MAX,
        };

        assert!(!tree.is_running().unwrap());
        assert_eq!(tree.state.load(Ordering::SeqCst), TREE_DISARMED);
    }
}
