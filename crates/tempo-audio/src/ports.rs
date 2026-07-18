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
    std::panic::catch_unwind(|| match serialport::available_ports() {
        Ok(ports) => ports.into_iter().map(|p| p.port_name).collect(),
        Err(_) => Vec::new(),
    })
    .unwrap_or_else(|_| {
        // NEVER swallow silently: a per-poll panic here is invisible but costs real
        // CPU (unwind + panic hook each time) — the "sluggish laptop" failure mode.
        // Rate-limited so a storm doesn't also flood stderr.
        use std::sync::atomic::{AtomicU32, Ordering};
        static CAUGHT: AtomicU32 = AtomicU32::new(0);
        let n = CAUGHT.fetch_add(1, Ordering::Relaxed) + 1;
        if n == 1 || n % 100 == 0 {
            eprintln!(
                "nexus: serial-port enumeration panicked (caught; occurrence {n}) — \
                 a driver/udev issue on this system; ports list returned empty"
            );
        }
        Vec::new()
    })
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

    #[test]
    fn available_ports_is_callable() {
        // We can't assert hardware is present; just prove the function exists
        // and returns a Vec in either build configuration.
        let _ports: Vec<String> = available_ports();
    }
}
