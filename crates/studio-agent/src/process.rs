//! Spawning CLI processes: no console flash, and they die with the app.

use std::process::{Child, Command};

pub(crate) fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Kill `child` when the app exits (Windows job object; elsewhere the child is killed on
/// drop of its turn).
#[cfg(windows)]
pub(crate) fn tie_to_app(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        job as usize
    });
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn tie_to_app(_child: &Child) {}

/// Make the child lead its own process group (Unix) so the whole tree can be signalled.
/// Windows uses a per-turn job object instead ([`ProcessTree`]).
pub(crate) fn own_group(cmd: &mut Command) -> &mut Command {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd
}

/// A spawned CLI and everything it started (npm `.cmd` shims, node, tool subprocesses),
/// killable as one.
///
/// Windows: a job object per turn with KILL_ON_JOB_CLOSE, so dropping it also kills the
/// tree. The child is assigned right after `spawn`; a descendant it forks in that first
/// instant (before the assignment) would escape the job. `CREATE_SUSPENDED` would close the
/// window but std doesn't expose the main thread handle to resume, and the CLI spends its
/// first moments starting up, so the window is accepted.
/// Unix: the child leads its own process group (see [`own_group`]) and the group is
/// signalled.
pub(crate) struct ProcessTree {
    #[cfg(windows)]
    job: usize,
    #[cfg(unix)]
    pgid: u32,
    /// The leader has been reaped: its pid (= group id on Unix) may be reused.
    #[cfg(unix)]
    done: std::sync::atomic::AtomicBool,
}

#[cfg(windows)]
impl ProcessTree {
    pub(crate) fn attach(child: &Child) -> Self {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Self { job: 0 };
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            // Nested jobs are fine on Windows 8+ (the app-lifetime job stays a parent).
            AssignProcessToJobObject(job, child.as_raw_handle() as _);
            Self { job: job as usize }
        }
    }

    /// Kill the CLI and all its descendants.
    pub(crate) fn kill(&self) {
        use windows_sys::Win32::System::JobObjects::TerminateJobObject;
        if self.job != 0 {
            unsafe {
                TerminateJobObject(self.job as _, 1);
            }
        }
    }

    /// The leader was reaped (no-op on Windows: the job handle stays valid).
    pub(crate) fn reaped(&self) {}
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        if self.job != 0 {
            // KILL_ON_JOB_CLOSE ends anything still running.
            unsafe {
                CloseHandle(self.job as _);
            }
        }
    }
}

#[cfg(unix)]
impl ProcessTree {
    pub(crate) fn attach(child: &Child) -> Self {
        Self { pgid: child.id(), done: Default::default() }
    }

    pub(crate) fn kill(&self) {
        if self.done.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.pgid)])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    pub(crate) fn reaped(&self) {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(not(any(windows, unix)))]
impl ProcessTree {
    pub(crate) fn attach(_child: &Child) -> Self {
        Self {}
    }
    pub(crate) fn kill(&self) {}
    pub(crate) fn reaped(&self) {}
}
