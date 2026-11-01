//! Child-process helpers: console-less spawning and a Job Object that takes every
//! child down with the launcher so no orphaned node.exe keeps port 8000 busy.

use std::ffi::{c_void, OsString};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};

use anyhow::{Context, Result};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A `Command` that never flashes a console window.
pub fn quiet_command(program: impl AsRef<Path>) -> Command {
    let mut cmd = Command::new(program.as_ref());
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// PATH with the portable Node folder in front, for npm and anything it spawns.
pub fn prefixed_path(node_dir: &Path) -> OsString {
    let mut p = node_dir.as_os_str().to_owned();
    p.push(";");
    if let Some(existing) = std::env::var_os("PATH") {
        p.push(existing);
    }
    p
}

/// Windows Job Object configured so every process assigned to it is killed when
/// the job handle closes, i.e. when this launcher exits for any reason.
pub struct JobObject {
    handle: HANDLE,
}

// HANDLEs are plain kernel handles; the job is only ever used through &self.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    pub fn new() -> Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null()).context("CreateJobObject failed")?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .context("SetInformationJobObject failed")?;
            Ok(Self { handle })
        }
    }

    pub fn assign(&self, child: &Child) -> Result<()> {
        unsafe {
            AssignProcessToJobObject(self.handle, HANDLE(child.as_raw_handle() as *mut c_void))
                .context("AssignProcessToJobObject failed")
        }
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// Blocks until the process with `pid` has exited (or `timeout` passes). Used by a freshly
/// self-updated launcher so the single-instance guard does not bounce it off the old one.
pub fn wait_for_pid(pid: u32, timeout: std::time::Duration) {
    use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return; // already gone
        };
        WaitForSingleObject(handle, timeout.as_millis() as u32);
        let _ = CloseHandle(handle);
    }
}

/// Assigns `child` to the job when one exists; logs instead of failing otherwise.
pub fn attach_to_job(job: Option<&JobObject>, child: &Child, what: &str) {
    if let Some(job) = job {
        if let Err(e) = job.assign(child) {
            log::warn!("could not attach {what} to the job object: {e:#}");
        }
    }
}
