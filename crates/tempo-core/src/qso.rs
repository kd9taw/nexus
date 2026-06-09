//! Auto-sequenced FT1 QSO state machine (one [`Station`] per operator), plus a
//! headless loopback driver that runs a full QSO between two stations over the
//! [`crate::channel::VirtualAir`] on alternating slots.
//!
//! Standard exchange (initiator calls CQ, responder answers):
//! ```text
//!   slot 0  A: CQ W9XYZ EN37
//!   slot 1  B: W9XYZ K2DEF FN31
//!   slot 2  A: K2DEF W9XYZ -10
//!   slot 3  B: W9XYZ K2DEF R-12
//!   slot 4  A: K2DEF W9XYZ RR73
//!   slot 5  B: K2DEF W9XYZ 73
//! ```
//! Each station retransmits its current message on its TX slots until it hears
//! the expected reply (the alternating-slot ARQ behavior).

use crate::message::Msg;
use modes::Decode;

/// IR-HARQ redundancy versions per exchange step before the cycle wraps (0,1,2).
const RV_CYCLE: u32 = 3;
/// Max transmissions of one step before the sequencer stops escalating and the
/// step is considered failed (2 full RV cycles). The app may then time out the
/// QSO or return to listening.
pub const MAX_TX_PER_STEP: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Monitoring; will answer the first CQ heard.
    Listening,
    /// Calling CQ; awaiting a grid reply addressed to me.
    CallingCq,
    /// (Responder) sent my grid; awaiting a report addressed to me.
    AwaitReport,
    /// (Initiator) sent a report; awaiting a rogered report.
    AwaitRoger,
    /// (Responder) sent a rogered report; awaiting RR73/RRR.
    AwaitRr73,
    /// (Initiator) sent RR73; awaiting the final 73.
    Confirming,
    /// QSO complete.
    Done,
}

impl State {
    /// Map to WSJT-X `nQSOProgress` (0..5) — the a-priori (AP) pass-schedule
    /// index the golden FT8/FT4 decoder (`ft8b`/`ft4_decode`) keys off via
    /// `naptypes`/`nappasses`. WSJT-X's index is the Tx stage you *last sent*
    /// (its enum `CALLING, REPLYING, REPORT, ROGER_REPORT, ROGERS, SIGNOFF`),
    /// which the decoder uses to predict the partner's *incoming* message and
    /// freeze the known fields (MyCall/DxCall/RRR/73/RR73) in the AP mask.
    ///
    /// Our 7 sequencer states bijection cleanly onto WSJT-X's 6 levels — the
    /// two pre-QSO states (`Listening`/`CallingCq`) both sit at CALLING(0):
    ///
    /// | `State`       | sent (Tx) | nQSOProgress |
    /// |---------------|-----------|--------------|
    /// | Listening     | —         | 0 CALLING    |
    /// | CallingCq     | CQ (Tx6)  | 0 CALLING    |
    /// | AwaitReport   | grid (T1) | 1 REPLYING   |
    /// | AwaitRoger    | rpt (T2)  | 2 REPORT     |
    /// | AwaitRr73     | R+rpt(T3) | 3 ROGER_RPT  |
    /// | Confirming    | RR73 (T4) | 4 ROGERS     |
    /// | Done          | 73 (T5)   | 5 SIGNOFF    |
    pub fn nqso_progress(self) -> i32 {
        match self {
            State::Listening | State::CallingCq => 0,
            State::AwaitReport => 1,
            State::AwaitRoger => 2,
            State::AwaitRr73 => 3,
            State::Confirming => 4,
            State::Done => 5,
        }
    }
}

/// One station's auto-sequencer.
#[derive(Debug, Clone)]
pub struct Station {
    pub mycall: String,
    pub mygrid: String,
    pub dxcall: Option<String>,
    /// The DX station's Maidenhead grid, captured from their CQ/grid message (or
    /// pre-seeded by the operator when starting a directed call). For the log.
    pub dxgrid: Option<String>,
    pub state: State,
    /// Message transmitted on each of my TX slots (None = stay silent / listen).
    pub pending: Option<Msg>,
    /// The signal report I received about my own signal.
    pub rx_report: Option<i32>,
    /// IR-HARQ redundancy version for the next transmission of `pending`: 0 on a
    /// fresh step, escalating 0→1→2→0 each time the step is retransmitted without
    /// the partner advancing (implicit NAK). Reset to 0 when the partner advances
    /// (implicit ACK). Lets the receiver joint-combine retransmissions.
    pub rv_count: u8,
    /// Transmissions of the current step so far (resets when the step advances).
    pub tx_count: u32,
    /// Operator preference: roger the final report with `RRR` (acknowledge only,
    /// partner still owes a 73) instead of the combined `RR73`. Default `false`
    /// (RR73 — modern FT8 practice). Mirrors WSJT-X's "Settings ▸ behaviour".
    pub confirm_with_rrr: bool,
    /// Human-readable event log.
    pub transcript: Vec<String>,
}

impl Station {
    /// A station that calls CQ to start a QSO.
    pub fn calling_cq(mycall: &str, mygrid: &str) -> Self {
        Self {
            mycall: mycall.into(),
            mygrid: mygrid.into(),
            dxcall: None,
            dxgrid: None,
            state: State::CallingCq,
            pending: Some(Msg::Cq {
                de: mycall.into(),
                grid: mygrid.into(),
            }),
            rx_report: None,
            rv_count: 0,
            tx_count: 0,
            confirm_with_rrr: false,
            transcript: Vec::new(),
        }
    }

    /// A station that listens and answers the first CQ it hears.
    pub fn monitoring(mycall: &str, mygrid: &str) -> Self {
        Self {
            mycall: mycall.into(),
            mygrid: mygrid.into(),
            dxcall: None,
            dxgrid: None,
            state: State::Listening,
            pending: None,
            rx_report: None,
            rv_count: 0,
            tx_count: 0,
            confirm_with_rrr: false,
            transcript: Vec::new(),
        }
    }

    /// A station that initiates a QSO with a **specific** station — e.g. the
    /// operator clicked a heard station to work them (WSJT-X "double-click to
    /// call"). It sends its grid to `dxcall` and then runs the responder side of
    /// the exchange, exactly as if it had just heard that station's CQ.
    ///
    /// This is [`Station::start`] with no message context — always starts at the
    /// grid (Tx1). Prefer [`start`] when you know the message being answered.
    ///
    /// [`start`]: Station::start
    pub fn answering(mycall: &str, mygrid: &str, dxcall: &str) -> Self {
        Self::start(mycall, mygrid, dxcall, None, false)
    }

    /// Begin a directed QSO with `dxcall`, jumping straight to the Tx state the
    /// message we're answering implies — WSJT-X's double-click semantics (its
    /// `processMessage`). `context` is the decoded message we are responding to
    /// (the line the operator double-clicked, or the latest message from `dxcall`
    /// addressed to us) paired with the SNR we decoded it at — that SNR becomes
    /// the report we send back.
    ///
    /// The next message you send is fixed by what the DX last sent **to you**:
    ///
    /// | DX sent (to me)        | I send       | start state  |
    /// |------------------------|--------------|--------------|
    /// | CQ / call / `None`     | my grid (T1) | AwaitReport  |
    /// | my-grid reply (Grid)   | report  (T2) | AwaitRoger   |
    /// | report (Report)        | R+rpt   (T3) | AwaitRr73    |
    /// | R+report (RReport)     | RR73/RRR(T4) | Confirming   |
    /// | RRR / RR73             | 73      (T5) | Done         |
    /// | 73 (Bye73)             | — (log)      | Done         |
    ///
    /// This is the fix for "clicking a station that already answered restarts at
    /// the grid message": a context addressed to me advances the start state, so
    /// answering a station that already sent you a report goes straight to the
    /// R-report — never back to the grid. A CQ / not-addressed-to-me / `None`
    /// context starts at the grid, exactly like working a fresh CQ.
    pub fn start(
        mycall: &str,
        mygrid: &str,
        dxcall: &str,
        context: Option<(&Msg, i32)>,
        prefer_rrr: bool,
    ) -> Self {
        let mycall_s: String = mycall.into();
        let mut dxgrid: Option<String> = None;
        let mut rx_report: Option<i32> = None;

        // Default: start the exchange — send our grid to dxcall (Tx1).
        let grid_start = || {
            (
                State::AwaitReport,
                Some(Msg::Grid {
                    to: dxcall.into(),
                    de: mycall_s.clone(),
                    grid: mygrid.into(),
                }),
                format!("calling {dxcall} with grid"),
            )
        };

        // Only a message addressed to *us* advances the start state; a CQ or a
        // message to someone else means we're initiating, so we start at the grid.
        // Base-call comparison so portable/compound calls (KD9TAW/P) still match.
        let to_me = context
            .and_then(|(m, _)| m.addressee())
            .map(|to| crate::message::same_call(to, &mycall_s))
            .unwrap_or(false);

        let (state, pending, log_line) = match context {
            // `rpt` (the SNR we decoded the DX at) is the report we send them.
            Some((msg, rpt)) if to_me => match msg {
                // DX answered our CQ with their grid → send them a report.
                Msg::Grid { de, grid, .. } => {
                    dxgrid = Some(grid.clone());
                    (
                        State::AwaitRoger,
                        Some(Msg::Report {
                            to: de.clone(),
                            de: mycall_s.clone(),
                            snr: rpt,
                        }),
                        format!("{de} answered with grid → sending report {rpt}"),
                    )
                }
                // DX sent us a bare report → roger it with R + our report.
                Msg::Report { de, snr, .. } => {
                    rx_report = Some(*snr);
                    (
                        State::AwaitRr73,
                        Some(Msg::RReport {
                            to: de.clone(),
                            de: mycall_s.clone(),
                            snr: rpt,
                        }),
                        format!(
                            "got report {snr} → sending R{}",
                            crate::message::fmt_report(rpt)
                        ),
                    )
                }
                // DX sent R + report → send the roger (RR73, or RRR by preference).
                Msg::RReport { de, snr, .. } => {
                    rx_report = Some(*snr);
                    let roger = if prefer_rrr {
                        Msg::Rrr {
                            to: de.clone(),
                            de: mycall_s.clone(),
                        }
                    } else {
                        Msg::Rr73 {
                            to: de.clone(),
                            de: mycall_s.clone(),
                        }
                    };
                    (
                        State::Confirming,
                        Some(roger),
                        format!(
                            "got R-report → sending {}",
                            if prefer_rrr { "RRR" } else { "RR73" }
                        ),
                    )
                }
                // DX already rogered (RRR/RR73) → send the final 73.
                Msg::Rrr { de, .. } | Msg::Rr73 { de, .. } => (
                    State::Done,
                    Some(Msg::Bye73 {
                        to: de.clone(),
                        de: mycall_s.clone(),
                    }),
                    "got RR73 → sending 73, QSO complete".into(),
                ),
                // DX already signed 73 → nothing to send; ready to log.
                Msg::Bye73 { .. } => (State::Done, None, "got 73 → QSO complete".into()),
                _ => grid_start(),
            },
            _ => grid_start(),
        };

        Self {
            mycall: mycall_s,
            mygrid: mygrid.into(),
            dxcall: Some(dxcall.into()),
            dxgrid,
            state,
            pending,
            rx_report,
            rv_count: 0,
            tx_count: 0,
            confirm_with_rrr: prefer_rrr,
            transcript: vec![log_line],
        }
    }

    pub fn done(&self) -> bool {
        self.state == State::Done && self.pending.is_none()
    }

    /// The message to transmit on my next TX slot, if any (RV-agnostic).
    pub fn outgoing(&self) -> Option<Msg> {
        self.pending.clone()
    }

    /// The message **and IR-HARQ redundancy version** to transmit on my next TX
    /// slot. Returns `None` when there is nothing to send OR the current step has
    /// been retransmitted [`MAX_TX_PER_STEP`] times without acknowledgement (the
    /// step has failed — the caller should time out or return to listening).
    pub fn outgoing_rv(&self) -> Option<(Msg, u8)> {
        // Operating policy: a CQ stops after MAX_TX_PER_STEP calls (don't spam an
        // empty band — the operator re-arms to call again), but a directed call /
        // in-QSO step you're working repeats INDEFINITELY until the station
        // responds (or the operator / Tx watchdog stops it). So the cap applies
        // ONLY to CallingCq.
        if self.state == State::CallingCq && self.tx_count >= MAX_TX_PER_STEP {
            return None;
        }
        self.pending.clone().map(|m| (m, self.rv_count))
    }

    /// True when the current step has hit the retransmission limit without the
    /// partner advancing — i.e. we have an outgoing message but [`outgoing_rv`]
    /// is withholding it. The app may time out the QSO at this point.
    pub fn stalled(&self) -> bool {
        // Only a CQ "stalls" (stops after its call budget); a station you're
        // working keeps calling until it answers or the watchdog trips.
        self.state == State::CallingCq
            && self.pending.is_some()
            && self.tx_count >= MAX_TX_PER_STEP
    }

    /// The current outgoing message as on-air text (the "Now sending" readout),
    /// regardless of whether it is currently being withheld by a stall. `None`
    /// when there is nothing queued (listening, or the QSO is complete).
    pub fn pending_text(&self) -> Option<String> {
        self.pending.as_ref().map(|m| m.to_text())
    }

    /// Operator "Resend": re-arm the current step. Clears the retransmission
    /// counter (and HARQ escalation) so a stalled step transmits again on the
    /// next TX slot — the partner did not copy and we want another round.
    /// No-op when there is nothing pending.
    pub fn resend(&mut self) {
        if self.pending.is_some() {
            self.tx_count = 0;
            self.rv_count = 0;
            self.log("operator resend → re-arming current message".into());
        }
    }

    /// Operator override: replace the next transmission with `msg` (e.g. an
    /// in-QSO free-text Tx5, or forcing a specific standard message), starting a
    /// fresh HARQ cycle. The auto-sequencer's [`observe`] still advances on the
    /// matching reply, so a forced resend rejoins the normal flow.
    pub fn override_next(&mut self, msg: Msg) {
        self.log(format!("operator override → {}", msg.to_text()));
        self.pending = Some(msg);
        self.tx_count = 0;
        self.rv_count = 0;
    }

    /// Called after I transmit `pending`. Escalates the IR-HARQ redundancy
    /// version for the next retransmission of the SAME step (0→1→2→0), and counts
    /// transmissions of this step. (A partner advance in [`observe`] resets both,
    /// so at good SNR — where every transmission is acknowledged — RV stays 0.)
    /// Also clears `pending` once the QSO is complete so the final 73 goes once.
    pub fn after_tx(&mut self) {
        self.tx_count = self.tx_count.saturating_add(1);
        self.rv_count = (self.tx_count % RV_CYCLE) as u8;
        if self.state == State::Done {
            self.pending = None;
        }
    }

    fn log(&mut self, s: String) {
        self.transcript.push(s);
    }

    /// True when `sender` is the station we're working (or we haven't locked one
    /// yet). Once a QSO is in progress, the auto-sequencer must only advance on
    /// messages FROM the worked DX — a reply from a different station must not
    /// hijack the sequence (WSJT-X checks the sender against DX Call). Compared on
    /// base calls so a portable suffix still matches.
    fn from_dx(&self, sender: &str) -> bool {
        self.dxcall
            .as_deref()
            .map_or(true, |dx| crate::message::same_call(sender, dx))
    }

    /// Process the signals decoded this RX slot and advance the sequence.
    pub fn observe(&mut self, decodes: &[Decode]) {
        let state_before = self.state;
        for d in decodes {
            let m = Msg::parse(&d.message);
            let rpt = d.snr.clamp(-30, 49);
            match (self.state, &m) {
                // NOTE: there is intentionally NO (Listening, Cq) auto-answer arm.
                // "Monitor" is passive RX — it must NEVER key up on its own (an
                // unsolicited transmission is unacceptable, and it's not how WSJT-X
                // works). The operator works a station explicitly by double-clicking
                // a decode, which builds an `answering`/`start(..)` station.
                (State::CallingCq, Msg::Grid { to, de, grid }) if crate::message::same_call(to, &self.mycall) => {
                    self.dxcall = Some(de.clone());
                    self.dxgrid = Some(grid.clone());
                    self.pending = Some(Msg::Report {
                        to: de.clone(),
                        de: self.mycall.clone(),
                        snr: rpt,
                    });
                    self.state = State::AwaitRoger;
                    self.log(format!("{de} answered → sending report {rpt}"));
                }
                (State::AwaitReport, Msg::Report { to, de, snr })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.rx_report = Some(*snr);
                    self.pending = Some(Msg::RReport {
                        to: de.clone(),
                        de: self.mycall.clone(),
                        snr: rpt,
                    });
                    self.state = State::AwaitRr73;
                    self.log(format!(
                        "got report {snr} → sending R{}",
                        crate::message::fmt_report(rpt)
                    ));
                }
                (State::AwaitRoger, Msg::RReport { to, de, snr })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.rx_report = Some(*snr);
                    // RR73 (combined roger+73, modern default) unless the operator
                    // prefers a bare RRR (roger only; partner still owes a 73).
                    self.pending = Some(if self.confirm_with_rrr {
                        Msg::Rrr {
                            to: de.clone(),
                            de: self.mycall.clone(),
                        }
                    } else {
                        Msg::Rr73 {
                            to: de.clone(),
                            de: self.mycall.clone(),
                        }
                    });
                    self.state = State::Confirming;
                    self.log(format!(
                        "got R-report → sending {}",
                        if self.confirm_with_rrr { "RRR" } else { "RR73" }
                    ));
                }
                (State::AwaitRr73, Msg::Rr73 { to, de })
                | (State::AwaitRr73, Msg::Rrr { to, de })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.pending = Some(Msg::Bye73 {
                        to: de.clone(),
                        de: self.mycall.clone(),
                    });
                    self.state = State::Done;
                    self.log("got RR73 → sending 73, QSO complete".into());
                }
                (State::Confirming, Msg::Bye73 { to, de })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.pending = None;
                    self.state = State::Done;
                    self.log("got 73 → QSO complete".into());
                }
                // --- Out-of-order / step-skipping partners (mirror the `start()` resume
                // table so a running QSO can't hang re-sending the same message forever
                // when the DX skips a step — exactly what WSJT-X handles). ---
                // A caller awaiting a report whose DX combines R + report (skips the bare
                // report): capture it and send the roger.
                (State::AwaitReport, Msg::RReport { to, de, snr })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.rx_report = Some(*snr);
                    self.pending = Some(if self.confirm_with_rrr {
                        Msg::Rrr { to: de.clone(), de: self.mycall.clone() }
                    } else {
                        Msg::Rr73 { to: de.clone(), de: self.mycall.clone() }
                    });
                    self.state = State::Confirming;
                    self.log(format!(
                        "got R-report → sending {}",
                        if self.confirm_with_rrr { "RRR" } else { "RR73" }
                    ));
                }
                // A caller awaiting a report whose DX rogers directly (RR73/RRR, skipping
                // the report entirely): the DX confirmed → send the final 73 and finish.
                (State::AwaitReport, Msg::Rr73 { to, de })
                | (State::AwaitReport, Msg::Rrr { to, de })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.pending = Some(Msg::Bye73 {
                        to: de.clone(),
                        de: self.mycall.clone(),
                    });
                    self.state = State::Done;
                    self.log("DX rogered after our grid → sending 73, QSO complete".into());
                }
                // We're rogering (AwaitRr73, expecting RR73) and the DX closes with a bare
                // 73 instead: that's their roger+signoff → QSO complete.
                (State::AwaitRr73, Msg::Bye73 { to, de })
                    if crate::message::same_call(to, &self.mycall) && self.from_dx(de) =>
                {
                    self.pending = None;
                    self.state = State::Done;
                    self.log("got 73 → QSO complete".into());
                }
                // We're calling CQ and a station answers with a bare report (grid skipped):
                // lock onto them, capture the report, and roger with R + our report.
                (State::CallingCq, Msg::Report { to, de, snr })
                    if crate::message::same_call(to, &self.mycall) =>
                {
                    self.dxcall = Some(de.clone());
                    self.rx_report = Some(*snr);
                    self.pending = Some(Msg::RReport {
                        to: de.clone(),
                        de: self.mycall.clone(),
                        snr: rpt,
                    });
                    self.state = State::AwaitRr73;
                    self.log(format!(
                        "{de} answered with a report → R{}",
                        crate::message::fmt_report(rpt)
                    ));
                }
                _ => {}
            }
        }
        // Implicit ACK: the partner advanced us to a new step, so the next
        // transmission is a fresh message — restart the RV escalation at 0.
        if self.state != state_before {
            self.rv_count = 0;
            self.tx_count = 0;
        }
    }
}

/// One transmission heard on the (virtual) air.
#[derive(Debug, Clone)]
pub struct AirLog {
    pub slot: u64,
    pub from: String,
    pub text: String,
}

/// Run a full QSO between two stations over an in-process virtual channel.
///
/// `a` transmits on even slots, `b` on odd. Each transmitted frame is placed in
/// the channel (on-time, at `snr_db`, with AWGN) and decoded by the other
/// station via the full acquisition path. Stops when both stations are done or
/// `max_slots` is reached. Returns the on-air transcript.
pub fn run_loopback_qso(
    a: &mut Station,
    b: &mut Station,
    snr_db: f32,
    max_slots: u64,
) -> Vec<AirLog> {
    use crate::channel::{to_i16, VirtualAir, ON_TIME_OFFSET};
    use crate::tx;

    let mut air = VirtualAir::new(ft1::SAMPLE_RATE, 0xC0FFEE);
    let mut log = Vec::new();

    for slot in 0..max_slots {
        let (txs, rxs): (&mut Station, &mut Station) = if slot % 2 == 0 {
            (&mut *a, &mut *b)
        } else {
            (&mut *b, &mut *a)
        };

        if let Some(msg) = txs.outgoing() {
            let text = msg.to_text();
            let frame = tx::build(&text, ft1::SAMPLE_RATE, 1500.0);
            let rx_f32 = air.receive(&frame.wave, ON_TIME_OFFSET, snr_db);
            let iwave = to_i16(&rx_f32);
            let decodes: Vec<Decode> = ft1::decode_frame(
                &iwave,
                200,
                2900,
                3,
                rxs.mycall.as_str(),
                txs.mycall.as_str(),
                0,
                (slot as i64).wrapping_mul(4000), // monotonic ms for IR-HARQ keying
            )
            .into_iter()
            .map(Into::into)
            .collect();
            log.push(AirLog {
                slot,
                from: txs.mycall.clone(),
                text,
            });
            rxs.observe(&decodes);
            txs.after_tx();
        }

        if a.done() && b.done() {
            break;
        }
    }
    log
}

#[cfg(test)]
mod nqso_progress_tests {
    use super::*;

    #[test]
    fn maps_states_to_wsjtx_nqso_progress_bijectively() {
        // The two pre-QSO states both sit at CALLING(0); the rest map 1:1 onto
        // WSJT-X's CALLING..SIGNOFF (0..5), which selects the AP pass schedule.
        assert_eq!(State::Listening.nqso_progress(), 0);
        assert_eq!(State::CallingCq.nqso_progress(), 0);
        assert_eq!(State::AwaitReport.nqso_progress(), 1);
        assert_eq!(State::AwaitRoger.nqso_progress(), 2);
        assert_eq!(State::AwaitRr73.nqso_progress(), 3);
        assert_eq!(State::Confirming.nqso_progress(), 4);
        assert_eq!(State::Done.nqso_progress(), 5);
    }

    #[test]
    fn nqso_progress_is_always_in_decoder_range() {
        // naptypes/nappasses in ft8b/ft4_decode are dimensioned (0:5); an
        // out-of-range index is an out-of-bounds read in the Fortran. Guard it.
        for st in [
            State::Listening,
            State::CallingCq,
            State::AwaitReport,
            State::AwaitRoger,
            State::AwaitRr73,
            State::Confirming,
            State::Done,
        ] {
            let p = st.nqso_progress();
            assert!((0..=5).contains(&p), "{st:?} -> {p} out of 0..=5");
        }
    }
}

#[cfg(test)]
mod start_context_tests {
    //! WSJT-X double-click semantics: starting a directed QSO jumps to the Tx
    //! state implied by the message we're answering (its `processMessage`). The
    //! bug this guards: clicking a station that already answered us must NOT reset
    //! to the grid (Tx1) — it must advance to the correct next message.
    use super::*;
    use crate::message::Msg;

    const ME: &str = "KD9TAW";
    const MY_GRID: &str = "EN61";
    const DX: &str = "W9XYZ";

    fn start(text: &str, snr: i32) -> Station {
        let m = Msg::parse(text);
        Station::start(ME, MY_GRID, DX, Some((&m, snr)), false)
    }

    #[test]
    fn clicking_a_cq_starts_at_the_grid() {
        let s = start("CQ W9XYZ FN31", -7);
        assert_eq!(s.state, State::AwaitReport);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW EN61"));
    }

    #[test]
    fn no_context_starts_at_the_grid() {
        let s = Station::start(ME, MY_GRID, DX, None, false);
        assert_eq!(s.state, State::AwaitReport);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW EN61"));
    }

    #[test]
    fn dx_answered_my_cq_with_grid_sends_report() {
        // I called CQ; DX replied with their grid addressed to me → I send a report
        // (the SNR I decoded them at), NOT my grid. dxgrid is captured for the log.
        let s = start("KD9TAW W9XYZ FN31", -12);
        assert_eq!(s.state, State::AwaitRoger);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW -12"));
        assert_eq!(s.dxgrid.as_deref(), Some("FN31"));
    }

    #[test]
    fn dx_sent_a_report_sends_r_report() {
        // The user's exact bug: they sent their call, DX came back with a report;
        // clicking must send R+report, not the grid square.
        let s = start("KD9TAW W9XYZ -09", -11);
        assert_eq!(s.state, State::AwaitRr73);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW R-11"));
        assert_eq!(s.rx_report, Some(-9), "captured the report DX gave us");
    }

    #[test]
    fn dx_sent_r_report_sends_rr73() {
        let s = start("KD9TAW W9XYZ R-15", -8);
        assert_eq!(s.state, State::Confirming);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW RR73"));
        assert_eq!(s.rx_report, Some(-15));
    }

    #[test]
    fn dx_sent_r_report_with_rrr_preference_sends_rrr() {
        let m = Msg::parse("KD9TAW W9XYZ R-15");
        let s = Station::start(ME, MY_GRID, DX, Some((&m, -8)), true);
        assert_eq!(s.state, State::Confirming);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW RRR"));
    }

    #[test]
    fn dx_sent_rr73_sends_final_73() {
        let s = start("KD9TAW W9XYZ RR73", -8);
        assert_eq!(s.state, State::Done);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW 73"));
    }

    #[test]
    fn dx_sent_73_completes_with_nothing_to_send() {
        let s = start("KD9TAW W9XYZ 73", -8);
        assert_eq!(s.state, State::Done);
        assert!(s.pending.is_none());
        assert!(s.done());
    }

    #[test]
    fn portable_mycall_still_matches_a_reply_to_the_base_call() {
        // I operate as KD9TAW/P; the DX reports my base call KD9TAW. The QSO must
        // still resume (send R-report), not stall at the grid.
        let m = Msg::parse("KD9TAW W9XYZ -09");
        let s = Station::start("KD9TAW/P", MY_GRID, DX, Some((&m, -11)), false);
        assert_eq!(s.state, State::AwaitRr73);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW/P R-11"));
    }

    #[test]
    fn message_addressed_to_someone_else_starts_at_grid() {
        // DX is working another station — clicking DX means I initiate, so grid.
        let s = start("N0ABC W9XYZ -05", -8);
        assert_eq!(s.state, State::AwaitReport);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW EN61"));
    }

    #[test]
    fn resumed_qso_then_advances_normally_via_observe() {
        // Resume at "DX sent report" (→ we send R-report), then the partner sends
        // RR73 → we advance to the final 73 through the normal observe() path.
        let mut s = start("KD9TAW W9XYZ -09", -11);
        assert_eq!(s.state, State::AwaitRr73);
        let rr73 = Msg::Rr73 {
            to: ME.into(),
            de: DX.into(),
        };
        s.observe(&[Decode {
            message: rr73.to_text(),
            sync: 1.0,
            snr: 0,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }]);
        assert_eq!(s.state, State::Done);
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW 73"));
    }

    fn dec(text: &str, snr: i32) -> Decode {
        Decode {
            message: text.into(),
            sync: 1.0,
            snr,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn locked_qso_ignores_a_different_station() {
        // Working W9XYZ (we sent our grid, awaiting their report). A REPORT from a
        // DIFFERENT station addressed to us must NOT advance our sequence — only the
        // station we're working can (WSJT-X sender check). Then the real DX's report
        // does advance it.
        let mut s = Station::answering(ME, MY_GRID, DX); // dxcall = W9XYZ, AwaitReport
        assert_eq!(s.state, State::AwaitReport);
        s.observe(&[dec("KD9TAW N0ABC -05", -5)]); // a different station reports us
        assert_eq!(s.state, State::AwaitReport, "a non-DX reply must not advance");
        s.observe(&[dec("KD9TAW W9XYZ -12", -8)]); // the worked DX reports us
        assert_eq!(s.state, State::AwaitRr73, "the worked DX advances the sequence");
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW R-08"));
    }

    // --- Step-skipping partners: the sequencer must complete, not hang re-sending. ---

    #[test]
    fn dx_rogers_after_our_grid_skipping_the_report() {
        // We answered a CQ (sent our grid, AwaitReport). The DX rogers directly with
        // RR73 (skipping the bare report) → we send 73 and finish, NOT re-send the grid.
        let mut s = Station::answering(ME, MY_GRID, DX);
        assert_eq!(s.state, State::AwaitReport);
        s.observe(&[dec("KD9TAW W9XYZ RR73", -8)]);
        assert_eq!(s.state, State::Done, "an early RR73 completes the QSO");
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW 73"));
    }

    #[test]
    fn dx_sends_combined_r_report_after_our_grid() {
        // AwaitReport, DX combines R + report → capture it and send the roger.
        let mut s = Station::answering(ME, MY_GRID, DX);
        s.observe(&[dec("KD9TAW W9XYZ R-13", -7)]);
        assert_eq!(s.state, State::Confirming);
        assert_eq!(s.rx_report, Some(-13));
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW RR73"));
    }

    #[test]
    fn dx_closes_with_bare_73_instead_of_rr73() {
        // We sent our R-report (AwaitRr73). The DX closes with a plain 73 → QSO complete
        // (instead of re-sending the R-report forever waiting for an RR73).
        let mut s = Station::answering(ME, MY_GRID, DX);
        s.observe(&[dec("KD9TAW W9XYZ -12", -8)]); // their report → AwaitRr73
        assert_eq!(s.state, State::AwaitRr73);
        s.observe(&[dec("KD9TAW W9XYZ 73", -8)]); // a bare 73 instead of RR73
        assert_eq!(s.state, State::Done, "a bare 73 closes the QSO");
    }

    #[test]
    fn cq_answered_with_a_bare_report_locks_and_rogers() {
        // Calling CQ; a station answers with a bare report (grid skipped) → lock onto
        // them, capture the report, and send R + our report.
        let mut s = Station::calling_cq(ME, MY_GRID);
        assert_eq!(s.state, State::CallingCq);
        s.observe(&[dec("KD9TAW W9XYZ -15", -10)]);
        assert_eq!(s.state, State::AwaitRr73);
        assert_eq!(s.dxcall.as_deref(), Some("W9XYZ"));
        assert_eq!(s.rx_report, Some(-15), "captured the report they gave us");
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW R-10"));
    }

    #[test]
    fn running_cq_stops_after_its_call_budget() {
        // Operating policy: a CQ stops after MAX_TX_PER_STEP calls (don't spam an
        // empty band) — the operator re-arms to call again.
        let mut s = Station::calling_cq(ME, MY_GRID);
        for _ in 0..MAX_TX_PER_STEP {
            assert!(s.outgoing_rv().is_some(), "CQ calls within the budget");
            s.after_tx();
        }
        assert!(s.outgoing_rv().is_none(), "CQ stops after its budget");
        assert!(s.stalled(), "a finished CQ reports stalled (Resend re-arms)");
    }

    #[test]
    fn calling_a_station_repeats_indefinitely() {
        // A station you're working (here: answering — sending your grid, awaiting
        // their report) keeps calling FAR past the CQ budget, until they respond or
        // the Tx watchdog stops it.
        let mut s = Station::answering(ME, MY_GRID, DX);
        for _ in 0..(MAX_TX_PER_STEP * 3 + 5) {
            assert!(s.outgoing_rv().is_some(), "keeps calling the station");
            assert!(!s.stalled(), "calling a station never auto-stalls");
            s.after_tx();
        }
        assert_eq!(s.pending_text().as_deref(), Some("W9XYZ KD9TAW EN61"));
    }
}

#[cfg(test)]
mod harq_seq_tests {
    use super::*;
    use crate::message::Msg;

    fn decode(text: &str) -> Decode {
        Decode {
            message: text.into(),
            sync: 1.0,
            snr: 0,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn rv_escalates_on_unacknowledged_retransmits() {
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        assert_eq!(s.outgoing_rv().unwrap().1, 0, "initial TX is RV0");
        s.after_tx();
        assert_eq!(
            s.outgoing_rv().unwrap().1,
            1,
            "1st unacked retransmit -> RV1"
        );
        s.after_tx();
        assert_eq!(s.outgoing_rv().unwrap().1, 2, "2nd -> RV2");
        s.after_tx();
        assert_eq!(
            s.outgoing_rv().unwrap().1,
            0,
            "3rd wraps to RV0 (fresh HARQ cycle)"
        );
    }

    #[test]
    fn rv_resets_when_partner_advances() {
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        s.after_tx();
        s.after_tx(); // escalate to RV2
        assert_eq!(s.outgoing_rv().unwrap().1, 2);
        // Partner answers our CQ with a grid addressed to us -> the step advances
        // (implicit ACK of our CQ).
        let reply = Msg::Grid {
            to: "W9XYZ".into(),
            de: "K2DEF".into(),
            grid: "FN31".into(),
        };
        s.observe(&[decode(&reply.to_text())]);
        assert_eq!(s.state, State::AwaitRoger, "advanced to the next step");
        assert_eq!(
            s.outgoing_rv().unwrap().1,
            0,
            "RV resets to 0 on implicit ACK"
        );
        assert_eq!(s.tx_count, 0, "TX counter resets on advance");
    }

    #[test]
    fn step_stalls_after_max_tx_without_ack() {
        // A CQ stops after its call budget (the only step that auto-stalls).
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        for i in 0..MAX_TX_PER_STEP {
            assert!(
                s.outgoing_rv().is_some(),
                "TX {i} of the step should be allowed"
            );
            s.after_tx();
        }
        assert!(
            s.outgoing_rv().is_none(),
            "step exhausted -> withhold further TX"
        );
        assert!(s.stalled(), "stalled() true once the step hits the TX cap");
    }

    #[test]
    fn resend_clears_a_stall_and_re_arms() {
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        for _ in 0..MAX_TX_PER_STEP {
            s.after_tx();
        }
        assert!(s.stalled(), "step exhausted");
        assert!(s.outgoing_rv().is_none(), "withheld while stalled");
        s.resend();
        assert!(!s.stalled(), "resend clears the stall");
        assert_eq!(
            s.outgoing_rv().map(|(_, rv)| rv),
            Some(0),
            "resend re-arms at RV0"
        );
        assert_eq!(s.pending_text().as_deref(), Some("CQ W9XYZ EN37"));
    }

    #[test]
    fn override_next_swaps_message_and_resets_cycle() {
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        s.after_tx();
        s.after_tx(); // escalate
        let free = Msg::Other("K2DEF W9XYZ GL OM".into());
        s.override_next(free.clone());
        assert_eq!(s.pending_text().as_deref(), Some("K2DEF W9XYZ GL OM"));
        assert_eq!(s.outgoing_rv().unwrap().1, 0, "override starts fresh HARQ");
        assert_eq!(s.tx_count, 0);
    }

    #[test]
    fn confirm_with_rrr_sends_rrr_not_rr73() {
        // Initiator who prefers a bare RRR: after CQ → grid → report → R-report,
        // the roger message is RRR instead of RR73.
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        s.confirm_with_rrr = true;
        s.observe(&[decode("W9XYZ K2DEF FN31")]); // grid reply → sends report
        assert_eq!(s.state, State::AwaitRoger);
        s.observe(&[decode("W9XYZ K2DEF R-12")]); // R-report → roger
        assert_eq!(s.state, State::Confirming);
        assert!(
            matches!(s.pending, Some(Msg::Rrr { .. })),
            "prefers RRR, got {:?}",
            s.pending
        );
        // Default (RR73) for contrast.
        let mut d = Station::calling_cq("W9XYZ", "EN37");
        d.observe(&[decode("W9XYZ K2DEF FN31")]);
        d.observe(&[decode("W9XYZ K2DEF R-12")]);
        assert!(matches!(d.pending, Some(Msg::Rr73 { .. })), "default RR73");
    }

    #[test]
    fn implicit_nak_does_not_reset_escalation() {
        // Only a genuine step advance resets RV. An unrelated decode (someone
        // else's CQ, or noise) is an implicit NAK and must keep escalating.
        let mut s = Station::calling_cq("W9XYZ", "EN37");
        s.after_tx(); // -> RV1
        assert_eq!(s.outgoing_rv().unwrap().1, 1);
        s.observe(&[decode("CQ N0XYZ FN20")]); // not addressed to us
        assert_eq!(s.state, State::CallingCq, "no advance");
        assert_eq!(
            s.outgoing_rv().unwrap().1,
            1,
            "RV unchanged (still awaiting ACK)"
        );
    }
}
