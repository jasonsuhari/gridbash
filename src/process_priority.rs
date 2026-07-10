use std::io;

use crate::config::PaneProcessPriority;

#[cfg(windows)]
pub fn set_process_priority(process_id: u32, priority: PaneProcessPriority) -> io::Result<()> {
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

    // SAFETY: OpenProcess returns either a valid owned handle or null. The handle
    // is used only with SetPriorityClass and is closed before this function returns.
    let process = unsafe { OpenProcess(PROCESS_SET_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: process is a valid process handle with PROCESS_SET_INFORMATION.
    let changed = unsafe { SetPriorityClass(process, priority_class) };
    let error = (changed == 0).then(io::Error::last_os_error);
    // SAFETY: process is an owned handle returned by OpenProcess above.
    unsafe { CloseHandle(process) };

    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

#[cfg(not(windows))]
pub fn set_process_priority(_process_id: u32, _priority: PaneProcessPriority) -> io::Result<()> {
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::Command;

    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, GetPriorityClass, OpenProcess,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    use super::*;

    #[test]
    fn sets_a_child_process_to_below_normal_priority() {
        let mut child = Command::new("cmd.exe")
            .args(["/d", "/c", "ping 127.0.0.1 -n 10 > nul"])
            .spawn()
            .expect("spawn test child");

        set_process_priority(child.id(), PaneProcessPriority::BelowNormal)
            .expect("lower child priority");

        // SAFETY: the child is still owned by this test and the returned handle is
        // checked for null, queried, then closed.
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, child.id()) };
        assert!(!process.is_null(), "open child for priority query");
        // SAFETY: process is a valid query handle.
        let priority = unsafe { GetPriorityClass(process) };
        // SAFETY: process is an owned handle returned by OpenProcess above.
        unsafe { CloseHandle(process) };

        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(priority, BELOW_NORMAL_PRIORITY_CLASS);
    }
}
