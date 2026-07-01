//! Parse a live CW decode transcript into operating context — the backend of the "CW
//! copilot" that lets an operator who can't read Morse still work CW. Pulls the other
//! station's callsign (candidates, ranked) and exchange (RST + name) out of the decoded
//! text, and infers the QSO state to drive the guided next-key prompt.
//!
//! Pure + deterministic. Callsign FORMAT validation uses [`crate::message::is_callsign`];
//! whether a prefix is a REAL DXCC prefix is supplied by the caller as a closure, so this
//! crate stays free of the DXCC tables (which live in the `propagation` crate). CW decode
//! is imperfect, so callers CONFIRM a candidate before it's used — nothing here transmits.

use crate::message::{base_call, is_callsign, same_call};

/// A candidate worked-station callsign pulled from the decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallCandidate {
    /// Base call, uppercased (portable affixes stripped for display/matching).
    pub call: String,
    /// Confidence score (higher = more trustworthy); ranks the chips.
    pub score: u32,
    /// The top pick — pre-highlighted, but the operator still confirms.
    pub best: bool,
}

/// Best-effort exchange fields read from the decode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CwExchange {
    /// Signal report they sent us, e.g. "599" (cut numbers decoded: N→9, T→0).
    pub rst: Option<String>,
    /// Their name, e.g. "BOB" (after NAME/OP).
    pub name: Option<String>,
}

/// Guided-mode read of the QSO: a machine tag, plain-English state, an instruction, and
/// the single recommended action so the UI can highlight one key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CwGuidance {
    /// Machine tag: "listening" | "cq" | "answered" | "report" | "73".
    pub state: String,
    /// Plain English, e.g. "W1ABC is calling CQ".
    pub headline: String,
    /// What to do, e.g. "Press Answer (F2) to call them".
    pub prompt: String,
    /// Recommended action id: "F2" | "F3" | "log", or None.
    pub recommended: Option<String>,
}

/// The full copilot read of the current decode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CwAssist {
    pub candidates: Vec<CallCandidate>,
    pub exchange: CwExchange,
    pub guidance: CwGuidance,
}

/// How many trailing whitespace-tokens of the transcript to analyze — enough to cover the
/// current over/exchange without letting an earlier QSO in the scrollback dominate.
const TAIL_TOKENS: usize = 60;

/// Decode a CW "cut number" report token to plain digits: T→0, N→9 (the common ones), and
/// pass digits through. Returns None if it isn't a report-shaped token (3 report digits).
fn decut_rst(tok: &str) -> Option<String> {
    let digits: String = tok
        .chars()
        .map(|c| match c {
            'T' => '0',
            'N' => '9',
            d => d,
        })
        .collect();
    // A signal report is three digits (RST), e.g. 599 / 579. Accept exactly that.
    if digits.len() == 3 && digits.chars().all(|c| c.is_ascii_digit()) {
        Some(digits)
    } else {
        None
    }
}

/// Analyze a CW decode `transcript` (persistent, from `CwStreamDecoder`) plus what we've
/// `sent` this QSO. `mycall` filters our own call out of the candidates; `worked` is the
/// operator-confirmed station (if any) so the state machine knows we're mid-QSO with them.
/// `is_real_prefix` returns true when a base call resolves to a real DXCC prefix — pass a
/// DXCC resolver from the caller (or `|_| true` in tests / when unavailable).
pub fn analyze(
    transcript: &str,
    sent: &[String],
    mycall: &str,
    worked: Option<&str>,
    is_real_prefix: impl Fn(&str) -> bool,
) -> CwAssist {
    let all: Vec<String> = transcript.split_whitespace().map(up).collect();
    let start = all.len().saturating_sub(TAIL_TOKENS);
    let toks = &all[start..];

    let candidates = call_candidates(toks, mycall, &is_real_prefix);
    let exchange = exchange(toks);
    // The station in focus: the confirmed one, else the best candidate.
    let focus = worked
        .map(base_call)
        .or_else(|| candidates.iter().find(|c| c.best).map(|c| c.call.clone()));
    let guidance = infer_state(toks, sent, mycall, focus.as_deref(), &exchange);

    CwAssist {
        candidates,
        exchange,
        guidance,
    }
}

fn up(s: &str) -> String {
    s.trim().to_ascii_uppercase()
}

/// Rank callsign candidates from the recent tokens. A token right after `DE` is the sender
/// (strongest cue); repetition + a real DXCC prefix raise confidence; our own call and
/// bare non-calls are dropped.
fn call_candidates(
    toks: &[String],
    mycall: &str,
    is_real_prefix: &impl Fn(&str) -> bool,
) -> Vec<CallCandidate> {
    use std::collections::HashMap;
    let mine = base_call(mycall);
    let mut scores: HashMap<String, u32> = HashMap::new();
    for (i, t) in toks.iter().enumerate() {
        if !is_callsign(t) {
            continue;
        }
        let b = base_call(t);
        if b == mine || b.is_empty() {
            continue; // never our own call
        }
        let mut s = 1; // well-formed call
        if is_real_prefix(&b) {
            s += 4; // resolves to a real DXCC prefix → strongest misdecode filter
        }
        if i > 0 && toks[i - 1] == "DE" {
            s += 2; // the sender, by the "DE <call>" convention
        }
        *scores.entry(b).or_insert(0) += s;
    }
    let mut cands: Vec<CallCandidate> = scores
        .into_iter()
        .map(|(call, score)| CallCandidate {
            call,
            score,
            best: false,
        })
        .collect();
    // Highest score first; ties broken by call for determinism.
    cands.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.call.cmp(&b.call)));
    cands.truncate(5);
    if let Some(first) = cands.first_mut() {
        first.best = true;
    }
    cands
}

/// Pull the RST they sent (after UR/RST, cut-number aware) and their name (after NAME/OP).
fn exchange(toks: &[String]) -> CwExchange {
    let mut ex = CwExchange::default();
    for (i, t) in toks.iter().enumerate() {
        match t.as_str() {
            "UR" | "RST" if ex.rst.is_none() => {
                // The report is the next report-shaped token (599, 5NN, …).
                if let Some(r) = toks.get(i + 1).and_then(|n| decut_rst(n)) {
                    ex.rst = Some(r);
                }
            }
            "NAME" | "OP" if ex.name.is_none() => {
                // A name is the next all-alphabetic token (not a callsign / not "IS").
                if let Some(n) = toks.get(i + 1) {
                    if n.len() >= 2
                        && n.chars().all(|c| c.is_ascii_alphabetic())
                        && !is_callsign(n)
                        && n != "IS"
                    {
                        ex.name = Some(n.clone());
                    }
                }
            }
            _ => {}
        }
    }
    ex
}

/// Infer the QSO state from the recent decode + what we've sent, and recommend the one
/// next key. Heuristic + text-based (the CW analogue of the FT8 sequencer in `qso.rs`).
fn infer_state(
    toks: &[String],
    sent: &[String],
    mycall: &str,
    focus: Option<&str>,
    exchange: &CwExchange,
) -> CwGuidance {
    let Some(call) = focus else {
        return CwGuidance {
            state: "listening".into(),
            headline: "Listening…".into(),
            prompt: "Tune in a CW station — decoded calls will appear as chips to work.".into(),
            recommended: None,
        };
    };
    let has = |w: &str| toks.iter().any(|t| t == w);
    let they_addressed_me = toks.iter().any(|t| same_call(t, mycall));
    let i_sent_anything = sent.iter().any(|s| !s.trim().is_empty());
    let closing = has("73") || has("SK") || has("TU") || has("GB");

    // Closing beats everything: the QSO is wrapping up → log it.
    if closing && i_sent_anything {
        return CwGuidance {
            state: "73".into(),
            headline: format!("{call} is signing 73"),
            prompt: format!("Send 73 (F4) to close, then log {call}."),
            recommended: Some("F4".into()),
        };
    }
    // They sent us a report → acknowledge + finish.
    if exchange.rst.is_some() {
        let rst = exchange.rst.as_deref().unwrap_or("599");
        return CwGuidance {
            state: "report".into(),
            headline: format!("{call} sent you {rst}"),
            prompt: format!("Press 73 (F4) to confirm and finish with {call}."),
            recommended: Some("F4".into()),
        };
    }
    // They came back to your call (answered your CQ, or replied to you).
    if they_addressed_me {
        return CwGuidance {
            state: "answered".into(),
            headline: format!("{call} answered you"),
            prompt: format!("Press Reply (F3) to send {call} your report + name."),
            recommended: Some("F3".into()),
        };
    }
    // They're calling CQ (heard CQ, not addressed to you) → answer them.
    if has("CQ") {
        return CwGuidance {
            state: "cq".into(),
            headline: format!("{call} is calling CQ"),
            prompt: format!("Press Call (F2) to answer {call}."),
            recommended: Some("F2".into()),
        };
    }
    // A call is in focus but no clear cue yet.
    CwGuidance {
        state: "cq".into(),
        headline: format!("Heard {call}"),
        prompt: format!("Press Call (F2) to answer {call}, or wait for more."),
        recommended: Some("F2".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A permissive DXCC stub for tests: real US/common prefixes resolve, gibberish doesn't.
    fn real(base: &str) -> bool {
        matches!(
            base.chars().next(),
            Some('W' | 'K' | 'N' | 'A' | 'V' | 'G' | 'D' | 'J' | 'F')
        )
    }

    #[test]
    fn extracts_the_cq_caller_and_recommends_answer() {
        let a = analyze("CQ CQ DE W1ABC W1ABC K", &[], "KD9TAW", None, real);
        assert!(a.candidates.iter().any(|c| c.call == "W1ABC" && c.best));
        assert_eq!(a.guidance.state, "cq");
        assert!(a.guidance.headline.contains("W1ABC"));
        assert_eq!(a.guidance.recommended.as_deref(), Some("F2"));
    }

    #[test]
    fn de_call_outranks_a_bare_call() {
        // Both are valid calls, but W1ABC follows DE (the sender) → it wins.
        let a = analyze("K5XYZ DE W1ABC", &[], "KD9TAW", None, real);
        assert_eq!(a.candidates.first().unwrap().call, "W1ABC");
    }

    #[test]
    fn drops_my_own_call_from_candidates() {
        let a = analyze("KD9TAW DE W1ABC UR 599", &[], "KD9TAW", None, real);
        assert!(a.candidates.iter().all(|c| c.call != "KD9TAW"));
        assert!(a.candidates.iter().any(|c| c.call == "W1ABC"));
    }

    #[test]
    fn reads_report_and_name_with_cut_numbers() {
        let ex = analyze(
            "W1ABC DE K2DEF UR 5NN 5NN NAME BOB BOB",
            &[],
            "W1ABC",
            None,
            real,
        )
        .exchange;
        assert_eq!(ex.rst.as_deref(), Some("599")); // 5NN → 599
        assert_eq!(ex.name.as_deref(), Some("BOB"));
    }

    #[test]
    fn report_received_recommends_73() {
        // We answered; they sent a report → next is 73.
        let sent = vec!["W1ABC DE KD9TAW UR 599 NAME SETH HW? W1ABC".to_string()];
        let a = analyze(
            "KD9TAW DE W1ABC UR 599 NAME BOB",
            &sent,
            "KD9TAW",
            Some("W1ABC"),
            real,
        );
        assert_eq!(a.guidance.state, "report");
        assert_eq!(a.guidance.recommended.as_deref(), Some("F4")); // 73
    }

    #[test]
    fn they_answered_me_recommends_reply() {
        // We called CQ; they came back to our call, no report yet.
        let sent = vec!["CQ CQ DE KD9TAW KD9TAW K".to_string()];
        let a = analyze("KD9TAW DE W1ABC", &sent, "KD9TAW", Some("W1ABC"), real);
        assert_eq!(a.guidance.state, "answered");
        assert_eq!(a.guidance.recommended.as_deref(), Some("F3")); // Reply
    }

    #[test]
    fn closing_recommends_finish_and_log() {
        let sent = vec!["W1ABC DE KD9TAW UR 599 SETH".to_string()];
        let a = analyze(
            "KD9TAW DE W1ABC TU 73 SK",
            &sent,
            "KD9TAW",
            Some("W1ABC"),
            real,
        );
        assert_eq!(a.guidance.state, "73");
        assert!(a.guidance.prompt.to_lowercase().contains("log"));
    }

    #[test]
    fn empty_decode_is_listening() {
        let a = analyze("", &[], "KD9TAW", None, real);
        assert_eq!(a.guidance.state, "listening");
        assert!(a.candidates.is_empty());
        assert!(a.guidance.recommended.is_none());
    }

    #[test]
    fn gibberish_prefix_scores_below_a_real_one() {
        // "Q1ABC" is well-formed but not a real prefix; "W1ABC" resolves → ranks higher.
        let a = analyze("DE Q1ABC W1ABC", &[], "KD9TAW", None, real);
        assert_eq!(a.candidates.first().unwrap().call, "W1ABC");
    }
}
