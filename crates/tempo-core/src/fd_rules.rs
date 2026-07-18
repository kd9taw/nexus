//! The centralized Field Day ruleset — SFD (ARRL Field Day) and WFD (Winter
//! Field Day) rules defined in exactly ONE place so every surface (sequencer,
//! scoring, exports, N3FJP, the sections board) stays consistent across all
//! modes, and a pre-contest refresh is a one-table edit (Field Day spec §2 +
//! its §7 update model).
//!
//! `rules_year` stamps each ruleset; the pinned per-event score fixtures in the
//! tests below fail if a rule edit changes a score without a matching test
//! update, catching drift before it ships.

use crate::fieldday::{FdEvent, FieldDayLog};

/// The rules year this build targets. Only 2026 rulesets exist today, so
/// [`ruleset`] selects purely on the event; the year is carried for the
/// forthcoming multi-year table and to stamp exports.
pub const CURRENT_RULES_YEAR: u16 = 2026;

/// One Field Day bonus (replaces the old `tempo_app::FD_BONUSES` tuple table).
/// `id` is the stable settings key; `points` is what a claimed bonus scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bonus {
    pub id: &'static str,
    pub label: &'static str,
    pub points: u32,
}

/// The exchange both events use today: a transmitter Class (e.g. `3A`) and an
/// ARRL/RAC Section (e.g. `WI`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExchangeSpec {
    pub class_label: &'static str,
    pub section_label: &'static str,
}

/// One ARRL/RAC Field Day section: the exchange abbreviation sent on the air
/// (e.g. `WI`), its full name, and the ARRL division it sits in (so the
/// worked-sections board can lay the cells out division-by-division).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Section {
    pub code: &'static str,
    pub name: &'static str,
    pub division: &'static str,
}

/// The dupe key both events use today: a station counts once per (call, band,
/// mode-class). Descriptive metadata — the check itself is enforced in
/// [`FieldDayLog`](crate::fieldday::FieldDayLog).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DupeRule {
    pub by_call: bool,
    pub by_band: bool,
    pub by_mode_class: bool,
}

/// A time window for one occurrence of an event (Unix seconds, UTC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventWindow {
    pub start_unix: u64,
    pub end_unix: u64,
}

/// How an event turns a log into a score.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringModel {
    /// ARRL Field Day: sum the per-mode QSO points (phone 1, CW/digital 2 —
    /// from [`qso_points_for_mode`](crate::fieldday::qso_points_for_mode)),
    /// multiply by a legal power tier, then add claimed bonus points. This is
    /// the exact math the engine used inline before centralization.
    PoweredMultiplier { power_tiers: &'static [u32] },
    /// Winter Field Day: QSOs × (objectives + 1). Objective multipliers are
    /// applied at submission, so with no objective values in the code yet this
    /// scores RAW QSO points on the air and flags that the total is provisional
    /// (`multipliers_at_submission`). See the module-level concerns.
    Objectives { multipliers_at_submission: bool },
}

impl ScoringModel {
    /// `(qso_pts, powered_pts)` for this log at the given stored power tier.
    /// `PoweredMultiplier` snaps the tier to a legal value and multiplies;
    /// `Objectives` returns `powered == qso_pts` (no on-air power multiplier).
    pub fn qso_and_powered(&self, log: &FieldDayLog, power_mult: u32) -> (u32, u32) {
        let qso_pts = log.qso_points();
        match self {
            ScoringModel::PoweredMultiplier { power_tiers } => {
                (qso_pts, qso_pts * legal_power(power_tiers, power_mult))
            }
            ScoringModel::Objectives { .. } => (qso_pts, qso_pts),
        }
    }
}

/// The complete rules for one Field Day event in one year — the single source
/// every surface reads.
#[derive(Debug)]
pub struct FdRuleset {
    pub event: FdEvent,
    pub rules_year: u16,
    pub contest_id: &'static str,
    pub exchange: ExchangeSpec,
    pub scoring: ScoringModel,
    pub bonuses: &'static [Bonus],
    pub dupe_rule: DupeRule,
    /// Tempo (FT1 keyboard chat) is a first-class FD contact surface for this
    /// event: WFD `true` (the digital-friendly event), SFD `false`.
    pub tempo_fd: bool,
    /// On-air modes this event's rules BAN outright (uppercase ADIF-style
    /// names). WFD 2026 bans every WSJT mode while explicitly keeping RTTY and
    /// SSTV legal as Digital; ARRL FD bans none. Advisory data for a UI guard —
    /// scoring and dupes never consult it.
    pub banned_modes: &'static [&'static str],
    /// Algorithmic event window for a year (never hand-edited — spec §2.3).
    pub date: fn(u16) -> EventWindow,
}

impl FdRuleset {
    /// Points for one claimed bonus id (`None` = unknown id, scores nothing) —
    /// the moved `fd_bonus_points` semantics.
    pub fn bonus(&self, id: &str) -> Option<u32> {
        self.bonuses.iter().find(|b| b.id == id).map(|b| b.points)
    }

    /// Total points for a set of claimed bonus ids (unknown ids score nothing).
    pub fn bonus_points(&self, claimed: &[String]) -> u32 {
        claimed.iter().filter_map(|id| self.bonus(id)).sum()
    }

    /// True if the ACTUAL on-air mode (e.g. `FT8`, `RTTY`) is banned by this
    /// event's rules (case-insensitive; whitespace trimmed). An empty mode — a
    /// legacy row with no recorded actual mode — is never banned.
    pub fn mode_banned(&self, mode: &str) -> bool {
        let up = mode.trim().to_ascii_uppercase();
        self.banned_modes.iter().any(|&m| m == up)
    }
}

/// The active ruleset for an event + year. Only the 2026 rulesets exist today,
/// so `year` is currently advisory; when a second year's rules land this
/// selects on both.
pub fn ruleset(event: FdEvent, _year: u16) -> &'static FdRuleset {
    match event {
        FdEvent::ArrlFd => &SFD_2026,
        FdEvent::WinterFd => &WFD_2026,
    }
}

/// Snap a stored power multiplier to the highest legal tier ≤ `v` (or the
/// smallest tier). Replaces the engine's old `legal_fd_power` for the ARRL
/// `{1, 2, 5}` tiers — a hand-edited settings file must never score with a
/// ×3/×4 that isn't a real tier.
fn legal_power(tiers: &[u32], v: u32) -> u32 {
    tiers
        .iter()
        .rev()
        .copied()
        .find(|&t| v >= t)
        .unwrap_or_else(|| tiers.first().copied().unwrap_or(1))
}

const EXCHANGE_CLASS_SECTION: ExchangeSpec = ExchangeSpec {
    class_label: "Class",
    section_label: "Section",
};

const DUPE_CALL_BAND_MODE: DupeRule = DupeRule {
    by_call: true,
    by_band: true,
    by_mode_class: true,
};

/// The WFD 2026 banned-mode list: every WSJT mode (the rules ban the whole
/// WSJT-X suite by name while explicitly allowing RTTY and SSTV as Digital).
/// Uppercase ADIF-style names — [`FdRuleset::mode_banned`] compares uppercased.
const WFD_BANNED_MODES: &[&str] = &[
    "FST4", "FT4", "FT8", "JT4", "JT9", "JT65", "Q65", "MSK144", "WSPR", "FST4W", "ECHO",
];

/// The ARRL Field Day bonus menu (id, label, points) — moved verbatim from the
/// old `tempo_app::FD_BONUSES` so scores don't change. WFD reuses this table
/// today (its own bonus specifics aren't modeled yet — see concerns).
pub const FD_BONUSES: &[Bonus] = &[
    Bonus {
        id: "emergency-power",
        label: "100% emergency power",
        points: 100,
    },
    Bonus {
        id: "media-publicity",
        label: "Media publicity",
        points: 100,
    },
    Bonus {
        id: "public-location",
        label: "Public location",
        points: 100,
    },
    Bonus {
        id: "public-info-table",
        label: "Public information table",
        points: 100,
    },
    Bonus {
        id: "nts-message",
        label: "Message to ARRL SM/SEC",
        points: 100,
    },
    Bonus {
        id: "w1aw-bulletin",
        label: "W1AW bulletin copied",
        points: 100,
    },
    Bonus {
        id: "natural-power",
        label: "Natural power QSOs",
        points: 100,
    },
    Bonus {
        id: "site-visit-official",
        label: "Site visit: elected official",
        points: 100,
    },
    Bonus {
        id: "site-visit-agency",
        label: "Site visit: agency representative",
        points: 100,
    },
    Bonus {
        id: "gota",
        label: "GOTA station max",
        points: 100,
    },
    Bonus {
        id: "youth",
        label: "Youth participation",
        points: 100,
    },
    Bonus {
        id: "web-submission",
        label: "Web submission",
        points: 50,
    },
    Bonus {
        id: "safety-officer",
        label: "Safety officer",
        points: 100,
    },
    Bonus {
        id: "social-media",
        label: "Social media",
        points: 100,
    },
    Bonus {
        id: "educational",
        label: "Educational activity",
        points: 100,
    },
];

/// The ARRL/RAC Field Day section master list — the section universe the
/// worked-sections board (spec §5) and setup validation read from. Ordered and
/// grouped by ARRL division (the `division` field) so the board renders one
/// tidy block per division. Rare edits (years apart) happen here + the
/// completeness test; see the spec §7 update model. 71 US ARRL sections + 12
/// RAC (Canada) = 83.
pub const ARRL_SECTIONS: &[Section] = &[
    // Atlantic (New York splits across Atlantic + Hudson).
    Section {
        code: "DE",
        name: "Delaware",
        division: "Atlantic",
    },
    Section {
        code: "EPA",
        name: "Eastern Pennsylvania",
        division: "Atlantic",
    },
    Section {
        code: "MDC",
        name: "Maryland-DC",
        division: "Atlantic",
    },
    Section {
        code: "NNY",
        name: "Northern New York",
        division: "Atlantic",
    },
    Section {
        code: "SNJ",
        name: "Southern New Jersey",
        division: "Atlantic",
    },
    Section {
        code: "WNY",
        name: "Western New York",
        division: "Atlantic",
    },
    Section {
        code: "WPA",
        name: "Western Pennsylvania",
        division: "Atlantic",
    },
    // Central.
    Section {
        code: "IL",
        name: "Illinois",
        division: "Central",
    },
    Section {
        code: "IN",
        name: "Indiana",
        division: "Central",
    },
    Section {
        code: "WI",
        name: "Wisconsin",
        division: "Central",
    },
    // Dakota.
    Section {
        code: "MN",
        name: "Minnesota",
        division: "Dakota",
    },
    Section {
        code: "ND",
        name: "North Dakota",
        division: "Dakota",
    },
    Section {
        code: "SD",
        name: "South Dakota",
        division: "Dakota",
    },
    // Delta.
    Section {
        code: "AR",
        name: "Arkansas",
        division: "Delta",
    },
    Section {
        code: "LA",
        name: "Louisiana",
        division: "Delta",
    },
    Section {
        code: "MS",
        name: "Mississippi",
        division: "Delta",
    },
    Section {
        code: "TN",
        name: "Tennessee",
        division: "Delta",
    },
    // Great Lakes.
    Section {
        code: "KY",
        name: "Kentucky",
        division: "Great Lakes",
    },
    Section {
        code: "MI",
        name: "Michigan",
        division: "Great Lakes",
    },
    Section {
        code: "OH",
        name: "Ohio",
        division: "Great Lakes",
    },
    // Hudson.
    Section {
        code: "ENY",
        name: "Eastern New York",
        division: "Hudson",
    },
    Section {
        code: "NLI",
        name: "New York City-Long Island",
        division: "Hudson",
    },
    Section {
        code: "NNJ",
        name: "Northern New Jersey",
        division: "Hudson",
    },
    // Midwest.
    Section {
        code: "IA",
        name: "Iowa",
        division: "Midwest",
    },
    Section {
        code: "KS",
        name: "Kansas",
        division: "Midwest",
    },
    Section {
        code: "MO",
        name: "Missouri",
        division: "Midwest",
    },
    Section {
        code: "NE",
        name: "Nebraska",
        division: "Midwest",
    },
    // New England.
    Section {
        code: "CT",
        name: "Connecticut",
        division: "New England",
    },
    Section {
        code: "EMA",
        name: "Eastern Massachusetts",
        division: "New England",
    },
    Section {
        code: "ME",
        name: "Maine",
        division: "New England",
    },
    Section {
        code: "NH",
        name: "New Hampshire",
        division: "New England",
    },
    Section {
        code: "RI",
        name: "Rhode Island",
        division: "New England",
    },
    Section {
        code: "VT",
        name: "Vermont",
        division: "New England",
    },
    Section {
        code: "WMA",
        name: "Western Massachusetts",
        division: "New England",
    },
    // Northwestern (Washington splits east/west).
    Section {
        code: "AK",
        name: "Alaska",
        division: "Northwestern",
    },
    Section {
        code: "EWA",
        name: "Eastern Washington",
        division: "Northwestern",
    },
    Section {
        code: "ID",
        name: "Idaho",
        division: "Northwestern",
    },
    Section {
        code: "MT",
        name: "Montana",
        division: "Northwestern",
    },
    Section {
        code: "OR",
        name: "Oregon",
        division: "Northwestern",
    },
    Section {
        code: "WWA",
        name: "Western Washington",
        division: "Northwestern",
    },
    // Pacific (Northern California splits into several sections).
    Section {
        code: "EB",
        name: "East Bay",
        division: "Pacific",
    },
    Section {
        code: "NV",
        name: "Nevada",
        division: "Pacific",
    },
    Section {
        code: "PAC",
        name: "Pacific",
        division: "Pacific",
    },
    Section {
        code: "SCV",
        name: "Santa Clara Valley",
        division: "Pacific",
    },
    Section {
        code: "SF",
        name: "San Francisco",
        division: "Pacific",
    },
    Section {
        code: "SJV",
        name: "San Joaquin Valley",
        division: "Pacific",
    },
    Section {
        code: "SV",
        name: "Sacramento Valley",
        division: "Pacific",
    },
    // Roanoke.
    Section {
        code: "NC",
        name: "North Carolina",
        division: "Roanoke",
    },
    Section {
        code: "SC",
        name: "South Carolina",
        division: "Roanoke",
    },
    Section {
        code: "VA",
        name: "Virginia",
        division: "Roanoke",
    },
    Section {
        code: "WV",
        name: "West Virginia",
        division: "Roanoke",
    },
    // Rocky Mountain.
    Section {
        code: "CO",
        name: "Colorado",
        division: "Rocky Mountain",
    },
    Section {
        code: "NM",
        name: "New Mexico",
        division: "Rocky Mountain",
    },
    Section {
        code: "UT",
        name: "Utah",
        division: "Rocky Mountain",
    },
    Section {
        code: "WY",
        name: "Wyoming",
        division: "Rocky Mountain",
    },
    // Southeastern (Florida splits into three).
    Section {
        code: "AL",
        name: "Alabama",
        division: "Southeastern",
    },
    Section {
        code: "GA",
        name: "Georgia",
        division: "Southeastern",
    },
    Section {
        code: "NFL",
        name: "Northern Florida",
        division: "Southeastern",
    },
    Section {
        code: "PR",
        name: "Puerto Rico",
        division: "Southeastern",
    },
    Section {
        code: "SFL",
        name: "Southern Florida",
        division: "Southeastern",
    },
    Section {
        code: "VI",
        name: "Virgin Islands",
        division: "Southeastern",
    },
    Section {
        code: "WCF",
        name: "West Central Florida",
        division: "Southeastern",
    },
    // Southwestern (Southern California splits into several sections).
    Section {
        code: "AZ",
        name: "Arizona",
        division: "Southwestern",
    },
    Section {
        code: "LAX",
        name: "Los Angeles",
        division: "Southwestern",
    },
    Section {
        code: "ORG",
        name: "Orange",
        division: "Southwestern",
    },
    Section {
        code: "SB",
        name: "Santa Barbara",
        division: "Southwestern",
    },
    Section {
        code: "SDG",
        name: "San Diego",
        division: "Southwestern",
    },
    // West Gulf (Texas splits into three).
    Section {
        code: "NTX",
        name: "North Texas",
        division: "West Gulf",
    },
    Section {
        code: "OK",
        name: "Oklahoma",
        division: "West Gulf",
    },
    Section {
        code: "STX",
        name: "South Texas",
        division: "West Gulf",
    },
    Section {
        code: "WTX",
        name: "West Texas",
        division: "West Gulf",
    },
    // RAC (Canada) — one "division" for board layout.
    Section {
        code: "MAR",
        name: "Maritime",
        division: "RAC",
    },
    Section {
        code: "NL",
        name: "Newfoundland/Labrador",
        division: "RAC",
    },
    Section {
        code: "QC",
        name: "Quebec",
        division: "RAC",
    },
    Section {
        code: "ONE",
        name: "Ontario East",
        division: "RAC",
    },
    Section {
        code: "ONN",
        name: "Ontario North",
        division: "RAC",
    },
    Section {
        code: "ONS",
        name: "Ontario South",
        division: "RAC",
    },
    Section {
        code: "GTA",
        name: "Greater Toronto Area",
        division: "RAC",
    },
    Section {
        code: "MB",
        name: "Manitoba",
        division: "RAC",
    },
    Section {
        code: "SK",
        name: "Saskatchewan",
        division: "RAC",
    },
    Section {
        code: "AB",
        name: "Alberta",
        division: "RAC",
    },
    Section {
        code: "BC",
        name: "British Columbia",
        division: "RAC",
    },
    Section {
        code: "NT",
        name: "Northern Territories",
        division: "RAC",
    },
];

/// True if `code` is a known ARRL/RAC section (case-insensitive; leading/trailing
/// whitespace trimmed). The section universe is [`ARRL_SECTIONS`] — the same list
/// the worked-sections board and setup validation read.
pub fn valid_section(code: &str) -> bool {
    let up = code.trim().to_ascii_uppercase();
    ARRL_SECTIONS.iter().any(|s| s.code == up)
}

/// ARRL Field Day 2026 (the June "SFD" event).
pub static SFD_2026: FdRuleset = FdRuleset {
    event: FdEvent::ArrlFd,
    rules_year: 2026,
    contest_id: "ARRL-FIELD-DAY",
    exchange: EXCHANGE_CLASS_SECTION,
    scoring: ScoringModel::PoweredMultiplier {
        power_tiers: &[1, 2, 5],
    },
    bonuses: FD_BONUSES,
    dupe_rule: DUPE_CALL_BAND_MODE,
    tempo_fd: false,
    banned_modes: &[],
    date: sfd_window,
};

/// Winter Field Day 2026 (the January "WFD" event).
pub static WFD_2026: FdRuleset = FdRuleset {
    event: FdEvent::WinterFd,
    rules_year: 2026,
    contest_id: "WFD",
    exchange: EXCHANGE_CLASS_SECTION,
    scoring: ScoringModel::Objectives {
        multipliers_at_submission: true,
    },
    bonuses: FD_BONUSES,
    dupe_rule: DUPE_CALL_BAND_MODE,
    tempo_fd: true,
    banned_modes: WFD_BANNED_MODES,
    date: wfd_window,
};

// ---- Algorithmic event dates (spec §2.3 — never hand-edited) --------------

/// ARRL Field Day: 4th full weekend of June, 1800Z Saturday (~27 h event).
fn sfd_window(year: u16) -> EventWindow {
    let sat = full_weekend_saturdays(year as i64, 6, 30)[3];
    window(year as i64, 6, sat, 18, 27 * 3600)
}

/// Winter Field Day: last FULL weekend of January (both days in January — the
/// Feb-spill correction). Per the WFD rules the contest period is **30 hours**,
/// 1600Z Saturday through 21:59Z Sunday — the exclusive 2200Z Sunday end here
/// matches the official 21:59 close. (The old 24 h duration dropped QSOs made
/// in the final six hours out of the app window.)
fn wfd_window(year: u16) -> EventWindow {
    let sats = full_weekend_saturdays(year as i64, 1, 31);
    let sat = *sats.last().expect("January always has a full weekend");
    window(year as i64, 1, sat, 16, 30 * 3600)
}

const SATURDAY: i64 = 6; // 0 = Sunday … 6 = Saturday

/// Saturdays of `month` whose Saturday+Sunday both fall within `[1, last_day]`.
fn full_weekend_saturdays(year: i64, month: u32, last_day: u32) -> Vec<u32> {
    (1..=last_day)
        .filter(|&d| d < last_day && weekday(days_from_civil(year, month, d)) == SATURDAY)
        .collect()
}

fn window(year: i64, month: u32, sat_day: u32, start_hour: u64, dur_secs: u64) -> EventWindow {
    let start = days_from_civil(year, month, sat_day) as u64 * 86_400 + start_hour * 3600;
    EventWindow {
        start_unix: start,
        end_unix: start + dur_secs,
    }
}

/// Weekday (0 = Sunday … 6 = Saturday) of a day count since the Unix epoch
/// (1970-01-01 was a Thursday).
fn weekday(days: i64) -> i64 {
    (days.rem_euclid(7) + 4) % 7
}

/// Days since 1970-01-01 for a civil UTC date (Howard Hinnant's algorithm;
/// mirrors `fieldday::unix_from_ymdhms` — no date crate needed).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fieldday::{Exchange, FieldDayLog};

    /// Build a log from `(call, mode-class)` pairs — distinct calls so nothing
    /// dupes; class/section are constant (irrelevant to the point math).
    fn log_with(contacts: &[(&str, &str)]) -> FieldDayLog {
        let mut log = FieldDayLog::new("W9XYZ", Exchange::new("3A", "WI"), "20m");
        for (i, (call, mode)) in contacts.iter().enumerate() {
            assert!(log.log_mode_at(call, "2A", "IL", mode, 0, 100 + i as u64));
        }
        log
    }

    #[test]
    fn sfd_pinned_score_10_qsos_x2_power_w1aw_bonus() {
        // 10 distinct QSOs: 4 phone (1 pt) + 3 CW + 3 DIG (2 pt) = 16 QSO pts.
        let log = log_with(&[
            ("PH1AA", "PH"),
            ("PH2AA", "PH"),
            ("PH3AA", "PH"),
            ("PH4AA", "PH"),
            ("CW1AA", "CW"),
            ("CW2AA", "CW"),
            ("CW3AA", "CW"),
            ("DG1AA", "DIG"),
            ("DG2AA", "DIG"),
            ("DG3AA", "DIG"),
        ]);
        assert_eq!(log.qso_count(), 10);
        let rs = ruleset(FdEvent::ArrlFd, CURRENT_RULES_YEAR);
        let (qso_pts, powered) = rs.scoring.qso_and_powered(&log, 2);
        assert_eq!(qso_pts, 16, "4×1 + 6×2");
        assert_eq!(powered, 32, "16 QSO pts × ×2 power tier");
        let bonus = rs.bonus_points(&["w1aw-bulletin".to_string()]);
        assert_eq!(bonus, 100, "the W1AW-bulletin bonus");
        assert_eq!(powered + bonus, 132, "the score-board total");
    }

    #[test]
    fn wfd_scores_raw_qso_points_regardless_of_power() {
        // Winter FD is QSOs × (objectives+1); with no objective values in the
        // code the on-air total is RAW QSO points — no ARRL power multiplier,
        // even at the ×5 tier — and it flags multipliers-at-submission.
        let log = log_with(&[("K1ABC", "CW"), ("W1AW", "PH"), ("N0XYZ", "DIG")]);
        assert_eq!(log.qso_points(), 2 + 1 + 2);
        let rs = ruleset(FdEvent::WinterFd, CURRENT_RULES_YEAR);
        let (qso_pts, powered) = rs.scoring.qso_and_powered(&log, 5);
        assert_eq!((qso_pts, powered), (5, 5), "raw points, power tier ignored");
        assert!(matches!(
            rs.scoring,
            ScoringModel::Objectives {
                multipliers_at_submission: true
            }
        ));
    }

    #[test]
    fn tempo_fd_is_wfd_only() {
        assert!(
            ruleset(FdEvent::WinterFd, 2026).tempo_fd,
            "WFD is a Tempo FD event"
        );
        assert!(!ruleset(FdEvent::ArrlFd, 2026).tempo_fd, "SFD is not");
    }

    #[test]
    fn contest_ids_match_the_event() {
        assert_eq!(ruleset(FdEvent::ArrlFd, 2026).contest_id, "ARRL-FIELD-DAY");
        assert_eq!(ruleset(FdEvent::WinterFd, 2026).contest_id, "WFD");
        // The ruleset id must never drift from the export id.
        for e in [FdEvent::ArrlFd, FdEvent::WinterFd] {
            assert_eq!(ruleset(e, 2026).contest_id, e.contest_id());
        }
    }

    #[test]
    fn legal_power_snaps_to_arrl_tiers() {
        let tiers = &[1u32, 2, 5][..];
        // Matches the engine's old `legal_fd_power`: ≥5→5, ≥2→2, else 1.
        for (v, want) in [
            (0, 1),
            (1, 1),
            (2, 2),
            (3, 2),
            (4, 2),
            (5, 5),
            (6, 5),
            (150, 5),
        ] {
            assert_eq!(legal_power(tiers, v), want, "power {v}");
        }
    }

    #[test]
    fn bonus_lookup_matches_the_old_table_semantics() {
        let rs = ruleset(FdEvent::ArrlFd, 2026);
        assert_eq!(rs.bonus("w1aw-bulletin"), Some(100));
        assert_eq!(rs.bonus("web-submission"), Some(50));
        assert_eq!(rs.bonus("not-a-bonus"), None, "unknown id scores nothing");
        assert_eq!(
            rs.bonus_points(&[
                "w1aw-bulletin".into(),
                "web-submission".into(),
                "junk".into()
            ]),
            150,
        );
        assert_eq!(FD_BONUSES.len(), 15, "the full ARRL bonus menu");
    }

    #[test]
    fn arrl_sections_are_complete_and_unique() {
        use std::collections::HashSet;
        // 71 US ARRL sections + 12 RAC = the full ~85-section universe.
        assert_eq!(ARRL_SECTIONS.len(), 83, "the ARRL/RAC section master list");
        // No duplicate codes (a copy-paste slip would double-count a section).
        let codes: HashSet<&str> = ARRL_SECTIONS.iter().map(|s| s.code).collect();
        assert_eq!(codes.len(), ARRL_SECTIONS.len(), "section codes are unique");
        // Codes are stored canonically (uppercase, non-empty) and every section
        // names a division so the board can group it.
        for s in ARRL_SECTIONS {
            assert!(!s.code.is_empty() && s.code == s.code.to_ascii_uppercase());
            assert!(!s.name.is_empty() && !s.division.is_empty(), "{}", s.code);
        }
        // Spot-check the tricky split-state + RAC entries the spec calls out.
        for code in [
            "EMA", "WMA", "STX", "NTX", "WTX", "SDG", "ORG", "SCV", "NNY", "GTA", "NT",
        ] {
            assert!(codes.contains(code), "missing section {code}");
        }
    }

    #[test]
    fn valid_section_accepts_known_case_insensitively_and_rejects_junk() {
        assert!(valid_section("WI"));
        assert!(valid_section("wi"), "case-insensitive");
        assert!(valid_section("  eMa "), "trims + case-insensitive");
        assert!(valid_section("ONS"), "a RAC section");
        assert!(!valid_section("ZZ"), "not a section");
        assert!(!valid_section(""), "empty is not a section");
        assert!(!valid_section("WISCONSIN"), "the name is not the code");
    }

    #[test]
    fn event_windows_are_algorithmic_and_dodge_the_feb_spill() {
        // 4th full weekend of June 2026 = the 27th (Sat) at 1800Z.
        assert_eq!(full_weekend_saturdays(2026, 6, 30), vec![6, 13, 20, 27]);
        let sfd = sfd_window(2026);
        assert_eq!(sfd.start_unix % 86_400, 18 * 3600, "1800Z start");
        // Last FULL weekend of January 2026 = the 24th, NOT the 31st (whose
        // Sunday spills into February).
        assert_eq!(full_weekend_saturdays(2026, 1, 31), vec![3, 10, 17, 24]);
        let wfd = wfd_window(2026);
        assert_eq!(wfd.start_unix % 86_400, 16 * 3600, "1600Z start");
        // WFD is a 30-HOUR event (1600Z Sat → 21:59Z Sun); the old 24 h window
        // dropped the final six hours. Exclusive 2200Z Sunday end = 21:59 close.
        assert_eq!(
            wfd.end_unix - wfd.start_unix,
            30 * 3600,
            "30-hour WFD period"
        );
        assert_eq!(wfd.end_unix % 86_400, 22 * 3600, "2200Z Sunday end");
    }

    #[test]
    fn wfd_bans_wsjt_modes_but_never_rtty_or_sstv() {
        let wfd = ruleset(FdEvent::WinterFd, 2026);
        // The whole WSJT suite is out at WFD 2026…
        for m in ["FT8", "FT4", "FST4", "JT65", "Q65", "MSK144", "WSPR"] {
            assert!(wfd.mode_banned(m), "{m} is banned at WFD");
        }
        assert!(wfd.mode_banned(" ft8 "), "case-insensitive + trimmed");
        // …while RTTY and SSTV are explicitly legal Digital, and the classic
        // mode classes are untouched. A legacy row with no recorded actual
        // mode is never flagged.
        for m in ["RTTY", "SSTV", "CW", "SSB", ""] {
            assert!(!wfd.mode_banned(m), "{m:?} is not banned at WFD");
        }
        // ARRL FD bans nothing.
        let sfd = ruleset(FdEvent::ArrlFd, 2026);
        assert!(sfd.banned_modes.is_empty());
        assert!(!sfd.mode_banned("FT8"));
    }
}
