//! PSK Reporter **MQTT firehose** semantics — the live "who hears me / who I
//! hear" upgrade to the rate-limited XML query ([`crate::live::pskreporter`]).
//!
//! Pure (no network): the subscription topic filters + a parser that turns a
//! `pskr/filter/v2/...` topic into a [`PathSpot`] the advisor already consumes.
//! The MQTT transport itself is `tempo_net::mqtt`. The topic carries band / mode
//! / calls / locators, so we score paths from the topic alone (SNR + exact
//! frequency live only in the JSON payload, which the advisor doesn't need).
//!
//! Topic layout (verified against the official mqtt.pskreporter.info `v2` feed —
//! 11 segments; trailing fields are ADIF **DXCC numbers**, NOT regions; the
//! **frequency is NOT in the topic** — it's payload-only):
//! `pskr/filter/v2/{band}/{mode}/{txCall}/{rxCall}/{txGrid}/{rxGrid}/{txDxcc}/{rxDxcc}`

use crate::model::{Band, PathSpot};
use std::collections::VecDeque;

/// A bounded buffer of recent live PSK Reporter [`PathSpot`]s (from the MQTT
/// firehose), merged into the propagation advisory. No dedup — the advisor
/// dedupes "who hears me / who I hear" by callsign via sets.
///
/// Eviction is by COUNT (oldest-first), but [`Self::recent`] reads by TIME. The
/// cap is sized so that the read window stays fully covered even for a very
/// well-heard station during a contest/opening: the firehose only carries the
/// operator's OWN two topic filters (who-hears-me + who-I-hear), so a worst-case
/// of a few thousand reports per 30 min fits inside the cap with margin. At
/// ~100 B/spot the buffer is well under ~3 MB, so we keep it generous rather
/// than letting a count cap silently truncate the requested time window.
#[derive(Debug, Clone)]
pub struct LiveSpots {
    spots: VecDeque<PathSpot>,
    cap: usize,
}

impl Default for LiveSpots {
    fn default() -> Self {
        Self::new(20_000)
    }
}

impl LiveSpots {
    pub fn new(cap: usize) -> Self {
        Self {
            spots: VecDeque::new(),
            cap: cap.max(1),
        }
    }
    pub fn push(&mut self, spot: PathSpot) {
        self.spots.push_back(spot);
        while self.spots.len() > self.cap {
            self.spots.pop_front();
        }
    }
    /// Spots no older than `window_secs` as of `now` (Unix secs).
    pub fn recent(&self, now: i64, window_secs: i64) -> Vec<PathSpot> {
        let cutoff = now - window_secs;
        self.spots
            .iter()
            .filter(|s| s.time >= cutoff)
            .cloned()
            .collect()
    }
    pub fn len(&self) -> usize {
        self.spots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }
    /// Evict spots older than `cutoff` (Unix secs) from the front. The regional
    /// opening buffer calls this on push so a wide opening can't push the baseline
    /// window out via the count cap (which would evict quiet baseline bins ahead of
    /// the hot ones and manufacture a false-normal). The count cap then serves only
    /// as a hard memory backstop.
    pub fn trim_older_than(&mut self, cutoff: i64) {
        while let Some(front) = self.spots.front() {
            if front.time < cutoff {
                self.spots.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Buffer cap for the near-region opening firehose (Phase 2) — larger than the
/// own-call default since a `{band}/#` stream during an opening is much wider.
/// Backstop only; `LiveSpots::trim_older_than` is the primary (time-based) evictor.
pub const REGION_SPOT_CAP: usize = 60_000;

/// MQTT topic filters for the operator's own paths: "who hears me" (we're the
/// sender) and "who I hear" (we're the receiver). `#` matches the trailing topic
/// levels so it's robust to PSK Reporter schema tweaks.
pub fn mqtt_topics(mycall: &str) -> Vec<String> {
    let c = mycall.trim().to_ascii_uppercase();
    vec![
        format!("pskr/filter/v2/+/+/{c}/#"), // sender == me  → who heard me
        format!("pskr/filter/v2/+/+/+/{c}/#"), // receiver == me → who I hear
    ]
}

/// MQTT topic filters for the near-region opening bands (Phase 2): the per-band
/// global stream `pskr/filter/v2/{band}/#` (band fixed, everything after it
/// wildcarded — the broadest broker-side filter that still isolates a band, since
/// grids can't be prefix-matched at the topic level). The caller narrows to "near
/// me" client-side (grid distance) and drops own-call spots. VHF + 10 m only (the
/// Es/opening bands, self-throttling); HF F2 stays own-call-only to bound volume.
pub fn region_topics() -> Vec<String> {
    ["10m", "6m", "4m", "2m"]
        .iter()
        .map(|b| format!("pskr/filter/v2/{b}/#"))
        .collect()
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Parse a PSK Reporter MQTT topic into a [`PathSpot`] stamped `now` (Unix secs).
/// `None` if it isn't a `pskr/filter/v2` reception report on a band we model.
/// (SNR + exact frequency are payload-only, so SNR is left `None`; the band comes
/// from the topic's band-label segment and the advisor treats SNR as optional.)
pub fn parse_mqtt_report(topic: &str, now: i64) -> Option<PathSpot> {
    let p: Vec<&str> = topic.split('/').collect();
    if p.len() < 11 || p[0] != "pskr" || p[1] != "filter" || p[2] != "v2" {
        return None;
    }
    let band = Band::from_label(p[3])?; // band label, e.g. "20m"
    let mode = p[4];
    let sender = p[5];
    let receiver = p[6];
    // A real published topic carries concrete calls, never the +/# wildcards.
    if sender.is_empty()
        || receiver.is_empty()
        || sender.contains(['+', '#'])
        || receiver.contains(['+', '#'])
    {
        return None;
    }
    Some(PathSpot {
        time: now,
        tx_call: sender.to_ascii_uppercase(),
        tx_grid: non_empty(p[7]),
        rx_call: receiver.to_ascii_uppercase(),
        rx_grid: non_empty(p[8]),
        band,
        mode: non_empty(mode),
        snr: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topics_cover_both_directions() {
        let t = mqtt_topics("kd9taw");
        assert_eq!(t[0], "pskr/filter/v2/+/+/KD9TAW/#"); // who heard me (sender slot)
        assert_eq!(t[1], "pskr/filter/v2/+/+/+/KD9TAW/#"); // who I hear (receiver slot)
    }

    #[test]
    fn region_topics_are_per_band_streams() {
        let t = region_topics();
        assert!(t.contains(&"pskr/filter/v2/6m/#".to_string()));
        assert!(t
            .iter()
            .all(|s| s.starts_with("pskr/filter/v2/") && s.ends_with("/#")));
        assert!(
            !t.iter().any(|s| s.contains("20m")),
            "HF stays own-call-only"
        );
    }

    #[test]
    fn trim_older_than_evicts_only_out_of_window() {
        let mut buf = LiveSpots::new(100);
        let mk = |t: i64| PathSpot {
            time: t,
            tx_call: "A".into(),
            tx_grid: Some("FN42".into()),
            rx_call: "B".into(),
            rx_grid: Some("FN31".into()),
            band: Band::B6,
            mode: None,
            snr: None,
        };
        for k in 0..10i64 {
            buf.push(mk(1_000 + k * 600)); // 1000, 1600, ..., 6400 (oldest first)
        }
        buf.trim_older_than(4_000); // drop everything before t=4000
        let kept = buf.recent(10_000, 100_000);
        assert!(kept.iter().all(|s| s.time >= 4_000), "only in-window kept");
        assert_eq!(kept.len(), 5, "t = 4000,4600,5200,5800,6400");
    }

    #[test]
    fn parses_a_reception_report_topic() {
        // Real v2 layout: …/{band}/{mode}/{tx}/{rx}/{txGrid}/{rxGrid}/{txDxcc}/{rxDxcc}
        // (trailing fields are ADIF DXCC numbers; no frequency segment).
        let topic = "pskr/filter/v2/20m/FT8/W1AW/JA1XYZ/FN31/PM95/291/339";
        let s = parse_mqtt_report(topic, 1_700_000_000).unwrap();
        assert_eq!(s.tx_call, "W1AW");
        assert_eq!(s.rx_call, "JA1XYZ");
        assert_eq!(s.tx_grid.as_deref(), Some("FN31"));
        assert_eq!(s.rx_grid.as_deref(), Some("PM95"));
        assert_eq!(s.band, Band::B20); // from the "20m" label segment
        assert_eq!(s.mode.as_deref(), Some("FT8"));
        assert_eq!(s.time, 1_700_000_000);
    }

    #[test]
    fn rejects_non_pskr_or_malformed_topics() {
        assert!(parse_mqtt_report("foo/bar/baz", 0).is_none());
        assert!(parse_mqtt_report("pskr/filter/v2/20m/FT8/W1AW", 0).is_none()); // too short
                                                                                // unknown band label → not a band we model.
        assert!(
            parse_mqtt_report("pskr/filter/v2/zz/FT8/W1AW/JA1XYZ/FN31/PM95/291/339", 0).is_none()
        );
    }

    #[test]
    fn live_spots_keeps_recent_and_caps() {
        let mut b = LiveSpots::new(3);
        let mk = |call: &str, t: i64| PathSpot {
            time: t,
            tx_call: call.into(),
            tx_grid: None,
            rx_call: "ME".into(),
            rx_grid: None,
            band: Band::B20,
            mode: Some("FT8".into()),
            snr: None,
        };
        b.push(mk("A", 1000)); // old
        b.push(mk("B", 5000));
        b.push(mk("C", 5001));
        // window: as of t=5500, keep within 1000s → drops A(1000).
        let recent = b.recent(5500, 1000);
        let calls: Vec<&str> = recent.iter().map(|s| s.tx_call.as_str()).collect();
        assert_eq!(calls, vec!["B", "C"]);
        // cap 3: a 4th push evicts the oldest (A).
        b.push(mk("D", 5002));
        assert_eq!(b.len(), 3);
        assert!(b.recent(0, i64::MAX).iter().all(|s| s.tx_call != "A"));
    }

    #[test]
    fn empty_locator_becomes_none() {
        let topic = "pskr/filter/v2/40m/CW/DL1ABC/W1AW///230/291";
        let s = parse_mqtt_report(topic, 0).unwrap();
        assert_eq!(s.tx_call, "DL1ABC");
        assert!(s.tx_grid.is_none());
        assert!(s.rx_grid.is_none());
        assert_eq!(s.band, Band::B40);
    }
}
