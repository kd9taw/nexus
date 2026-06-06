//! Gamification achievements — milestone unlocks derived from award progress.
//!
//! Pure: a fixed catalog evaluated against [`AchievementStats`] (a subset of the
//! [`AwardSummary`](crate::AwardSummary)). Each achievement carries its progress
//! (`current` / `target`) so the UI can show "almost there" nudges, and a
//! `critical` flag — the **only** ones the app celebrates with a toast. The
//! frontend owns the non-chatty policy (baseline the already-unlocked set
//! silently on first run; toast only newly-unlocked criticals).

use serde::{Deserialize, Serialize};

/// One milestone the operator can unlock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Achievement {
    /// Stable id (e.g. `"dxcc-100"`) — the key the UI tracks "seen" by.
    pub id: String,
    pub title: String,
    pub detail: String,
    /// Grouping label: "QSOs" | "DXCC" | "Challenge".
    pub category: String,
    pub unlocked: bool,
    /// Progress toward `target` (the live stat, e.g. 87 confirmed entities).
    pub current: u32,
    pub target: u32,
    /// Celebrate with a toast when newly unlocked (a big moment). Non-critical
    /// achievements accrue silently in the dashboard.
    pub critical: bool,
}

/// The award stats achievements are evaluated against (subset of `AwardSummary`).
pub struct AchievementStats {
    pub qsos: u32,
    pub confirmed_qsos: u32,
    pub dxcc_worked: u32,
    pub dxcc_confirmed: u32,
    pub slots_confirmed: u32,
    /// Most-wanted (DXpedition-grade) entities worked — entities you essentially
    /// only get via a DXpedition (Bouvet, P5, Scarborough Reef, …).
    pub rare_worked: u32,
    /// CQ zones confirmed (toward Worked All Zones — 40 zones).
    pub zones_confirmed: u32,
}

fn mk(
    id: &str,
    title: &str,
    detail: &str,
    category: &str,
    current: u32,
    target: u32,
    critical: bool,
) -> Achievement {
    Achievement {
        id: id.to_string(),
        title: title.to_string(),
        detail: detail.to_string(),
        category: category.to_string(),
        unlocked: current >= target,
        current,
        target,
        critical,
    }
}

/// Evaluate the full achievement catalog against the current stats. The order is
/// the display order (within each category, ascending difficulty).
pub fn evaluate(s: &AchievementStats) -> Vec<Achievement> {
    let q = s.qsos;
    let dw = s.dxcc_worked;
    let dc = s.dxcc_confirmed;
    let sl = s.slots_confirmed;
    let rw = s.rare_worked;
    let zc = s.zones_confirmed;
    vec![
        // --- QSOs ---
        mk(
            "qso-1",
            "First Contact",
            "Log your first QSO",
            "QSOs",
            q,
            1,
            true,
        ),
        mk(
            "qso-10",
            "Getting Going",
            "10 QSOs in the log",
            "QSOs",
            q,
            10,
            false,
        ),
        mk(
            "qso-100",
            "Century",
            "100 QSOs logged",
            "QSOs",
            q,
            100,
            false,
        ),
        mk(
            "qso-1000",
            "Worked the World",
            "1,000 QSOs logged",
            "QSOs",
            q,
            1000,
            true,
        ),
        // --- DXCC (worked entities) ---
        mk(
            "dx-first",
            "First DX",
            "Work your first DX entity",
            "DXCC",
            dw,
            2,
            true,
        ),
        // --- DXpeditions (most-wanted, DXpedition-only entities) ---
        mk(
            "rare-1",
            "DXpedition Contact",
            "Work a most-wanted DXCC entity",
            "DXpeditions",
            rw,
            1,
            true,
        ),
        mk(
            "rare-5",
            "DXpedition Hunter",
            "Work 5 most-wanted entities",
            "DXpeditions",
            rw,
            5,
            false,
        ),
        mk(
            "dx-25",
            "Globetrotter",
            "25 entities worked",
            "DXCC",
            dw,
            25,
            false,
        ),
        mk(
            "dx-50",
            "Half-Century DX",
            "50 entities worked",
            "DXCC",
            dw,
            50,
            false,
        ),
        // --- DXCC (confirmed — the award) ---
        mk(
            "cfm-1",
            "First Confirmation",
            "Confirm your first entity",
            "DXCC",
            dc,
            1,
            false,
        ),
        mk(
            "dxcc-100",
            "DXCC",
            "100 confirmed entities — the DXCC award!",
            "DXCC",
            dc,
            100,
            true,
        ),
        // --- DXCC Challenge (confirmed band slots) ---
        mk(
            "chal-100",
            "Slot Collector",
            "100 confirmed band slots",
            "Challenge",
            sl,
            100,
            false,
        ),
        mk(
            "chal-500",
            "Slot Hunter",
            "500 confirmed band slots",
            "Challenge",
            sl,
            500,
            false,
        ),
        mk(
            "chal-1000",
            "DXCC Challenge",
            "1,000 confirmed slots — the Challenge!",
            "Challenge",
            sl,
            1000,
            true,
        ),
        // --- CQ WAZ (confirmed CQ zones, out of 40) ---
        mk(
            "waz-half",
            "Zone Collector",
            "Confirm 20 CQ zones",
            "WAZ",
            zc,
            20,
            false,
        ),
        mk(
            "waz-40",
            "Worked All Zones",
            "Confirm all 40 CQ zones — the WAZ award!",
            "WAZ",
            zc,
            40,
            true,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_milestones_progress_and_critical() {
        let s = AchievementStats {
            qsos: 120,
            confirmed_qsos: 90,
            dxcc_worked: 60,
            dxcc_confirmed: 45,
            slots_confirmed: 150,
            rare_worked: 3,
            zones_confirmed: 22,
        };
        let a = evaluate(&s);
        let by = |id: &str| a.iter().find(|x| x.id == id).unwrap();

        // unlocked / locked thresholds
        assert!(by("qso-100").unlocked && !by("qso-1000").unlocked);
        assert!(by("dx-50").unlocked && !by("dxcc-100").unlocked);
        assert!(by("chal-100").unlocked && !by("chal-500").unlocked);
        assert!(by("rare-1").unlocked && by("rare-1").critical && !by("rare-5").unlocked);
        assert!(by("waz-half").unlocked && !by("waz-40").unlocked);
        assert!(by("waz-40").critical && !by("waz-half").critical);

        // critical flags = the big moments only
        assert!(by("qso-1").critical && by("dxcc-100").critical && by("chal-1000").critical);
        assert!(!by("qso-10").critical && !by("chal-100").critical);

        // progress passthrough for the nudges
        assert_eq!(by("dxcc-100").current, 45);
        assert_eq!(by("dxcc-100").target, 100);
    }

    #[test]
    fn empty_log_unlocks_nothing() {
        let a = evaluate(&AchievementStats {
            qsos: 0,
            confirmed_qsos: 0,
            dxcc_worked: 0,
            dxcc_confirmed: 0,
            slots_confirmed: 0,
            rare_worked: 0,
            zones_confirmed: 0,
        });
        assert!(a.iter().all(|x| !x.unlocked));
        assert!(!a.is_empty(), "catalog still lists locked achievements");
    }
}
