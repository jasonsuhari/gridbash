use std::io;

use crate::config::{PaneProcessPriority, PaneWorkloadPolicy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneWorkloadClass {
    Focused,
    Selected,
    Visible,
    Background,
}

#[cfg(windows)]
impl PaneWorkloadClass {
    fn weight(self) -> u32 {
        match self {
            Self::Focused => 9,
            Self::Selected => 7,
            Self::Visible => 3,
            Self::Background => 1,
        }
    }
}

pub struct PaneWorkloadController {
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

impl PaneWorkloadController {
    pub fn attach(
        process_id: u32,
        priority: PaneProcessPriority,
        policy: PaneWorkloadPolicy,
    ) -> io::Result<Self> {
        set_process_priority(process_id, priority)?;
        Self::attach_platform(process_id, policy)
    }

    pub fn unmanaged() -> Self {
        Self {
            #[cfg(windows)]
            job: std::ptr::null_mut(),
        }
    }

    pub fn apply(&self, policy: PaneWorkloadPolicy, class: PaneWorkloadClass) -> io::Result<()> {
        self.apply_platform(policy, class)
    }
}

#[cfg(windows)]
impl PaneWorkloadController {
    fn attach_platform(process_id: u32, policy: PaneWorkloadPolicy) -> io::Result<Self> {
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::{
                JobObjects::{AssignProcessToJobObject, CreateJobObjectW},
                Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
            },
        };

        // SAFETY: null security attributes and name create an unnamed job owned
        // by the returned handle.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }

        // AssignProcessToJobObject requires quota and terminate access. The
        // process remains owned by portable-pty; this handle is query-only here.
        let process = unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, process_id) };
        if process.is_null() {
            let error = io::Error::last_os_error();
            unsafe { CloseHandle(job) };
            return Err(error);
        }

        let assigned = unsafe { AssignProcessToJobObject(job, process) };
        let error = (assigned == 0).then(io::Error::last_os_error);
        unsafe { CloseHandle(process) };
        if let Some(error) = error {
            unsafe { CloseHandle(job) };
            return Err(error);
        }

        let controller = Self { job };
        controller.apply(policy, PaneWorkloadClass::Visible)?;
        Ok(controller)
    }

    fn apply_platform(
        &self,
        policy: PaneWorkloadPolicy,
        class: PaneWorkloadClass,
    ) -> io::Result<()> {
        use windows_sys::Win32::System::JobObjects::{
            JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_WEIGHT_BASED,
            JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0,
            JobObjectCpuRateControlInformation, SetInformationJobObject,
        };

        if self.job.is_null() {
            return Ok(());
        }

        let info = match policy {
            PaneWorkloadPolicy::Adaptive => JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE
                    | JOB_OBJECT_CPU_RATE_CONTROL_WEIGHT_BASED,
                Anonymous: JOBOBJECT_CPU_RATE_CONTROL_INFORMATION_0 {
                    Weight: class.weight(),
                },
            },
            PaneWorkloadPolicy::Unrestricted => JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default(),
        };

        let changed = unsafe {
            SetInformationJobObject(
                self.job,
                JobObjectCpuRateControlInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        };
        if changed == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(windows))]
impl PaneWorkloadController {
    fn attach_platform(_process_id: u32, _policy: PaneWorkloadPolicy) -> io::Result<Self> {
        Ok(Self {})
    }

    fn apply_platform(
        &self,
        _policy: PaneWorkloadPolicy,
        _class: PaneWorkloadClass,
    ) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for PaneWorkloadController {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        if !self.job.is_null() {
            // No kill-on-close limit is set; closing only releases GridBash's job
            // handle and preserves the existing child termination semantics.
            unsafe { CloseHandle(self.job) };
        }
    }
}

#[cfg(windows)]
fn set_process_priority(process_id: u32, priority: PaneProcessPriority) -> io::Result<()> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, OpenProcess,
            PROCESS_SET_INFORMATION, SetPriorityClass,
        },
    };

    let priority_class = match priority {
        PaneProcessPriority::Normal => NORMAL_PRIORITY_CLASS,
        PaneProcessPriority::BelowNormal => BELOW_NORMAL_PRIORITY_CLASS,
    };
    let process = unsafe { OpenProcess(PROCESS_SET_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let changed = unsafe { SetPriorityClass(process, priority_class) };
    let error = (changed == 0).then(io::Error::last_os_error);
    unsafe { CloseHandle(process) };
    error.map_or(Ok(()), Err)
}

#[cfg(not(windows))]
fn set_process_priority(_process_id: u32, _priority: PaneProcessPriority) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::Command;

    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{
            JobObjects::{
                IsProcessInJob, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
                JobObjectCpuRateControlInformation, QueryInformationJobObject,
            },
            Threading::{
                BELOW_NORMAL_PRIORITY_CLASS, GetPriorityClass, OpenProcess,
                PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
    };

    use super::*;

    #[test]
    fn attaches_child_and_updates_adaptive_weight() {
        let mut child = Command::new("cmd.exe")
            .args(["/d", "/c", "ping 127.0.0.1 -n 10 > nul"])
            .spawn()
            .expect("spawn test child");
        let controller = PaneWorkloadController::attach(
            child.id(),
            PaneProcessPriority::BelowNormal,
            PaneWorkloadPolicy::Adaptive,
        )
        .expect("attach workload controller");
        controller
            .apply(PaneWorkloadPolicy::Adaptive, PaneWorkloadClass::Focused)
            .expect("set focused weight");

        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child.id()) };
        assert!(!process.is_null(), "open child for query");
        let mut in_job = 0;
        assert_ne!(
            unsafe { IsProcessInJob(process, controller.job, &mut in_job) },
            0
        );
        assert_ne!(in_job, 0);
        assert_eq!(
            unsafe { GetPriorityClass(process) },
            BELOW_NORMAL_PRIORITY_CLASS
        );

        let mut info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    controller.job,
                    JobObjectCpuRateControlInformation,
                    std::ptr::from_mut(&mut info).cast(),
                    std::mem::size_of_val(&info) as u32,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        assert_eq!(unsafe { info.Anonymous.Weight }, 9);

        controller
            .apply(PaneWorkloadPolicy::Unrestricted, PaneWorkloadClass::Visible)
            .expect("disable adaptive scheduling");
        let mut unrestricted = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    controller.job,
                    JobObjectCpuRateControlInformation,
                    std::ptr::from_mut(&mut unrestricted).cast(),
                    std::mem::size_of_val(&unrestricted) as u32,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        assert_eq!(unrestricted.ControlFlags, 0);

        unsafe { CloseHandle(process) };
        let _ = child.kill();
        let _ = child.wait();
    }
}
