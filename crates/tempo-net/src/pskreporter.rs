//! PSK Reporter spot upload — the UDP datagram WSJT-X sends to
//! `report.pskreporter.info:4739`.
//!
//! PSK Reporter ingests an **IPFIX-like** binary format (it predates and loosely
//! follows RFC 5101 / IPFIX). Each datagram is:
//!
//! ```text
//! Header (16 bytes, big-endian):
//!   u16 version          = 0x000A
//!   u16 length           = total datagram byte length
//!   u32 export_time      = Unix seconds
//!   u32 sequence_number  = a counter (we use 1; the server tolerates this)
//!   u32 observation_id   = a random-ish "random identifier" for this sender
//!
//! Then a sequence of "sets", each: u16 set_id, u16 set_length, padded to a
//! 4-byte boundary. Two kinds appear:
//!
//!   * Template/Options-Template sets (set_id 2 / 3) declaring the field layout
//!     of the records that follow. WSJT-X embeds two well-known templates with
//!     fixed ids 0x50E2 (receiver info) and 0x50E3 (sender/spot info).
//!   * Data sets, whose set_id equals a previously declared template id.
//!
//! In practice WSJT-X sends, in one datagram:
//!   1. the two template definitions (so the server is self-describing),
//!   2. a *receiver* data record (our call / grid / software),
//!   3. one or more *sender* data records (each a spotted station).
//!
//! ## Assumptions / notes
//! - Strings are length-prefixed by a single byte (variable-length IE encoding
//!   < 255 bytes), then the UTF-8 bytes; the whole record is padded with NULs to
//!   a 4-byte boundary, as WSJT-X does.
//! - The two template ids `0x50E2`/`0x50E3` and their enterprise-numbered
//!   information elements are the de-facto values from the WSJT-X source
//!   (`PSKReporter.cpp`). The server keys on the *template field list*, so as
//!   long as the declared templates match the data records they parse. We
//!   reproduce the minimal field set WSJT-X uses.
//! - We pin `sequence_number = 1` per datagram. The real server uses it only for
//!   loss detection; a constant is accepted (WSJT-X itself increments, which is
//!   a harmless superset of this behavior).
//! - Spot `time` is Unix seconds (the "flowStartSeconds" IE).
//!
//! Everything here builds bytes; tests assert structure and bind loopback only.

use std::time::{SystemTime, UNIX_EPOCH};

/// Default PSK Reporter ingest endpoint.
pub const DEFAULT_TARGET: &str = "report.pskreporter.info:4739";

/// IPFIX version word.
const IPFIX_VERSION: u16 = 0x000A;
/// Template-set id (a set whose records define a template).
const SET_ID_TEMPLATE: u16 = 0x0002;
/// Options-template-set id.
const SET_ID_OPTIONS_TEMPLATE: u16 = 0x0003;
/// Template id WSJT-X uses for the receiver-information record.
const TEMPLATE_RX: u16 = 0x50E2;
/// Template id WSJT-X uses for each sender (spot) record.
const TEMPLATE_TX: u16 = 0x50E3;

/// One spotted station to report.
#[derive(Debug, Clone)]
pub struct Spot {
    /// The call sign we copied.
    pub call: String,
    /// Frequency in Hz the station was heard on.
    pub freq_hz: u64,
    /// Signal report (dB), as decoded.
    pub snr: i32,
    /// Mode string, e.g. "FT8".
    pub mode: String,
    /// Reception time, Unix seconds (UTC).
    pub time_secs: u32,
}

/// Builds and sends PSK Reporter datagrams.
pub struct PskReporter {
    target: String,
    /// Per-sender random identifier echoed in every datagram header.
    observation_id: u32,
}

impl PskReporter {
    /// New reporter sending to [`DEFAULT_TARGET`].
    pub fn new() -> Self {
        Self::with_target(DEFAULT_TARGET)
    }

    /// New reporter sending to a custom `host:port`.
    pub fn with_target(target: &str) -> Self {
        Self {
            target: target.to_string(),
            observation_id: derive_observation_id(),
        }
    }

    /// The configured target address string.
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Build the datagram bytes for a batch of spots without sending. Exposed so
    /// the structure can be unit-tested and so callers can inspect/log it.
    ///
    /// Returns `None` if there are no spots (PSK Reporter expects at least one
    /// sender record).
    pub fn build_datagram(
        &self,
        rx_call: &str,
        rx_grid: &str,
        software: &str,
        spots: &[Spot],
        export_time: u32,
    ) -> Option<Vec<u8>> {
        if spots.is_empty() {
            return None;
        }

        let mut body = Vec::new();
        // 1) Template declarations so the datagram is self-describing.
        body.extend_from_slice(&rx_template_set());
        body.extend_from_slice(&tx_template_set());
        // 2) The receiver record.
        body.extend_from_slice(&rx_data_set(rx_call, rx_grid, software));
        // 3) The sender (spot) records, all under one data set.
        body.extend_from_slice(&tx_data_set(spots));

        let total_len = 16 + body.len();
        let mut dgram = Vec::with_capacity(total_len);
        dgram.extend_from_slice(&IPFIX_VERSION.to_be_bytes());
        dgram.extend_from_slice(&(total_len as u16).to_be_bytes());
        dgram.extend_from_slice(&export_time.to_be_bytes());
        dgram.extend_from_slice(&1u32.to_be_bytes()); // sequence number
        dgram.extend_from_slice(&self.observation_id.to_be_bytes());
        dgram.extend_from_slice(&body);
        Some(dgram)
    }

    /// Build and send the spot datagram for `spots` over UDP.
    ///
    /// Uses the current system time as the export time. Returns the number of
    /// bytes sent, or an error. Does nothing (returns `Ok(0)`) if `spots` is
    /// empty.
    pub fn send_spots(
        &self,
        rx_call: &str,
        rx_grid: &str,
        software: &str,
        spots: &[Spot],
    ) -> std::io::Result<usize> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(0);
        let Some(dgram) = self.build_datagram(rx_call, rx_grid, software, spots, now) else {
            return Ok(0);
        };
        let socket = std::net::UdpSocket::bind("0.0.0.0:0")?;
        socket.send_to(&dgram, &self.target)
    }
}

impl Default for PskReporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Round `n` up to the next multiple of 4 (IPFIX sets/records are 4-byte aligned).
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Pad `buf` with NULs to a 4-byte boundary.
fn pad4(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

/// Append a single-byte-length-prefixed string (PSK Reporter variable-length IE).
fn push_str(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    let len = bytes.len().min(254);
    buf.push(len as u8);
    buf.extend_from_slice(&bytes[..len]);
}

/// Wrap record `body` in a data set with `set_id`, padding to 4 bytes.
fn data_set(set_id: u16, body: &[u8]) -> Vec<u8> {
    let set_len = align4(4 + body.len());
    let mut s = Vec::with_capacity(set_len);
    s.extend_from_slice(&set_id.to_be_bytes());
    s.extend_from_slice(&(set_len as u16).to_be_bytes());
    s.extend_from_slice(body);
    pad4(&mut s);
    s
}

/// The receiver-information template definition (a Template/Options set).
///
/// Declares: receiverCallsign, receiverLocator, decodingSoftware — the three
/// strings in the receiver data record. Field ids use PSK Reporter's enterprise
/// number (30351). The exact IE ids mirror WSJT-X; the server keys on the field
/// *list*, so the declaration and data record stay in lockstep.
fn rx_template_set() -> Vec<u8> {
    const ENTERPRISE: u32 = 30351;
    // (information-element id, length=0xFFFF means variable-length string)
    let fields: [(u16, u16); 3] = [
        (0x8002, 0xFFFF), // receiverCallsign
        (0x8004, 0xFFFF), // receiverLocator
        (0x8008, 0xFFFF), // decodingSoftware
    ];
    options_template_set(TEMPLATE_RX, ENTERPRISE, &fields, /*scope*/ 0)
}

/// The sender/spot template definition.
///
/// Declares: senderCallsign, frequency, snr, mode, flowStartSeconds.
fn tx_template_set() -> Vec<u8> {
    const ENTERPRISE: u32 = 30351;
    // For standard IEs (no enterprise bit) we still emit enterprise ids to keep
    // a single code path; PSK Reporter accepts the WSJT-X layout.
    let fields: [(u16, u16); 5] = [
        (0x8001, 0xFFFF), // senderCallsign (variable string)
        (0x8005, 4),      // frequency (u32 Hz; WSJT-X uses up to 8, see note)
        (0x8006, 1),      // snr (i8)
        (0x800A, 0xFFFF), // mode (variable string; 0x8007 is iMD, NOT mode)
        (150, 4),         // flowStartSeconds (standard IE 150, u32)
    ];
    plain_template_set(TEMPLATE_TX, ENTERPRISE, &fields)
}

/// Encode one template field. A field with an enterprise number sets the high
/// bit of the IE id and appends the 4-byte enterprise number.
fn push_template_field(buf: &mut Vec<u8>, ie_id: u16, len: u16, enterprise: u32) {
    if ie_id & 0x8000 != 0 {
        buf.extend_from_slice(&ie_id.to_be_bytes());
        buf.extend_from_slice(&len.to_be_bytes());
        buf.extend_from_slice(&enterprise.to_be_bytes());
    } else {
        buf.extend_from_slice(&ie_id.to_be_bytes());
        buf.extend_from_slice(&len.to_be_bytes());
    }
}

/// Build a plain Template set (set id 2) declaring `fields` under `template_id`.
fn plain_template_set(template_id: u16, enterprise: u32, fields: &[(u16, u16)]) -> Vec<u8> {
    let mut rec = Vec::new();
    rec.extend_from_slice(&template_id.to_be_bytes());
    rec.extend_from_slice(&(fields.len() as u16).to_be_bytes());
    for &(ie, len) in fields {
        push_template_field(&mut rec, ie, len, enterprise);
    }
    finish_set(SET_ID_TEMPLATE, rec)
}

/// Build an Options-Template set (set id 3) declaring `fields` under
/// `template_id`, with `scope` leading scope fields.
fn options_template_set(
    template_id: u16,
    enterprise: u32,
    fields: &[(u16, u16)],
    scope: u16,
) -> Vec<u8> {
    let mut rec = Vec::new();
    rec.extend_from_slice(&template_id.to_be_bytes());
    rec.extend_from_slice(&(fields.len() as u16).to_be_bytes());
    rec.extend_from_slice(&scope.to_be_bytes());
    for &(ie, len) in fields {
        push_template_field(&mut rec, ie, len, enterprise);
    }
    finish_set(SET_ID_OPTIONS_TEMPLATE, rec)
}

/// Wrap a template-record body in its set header (set id + length) and pad.
fn finish_set(set_id: u16, body: Vec<u8>) -> Vec<u8> {
    let set_len = align4(4 + body.len());
    let mut s = Vec::with_capacity(set_len);
    s.extend_from_slice(&set_id.to_be_bytes());
    s.extend_from_slice(&(set_len as u16).to_be_bytes());
    s.extend_from_slice(&body);
    pad4(&mut s);
    s
}

/// The receiver data record (our call/grid/software), as a data set keyed to
/// the receiver template id.
fn rx_data_set(rx_call: &str, rx_grid: &str, software: &str) -> Vec<u8> {
    let mut rec = Vec::new();
    push_str(&mut rec, rx_call);
    push_str(&mut rec, rx_grid);
    push_str(&mut rec, software);
    data_set(TEMPLATE_RX, &rec)
}

/// All spot records as a single data set keyed to the sender template id.
fn tx_data_set(spots: &[Spot]) -> Vec<u8> {
    let mut rec = Vec::new();
    for spot in spots {
        push_str(&mut rec, &spot.call);
        rec.extend_from_slice(&(spot.freq_hz as u32).to_be_bytes());
        rec.push(clamp_i8(spot.snr) as u8);
        push_str(&mut rec, &spot.mode);
        rec.extend_from_slice(&spot.time_secs.to_be_bytes());
    }
    data_set(TEMPLATE_TX, &rec)
}

/// Clamp an SNR into the i8 range PSK Reporter expects.
fn clamp_i8(v: i32) -> i8 {
    v.clamp(i8::MIN as i32, i8::MAX as i32) as i8
}

/// Derive a stable-ish per-process random identifier from the wall clock. PSK
/// Reporter only needs it to be reasonably unique per sender; any 32-bit value
/// works.
fn derive_observation_id() -> u32 {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // Mix the low/high words so we don't just get the seconds.
    (n as u32) ^ ((n >> 32) as u32) | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spots() -> Vec<Spot> {
        vec![
            Spot {
                call: "W1AW".to_string(),
                freq_hz: 14_074_000,
                snr: -7,
                mode: "FT8".to_string(),
                time_secs: 1_577_836_800,
            },
            Spot {
                call: "VE3ABC".to_string(),
                freq_hz: 14_074_500,
                snr: -15,
                mode: "FT8".to_string(),
                time_secs: 1_577_836_815,
            },
        ]
    }

    fn read_u16(b: &[u8], off: usize) -> u16 {
        u16::from_be_bytes([b[off], b[off + 1]])
    }
    fn read_u32(b: &[u8], off: usize) -> u32 {
        u32::from_be_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
    }

    #[test]
    fn tx_template_declares_the_pskreporter_mode_field() {
        // Field ids per PSK Reporter's registry (enterprise 30351), as used by
        // WSJT-X's PSKReporter.cpp: 1=senderCallsign, 5=frequency, 6=sNR,
        // 10=mode, plus standard IE 150=flowStartSeconds. Field 7 is iMD — a
        // PSK31 distortion metric; declaring the mode string there leaves the
        // report modeless and PSK Reporter displays its default (PSK31).
        let set = tx_template_set();
        assert_eq!(read_u16(&set, 0), SET_ID_TEMPLATE);
        assert_eq!(read_u16(&set, 4), TEMPLATE_TX);
        let n = read_u16(&set, 6) as usize;
        let mut ids = Vec::new();
        let mut off = 8;
        for _ in 0..n {
            let ie = read_u16(&set, off);
            ids.push(ie);
            off += 4; // ie id + length
            if ie & 0x8000 != 0 {
                off += 4; // enterprise number
            }
        }
        assert_eq!(ids, vec![0x8001, 0x8005, 0x8006, 0x800A, 150]);
    }

    #[test]
    fn empty_spots_produce_no_datagram() {
        let r = PskReporter::new();
        assert!(r
            .build_datagram("KD9TAW", "EN52", "Tempo 0.1", &[], 0)
            .is_none());
    }

    #[test]
    fn header_is_well_formed() {
        let r = PskReporter::with_target("127.0.0.1:4739");
        let dgram = r
            .build_datagram(
                "KD9TAW",
                "EN52",
                "Tempo 0.1",
                &sample_spots(),
                1_577_836_900,
            )
            .unwrap();

        assert_eq!(read_u16(&dgram, 0), IPFIX_VERSION);
        // Length field equals the actual datagram length.
        assert_eq!(read_u16(&dgram, 2) as usize, dgram.len());
        // Export time we passed in.
        assert_eq!(read_u32(&dgram, 4), 1_577_836_900);
        // Sequence number pinned to 1.
        assert_eq!(read_u32(&dgram, 8), 1);
        // Observation id is non-zero (derive_observation_id ORs in 1).
        assert_ne!(read_u32(&dgram, 12), 0);
        // Whole datagram is 4-byte aligned.
        assert_eq!(dgram.len() % 4, 0);
    }

    #[test]
    fn contains_both_templates_and_two_data_sets() {
        let r = PskReporter::new();
        let dgram = r
            .build_datagram("KD9TAW", "EN52", "Tempo", &sample_spots(), 100)
            .unwrap();

        // Walk the sets after the 16-byte header, collecting their ids.
        let mut off = 16;
        let mut set_ids = Vec::new();
        while off + 4 <= dgram.len() {
            let set_id = read_u16(&dgram, off);
            let set_len = read_u16(&dgram, off + 2) as usize;
            assert!(set_len >= 4, "set length must include the 4-byte header");
            assert_eq!(set_len % 4, 0, "every set is 4-byte aligned");
            set_ids.push(set_id);
            off += set_len;
        }
        assert_eq!(off, dgram.len(), "sets exactly tile the datagram");

        // Two template-definition sets (ids 2 and 3) then two data sets keyed to
        // the receiver and sender template ids.
        assert!(set_ids.contains(&SET_ID_OPTIONS_TEMPLATE)); // rx template
        assert!(set_ids.contains(&SET_ID_TEMPLATE)); // tx template
        assert!(set_ids.contains(&TEMPLATE_RX)); // rx data set
        assert!(set_ids.contains(&TEMPLATE_TX)); // tx data set
    }

    #[test]
    fn rx_data_set_carries_call_grid_software() {
        let r = PskReporter::new();
        let dgram = r
            .build_datagram("KD9TAW", "EN52", "Tempo 0.1", &sample_spots(), 100)
            .unwrap();
        // The receiver data set is keyed to TEMPLATE_RX; find it and check the
        // length-prefixed strings appear in order.
        let bytes = find_data_set(&dgram, TEMPLATE_RX);
        let body = &bytes[4..]; // strip set header
        let mut off = 0;
        assert_eq!(read_str(body, &mut off), "KD9TAW");
        assert_eq!(read_str(body, &mut off), "EN52");
        assert_eq!(read_str(body, &mut off), "Tempo 0.1");
    }

    #[test]
    fn tx_data_set_carries_each_spot() {
        let r = PskReporter::new();
        let spots = sample_spots();
        let dgram = r
            .build_datagram("KD9TAW", "EN52", "Tempo", &spots, 100)
            .unwrap();
        let bytes = find_data_set(&dgram, TEMPLATE_TX);
        let body = &bytes[4..];
        let mut off = 0;
        // First spot.
        assert_eq!(read_str(body, &mut off), "W1AW");
        assert_eq!(read_u32(body, off), 14_074_000);
        off += 4;
        assert_eq!(body[off] as i8, -7);
        off += 1;
        assert_eq!(read_str(body, &mut off), "FT8");
        assert_eq!(read_u32(body, off), 1_577_836_800);
        off += 4;
        // Second spot.
        assert_eq!(read_str(body, &mut off), "VE3ABC");
        assert_eq!(read_u32(body, off), 14_074_500);
        off += 4;
        assert_eq!(body[off] as i8, -15);
    }

    #[test]
    fn send_spots_over_loopback_succeeds() {
        // Bind a loopback listener and send to it (no real network egress).
        let listener = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        listener
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let r = PskReporter::with_target(&addr.to_string());
        let n = r
            .send_spots("KD9TAW", "EN52", "Tempo", &sample_spots())
            .unwrap();
        assert!(n > 16);

        let mut buf = [0u8; 4096];
        let (recv_n, _) = listener.recv_from(&mut buf).unwrap();
        assert_eq!(recv_n, n);
        assert_eq!(read_u16(&buf, 0), IPFIX_VERSION);
        assert_eq!(read_u16(&buf, 2) as usize, recv_n);
    }

    #[test]
    fn send_spots_empty_is_noop() {
        let r = PskReporter::with_target("127.0.0.1:1"); // unused
        assert_eq!(r.send_spots("KD9TAW", "EN52", "Tempo", &[]).unwrap(), 0);
    }

    // --- test helpers ---

    /// Find the data set whose set id is `template_id`, returning its bytes
    /// (including the 4-byte set header).
    fn find_data_set(dgram: &[u8], template_id: u16) -> Vec<u8> {
        let mut off = 16;
        while off + 4 <= dgram.len() {
            let set_id = read_u16(dgram, off);
            let set_len = read_u16(dgram, off + 2) as usize;
            if set_id == template_id {
                return dgram[off..off + set_len].to_vec();
            }
            off += set_len;
        }
        panic!("data set {template_id:#06x} not found");
    }

    /// Read a single-byte-length-prefixed string at `*off`, advancing it.
    fn read_str(body: &[u8], off: &mut usize) -> String {
        let len = body[*off] as usize;
        *off += 1;
        let s = String::from_utf8_lossy(&body[*off..*off + len]).into_owned();
        *off += len;
        s
    }
}
