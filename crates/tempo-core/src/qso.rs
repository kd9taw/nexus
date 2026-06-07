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

/// One station's auto-sequencer.
#[derive(Debug, Clone)]
pub struct Station {
    pub mycall: String,
    pub mygrid: String,
    pub dxcall: Option<String>,
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
    pub fn answering(mycall: &str, mygrid: &str, dxcall: &str) -> Self {
        Self {
            mycall: mycall.into(),
            mygrid: mygrid.into(),
            dxcall: Some(dxcall.into()),
            state: State::AwaitReport,
            pending: Some(Msg::Grid {
                to: dxcall.into(),
                de: mycall.into(),
                grid: mygrid.into(),
            }),
            rx_report: None,
            rv_count: 0,
            tx_count: 0,
            confirm_with_rrr: false,
            transcript: vec![format!("calling {dxcall} with grid")],
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
        if self.tx_count >= MAX_TX_PER_STEP {
            return None;
        }
        self.pending.clone().map(|m| (m, self.rv_count))
    }

    /// True when the current step has hit the retransmission limit without the
    /// partner advancing — i.e. we have an outgoing message but [`outgoing_rv`]
    /// is withholding it. The app may time out the QSO at this point.
    pub fn stalled(&self) -> bool {
        self.pending.is_some() && self.tx_count >= MAX_TX_PER_STEP
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

    /// Process the signals decoded this RX slot and advance the sequence.
    pub fn observe(&mut self, decodes: &[Decode]) {
        let state_before = self.state;
        for d in decodes {
            let m = Msg::parse(&d.message);
            let rpt = d.snr.clamp(-30, 30);
            match (self.state, &m) {
                (State::Listening, Msg::Cq { de, .. }) => {
                    self.dxcall = Some(de.clone());
                    self.pending = Some(Msg::Grid {
                        to: de.clone(),
                        de: self.mycall.clone(),
                        grid: self.mygrid.clone(),
                    });
                    self.state = State::AwaitReport;
                    self.log(format!("heard CQ {de} → answering with grid"));
                }
                (State::CallingCq, Msg::Grid { to, de, .. }) if to == &self.mycall => {
                    self.dxcall = Some(de.clone());
                    self.pending = Some(Msg::Report {
                        to: de.clone(),
                        de: self.mycall.clone(),
                        snr: rpt,
                    });
                    self.state = State::AwaitRoger;
                    self.log(format!("{de} answered → sending report {rpt}"));
                }
                (State::AwaitReport, Msg::Report { to, de, snr }) if to == &self.mycall => {
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
                (State::AwaitRoger, Msg::RReport { to, de, snr }) if to == &self.mycall => {
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
                    if to == &self.mycall =>
                {
                    self.pending = Some(Msg::Bye73 {
                        to: de.clone(),
                        de: self.mycall.clone(),
                    });
                    self.state = State::Done;
                    self.log("got RR73 → sending 73, QSO complete".into());
                }
                (State::Confirming, Msg::Bye73 { to, .. }) if to == &self.mycall => {
                    self.pending = None;
                    self.state = State::Done;
                    self.log("got 73 → QSO complete".into());
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
