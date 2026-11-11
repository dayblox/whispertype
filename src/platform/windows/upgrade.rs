use super::*;
use std::{path::Path, time::Instant};
use windows_sys::Win32::System::{Diagnostics::ToolHelp::*, Threading::*};

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Process {
    id: u32,
    handle: Handle,
}
unsafe fn image(handle: HANDLE) -> Result<std::path::PathBuf> {
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    ensure!(
        QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length) != 0,
        "Could not identify an application process"
    );
    Ok(
        std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]))
            .canonicalize()?,
    )
}
unsafe extern "system" fn request_quit(window: HWND, data: LPARAM) -> i32 {
    let processes = &*(data as *const Vec<Process>);
    let mut id = 0;
    GetWindowThreadProcessId(window, &mut id);
    if processes.iter().any(|p| p.id == id) {
        let mut name = [0u16; 100];
        let n = GetClassNameW(window, name.as_mut_ptr(), name.len() as i32);
        if String::from_utf16_lossy(&name[..n.max(0) as usize]) == CLASS {
            // Keep the existing IPC command numbers compatible with older releases.
            PostMessageW(window, CONTROL, 11, 0);
        }
    }
    1
}

/// Stop only processes backed by this exact installed EXE, including helpers.
/// The installer runs a temporary copy, so the running file can be replaced.
pub fn stop_for_upgrade(target: &Path) -> Result<()> {
    if !target.exists() {
        return Ok(());
    }
    let target = target.canonicalize()?;
    let name = target
        .file_name()
        .context("Invalid executable path")?
        .to_string_lossy();
    unsafe {
        let raw = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        ensure!(
            raw != INVALID_HANDLE_VALUE,
            "Could not inspect running applications"
        );
        let snapshot = Handle(raw);
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut processes = Vec::new();
        let mut more = Process32FirstW(snapshot.0, &mut entry);
        while more != 0 {
            let len = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            if entry.th32ProcessID != GetCurrentProcessId()
                && String::from_utf16_lossy(&entry.szExeFile[..len]).eq_ignore_ascii_case(&name)
            {
                let query = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !query.is_null() {
                    let query = Handle(query);
                    if image(query.0).is_ok_and(|path| {
                        path.to_string_lossy()
                            .eq_ignore_ascii_case(&target.to_string_lossy())
                    }) {
                        let handle = OpenProcess(
                            PROCESS_QUERY_LIMITED_INFORMATION
                                | PROCESS_TERMINATE
                                | PROCESS_SYNCHRONIZE,
                            0,
                            entry.th32ProcessID,
                        );
                        ensure!(!handle.is_null(), "Cannot stop the installed Whispertype process. Close it and retry Setup.");
                        let handle = Handle(handle);
                        // Verify again after reopening to protect against PID reuse.
                        if image(handle.0)?
                            .to_string_lossy()
                            .eq_ignore_ascii_case(&target.to_string_lossy())
                        {
                            processes.push(Process {
                                id: entry.th32ProcessID,
                                handle,
                            });
                        }
                    }
                }
            }
            more = Process32NextW(snapshot.0, &mut entry);
        }
        EnumWindows(
            Some(request_quit),
            &processes as *const Vec<Process> as isize,
        );
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until
            && processes
                .iter()
                .any(|p| WaitForSingleObject(p.handle.0, 0) == WAIT_TIMEOUT)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
        for process in processes {
            if WaitForSingleObject(process.handle.0, 0) == WAIT_TIMEOUT {
                ensure!(
                    TerminateProcess(process.handle.0, 0) != 0,
                    "Could not stop a Whispertype helper. Close it and retry Setup."
                );
                ensure!(
                    WaitForSingleObject(process.handle.0, 2000) == WAIT_OBJECT_0,
                    "Whispertype did not exit. Close it and retry Setup."
                );
            }
        }
    }
    // A different/elevated process might have denied enumeration access.
    // Check the file itself before Setup reaches its replacement step.
    // During uninstall this helper is itself running from the installed EXE.
    if std::env::current_exe()?.canonicalize()? != target {
        use std::os::windows::fs::OpenOptionsExt;
        let _exclusive = std::fs::OpenOptions::new().read(true).write(true).share_mode(0)
            .open(&target).context("The installed EXE is still in use or cannot be replaced. Close Whispertype and retry Setup.")?;
    }
    Ok(())
}
