//! WA7BNM Contest Calendar (RSS) adapter (the `live` feature) — the upcoming
//! HF/VHF contest schedule for the Connect "Contests" pane.
//!
//! Two halves, mirroring the other `live` modules:
//!  - a pure [`parse_contest_rss`] that turns the feed into [`ContestEvent`]s
//!    (fully deterministic + unit-testable offline), and
//!  - a thin [`fetch`] HTTPS transport.
//!
//! Each `<item>` carries a `<title>` (contest name), a `<link>`, and a
//! `<description>` holding the UTC time range with **no year**, e.g.
//!   `0000Z-0200Z, Jul 7`             (single-day, dash-separated times), or
//!   `1200Z, Jul 11 to 1200Z, Jul 12` (multi-day range, " to "-separated).
//!
//! The year is taken from the channel's `<lastBuildDate>` (RFC-822), rolling to
//! the next year when a listed month is earlier than the build month (the
//! Dec→Jan boundary). With no `<lastBuildDate>` there is no year anchor, so items
//! are skipped rather than fabricated — the command layer then falls back to its
//! cache or an honest empty state.

use std::time::Duration;

use quick_xml::events::{BytesCData, BytesRef, BytesText, Event};
use quick_xml::Reader;

use crate::geo::days_from_civil;

const UA: &str = "nexus-propagation/0.1 (+ham radio propagation nowcast)";

/// One upcoming contest from the WA7BNM calendar. `start_unix`/`end_unix` are the
/// UTC window; `url` is the detail page (`None` when the item carried no `<link>`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContestEvent {
    pub name: String,
    pub start_unix: i64,
    pub end_unix: i64,
    pub url: Option<String>,
}

/// Fetch the WA7BNM Contest Calendar RSS. The URL carries no secrets, so errors
/// surface the (safe) transport message. HTTPS enforced; 20 s timeout like peers.
pub fn fetch(url: &str) -> Result<String, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(UA)
        .https_only(true)
        .build()
        .map_err(|e| e.to_string())?;
    c.get(url)
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())
}

/// Parse the WA7BNM RSS body into upcoming contests. Malformed XML yields the
/// events parsed so far (never panics); an item is skipped when its title is
/// empty, its date range can't be parsed, or the feed carried no `<lastBuildDate>`
/// (no year anchor).
pub fn parse_contest_rss(xml: &str) -> Vec<ContestEvent> {
    let mut reader = Reader::from_str(xml);
    let mut buf = Vec::new();
    let mut out = Vec::new();

    // (year, month) from the channel `<lastBuildDate>`; set once, before the items.
    let mut ref_ym: Option<(i64, u32)> = None;
    let mut in_item = false;
    let mut tag: Vec<u8> = Vec::new();
    let (mut title, mut desc, mut link) = (String::new(), String::new(), String::new());

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                tag = e.name().as_ref().to_vec();
                if tag == b"item" {
                    in_item = true;
                    title.clear();
                    desc.clear();
                    link.clear();
                }
            }
            Ok(Event::Text(e)) => {
                capture(
                    in_item,
                    &tag,
                    &text_of(&e),
                    &mut title,
                    &mut desc,
                    &mut link,
                    &mut ref_ym,
                );
            }
            Ok(Event::CData(e)) => {
                capture(
                    in_item,
                    &tag,
                    &cdata_of(&e),
                    &mut title,
                    &mut desc,
                    &mut link,
                    &mut ref_ym,
                );
            }
            // quick-xml emits entity references (`&amp;`, `&#38;`) as their own
            // event between the surrounding text — resolve and route them so a
            // title like "Snowball &amp; Sprint" keeps its "&".
            Ok(Event::GeneralRef(e)) => {
                capture(
                    in_item,
                    &tag,
                    &ref_to_str(&e),
                    &mut title,
                    &mut desc,
                    &mut link,
                    &mut ref_ym,
                );
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"item" {
                    in_item = false;
                    if let Some(ev) = build_event(&title, &desc, &link, ref_ym) {
                        out.push(ev);
                    }
                }
                tag.clear();
            }
            // Malformed XML or EOF: return what we have — never panic.
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Route a text/CDATA node into the current field (inside an item) or capture the
/// channel `<lastBuildDate>` year anchor (outside one).
#[allow(clippy::too_many_arguments)]
fn capture(
    in_item: bool,
    tag: &[u8],
    text: &str,
    title: &mut String,
    desc: &mut String,
    link: &mut String,
    ref_ym: &mut Option<(i64, u32)>,
) {
    if in_item {
        match tag {
            b"title" => title.push_str(text),
            b"description" => desc.push_str(text),
            b"link" => link.push_str(text),
            _ => {}
        }
    } else if tag == b"lastBuildDate" && ref_ym.is_none() {
        *ref_ym = parse_rfc822_ym(text);
    }
}

/// Decode a text node. Entities arrive as separate [`Event::GeneralRef`] events
/// (see the loop), so a `Text` node never carries a raw `&amp;` to unescape here.
fn text_of(e: &BytesText) -> String {
    e.decode().map(|s| s.into_owned()).unwrap_or_default()
}

/// Decode a CDATA node (its content is literal — no entity unescaping needed).
fn cdata_of(e: &BytesCData) -> String {
    e.decode().map(|s| s.into_owned()).unwrap_or_default()
}

/// Resolve an entity reference to its text: numeric char refs (`&#38;`, `&#x26;`)
/// directly, then the five predefined XML entities. Unknown named entities (rare
/// in contest titles) resolve to empty.
fn ref_to_str(e: &BytesRef) -> String {
    if let Ok(Some(c)) = e.resolve_char_ref() {
        return c.to_string();
    }
    match e.decode().as_deref() {
        Ok("amp") => "&",
        Ok("lt") => "<",
        Ok("gt") => ">",
        Ok("quot") => "\"",
        Ok("apos") => "'",
        _ => "",
    }
    .to_string()
}

fn build_event(
    title: &str,
    desc: &str,
    link: &str,
    ref_ym: Option<(i64, u32)>,
) -> Option<ContestEvent> {
    let name = title.trim();
    if name.is_empty() {
        return None;
    }
    let (ref_year, ref_month) = ref_ym?; // no anchor → can't resolve the year → skip
    let (start_unix, end_unix) = parse_window(desc.trim(), ref_year, ref_month)?;
    let link = link.trim();
    Some(ContestEvent {
        name: name.to_string(),
        start_unix,
        end_unix,
        url: (!link.is_empty()).then(|| link.to_string()),
    })
}

/// Parse a `<description>` window into `(start_unix, end_unix)`. Handles the two
/// WA7BNM shapes; returns `None` for anything else.
fn parse_window(desc: &str, ref_year: i64, ref_month: u32) -> Option<(i64, i64)> {
    if let Some((left, right)) = desc.split_once(" to ") {
        // Multi-day: "1200Z, Jul 11 to 1200Z, Jul 12".
        let start = parse_point(left, ref_year, ref_month)?;
        let end = parse_point(right, ref_year, ref_month)?;
        Some((start, end))
    } else {
        // Single-day: "0000Z-0200Z, Jul 7" — one date, both times share it.
        let (time_part, date_part) = desc.split_once(", ")?;
        let (t1, t2) = time_part.split_once('-')?;
        let (y, m, d) = resolve_date(date_part, ref_year, ref_month)?;
        Some((
            to_unix(y, m, d, parse_hhmm(t1)?),
            to_unix(y, m, d, parse_hhmm(t2)?),
        ))
    }
}

/// Parse one "TIME, DATE" point like "1200Z, Jul 11" into a Unix timestamp.
fn parse_point(s: &str, ref_year: i64, ref_month: u32) -> Option<i64> {
    let (time, date) = s.trim().split_once(", ")?;
    let secs = parse_hhmm(time.trim())?;
    let (y, m, d) = resolve_date(date.trim(), ref_year, ref_month)?;
    Some(to_unix(y, m, d, secs))
}

/// "Jul 11" → (year, month, day). The feed omits the year: rolling to the next
/// year when the listed month precedes the build month covers the Dec→Jan wrap.
fn resolve_date(s: &str, ref_year: i64, ref_month: u32) -> Option<(i64, u32, u32)> {
    let mut it = s.split_whitespace();
    let month = month_num(it.next()?)?;
    let day: u32 = it.next()?.parse().ok()?;
    if !(1..=31).contains(&day) {
        return None;
    }
    let year = if month < ref_month {
        ref_year + 1
    } else {
        ref_year
    };
    Some((year, month, day))
}

/// "1200Z" → seconds into the day (0..=86_400). `2400Z` (end-of-day) maps to
/// 86_400, which [`to_unix`] carries into the next midnight arithmetically.
fn parse_hhmm(s: &str) -> Option<i64> {
    let s = s.trim().trim_end_matches(['Z', 'z']);
    if s.len() != 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i64 = s[..2].parse().ok()?;
    let m: i64 = s[2..].parse().ok()?;
    if h > 24 || m > 59 {
        return None;
    }
    Some(h * 3600 + m * 60)
}

fn to_unix(y: i64, m: u32, d: u32, secs_of_day: i64) -> i64 {
    days_from_civil(y, m, d) * 86_400 + secs_of_day
}

/// Parse the (year, month) out of an RFC-822 date like
/// "Tue, 07 Jul 2026 00:00:00 +0000" — scanning for the first month name and the
/// first 4-digit year token, so a missing weekday doesn't shift the fields.
fn parse_rfc822_ym(s: &str) -> Option<(i64, u32)> {
    let mut year = None;
    let mut month = None;
    for tok in s.split_whitespace() {
        if month.is_none() {
            if let Some(m) = month_num(tok) {
                month = Some(m);
                continue;
            }
        }
        if year.is_none() {
            if let Ok(y) = tok.parse::<i64>() {
                if (2000..2100).contains(&y) {
                    year = Some(y);
                }
            }
        }
    }
    Some((year?, month?))
}

fn month_num(m: &str) -> Option<u32> {
    Some(match m.to_ascii_lowercase().as_str() {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::days_from_civil;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
<channel>
<title>WA7BNM Contest Calendar</title>
<description>Calendar of ham radio contests</description>
<lastBuildDate>Tue, 07 Jul 2026 00:00:00 +0000</lastBuildDate>
<item>
<title>ARS Spartan Sprint</title>
<link>https://www.contestcalendar.com/weeklycontdetails.php?ref=003cnxe3</link>
<description>0000Z-0200Z, Jul 7</description>
<guid>https://www.contestcalendar.com/?g=00ths2o0020267</guid>
</item>
<item>
<title>IARU HF World Championship</title>
<link>https://www.contestcalendar.com/weeklycontdetails.php?ref=003cu4zn</link>
<description>1200Z, Jul 11 to 1200Z, Jul 12</description>
</item>
<item>
<title>New Year Snowball &amp; Sprint</title>
<link>https://www.contestcalendar.com/weeklycontdetails.php?ref=003zzz</link>
<description>0000Z, Jan 1 to 2359Z, Jan 1</description>
</item>
</channel>
</rss>"#;

    #[test]
    fn parses_names_links_and_utc_times() {
        let ev = parse_contest_rss(SAMPLE);
        assert_eq!(ev.len(), 3);

        // Single-day, dash-separated times sharing one date.
        assert_eq!(ev[0].name, "ARS Spartan Sprint");
        assert_eq!(ev[0].start_unix, 1_783_382_400); // 2026-07-07 00:00Z
        assert_eq!(ev[0].end_unix, 1_783_389_600); // 2026-07-07 02:00Z
        assert_eq!(
            ev[0].url.as_deref(),
            Some("https://www.contestcalendar.com/weeklycontdetails.php?ref=003cnxe3")
        );

        // Multi-day " to " range.
        assert_eq!(ev[1].name, "IARU HF World Championship");
        assert_eq!(ev[1].start_unix, 1_783_771_200); // 2026-07-11 12:00Z
        assert_eq!(ev[1].end_unix, 1_783_857_600); // 2026-07-12 12:00Z

        // XML entity in the title is unescaped, and the year rolls to 2027
        // because Jan precedes the July build month.
        assert_eq!(ev[2].name, "New Year Snowball & Sprint");
        assert_eq!(ev[2].start_unix, 1_798_761_600); // 2027-01-01 00:00Z
        assert_eq!(ev[2].end_unix, 1_798_847_940); // 2027-01-01 23:59Z
        assert!(ev[2].end_unix > ev[2].start_unix);
    }

    #[test]
    fn end_of_day_2400z_rolls_into_next_midnight() {
        let xml = r#"<rss><channel>
<lastBuildDate>Sat, 11 Jul 2026 00:00:00 +0000</lastBuildDate>
<item><title>SKCC WES</title><link>x</link>
<description>1200Z, Jul 11 to 2400Z, Jul 12</description></item>
</channel></rss>"#;
        let ev = parse_contest_rss(xml);
        assert_eq!(ev.len(), 1);
        // 2400Z on Jul 12 == 00:00Z on Jul 13.
        assert_eq!(ev[0].end_unix, days_from_civil(2026, 7, 13) * 86_400);
    }

    #[test]
    fn skips_unparseable_items_but_keeps_good_ones() {
        let xml = r#"<rss><channel>
<lastBuildDate>Tue, 07 Jul 2026 00:00:00 +0000</lastBuildDate>
<item><title>No Date Contest</title><description>All weekend</description></item>
<item><title>Good One</title><link>u</link><description>0000Z-0200Z, Jul 8</description></item>
<item><description>0000Z-0200Z, Jul 9</description></item>
</channel></rss>"#;
        let ev = parse_contest_rss(xml);
        // Only the middle item is complete + parseable.
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].name, "Good One");
    }

    #[test]
    fn no_last_build_date_skips_all_items() {
        // Without a year anchor we never fabricate a year.
        let xml = r#"<rss><channel>
<item><title>Orphan</title><description>0000Z-0200Z, Jul 8</description></item>
</channel></rss>"#;
        assert!(parse_contest_rss(xml).is_empty());
    }

    #[test]
    fn malformed_xml_does_not_panic() {
        assert!(parse_contest_rss("not xml at all <<< >>>").is_empty());
        assert!(parse_contest_rss("<rss><channel><item><title>Half").is_empty());
        assert!(parse_contest_rss("").is_empty());
    }
}
