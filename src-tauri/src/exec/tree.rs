//! Process-tree control for spawned commands.
//!
//! - Unix: the child leads its own process group (`process_group(0)`), and the
//!   whole group is killed with `killpg(SIGKILL)`.
//! - Windows: the child is assigned to a Job Object created with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; terminating or dropping the job
//!   kills every descendant. The assignment happens right after `spawn`, so a
//!   grandchild started in the first microseconds of the child's life is the
//!   one theoretical escape (the child is not created suspended because
//!   `std::process` does not expose the primary-thread handle to resume it).

use std::process::{Child, Command};

pub struct ProcessTree {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: Option<windows_job::JobHandle>,
}

/// Prepares `cmd` so its process tree can be killed as a unit. Call before
/// `spawn`.
pub fn configure(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console window flashes for a backend-spawned child.
        cmd.creation_flags(0x0800_0000);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = cmd;
    }
}

impl ProcessTree {
    /// Binds the freshly spawned `child` to a killable unit.
    pub fn attach(child: &Child) -> ProcessTree {
        #[cfg(unix)]
        {
            ProcessTree { pgid: child.id() as i32 }
        }
        #[cfg(windows)]
        {
            ProcessTree { job: windows_job::JobHandle::assign(child) }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = child;
            ProcessTree {}
        }
    }

    /// Kills the leader and every descendant. Errors are ignored: the tree may
    /// already be gone.
    pub fn kill(&self) {
        #[cfg(unix)]
        {
            // SAFETY: plain syscall with integer arguments; `pgid` is the pid of
            // a child this process spawned as a group leader.
            unsafe {
                libc::killpg(self.pgid, libc::SIGKILL);
            }
        }
        #[cfg(windows)]
        {
            if let Some(job) = &self.job {
                job.terminate();
            }
        }
    }
}

#[cfg(windows)]
mod windows_job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    pub struct JobHandle(HANDLE);

    // SAFETY: a Job Object handle is a process-wide kernel handle with no thread affinity.
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    impl JobHandle {
        /// Creates a kill-on-close job and assigns `child` to it. `None` when any
        /// step fails; the caller then falls back to killing the leader only.
        pub fn assign(child: &Child) -> Option<JobHandle> {
            // SAFETY: FFI calls with valid, initialised arguments; the handle is
            // closed on every failure path and otherwise by `Drop`.
            unsafe {
                let job = CreateJobObjectW(None, windows::core::PCWSTR::null()).ok()?;
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let set = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const core::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                let assigned = set.and_then(|_| AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())));
                match assigned {
                    Ok(()) => Some(JobHandle(job)),
                    Err(_) => {
                        let _ = CloseHandle(job);
                        None
                    }
                }
            }
        }

        pub fn terminate(&self) {
            // SAFETY: `self.0` is a live job handle owned by this value.
            unsafe {
                let _ = TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for JobHandle {
        fn drop(&mut self) {
            // SAFETY: the handle is owned and closed exactly once; kill-on-close
            // terminates any process still in the job.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}
