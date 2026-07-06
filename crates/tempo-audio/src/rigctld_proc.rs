//! Launching Hamlib's `rigctld` daemon ourselves.
//!
//! Instead of depending on `libhamlib` at build time, Tempo shells out to the
//! `rigctld` binary the operator already has installed and talks to it over TCP
//! (see [`crate::rig`]). [`rigctld_args`] builds the command line and is pure /
//! unit-tested; [`spawn_rigctld`] launches it and returns a kill-on-drop
//! [`Child`] so the daemon dies with Tempo.

use std::process::{Child, Command};

/// Build the `rigctld` argument vector.
///
/// Produces e.g. `["-m", "3073", "-r", "COM5", "-s", "38400", "-t", "4532"]`:
/// - `-m <model>` Hamlib rig model number.
/// - `-r <addr>` the radio: a serial device (COM5, /dev/ttyUSB0) OR, when `network`, a
///   `host:port` (e.g. a FlexRadio's SmartSDR IP:4992) — Hamlib's network backends take the
///   host:port on `-r`. Omitted when `addr` is empty (Dummy / NET rigs that need no port).
/// - `-s <baud>` serial speed — omitted for a network rig (`network`) or an empty addr, since
///   a TCP rig has no baud rate.
/// - `-t <tcp_port>` TCP port for the daemon to listen on.
pub fn rigctld_args(
    model: u32,
    addr: &str,
    baud: u32,
    tcp_port: u16,
    network: bool,
) -> Vec<String> {
    let mut args = vec!["-m".to_string(), model.to_string()];
    if !addr.is_empty() {
        args.push("-r".to_string());
        args.push(addr.to_string());
        if !network {
            args.push("-s".to_string());
            args.push(baud.to_string());
        }
    }
    args.push("-t".to_string());
    args.push(tcp_port.to_string());
    args
}

/// A spawned `rigctld` that is killed when this handle is dropped — and, on
/// Windows, also when Tempo's *process* exits even if this handle is never
/// dropped (e.g. the detached radio thread is torn down at shutdown without
/// unwinding). On drop it kills + reaps the child; on Windows the daemon is also
/// placed in a Job Object with `KILL_ON_JOB_CLOSE`, so closing the job handle
/// (explicitly on drop, or implicitly by the OS at process exit) kills rigctld
/// and frees the serial/COM port. This is what prevents a stuck COM port after
/// closing Tempo.
pub struct RigctldProc {
    child: Child,
    /// Job-object handle (Windows) as an `isize`; 0 = none. Held for the daemon's
    /// lifetime so the kill-on-close guarantee is in force until Tempo exits.
    #[cfg(windows)]
    job: isize,
}

impl RigctldProc {
    /// The underlying child's process id, for logging.
    pub fn id(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for RigctldProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(windows)]
        if self.job != 0 {
            // SAFETY: `job` is a valid job-object HANDLE we created and have not
            // yet closed. Closing it triggers KILL_ON_JOB_CLOSE for any survivor.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.job as *mut core::ffi::c_void);
            }
        }
    }
}

/// Place `child` in a new Job Object set to kill its processes when the job
/// handle closes, so rigctld dies with Tempo (clean exit, crash, or detached-
/// thread teardown). Returns the job HANDLE as an `isize` (0 on any failure, in
/// which case we just fall back to the Drop-time kill).
#[cfg(windows)]
fn assign_kill_on_close_job(child: &Child) -> isize {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    unsafe {
        let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = core::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set_ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            core::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set_ok == 0 || AssignProcessToJobObject(job, child.as_raw_handle()) == 0 {
            CloseHandle(job);
            return 0;
        }
        job as isize
    }
}

/// Locate the `rigctld` binary. Prefers one **bundled next to the app** — the
/// Windows installer ships Hamlib under the install dir (with its DLLs), so CAT
/// works with no separate Hamlib install — and falls back to `rigctld` on
/// `PATH`. Launching the bundled exe by full path lets Windows resolve its
/// co-located DLLs (libhamlib-4.dll etc.) from the exe's own directory.
fn resolve_rigctld() -> std::ffi::OsString {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for cand in [
                "hamlib/rigctld.exe",
                "resources/hamlib/rigctld.exe",
                "rigctld.exe",
                "hamlib/rigctld",
            ] {
                let p = dir.join(cand);
                if p.is_file() {
                    return p.into_os_string();
                }
            }
        }
    }
    std::ffi::OsString::from("rigctld")
}

/// Build the `rotctld` argument vector — same shape as [`rigctld_args`] minus
/// the network-rig special case (rotators are serial devices to Hamlib).
pub fn rotctld_args(model: u32, port: &str, baud: u32, tcp_port: u16) -> Vec<String> {
    let mut args = vec!["-m".to_string(), model.to_string()];
    if !port.is_empty() {
        args.push("-r".to_string());
        args.push(port.to_string());
        args.push("-s".to_string());
        args.push(baud.to_string());
    }
    args.push("-t".to_string());
    args.push(tcp_port.to_string());
    args
}

/// The bundled `rotctld` (ships beside rigctld in the Hamlib bundle), falling
/// back to PATH — same resolution as [`resolve_rigctld`].
fn resolve_rotctld() -> std::ffi::OsString {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for cand in [
                "hamlib/rotctld.exe",
                "resources/hamlib/rotctld.exe",
                "rotctld.exe",
                "hamlib/rotctld",
            ] {
                let p = dir.join(cand);
                if p.is_file() {
                    return p.into_os_string();
                }
            }
        }
    }
    std::ffi::OsString::from("rotctld")
}

/// Spawn `rotctld` for a ROTATOR `model` on `port`@`baud`, listening on
/// `tcp_port` — the rotator twin of [`spawn_rigctld`], with the same
/// kill-on-drop + Windows job-object lifetime guarantees (reuses
/// [`RigctldProc`]; the handle is daemon-agnostic).
pub fn spawn_rotctld(
    model: u32,
    port: &str,
    baud: u32,
    tcp_port: u16,
) -> std::io::Result<RigctldProc> {
    let args = rotctld_args(model, port, baud, tcp_port);
    let mut cmd = Command::new(resolve_rotctld());
    cmd.args(&args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd.spawn()?;
    #[cfg(windows)]
    let job = assign_kill_on_close_job(&child);
    Ok(RigctldProc {
        child,
        #[cfg(windows)]
        job,
    })
}

/// Spawn `rigctld` for `model` on `serial_port`@`baud`, listening on
/// `tcp_port`. Returns a kill-on-drop handle. Uses the bundled Hamlib if present
/// (see [`resolve_rigctld`]), otherwise a `rigctld` on `PATH`.
pub fn spawn_rigctld(
    model: u32,
    addr: &str,
    baud: u32,
    tcp_port: u16,
    network: bool,
) -> std::io::Result<RigctldProc> {
    let args = rigctld_args(model, addr, baud, tcp_port, network);
    let mut cmd = Command::new(resolve_rigctld());
    cmd.args(&args);
    // On Windows, don't pop a console window for the daemon (Tempo is a GUI app).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd.spawn()?;
    // On Windows, bind the daemon to a kill-on-close Job Object so it can't
    // outlive Tempo and keep the COM port locked.
    #[cfg(windows)]
    let job = assign_kill_on_close_job(&child);
    Ok(RigctldProc {
        child,
        #[cfg(windows)]
        job,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotctld_args_mirror_the_rig_shape() {
        assert_eq!(
            rotctld_args(601, "COM7", 9600, 4533),
            vec!["-m", "601", "-r", "COM7", "-s", "9600", "-t", "4533"]
        );
        // No port (Dummy rotator for testing) → no -r/-s.
        assert_eq!(
            rotctld_args(1, "", 9600, 4533),
            vec!["-m", "1", "-t", "4533"]
        );
    }

    #[test]
    fn args_with_serial_port() {
        let args = rigctld_args(3073, "COM5", 38400, 4532, false);
        assert_eq!(
            args,
            vec!["-m", "3073", "-r", "COM5", "-s", "38400", "-t", "4532"]
        );
    }

    #[test]
    fn args_for_network_rig_omit_baud() {
        // A FlexRadio over SmartSDR (or any TCP rig): host:port on -r, no baud.
        let args = rigctld_args(23005, "192.168.1.50:4992", 38400, 4532, true);
        assert_eq!(
            args,
            vec!["-m", "23005", "-r", "192.168.1.50:4992", "-t", "4532"]
        );
    }

    #[test]
    fn args_for_unix_serial_device() {
        let args = rigctld_args(1042, "/dev/ttyUSB0", 19200, 4533, false);
        assert_eq!(
            args,
            vec![
                "-m",
                "1042",
                "-r",
                "/dev/ttyUSB0",
                "-s",
                "19200",
                "-t",
                "4533"
            ]
        );
    }

    #[test]
    fn args_without_serial_port_omit_port_and_baud() {
        // Dummy / NET rigs need no serial device.
        let args = rigctld_args(1, "", 38400, 4532, false);
        assert_eq!(args, vec!["-m", "1", "-t", "4532"]);
    }
}
