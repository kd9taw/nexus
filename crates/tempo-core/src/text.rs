//! Free-text chat over FT1: word-wrapped chunking + reassembly.
//!
//! FT1's free-text frame holds ~13 characters from the WSJT-X alphabet
//! (`0-9 A-Z space + - . / ?`, case-insensitive, uppercased on encode) and
//! carries **no callsign**. To send arbitrary-length messages, Tempo splits text
//! into chunks framed as `<id><seq><tot><payload>`:
//!   - `id`  : 'A'..'Z' — message id within a session
//!   - `seq` : 1..9 — chunk number
//!   - `tot` : 1..9 — total chunks
//!   - payload: up to [`PAYLOAD`] chars
//!
//! Chunks are **word-wrapped** so a chunk never begins or ends with a space —
//! this avoids the modem trimming boundary spaces. Reassembly rejoins chunks
//! with single spaces. (Multiple/awkward spacing is normalized — fine for
//! human messages; the trade for reliability on a 13-char substrate.)

use std::collections::{BTreeMap, HashMap};

/// Max characters in a free-text frame (conservative; alpha-heavy limit).
pub const FREETEXT_MAX: usize = 13;
/// Chunk header length (`id` + `seq` + `tot`).
pub const HEADER: usize = 3;
/// Max payload chars per chunk.
pub const PAYLOAD: usize = FREETEXT_MAX - HEADER; // 10
/// Max chunks per message (seq/tot are single digits).
pub const MAX_CHUNKS: usize = 9;

const ALLOWED: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ +-./?";

/// Uppercase and restrict to the FT1 free-text charset (unsupported → '?').
pub fn sanitize(s: &str) -> String {
    s.to_uppercase()
        .chars()
        .map(|c| if ALLOWED.contains(c) { c } else { '?' })
        .collect()
}

/// Normalize whitespace the way reassembly will (single spaces, trimmed).
pub fn normalize(s: &str) -> String {
    sanitize(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split a message into Tempo free-text chunk frames. `id` should be 'A'..'Z'.
pub fn chunk(msg: &str, id: char) -> Vec<String> {
    let s = sanitize(msg);
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();

    for word in s.split_whitespace() {
        // Hard-split words longer than the payload budget.
        let mut chars: Vec<char> = word.chars().collect();
        while chars.len() > PAYLOAD {
            if !cur.is_empty() {
                chunks.push(std::mem::take(&mut cur));
            }
            chunks.push(chars[..PAYLOAD].iter().collect());
            chars.drain(..PAYLOAD);
        }
        let w: String = chars.iter().collect();
        if w.is_empty() {
            continue;
        }
        if cur.is_empty() {
            cur = w;
        } else if cur.chars().count() + 1 + w.chars().count() <= PAYLOAD {
            cur.push(' ');
            cur.push_str(&w);
        } else {
            chunks.push(std::mem::take(&mut cur));
            cur = w;
        }
    }
    if !cur.is_empty() {
        chunks.push(cur);
    }
    if chunks.is_empty() {
        chunks.push(String::new());
    }
    if chunks.len() > MAX_CHUNKS {
        chunks.truncate(MAX_CHUNKS);
    }

    let tot = chunks.len();
    chunks
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}{}{}{}", id, i + 1, tot, p))
        .collect()
}

/// If `frame` is a Tempo text chunk, return `(id, seq, total, payload)`.
pub fn parse_chunk(frame: &str) -> Option<(char, usize, usize, String)> {
    let cs: Vec<char> = frame.chars().collect();
    if cs.len() < HEADER {
        return None;
    }
    let id = cs[0];
    if !id.is_ascii_uppercase() {
        return None;
    }
    let seq = cs[1].to_digit(10)? as usize;
    let tot = cs[2].to_digit(10)? as usize;
    if seq < 1 || tot < 1 || seq > tot || tot > MAX_CHUNKS {
        return None;
    }
    Some((id, seq, tot, cs[HEADER..].iter().collect()))
}

/// Accumulates chunk frames and yields complete messages.
#[derive(Debug, Default)]
pub struct Reassembler {
    buffers: HashMap<char, (usize, BTreeMap<usize, String>)>,
}

impl Reassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a frame. Returns `Some(message)` when a chunk set is complete,
    /// `None` if the frame is not a chunk or the message is still partial.
    pub fn accept(&mut self, frame: &str) -> Option<String> {
        let (id, seq, tot, payload) = parse_chunk(frame)?;
        let entry = self.buffers.entry(id).or_insert((tot, BTreeMap::new()));
        entry.0 = tot;
        entry.1.insert(seq, payload);
        if entry.1.len() == tot {
            let (_, parts) = self.buffers.remove(&id).unwrap();
            let msg = parts.into_values().collect::<Vec<_>>().join(" ");
            Some(msg)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_uppercases_and_filters() {
        assert_eq!(sanitize("hello, world!"), "HELLO? WORLD?");
        assert_eq!(sanitize("MSG/01 ok-go."), "MSG/01 OK-GO.");
    }

    #[test]
    fn chunks_fit_frame_budget_and_have_no_boundary_spaces() {
        let frames = chunk("HELLO TEMPO THIS IS A LONGER TEST MESSAGE 73", 'A');
        assert!(frames.len() > 1);
        for f in &frames {
            assert!(f.chars().count() <= FREETEXT_MAX, "frame too long: {f}");
            let (_, _, _, payload) = parse_chunk(f).expect("valid chunk");
            assert!(
                !payload.starts_with(' ') && !payload.ends_with(' '),
                "boundary space in {f}"
            );
        }
    }

    #[test]
    fn chunk_then_reassemble_roundtrips() {
        let msg = "HELLO TEMPO THIS IS A LONGER TEST MESSAGE 73";
        let frames = chunk(msg, 'A');
        let mut r = Reassembler::new();
        let mut out = None;
        for f in &frames {
            if let Some(full) = r.accept(f) {
                out = Some(full);
            }
        }
        assert_eq!(out.as_deref(), Some(normalize(msg).as_str()));
    }

    #[test]
    fn reassembles_out_of_order() {
        let frames = chunk("ONE TWO THREE FOUR FIVE SIX SEVEN", 'B');
        let mut r = Reassembler::new();
        let mut out = None;
        for f in frames.iter().rev() {
            if let Some(full) = r.accept(f) {
                out = Some(full);
            }
        }
        assert_eq!(out.as_deref(), Some("ONE TWO THREE FOUR FIVE SIX SEVEN"));
    }

    #[test]
    fn non_chunk_frames_rejected() {
        assert!(parse_chunk("HELLO WORLD").is_none());
        assert!(parse_chunk("CQ W9XYZ EN37").is_none());
        assert!(parse_chunk("A12HELLO").is_some());
    }
}
