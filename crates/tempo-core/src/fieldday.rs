//! ARRL Field Day mode: the Class+Section exchange, an auto-sequencer that runs
//! operator-initiated two-way contacts, a dupe-checked log with scoring and a
//! section multiplier, and ADIF / Cabrillo export.
//!
//! Field Day requires operator-initiated contacts (no fully-automated QSOs), and
//! the exchange is **Class + ARRL/RAC Section** (e.g. `3A WI`). FT1 carries this
//! natively in one frame: `<to> <de> <class> <section>` (and the rogered
//! `<to> <de> R <class> <section>`).

use crate::message::Msg;
use modes::Decode;
use std::collections::HashSet;

/// Which Field Day event is running — they share the exchange SHAPE
/// (designator + ARRL/RAC section) but differ in designator grammar, contest
/// ids, and scoring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FdEvent {
    /// ARRL Field Day (4th full June weekend): class like `3A`, phone 1 pt,
    /// CW/digital 2 pts, power multiplier, ~100-pt bonus menu.
    #[default]
    ArrlFd,
    /// Winter Field Day (last full January weekend): category like `2O`
    /// (count + Home/Indoor/Mobile/Outdoor).
    WinterFd,
}

impl FdEvent {
    pub fn contest_id(self) -> &'static str {
        match self {
            FdEvent::ArrlFd => "ARRL-FIELD-DAY",
            FdEvent::WinterFd => "WFD",
        }
    }
    pub fn from_code(s: &str) -> Self {
        if s.trim().eq_ignore_ascii_case("wfd") {
            FdEvent::WinterFd
        } else {
            FdEvent::ArrlFd
        }
    }
}

/// Per-QSO points by operating mode class (both events: phone 1, CW/digital 2 —
/// the long-standing ARRL FD values; WFD currently matches for the base QSO
/// point, with its own multiplier system handled at the score layer).
pub fn qso_points_for_mode(mode: &str) -> u32 {
    match mode.to_ascii_uppercase().as_str() {
        "PH" | "PHONE" | "SSB" | "FM" => 1,
        _ => 2, // CW + digital
    }
}

/// A Field Day exchange: transmitter class (e.g. `3A`) + ARRL/RAC section (`WI`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exchange {
    pub class: String,
    pub section: String,
}

impl Exchange {
    pub fn new(class: &str, section: &str) -> Self {
        Self {
            class: class.to_string(),
            section: section.to_string(),
        }
    }
}

/// A logged Field Day contact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoggedQso {
    pub call: String,
    pub class: String,
    pub section: String,
    pub band: String,
    /// Mode class for scoring + per-band-mode dupes: "DIG" | "CW" | "PH".
    pub mode: String,
    pub slot: u64,
    /// Unix seconds when the contact was logged — Cabrillo requires a real
    /// `yyyy-mm-dd hhmm` per QSO line (the old `----------` placeholder would
    /// FAIL ARRL submission). 0 in legacy/test paths = falls back to the
    /// placeholder rather than inventing a date.
    pub when_unix: u64,
}

/// A dupe-checked Field Day log with scoring.
#[derive(Debug)]
pub struct FieldDayLog {
    pub mycall: String,
    pub myexch: Exchange,
    pub band: String,
    pub event: FdEvent,
    qsos: Vec<LoggedQso>,
    worked: HashSet<(String, String, String)>, // (call, band, mode class)
}

impl FieldDayLog {
    pub fn new(mycall: &str, myexch: Exchange, band: &str) -> Self {
        Self {
            mycall: mycall.to_string(),
            myexch,
            band: band.to_string(),
            event: FdEvent::ArrlFd,
            qsos: Vec::new(),
            worked: HashSet::new(),
        }
    }

    /// Already worked this call on this band IN THIS MODE CLASS? (ARRL FD
    /// rules: each station counts once per band-mode — CW, digital and phone
    /// are separate contacts.) The digital sequencer always logs "DIG".
    pub fn is_dupe(&self, call: &str) -> bool {
        self.is_dupe_mode(call, "DIG")
    }

    pub fn is_dupe_mode(&self, call: &str, mode: &str) -> bool {
        self.worked.contains(&(
            call.to_uppercase(),
            self.band.clone(),
            mode.to_ascii_uppercase(),
        ))
    }

    /// Log a contact. Returns false (and logs nothing) if it's a dupe.
    pub fn log(&mut self, call: &str, class: &str, section: &str, slot: u64) -> bool {
        self.log_mode_at(call, class, section, "DIG", slot, now_unix())
    }

    /// As [`log`](Self::log) with an explicit timestamp (tests / replays).
    pub fn log_at(
        &mut self,
        call: &str,
        class: &str,
        section: &str,
        slot: u64,
        when_unix: u64,
    ) -> bool {
        self.log_mode_at(call, class, section, "DIG", slot, when_unix)
    }

    /// All-mode entry (the CW/Phone cockpits log FD contacts too): mode is the
    /// scoring class "DIG" | "CW" | "PH".
    pub fn log_mode_at(
        &mut self,
        call: &str,
        class: &str,
        section: &str,
        mode: &str,
        slot: u64,
        when_unix: u64,
    ) -> bool {
        let mode = mode.to_ascii_uppercase();
        if self.is_dupe_mode(call, &mode) {
            return false;
        }
        self.worked
            .insert((call.to_uppercase(), self.band.clone(), mode.clone()));
        self.qsos.push(LoggedQso {
            call: call.to_string(),
            class: class.to_string(),
            section: section.to_string(),
            band: self.band.clone(),
            mode,
            slot,
            when_unix,
        });
        true
    }

    pub fn qso_count(&self) -> usize {
        self.qsos.len()
    }

    pub fn qsos(&self) -> &[LoggedQso] {
        &self.qsos
    }

    /// Distinct ARRL/RAC sections worked (the Field Day multiplier).
    pub fn sections(&self) -> usize {
        self.qsos
            .iter()
            .map(|q| q.section.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    /// Per-mode QSO points (phone 1, CW/digital 2) — power multiplier and
    /// bonuses are applied at the score layer (engine), not here.
    pub fn qso_points(&self) -> u32 {
        self.qsos
            .iter()
            .map(|q| qso_points_for_mode(&q.mode))
            .sum()
    }

    /// Export the log as ADIF records (one `<EOR>` per QSO).
    pub fn adif(&self) -> String {
        let mut s = String::from("ADIF Export from Nexus\n<PROGRAMID:5>Nexus\n<EOH>\n");
        for q in &self.qsos {
            s.push_str(&adif_field("CALL", &q.call));
            s.push_str(&adif_field(
                "MODE",
                match q.mode.as_str() {
                    "CW" => "CW",
                    "PH" => "SSB",
                    _ => "FT8",
                },
            ));
            s.push_str(&adif_field("BAND", &q.band));
            s.push_str(&adif_field("CONTEST_ID", self.event.contest_id()));
            s.push_str(&adif_field("CLASS", &q.class));
            s.push_str(&adif_field("ARRL_SECT", &q.section));
            s.push_str("<EOR>\n");
        }
        s
    }

    /// Export the log in Cabrillo QSO-line form for the given band frequency (kHz).
    pub fn cabrillo(&self, freq_khz: u32) -> String {
        let mut s = String::new();
        s.push_str("START-OF-LOG: 3.0\n");
        s.push_str(&format!("CONTEST: {}\n", self.event.contest_id()));
        s.push_str(&format!("CALLSIGN: {}\n", self.mycall));
        s.push_str(&format!(
            "CATEGORY-OPERATOR: MULTI-OP\nLOCATION: {}\nCREATED-BY: Nexus\n",
            self.myexch.section
        ));
        for q in &self.qsos {
            // QSO: freq mo date time mycall myexch call exch — ARRL requires a
            // REAL `yyyy-mm-dd hhmm`; the old `----------` placeholder failed
            // submission. Legacy rows without a stamp keep the placeholder so
            // we never invent a time.
            let (date, time) = if q.when_unix > 0 {
                cabrillo_datetime(q.when_unix)
            } else {
                ("----------".to_string(), "----".to_string()) // HHMM is 4 chars
            };
            // Mode token per Cabrillo 3.0: DG digital, CW, PH phone.
            let mo = match q.mode.as_str() {
                "CW" => "CW",
                "PH" => "PH",
                _ => "DG",
            };
            s.push_str(&format!(
                "QSO: {freq_khz} {mo} {date} {time} {} {} {} {} {} {}\n",
                self.mycall, self.myexch.class, self.myexch.section, q.call, q.class, q.section
            ));
        }
        s.push_str("END-OF-LOG:\n");
        s
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Unix seconds → ("yyyy-mm-dd", "hhmm") in UTC (Howard Hinnant's civil-from-days;
/// no external date crate needed for two Cabrillo fields).
fn cabrillo_datetime(unix: u64) -> (String, String) {
    let secs_of_day = unix % 86_400;
    let days = (unix / 86_400) as i64;
    let (h, m) = ((secs_of_day / 3600) as u32, ((secs_of_day % 3600) / 60) as u32);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if mo <= 2 { y + 1 } else { y };
    (format!("{y:04}-{mo:02}-{d:02}"), format!("{h:02}{m:02}"))
}

fn adif_field(name: &str, value: &str) -> String {
    format!("<{}:{}>{} ", name, value.len(), value)
}

// ---- Auto-sequencer -------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdState {
    /// Monitoring; will answer the first CQ heard (search & pounce).
    Listening,
    /// Calling CQ FD (running).
    CallingCq,
    /// (S&P) sent my exchange; awaiting the runner's rogered exchange.
    AwaitExchange,
    /// (Running) got a caller's exchange; awaiting their RR73 confirmation.
    AwaitConfirm,
    /// Contact complete (logged).
    Done,
}

/// One station's Field Day auto-sequencer.
#[derive(Debug)]
pub struct FieldDayStation {
    pub mygrid: String,
    pub state: FdState,
    pub pending: Option<Msg>,
    pub dxcall: Option<String>,
    /// A caller's exchange, remembered until their RR73 lets us log it.
    peer_exch: Option<(String, String, String)>, // (call, class, section)
    pub log: FieldDayLog,
    pub transcript: Vec<String>,
}

impl FieldDayStation {
    fn mycall(&self) -> &str {
        &self.log.mycall
    }

    /// A running station (calls CQ FD).
    pub fn running(mycall: &str, mygrid: &str, exch: Exchange, band: &str) -> Self {
        Self {
            mygrid: mygrid.to_string(),
            state: FdState::CallingCq,
            pending: Some(Msg::Cq {
                de: mycall.to_string(),
                grid: mygrid.to_string(),
                // Field Day's CQ is "CQ FD <call> <grid>" by convention — but the
                // exchange machinery has always sent a plain CQ here; keep as-is.
                dir: String::new(),
            }),
            dxcall: None,
            peer_exch: None,
            log: FieldDayLog::new(mycall, exch, band),
            transcript: Vec::new(),
        }
    }

    /// A search-and-pounce station (answers CQs).
    pub fn search_and_pounce(mycall: &str, mygrid: &str, exch: Exchange, band: &str) -> Self {
        Self {
            mygrid: mygrid.to_string(),
            state: FdState::Listening,
            pending: None,
            dxcall: None,
            peer_exch: None,
            log: FieldDayLog::new(mycall, exch, band),
            transcript: Vec::new(),
        }
    }

    pub fn done(&self) -> bool {
        self.state == FdState::Done && self.pending.is_none()
    }

    pub fn outgoing(&self) -> Option<Msg> {
        self.pending.clone()
    }

    pub fn after_tx(&mut self) {
        if self.state == FdState::Done {
            self.pending = None;
        }
    }

    fn my_exch_msg(&self, to: &str, roger: bool) -> Msg {
        Msg::FieldDay {
            to: to.to_string(),
            de: self.mycall().to_string(),
            roger,
            class: self.log.myexch.class.clone(),
            section: self.log.myexch.section.clone(),
        }
    }

    /// Process the signals decoded this slot and advance the exchange.
    pub fn observe(&mut self, decodes: &[Decode], slot: u64) {
        for d in decodes {
            let m = Msg::parse(&d.message);
            match (self.state, &m) {
                // S&P: heard a CQ → answer with my exchange.
                (FdState::Listening, Msg::Cq { de, .. }) => {
                    if self.log.is_dupe(de) {
                        self.transcript.push(format!("skip dupe {de}"));
                        continue;
                    }
                    self.dxcall = Some(de.clone());
                    self.pending = Some(self.my_exch_msg(de, false));
                    self.state = FdState::AwaitExchange;
                    self.transcript.push(format!("answer CQ {de}"));
                }
                // Running: a caller sent their exchange → roger + send mine.
                (
                    FdState::CallingCq,
                    Msg::FieldDay {
                        to,
                        de,
                        roger: false,
                        class,
                        section,
                    },
                ) if to == self.mycall() => {
                    self.dxcall = Some(de.clone());
                    self.peer_exch = Some((de.clone(), class.clone(), section.clone()));
                    self.pending = Some(self.my_exch_msg(de, true));
                    self.state = FdState::AwaitConfirm;
                    self.transcript
                        .push(format!("caller {de} {class} {section} → R + my exch"));
                }
                // S&P: the runner rogered + sent their exchange → log + RR73.
                (
                    FdState::AwaitExchange,
                    Msg::FieldDay {
                        to,
                        de,
                        roger: true,
                        class,
                        section,
                    },
                ) if to == self.mycall() => {
                    self.log.log(de, class, section, slot);
                    self.pending = Some(Msg::Rr73 {
                        to: de.clone(),
                        de: self.mycall().to_string(),
                    });
                    self.state = FdState::Done;
                    self.transcript
                        .push(format!("logged {de} {class} {section}; send RR73"));
                }
                // Running: caller confirmed → log them.
                (FdState::AwaitConfirm, Msg::Rr73 { to, .. })
                | (FdState::AwaitConfirm, Msg::Rrr { to, .. })
                    if to == self.mycall() =>
                {
                    if let Some((call, class, section)) = self.peer_exch.take() {
                        self.log.log(&call, &class, &section, slot);
                        self.transcript
                            .push(format!("logged {call} {class} {section}"));
                    }
                    self.pending = None;
                    self.state = FdState::Done;
                }
                _ => {}
            }
        }
    }
}

/// Run a Field Day exchange between a running and an S&P station over the
/// virtual channel. Stops when both have logged the contact or `max_slots`.
pub fn run_loopback_fieldday(
    running: &mut FieldDayStation,
    sp: &mut FieldDayStation,
    snr_db: f32,
    max_slots: u64,
) {
    use crate::channel::{to_i16, VirtualAir, ON_TIME_OFFSET};
    use crate::tx;

    let mut air = VirtualAir::new(ft1::SAMPLE_RATE, 0xFD0001);
    for slot in 0..max_slots {
        let (txs, rxs): (&mut FieldDayStation, &mut FieldDayStation) = if slot % 2 == 0 {
            (&mut *running, &mut *sp)
        } else {
            (&mut *sp, &mut *running)
        };
        if let Some(msg) = txs.outgoing() {
            let text = msg.to_text();
            let frame = tx::build(&text, ft1::SAMPLE_RATE, 1500.0);
            let rx_f32 = air.receive(&frame.wave, ON_TIME_OFFSET, snr_db);
            let decodes: Vec<Decode> = ft1::decode_frame(
                &to_i16(&rx_f32),
                200,
                2900,
                3,
                "",
                "",
                0,
                (slot as i64).wrapping_mul(4000), // monotonic ms for IR-HARQ keying
            )
            .into_iter()
            .map(Into::into)
            .collect();
            rxs.observe(&decodes, slot);
            txs.after_tx();
        }
        if running.log.qso_count() >= 1 && sp.log.qso_count() >= 1 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn arrl_scoring_per_mode_and_band_mode_dupes() {
        // ARRL FD: phone 1 pt, CW/digital 2 pts; a station counts once per
        // band PER MODE CLASS (the old (call, band) dupe key under-counted —
        // K1ABC on 20m CW and 20m FT8 are two legal contacts).
        let mut log = FieldDayLog::new(
            "W9XYZ",
            Exchange { class: "3A".into(), section: "WI".into() },
            "20m",
        );
        assert!(log.log_mode_at("K1ABC", "2A", "CT", "DIG", 0, 100));
        assert!(log.log_mode_at("K1ABC", "2A", "CT", "CW", 0, 110), "same call, new mode");
        assert!(log.log_mode_at("K1ABC", "2A", "CT", "PH", 0, 120));
        assert!(!log.log_mode_at("K1ABC", "2A", "CT", "DIG", 0, 130), "band+mode dupe");
        assert!(!log.log_mode_at("k1abc", "2A", "CT", "dig", 0, 140), "case-insensitive dupe");
        assert_eq!(log.qso_count(), 3);
        assert_eq!(log.qso_points(), 2 + 2 + 1, "DIG 2 + CW 2 + PH 1");
        // Cabrillo mode tokens per row.
        let cab = log.cabrillo(14074);
        assert!(cab.contains(" DG "));
        assert!(cab.contains(" CW "));
        assert!(cab.contains(" PH "));
        // WFD event flips the contest ids in both exports.
        log.event = FdEvent::WinterFd;
        assert!(log.cabrillo(14074).contains("CONTEST: WFD"));
        assert!(log.adif().contains("WFD"));
    }

    #[test]
    fn cabrillo_lines_carry_real_utc_timestamps() {
        // 2026-06-27 18:05:00 UTC (Field Day Saturday) = 1782583500.
        let (d, t) = cabrillo_datetime(1_782_583_500);
        assert_eq!((d.as_str(), t.as_str()), ("2026-06-27", "1805"));
        let mut log = FieldDayLog::new(
            "W9XYZ",
            Exchange { class: "3A".into(), section: "WI".into() },
            "20m",
        );
        assert!(log.log_at("K1ABC", "2A", "CT", 4, 1_782_583_500));
        let cab = log.cabrillo(14074);
        assert!(
            cab.contains("QSO: 14074 DG 2026-06-27 1805 W9XYZ 3A WI K1ABC 2A CT"),
            "real date/time on the QSO line (ARRL submission requires it): {cab}"
        );
        assert!(!cab.contains("----------"), "no placeholder when stamped");
    }

    use super::*;

    fn dec(msg: &str) -> Decode {
        Decode {
            message: msg.to_string(),
            sync: 1.0,
            snr: -5,
            dt: 0.0,
            freq: 1500.0,
            nap: 0,
            qual: 1.0,
            rv: None,
            mode: None,
        }
    }

    #[test]
    fn dupe_check_and_scoring() {
        let mut log = FieldDayLog::new("W9XYZ", Exchange::new("3A", "WI"), "20M");
        assert!(log.log("K2DEF", "3A", "IL", 1));
        assert!(log.log("N0ABC", "1D", "MN", 2));
        assert!(!log.log("K2DEF", "3A", "IL", 3)); // dupe on same band
        assert_eq!(log.qso_count(), 2);
        assert_eq!(log.sections(), 2); // IL, MN
        assert_eq!(log.qso_points(), 4); // 2 pts each
        assert!(log.adif().contains("ARRL_SECT") && log.adif().contains("K2DEF"));
        let cab = log.cabrillo(14_074);
        assert_eq!(cab.matches("QSO:").count(), 2);
        assert!(cab.contains("W9XYZ 3A WI K2DEF 3A IL"));
    }

    #[test]
    fn observe_runs_sp_side() {
        // S&P station hears a CQ, sends exchange, then the runner's rogered exch.
        let mut sp =
            FieldDayStation::search_and_pounce("K2DEF", "FN31", Exchange::new("2A", "IL"), "20M");
        sp.observe(&[dec("CQ W9XYZ EN37")], 0);
        assert_eq!(sp.state, FdState::AwaitExchange);
        assert_eq!(sp.outgoing().unwrap().to_text(), "W9XYZ K2DEF 2A IL");
        sp.observe(&[dec("K2DEF W9XYZ R 3A WI")], 1);
        assert_eq!(sp.state, FdState::Done);
        assert_eq!(sp.log.qso_count(), 1);
        assert_eq!(sp.log.qsos()[0].class, "3A");
        assert_eq!(sp.log.qsos()[0].section, "WI");
    }
}
