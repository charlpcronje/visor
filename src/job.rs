use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use windows::core::PCSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectA, TerminateJobObject,
};

/// Manages Windows Job Objects for safe process containment.
pub struct JobManager {
    jobs: Mutex<HashMap<String, HANDLE>>,
}

// HANDLE is a raw pointer wrapper; we manage lifetime carefully
unsafe impl Send for JobManager {}
unsafe impl Sync for JobManager {}

impl JobManager {
    pub fn new() -> Self {
        Self {
            jobs: Mutex::new(HashMap::new()),
        }
    }

    /// Create a named Job Object and store it.
    /// We do NOT set JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE — tracked processes
    /// must survive daemon restarts. When the daemon exits, its handles close
    /// and the job is destroyed, but the processes keep running. On restart,
    /// the daemon picks them up again by PID from SQLite.
    pub fn create_job(&self, job_name: &str) -> Result<HANDLE> {
        unsafe {
            let name_cstr = format!("Local\\visor-job-{job_name}\0");
            let handle = CreateJobObjectA(None, PCSTR(name_cstr.as_ptr()))
                .context("CreateJobObjectA failed")?;

            self.jobs.lock().unwrap().insert(job_name.to_string(), handle);
            Ok(handle)
        }
    }

    /// Assign a process to an existing job.
    pub fn assign_process(&self, job_name: &str, process_handle: HANDLE) -> Result<()> {
        let jobs = self.jobs.lock().unwrap();
        let job_handle = jobs
            .get(job_name)
            .ok_or_else(|| anyhow::anyhow!("Job '{job_name}' not found"))?;

        unsafe {
            AssignProcessToJobObject(*job_handle, process_handle)
                .context("AssignProcessToJobObject failed")?;
        }
        Ok(())
    }

    /// Terminate all processes in a job and clean up.
    pub fn terminate_job(&self, job_name: &str) -> Result<()> {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(handle) = jobs.remove(job_name) {
            unsafe {
                let _ = TerminateJobObject(handle, 1);
                let _ = CloseHandle(handle);
            }
        }
        Ok(())
    }

    /// Close a job handle without terminating (for cleanup of already-dead jobs).
    pub fn close_job(&self, job_name: &str) {
        let mut jobs = self.jobs.lock().unwrap();
        if let Some(handle) = jobs.remove(job_name) {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

impl Drop for JobManager {
    fn drop(&mut self) {
        let jobs = self.jobs.lock().unwrap();
        for handle in jobs.values() {
            unsafe {
                let _ = CloseHandle(*handle);
            }
        }
    }
}
