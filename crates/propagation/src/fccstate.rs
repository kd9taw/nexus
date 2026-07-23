//! Callsign → US-state index (FCC ULS-derived) for the WAS "New State" hint.
//!
//! Grid→state ([`crate::gridstate`]) only works for the two sources that carry a grid (FT8/FT4
//! decodes + PSK Reporter), and even there a 4-char grid is a 2°×1° cell that straddles state
//! lines. A DX-cluster / CW / SSB spot carries NO grid at all — only a callsign. This index
//! answers callsign → state directly (from the FCC licensee's address state), so New-State can
//! light up across the WHOLE spot firehose, and precisely (no border cell to guess).
//!
//! It is a HINT (the licensee's mailing state — usually but not always where they operate, so a
//! rover/portable can be off; a live decode grid refines that). Actual WAS credit still comes
//! from the confirmed QSO's logged ADIF `STATE`.
//!
//! ## File format (`fcc-states.bin`, downloaded — never bundled; ~5 MB for ~750k US hams)
//! - 16-byte header: magic `b"NEXFCCS1"` (8) · `count: u32 LE` (4) · reserved (4).
//! - `count` entries, each 7 bytes, **sorted ascending by callsign** for binary search:
//!   callsign (6 bytes, ASCII uppercase, space-padded — US calls are ≤ 6 chars) · state byte
//!   (`1..=50` → [`crate::awards::WAS_STATES`]`[byte-1]`).

use crate::awards::WAS_STATES;

const MAGIC: &[u8; 8] = b"NEXFCCS1";
const CALL_LEN: usize = 6;
const ENTRY_LEN: usize = CALL_LEN + 1;
const HEADER_LEN: usize = 16;

/// A loaded callsign→state index. Holds the raw bytes; lookups binary-search in place.
pub struct FccStates {
    data: Vec<u8>,
    count: usize,
}

impl FccStates {
    /// Parse a downloaded `fcc-states.bin`. Validates magic + length; `None` on a corrupt or
    /// foreign file, so the caller keeps its last-good index rather than crashing.
    pub fn load(data: Vec<u8>) -> Option<Self> {
        if data.len() < HEADER_LEN || &data[0..8] != MAGIC {
            return None;
        }
        let count = u32::from_le_bytes(data[8..12].try_into().ok()?) as usize;
        if data.len() < HEADER_LEN + count * ENTRY_LEN {
            return None;
        }
        Some(Self { data, count })
    }

    /// The 6-byte lookup key for a callsign: the base call (longest `/`-separated alphanumeric
    /// token — drops `W1/`, `/P`, `/M`, `/QRP`), uppercased, space-padded. `None` if it can't be
    /// a US call (empty or > 6 chars).
    fn call_key(raw: &str) -> Option<[u8; CALL_LEN]> {
        let base = raw
            .split('/')
            .map(|t| {
                t.chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect::<String>()
            })
            .max_by_key(|t| t.len())?
            .to_ascii_uppercase();
        if base.is_empty() || base.len() > CALL_LEN {
            return None;
        }
        let mut key = [b' '; CALL_LEN];
        key[..base.len()].copy_from_slice(base.as_bytes());
        Some(key)
    }

    /// Best-guess US state (2-letter WAS code) for a callsign, or `None` (non-US / not on file).
    pub fn state_for_call(&self, call: &str) -> Option<&'static str> {
        let key = Self::call_key(call)?;
        let entries = &self.data[HEADER_LEN..HEADER_LEN + self.count * ENTRY_LEN];
        let (mut lo, mut hi) = (0usize, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let call_bytes = &entries[mid * ENTRY_LEN..mid * ENTRY_LEN + CALL_LEN];
            match call_bytes.cmp(&key[..]) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    let code = entries[mid * ENTRY_LEN + CALL_LEN] as usize;
                    return WAS_STATES.get(code.wrapping_sub(1)).copied();
                }
            }
        }
        None
    }

    pub fn count(&self) -> usize {
        self.count
    }
}

/// Build a `fcc-states.bin` blob from `(callsign, state-code 1..=50)` pairs — the shared builder
/// used by the offline generator AND the tests. Skips invalid calls / out-of-range state codes,
/// dedups by call key (last wins), and emits the header + callsign-sorted entries.
pub fn build_index(pairs: &[(String, u8)]) -> Vec<u8> {
    let mut keyed: std::collections::BTreeMap<[u8; CALL_LEN], u8> = std::collections::BTreeMap::new();
    for (call, code) in pairs {
        if *code < 1 || *code as usize > WAS_STATES.len() {
            continue;
        }
        if let Some(k) = FccStates::call_key(call) {
            keyed.insert(k, *code); // BTreeMap keeps callsign order; last write wins
        }
    }
    let mut out = Vec::with_capacity(HEADER_LEN + keyed.len() * ENTRY_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(keyed.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]); // reserved
    for (key, code) in keyed {
        out.extend_from_slice(&key);
        out.push(code);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(st: &str) -> u8 {
        (WAS_STATES.iter().position(|s| *s == st).unwrap() + 1) as u8
    }

    #[test]
    fn round_trips_and_looks_up_by_base_call() {
        let idx = build_index(&[
            ("W1AW".into(), code("CT")),
            ("KD9TAW".into(), code("IL")),
            ("K7ABC".into(), code("AZ")),
        ]);
        let db = FccStates::load(idx).unwrap();
        assert_eq!(db.count(), 3);
        assert_eq!(db.state_for_call("W1AW"), Some("CT"));
        assert_eq!(db.state_for_call("kd9taw"), Some("IL")); // case-insensitive
        // Portable prefixes/suffixes resolve to the base call's home state (the licensed state).
        assert_eq!(db.state_for_call("KD9TAW/9"), Some("IL"));
        assert_eq!(db.state_for_call("W4/KD9TAW"), Some("IL"));
        assert_eq!(db.state_for_call("K7ABC/QRP"), Some("AZ"));
        // Not on file → None (a brand-new or non-US call; the grid hint would cover it).
        assert_eq!(db.state_for_call("DL1ABC"), None);
        assert_eq!(db.state_for_call("N0SUCH"), None);
    }

    #[test]
    fn rejects_corrupt_or_foreign_blobs() {
        assert!(FccStates::load(vec![]).is_none());
        assert!(FccStates::load(b"NOTNEXUS........".to_vec()).is_none());
        // Right magic but a count that overruns the buffer → rejected, not a panic.
        let mut bad = MAGIC.to_vec();
        bad.extend_from_slice(&9999u32.to_le_bytes());
        bad.extend_from_slice(&[0u8; 4]);
        assert!(FccStates::load(bad).is_none());
    }

    #[test]
    fn build_skips_junk_and_keeps_calls_sorted() {
        let idx = build_index(&[
            ("K7ABC".into(), code("AZ")),
            ("W1AW".into(), code("CT")),
            ("TOOLONGCALL".into(), code("TX")), // > 6 chars → skipped
            ("BADSTATE".into(), 200),           // out-of-range code → skipped
        ]);
        let db = FccStates::load(idx).unwrap();
        assert_eq!(db.count(), 2, "the two junk rows are dropped");
        // Both survivors resolve; the internal order is callsign-sorted (K7ABC < W1AW).
        assert_eq!(db.state_for_call("W1AW"), Some("CT"));
        assert_eq!(db.state_for_call("K7ABC"), Some("AZ"));
    }
}
