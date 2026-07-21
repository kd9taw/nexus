//! Serial-port enumeration for the rig-control UI (feature `serial`).
//!
//! [`available_ports`] is public in both builds so UI code can call it
//! unconditionally. With the `serial` feature it lists the OS serial ports via
//! `serialport`; without it (the headless build, which has no libudev) it
//! returns an empty list so a port can still be typed in by hand.

/// Names of the serial ports currently present (e.g. `"COM5"`, `"/dev/ttyUSB0"`).
///
/// Returns an empty `Vec` when built without the `serial` feature, or if the
/// platform enumeration fails.
#[cfg(feature = "serial")]
pub fn available_ports() -> Vec<String> {
    // serialport's Windows enumeration walks the registry / SetupAPI and can panic on some driver
    // setups (Flex/virtual COM ports). This runs when the Settings tab opens, so isolate it — a
    // panic yields an empty list (the operator can still type a COM port) instead of crashing.
    std::panic::catch_unwind(|| {
        // `mut` is only exercised by the Linux virtual-port union below; on other
        // targets that block compiles away (this was a warning in every Windows
        // cross-build — noise that trains eyes to skip warnings).
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut names: Vec<String> = match serialport::available_ports() {
            Ok(ports) => ports.into_iter().map(|p| p.port_name).collect(),
            Err(_) => Vec::new(),
        };
        // On Linux, union in the virtual ports udev structurally cannot report (see
        // `linux_virtual_ports`). Dedup by path — a node reported both ways must appear once.
        #[cfg(target_os = "linux")]
        for v in linux_virtual_ports(std::path::Path::new("/dev")) {
            if !names.contains(&v) {
                names.push(v);
            }
        }
        names
    })
    .unwrap_or_else(|_| {
        // NEVER swallow silently: a per-poll panic here is invisible but costs real
        // CPU (unwind + panic hook each time) — the "sluggish laptop" failure mode.
        // Rate-limited so a storm doesn't also flood stderr.
        use std::sync::atomic::{AtomicU32, Ordering};
        static CAUGHT: AtomicU32 = AtomicU32::new(0);
        let n = CAUGHT.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n.is_multiple_of(100) {
            eprintln!(
                "nexus: serial-port enumeration panicked (caught; occurrence {n}) — \
                 a driver/udev issue on this system; ports list returned empty"
            );
        }
        Vec::new()
    })
}

/// Virtual serial ports on Linux that `serialport`'s udev enumeration cannot see.
///
/// `serialport` 4.9 asks udev for the `tty` subsystem and then keeps a device only when
/// `parent.is_some() || is_rfcomm(..)` (posix/enumerate.rs). PTY-backed virtual ports have no
/// udev parent, and `/dev/pts/N` gets no persistent `/sys/class/tty` entry at all, so they are
/// invisible to that API BY DESIGN — not a bug we can fix upstream-side. Hams hit this whenever
/// a virtual pair bridges Nexus to another program (a rigctld/flrig bridge, WSJT-X interop, a
/// GPS feed). Reported symptom: "CAT works but no ports are listed" — CAT works because it
/// connects to a typed path or a network host and never needs enumeration.
///
/// We deliberately do NOT sweep `/dev/pts/*`: those are ordinary terminal sessions (every open
/// shell is one), and listing them would bury the real ports under junk. We match only what
/// virtual-serial tooling actually creates:
///   * a symlink in `/dev` resolving to a pts node — socat's `PTY,link=/dev/ttyV0` convention,
///     i.e. a path a human deliberately created to BE a serial port;
///   * `tnt*` — tty0tty's kernel-module nodes (the com0com equivalent).
///
/// Takes the directory so it is testable without root or a real virtual port.
#[cfg(all(feature = "serial", target_os = "linux"))]
fn linux_virtual_ports(dev: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dev) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            // tty0tty (and lookalikes): a real char device named tnt0..tnt7.
            if name.starts_with("tnt") {
                return Some(path.to_string_lossy().into_owned());
            }
            // socat-style: a symlink someone made to stand in for a serial port. Resolving to a
            // pts node is what distinguishes it from the many other symlinks in /dev.
            let meta = std::fs::symlink_metadata(&path).ok()?;
            if !meta.file_type().is_symlink() {
                return None;
            }
            let target = std::fs::read_link(&path).ok()?;
            target
                .to_string_lossy()
                .contains("pts/")
                .then(|| path.to_string_lossy().into_owned())
        })
        .collect();
    out.sort(); // stable order — the picker must not reshuffle between polls
    out
}

/// Names of the serial ports currently present.
///
/// Without the `serial` feature there is no enumeration backend, so this
/// returns an empty `Vec`; the operator can still type a port name manually.
#[cfg(not(feature = "serial"))]
pub fn available_ports() -> Vec<String> {
    Vec::new()
}

/// A USB serial port plus the descriptor fields zero-config setup reads to identify
/// the radio (model from `product`) and the bridge chip / driver (from `vid`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbPort {
    pub port_name: String,
    pub vid: u16,
    pub pid: u16,
    pub product: String,
    pub manufacturer: String,
}

/// USB serial ports currently present, with their USB descriptor fields. Non-USB
/// ports (legacy RS-232, Bluetooth SPP, …) are omitted — zero-config only reasons
/// about USB. Empty without the `serial` feature or if enumeration fails.
#[cfg(feature = "serial")]
pub fn available_usb_ports() -> Vec<UsbPort> {
    use serialport::SerialPortType;
    match serialport::available_ports() {
        Ok(ports) => ports
            .into_iter()
            .filter_map(|p| match p.port_type {
                SerialPortType::UsbPort(info) => Some(UsbPort {
                    port_name: p.port_name,
                    vid: info.vid,
                    pid: info.pid,
                    product: info.product.unwrap_or_default(),
                    manufacturer: info.manufacturer.unwrap_or_default(),
                }),
                _ => None,
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// USB serial ports — empty without the `serial` feature (no enumeration backend).
#[cfg(not(feature = "serial"))]
pub fn available_usb_ports() -> Vec<UsbPort> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "serial", target_os = "linux"))]
    #[test]
    fn linux_virtual_ports_finds_the_real_ones_and_ignores_terminals() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("nexus-devscan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pts = dir.join("pts");
        std::fs::create_dir_all(&pts).unwrap();

        // Terminal sessions — every open shell is one of these. They must NEVER be listed:
        // burying the operator's real ports under a dozen tty sessions is worse than the bug.
        for n in ["0", "1", "2"] {
            std::fs::write(pts.join(n), b"").unwrap();
        }
        // A socat-style virtual port: a symlink a human deliberately created to BE a port.
        symlink("pts/2", dir.join("ttyV0")).unwrap();
        // tty0tty's kernel node.
        std::fs::write(dir.join("tnt0"), b"").unwrap();
        // Decoys that must not match: a plain file, and a symlink pointing somewhere else.
        std::fs::write(dir.join("null"), b"").unwrap();
        symlink("../tmp", dir.join("shm")).unwrap();

        let found = linux_virtual_ports(&dir);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.rsplit('/').next().unwrap().to_string())
            .collect();

        assert!(
            names.contains(&"ttyV0".to_string()),
            "socat PTY link: {names:?}"
        );
        assert!(
            names.contains(&"tnt0".to_string()),
            "tty0tty node: {names:?}"
        );
        assert_eq!(names.len(), 2, "nothing else may be listed: {names:?}");
        assert!(
            !names.iter().any(|n| n == "0" || n == "1" || n == "2"),
            "raw pts terminal sessions must never reach the port picker: {names:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(all(feature = "serial", target_os = "linux"))]
    #[test]
    fn linux_virtual_ports_is_quiet_when_dev_is_unreadable() {
        // A missing/unreadable /dev must yield an empty list, never a panic — this runs every
        // time the Settings tab opens.
        assert!(linux_virtual_ports(std::path::Path::new("/nonexistent-xyz")).is_empty());
    }

    #[test]
    fn available_ports_is_callable() {
        // We can't assert hardware is present; just prove the function exists
        // and returns a Vec in either build configuration.
        let _ports: Vec<String> = available_ports();
    }
}
