//! A small safe wrapper around a Win32 Job Object, ensures that
//! iperf3 does not outlive the app.
//!
//! Terminate uses `Child::kill()`, but if the GUI crashes or is killed from
//! Task Manager, no Rust code runs. A job with `KILL_ON_JOB_CLOSE` terminates
//! its processes when its last handle closes, and Windows closes all handles
//! when a process dies.

use std::io;
use std::mem;
use std::os::windows::io::AsRawHandle;
use std::process::Child;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};

/// An anonymous job that kills its processes when this value is dropped.
#[derive(Debug)]
pub struct KillOnCloseJob {
    handle: HANDLE,
}

// SAFETY: a job object HANDLE is a kernel handle, not a pointer to Rust
// memory. Win32 job functions may be called from any thread.
unsafe impl Send for KillOnCloseJob {}
// SAFETY: `&KillOnCloseJob` exposes no methods that mutate the handle.
unsafe impl Sync for KillOnCloseJob {}

impl KillOnCloseJob {
    /// Creates a new job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` set.
    pub fn new() -> io::Result<Self> {
        // SAFETY: null security attributes and a null name create an
        // anonymous job with default security.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let job = Self { handle };

        // SAFETY: JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a plain C struct,
        // and all-zero is its default state.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `job.handle` is a valid job handle, and `info` is a live,
        // correctly sized struct matching `JobObjectExtendedLimitInformation`.
        let ok = unsafe {
            SetInformationJobObject(
                job.handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(job)
    }

    /// Puts `child` in this job, so it's killed when the job is dropped.
    pub fn assign(&self, child: &Child) -> io::Result<()> {
        // SAFETY: both handles are valid for the duration of the call. The
        // child's handle is owned by `child`, which the caller borrows.
        let ok = unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle() as HANDLE) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed exactly once.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
