// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Alexander Mohr

//! A Windows job object around a child and everything it starts: Windows'
//! nearest thing to a process group that can be killed as a whole.
//!
//! A process belongs to a job from its first instruction only if it is put
//! there before it runs, so [`spawn_in_job`] starts the child suspended,
//! assigns it, and only then lets it run. Whatever it starts afterwards
//! lands in the same job.

use std::{
    io,
    os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
    ptr,
};

use tokio::process::{Child, Command};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOBOBJECT_BASIC_ACCOUNTING_INFORMATION,
            JobObjectBasicAccountingInformation, QueryInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

/// The exit code a process killed through [`Job::terminate`] reports.
const KILLED_EXIT_CODE: u32 = 1;

/// A job object. Dropping it closes the handle without touching the
/// processes in it.
#[derive(Debug)]
pub struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: no security attributes and no name; the returned handle,
        // if any, is owned from here on.
        let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a fresh, valid handle nobody else owns.
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    /// Kills every process in the job.
    ///
    /// # Errors
    ///
    /// Fails if the job can't be terminated.
    pub fn terminate(&self) -> io::Result<()> {
        // SAFETY: the handle is a valid job handle with full access.
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), KILLED_EXIT_CODE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// How many processes in the job are still running.
    ///
    /// # Errors
    ///
    /// Fails if the job can't be queried.
    pub fn active_processes(&self) -> io::Result<u32> {
        // SAFETY: an all-zero accounting record is a valid value.
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = unsafe { std::mem::zeroed() };
        let size = u32::try_from(size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>())
            .map_err(io::Error::other)?;
        // SAFETY: `info` is a valid out-buffer of `size` bytes for this
        // information class.
        let queried = unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                JobObjectBasicAccountingInformation,
                (&raw mut info).cast(),
                size,
                ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.ActiveProcesses)
    }
}

/// Spawns `cmd` on a console of its own (see [`crate::console::isolate`])
/// and inside a new job, before it runs any code.
///
/// # Errors
///
/// Fails if the child can't be started, or can't be put in the job; such a
/// child is killed rather than left suspended.
pub fn spawn_in_job(cmd: &mut Command) -> io::Result<(Child, Job)> {
    crate::console::isolate(cmd);
    cmd.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    let job = Job::new()?;
    let mut child = cmd.spawn()?;
    let started = (|| {
        let process = child
            .raw_handle()
            .ok_or_else(|| io::Error::other("the child has no process handle"))?;
        // SAFETY: both handles are valid; the child has not run yet.
        if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), process) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the child has no process id"))?;
        resume_threads(pid)
    })();
    match started {
        Ok(()) => Ok((child, job)),
        Err(e) => {
            let _ = child.start_kill();
            Err(e)
        }
    }
}

/// Resumes the threads of `pid`, a process started suspended (which has
/// exactly one).
fn resume_threads(pid: u32) -> io::Result<()> {
    // SAFETY: a thread snapshot of the whole system takes no pointers.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `snapshot` is a fresh, valid handle nobody else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    // SAFETY: an all-zero THREADENTRY32 is a valid value.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = u32::try_from(size_of::<THREADENTRY32>()).map_err(io::Error::other)?;
    let mut resumed = 0u32;
    // SAFETY: `snapshot` is valid and `entry` has its size set.
    let mut more = unsafe { Thread32First(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            resume_thread(entry.th32ThreadID)?;
            resumed = resumed.saturating_add(1);
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snapshot.as_raw_handle(), &raw mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(io::Error::other(format!(
            "found no thread of process {pid} to resume"
        )));
    }
    Ok(())
}

fn resume_thread(thread_id: u32) -> io::Result<()> {
    // SAFETY: opening a thread by id takes no pointers.
    let thread: HANDLE = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, thread_id) };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `thread` is a fresh, valid handle nobody else owns.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
    // SAFETY: `thread` was opened with THREAD_SUSPEND_RESUME.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{process::Stdio, time::Duration};

    use super::*;

    /// Polls `job` until `done` holds for its number of running processes,
    /// for up to ten seconds.
    async fn wait_for(job: &Job, done: impl Fn(u32) -> bool) -> u32 {
        let mut active = job.active_processes().unwrap();
        for _ in 0..100 {
            if done(active) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            active = job.active_processes().unwrap();
        }
        active
    }

    /// `cmd`, a program it starts in the background and one it waits for
    /// all run in the job, and terminating it ends every one of them, the
    /// background one included, which killing `cmd` alone would leave.
    #[tokio::test]
    async fn terminating_the_job_ends_everything_the_child_started() {
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/C")
            .raw_arg("start /b ping -n 120 127.0.0.1 >nul & ping -n 120 127.0.0.1 >nul")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let (mut child, job) = spawn_in_job(&mut cmd).unwrap();

        let started = wait_for(&job, |active| active >= 3).await;
        assert!(started >= 3, "cmd and both pings run in the job: {started}");

        job.terminate().unwrap();
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .expect("cmd exits once the job is terminated")
            .unwrap();
        assert_eq!(status.code(), Some(1));
        assert_eq!(wait_for(&job, |active| active == 0).await, 0);
    }

    #[tokio::test]
    async fn a_child_in_a_job_runs_to_completion() {
        let mut cmd = Command::new("cmd.exe");
        cmd.args(["/C", "exit 7"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let (mut child, job) = spawn_in_job(&mut cmd).unwrap();

        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .expect("a resumed child runs")
            .unwrap();

        assert_eq!(status.code(), Some(7));
        assert_eq!(wait_for(&job, |active| active == 0).await, 0);
    }
}
