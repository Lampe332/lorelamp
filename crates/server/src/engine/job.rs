//! Model servers end with the app, however the app ends. On Windows every process the engine
//! starts is put into one job object of this process that kills its members when its last handle
//! closes. Only this process holds that handle (it is never closed on purpose), so when the app
//! ends — window closed, crash, `taskkill /F`, Task Manager — Windows ends them too, also a
//! llama-server that is still loading its model. Only processes this app started go in; programs
//! started elsewhere (LM Studio, another llama-server on the port) are never touched.

use std::process::Child;

/// Puts `child` into the job. Err: it would outlive a hard end of the app (the caller logs it).
#[cfg(windows)]
pub fn adopt(child: &Child) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;

    let job = job()?;
    // SAFETY: both handles are valid: the job lives as long as the process, `child` is borrowed.
    if unsafe { AssignProcessToJobObject(job as _, child.as_raw_handle() as _) } == 0 {
        return Err(format!("job object: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// The app is built for Windows; elsewhere the model server is only stopped by a normal exit.
#[cfg(not(windows))]
pub fn adopt(_child: &Child) -> Result<(), String> {
    Ok(())
}

/// The process-wide job (created on first use) as a plain number, so it can sit in a static.
#[cfg(windows)]
fn job() -> Result<usize, String> {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: OnceLock<Result<usize, String>> = OnceLock::new();
    JOB.get_or_init(|| {
        // SAFETY: plain Win32 calls with valid pointers; the handle is checked before use.
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err(format!("job object: {}", std::io::Error::last_os_error()));
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                let e = std::io::Error::last_os_error();
                CloseHandle(handle);
                return Err(format!("job object: {e}"));
            }
            Ok(handle as usize)
        }
    })
    .clone()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::process::{Command, Stdio};
    use windows_sys::Win32::System::JobObjects::IsProcessInJob;

    fn in_our_job(child: &Child) -> bool {
        let mut result = 0;
        // SAFETY: valid handles, `result` is a valid out pointer.
        let ok = unsafe { IsProcessInJob(child.as_raw_handle() as _, job().unwrap() as _, &mut result) };
        ok != 0 && result != 0
    }

    #[test]
    fn a_started_process_is_put_into_the_kill_on_close_job() {
        let mut child = Command::new("cmd").args(["/c", "ping -n 30 127.0.0.1 >nul"]).stdout(Stdio::null()).spawn().unwrap();
        assert!(!in_our_job(&child));
        adopt(&child).unwrap();
        assert!(in_our_job(&child), "adopted process is in the job");
        let _ = child.kill();
        let _ = child.wait();
    }

    fn sleeper() -> Child {
        Command::new("cmd").args(["/c", "ping -n 60 127.0.0.1 >nul"]).stdout(Stdio::null()).spawn().unwrap()
    }

    /// Waits up to `ms` for process `pid` to end; true when it has ended.
    fn ends_within(pid: u32, ms: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
        // SAFETY: plain Win32 calls; the handle is checked and closed.
        unsafe {
            let h = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return true; // no such process any more
            }
            let ended = WaitForSingleObject(h, ms) == WAIT_OBJECT_0;
            CloseHandle(h);
            ended
        }
    }

    /// Run by `a_hard_end_of_the_parent_ends_its_children_only` in a child test process: starts and
    /// adopts a process, prints its pid, then waits to be killed.
    #[test]
    #[ignore = "helper, started by another test"]
    fn helper_adopts_and_waits() {
        if std::env::var("RP_JOB_HELPER").is_err() {
            return;
        }
        let mut child = sleeper();
        adopt(&child).unwrap();
        println!("PID {}", child.id());
        std::thread::sleep(std::time::Duration::from_secs(60));
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn a_hard_end_of_the_parent_ends_its_children_only() {
        use std::io::BufRead;
        let mut foreign = sleeper(); // started elsewhere: must survive
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "engine::job::tests::helper_adopts_and_waits", "--ignored", "--nocapture", "--test-threads=1"])
            .env("RP_JOB_HELPER", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = std::io::BufReader::new(parent.stdout.take().unwrap()).lines();
        let pid: u32 = lines
            .by_ref()
            .map_while(Result::ok)
            // libtest may print the test's name on the same line first
            .find_map(|l| l.split("PID ").nth(1).and_then(|p| p.trim().parse().ok()))
            .expect("helper prints the pid");
        assert!(!ends_within(pid, 300), "adopted process runs while its parent lives");
        parent.kill().unwrap(); // TerminateProcess: no cleanup code of the parent runs
        let _ = parent.wait();
        assert!(ends_within(pid, 5000), "adopted process ended with its parent");
        assert!(!ends_within(foreign.id(), 300), "a process the parent did not start is left alone");
        let _ = foreign.kill();
        let _ = foreign.wait();
    }
}
