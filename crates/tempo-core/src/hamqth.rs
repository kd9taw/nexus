//! Pure HamQTH.com XML callsign-lookup helpers — the offline, unit-testable core
//! of the HamQTH enrichment connector, the **free-account fallback for
//! [`crate::qrz`]** (which needs a paid subscription for grid/state). No I/O.
//!
//! HamQTH's XML API is a **two-step session-id** flow, structurally identical to
//! QRZ's session-key flow: log in once with the account username+password to get
//! an opaque `<session_id>` (valid ~1 h), then look up callsigns with
//! `?id=<session_id>&callsign=<call>&prg=<program>`. This module builds both URLs,
//! parses the `<session>` block (id / error) and the `<search>` data block, and
//! detects session expiry. The thin HTTPS transport lives behind the `live`
//! feature elsewhere (mirroring [`crate::qrz`]).
//!
//! It parses into [`HamQthLookup`], which has the **same fields** as
//! [`crate::qrz::QrzLookup`] (call / name / qth / grid / state / country / dxcc /
//! zones) so the two are interchangeable — the shell tries QRZ first and falls
//! back to HamQTH, both flowing into the same lookup DTO.
//!
//! ⚠️ Both the login URL (password) and the lookup URL (session id) are
//! secret-bearing; the transport must redact errors. The `session_id` is a bearer
//! secret, so [`HamQthSession`]'s `Debug` redacts it (and [`HamQthLogin`]'s
//! redacts the password).
//!
//! ⚠️ Unlike QRZ, HamQTH does **not** echo the session id on a successful lookup
//! (only on the login response). So "no session id" is NOT expiry —
//! [`HamQthSession::needs_login`] triggers only on an explicit session/expired
//! *error*, and a `<search>` result legitimately carries no `<session>` block.

/// The HamQTH XML endpoint. Host + scheme are a hard-coded https constant so the
/// secret-bearing query strings can only ever go to HamQTH over TLS.
pub const HAMQTH_XML_URL: &str = "https://www.hamqth.com/xml.php";

/// The `prg=` program identifier HamQTH asks each client to send on lookups.
pub const HAMQTH_PRG: &str = "Nexus";

/// Login inputs (exchanged once for a session id). `Debug` redacts the password.
#[derive(Clone)]
pub struct HamQthLogin {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for HamQthLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HamQthLogin")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// The parsed `<session>` block. On login, `session_id == None` (with an `error`)
/// ⇒ bad credentials. On a lookup, an `error` mentioning "session"/"expired" ⇒ the
/// session died and the caller must re-login. `Debug` redacts the id (a bearer
/// secret).
#[derive(Clone, Default, PartialEq)]
pub struct HamQthSession {
    pub session_id: Option<String>,
    pub error: Option<String>,
}

impl std::fmt::Debug for HamQthSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HamQthSession")
            .field(
                "session_id",
                &self.session_id.as_ref().map(|_| "<redacted>"),
            )
            .field("error", &self.error)
            .finish()
    }
}

impl HamQthSession {
    /// True iff HamQTH reported the session expired/invalid on a **lookup** — the
    /// caller should re-login and retry once.
    ///
    /// ⚠️ Unlike QRZ (which echoes its key on every response), HamQTH omits the
    /// session block from a *successful* lookup — so this must NOT key off a
    /// missing `session_id` (that would re-login on every hit). It keys only off an
    /// explicit session/expired error ("Session does not exist or expired"). A
    /// lookup-level "Callsign not found" contains neither word, so it correctly
    /// does NOT trigger a needless re-login.
    pub fn needs_login(&self) -> bool {
        self.error.as_deref().is_some_and(|e| {
            let e = e.to_ascii_lowercase();
            e.contains("session") || e.contains("expired")
        })
    }
}

/// A parsed HamQTH callsign record. **Pure** (no serde — it reuses the same DTO as
/// QRZ in tempo-app). Same fields as [`crate::qrz::QrzLookup`] so the two lookups
/// are interchangeable; `state` is populated from HamQTH's `<us_state>` element.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HamQthLookup {
    pub call: String,
    pub name: Option<String>,
    /// City (HamQTH `qth`).
    pub qth: Option<String>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub dxcc: Option<u32>,
    pub cq_zone: Option<u32>,
    pub itu_zone: Option<u32>,
}

/// Percent-encode a query value (RFC 3986 unreserved set). Same encoder as the
/// sibling connectors; kept local. Encodes `&`/`=`/space etc. so a password or
/// callsign can't break the query string.
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Build the login URL (carries the password — secret). `rectype=user` selects the
/// user-account login flow.
pub fn build_login_url(l: &HamQthLogin) -> String {
    format!(
        "{HAMQTH_XML_URL}?u={}&p={}&rectype=user",
        pct(&l.username),
        pct(&l.password),
    )
}

/// Build a lookup URL (carries the session id — secret). `prg` is the calling
/// program's name ([`HAMQTH_PRG`]).
pub fn build_lookup_url(session_id: &str, callsign: &str, prg: &str) -> String {
    format!(
        "{HAMQTH_XML_URL}?id={}&callsign={}&prg={}",
        pct(session_id.trim()),
        pct(callsign.trim()),
        pct(prg.trim()),
    )
}

/// True iff `body` is a HamQTH XML response (not an HTML/error page). The `<HamQTH`
/// root is HamQTH-specific, so a `contains` check is safe here (an HTML page never
/// carries it).
pub fn is_hamqth_xml(body: &str) -> bool {
    body.to_ascii_lowercase().contains("<hamqth")
}

/// Parse the `<session>` block. Missing fields ⇒ `None`.
pub fn parse_session(xml: &str) -> HamQthSession {
    HamQthSession {
        session_id: tag(xml, "session_id"),
        error: tag(xml, "error"),
    }
}

/// Parse the `<search>` data block. `None` if there is no callsign record (e.g. a
/// login-only response or a "Callsign not found" error).
pub fn parse_callsign(xml: &str) -> Option<HamQthLookup> {
    let call = tag(xml, "callsign")?;
    // Prefer the full postal name (`adr_name`); fall back to the operator nick.
    let name = tag(xml, "adr_name")
        .or_else(|| tag(xml, "nick"))
        .or_else(|| tag(xml, "name"));
    Some(HamQthLookup {
        call,
        name,
        qth: tag(xml, "qth"),
        grid: tag(xml, "grid"),
        state: tag(xml, "us_state"),
        // The address country is the operator's real DXCC; fall back to the
        // callsign country when the address is hidden.
        country: tag(xml, "adr_country").or_else(|| tag(xml, "country")),
        dxcc: tag(xml, "adif")
            .or_else(|| tag(xml, "adr_adif"))
            .and_then(|d| d.parse().ok()),
        cq_zone: tag(xml, "cq").and_then(|d| d.parse().ok()),
        itu_zone: tag(xml, "itu").and_then(|d| d.parse().ok()),
    })
}

/// Extract the text content of the first `<name>…</name>` element (case-insensitive
/// tag name), XML-unescaped. `None` if absent/empty. Matches only the attribute-free
/// `<name>` form — HamQTH data elements never carry attributes, so an attributed tag
/// safely yields `None` rather than a mis-bounded value (no `>`-in-attribute hazard).
/// The trailing `>` in the open pattern also prevents prefix collisions (`<cq>`
/// never matches inside `<cqzone>`, `<adif>` never matches inside `<adr_adif>`).
fn tag(xml: &str, name: &str) -> Option<String> {
    // Lowercased copy for case-insensitive search; `to_ascii_lowercase` preserves
    // byte length + boundaries, so indices map 1:1 onto `xml`.
    let lower = xml.to_ascii_lowercase();
    let n = name.to_ascii_lowercase();
    let open = format!("<{n}>");
    let start = lower.find(&open)?;
    let after_open = start + open.len();
    let close = format!("</{n}>");
    let close_rel = lower[after_open..].find(&close)?;
    let raw = xml[after_open..after_open + close_rel].trim();
    let v = unescape_xml(raw);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Decode the common XML entities (`&amp;` LAST so it doesn't double-decode).
fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_encodes_password_and_selects_user_rectype() {
        let url = build_login_url(&HamQthLogin {
            username: "ok7an".into(),
            password: "p@ss w&rd".into(),
        });
        assert!(url.starts_with("https://www.hamqth.com/xml.php?u=ok7an&p="));
        assert!(url.contains("p=p%40ss%20w%26rd")); // space and & encoded
        assert!(!url.contains("p@ss w&rd"));
        assert!(url.ends_with("&rectype=user"));
    }

    #[test]
    fn lookup_url_encodes_id_call_and_prg() {
        let url = build_lookup_url("SID 123", "dl1abc", HAMQTH_PRG);
        assert_eq!(
            url,
            "https://www.hamqth.com/xml.php?id=SID%20123&callsign=dl1abc&prg=Nexus"
        );
    }

    #[test]
    fn login_debug_redacts_password() {
        let dbg = format!(
            "{:?}",
            HamQthLogin {
                username: "ok7an".into(),
                password: "sekret".into(),
            }
        );
        assert!(dbg.contains("<redacted>") && !dbg.contains("sekret"));
    }

    const LOGIN_OK: &str = "<?xml version=\"1.0\"?>\n\
<HamQTH version=\"2.7\" xmlns=\"https://www.hamqth.com\">\n\
<session><session_id>09b0ae90050be03c452ad235a1f2915ad684393c</session_id></session>\n\
</HamQTH>";

    const LOGIN_BAD: &str = "<?xml version=\"1.0\"?><HamQTH version=\"2.7\">\
<session><error>Wrong user name or password</error></session></HamQTH>";

    const EXPIRED: &str = "<?xml version=\"1.0\"?><HamQTH version=\"2.7\">\
<session><error>Session does not exist or expired</error></session></HamQTH>";

    const NOT_FOUND: &str = "<HamQTH version=\"2.7\">\
<session><error>Callsign not found</error></session></HamQTH>";

    // A US station — carries <us_state> (the field QRZ hides behind a paid sub).
    const LOOKUP_US: &str = "<?xml version=\"1.0\"?>\n\
<HamQTH version=\"2.7\" xmlns=\"https://www.hamqth.com\">\n\
<search><callsign>w1aw</callsign><nick>ARRL HQ</nick><qth>Newington</qth>\
<country>United States</country><adif>291</adif><itu>8</itu><cq>5</cq><grid>FN31pr</grid>\
<adr_name>ARRL Headquarters Operators Club</adr_name><adr_city>Newington</adr_city>\
<adr_country>United States</adr_country><adr_adif>291</adr_adif>\
<us_state>CT</us_state><us_county>Hartford</us_county></search>\n</HamQTH>";

    // A DX station with no postal name and no us_state — name falls back to <nick>.
    const LOOKUP_DX: &str = "<HamQTH version=\"2.7\"><search><callsign>ok7an</callsign>\
<nick>Petr</nick><qth>Neratovice</qth><adr_country>Czech Republic</adr_country>\
<adif>503</adif><itu>28</itu><cq>15</cq><grid>JO70gg</grid></search></HamQTH>";

    #[test]
    fn session_login_ok_has_id_no_relogin() {
        let s = parse_session(LOGIN_OK);
        assert_eq!(
            s.session_id.as_deref(),
            Some("09b0ae90050be03c452ad235a1f2915ad684393c")
        );
        assert!(!s.needs_login());
    }

    #[test]
    fn session_bad_login_surfaces_error_no_id() {
        // Wrong credentials: no id, error text preserved for the user. This is NOT a
        // session-expiry error, so needs_login stays false (the login path decides
        // via the missing id — it must not be mistaken for a retry-able expiry).
        let s = parse_session(LOGIN_BAD);
        assert!(s.session_id.is_none());
        assert_eq!(s.error.as_deref(), Some("Wrong user name or password"));
        assert!(!s.needs_login());
    }

    #[test]
    fn session_expired_needs_login() {
        let s = parse_session(EXPIRED);
        assert!(s.session_id.is_none());
        assert!(s.needs_login());
    }

    #[test]
    fn not_found_does_not_trigger_relogin_and_has_no_record() {
        // "Callsign not found" is a valid session answering a lookup — it must NOT
        // re-login, and it yields no record.
        let s = parse_session(NOT_FOUND);
        assert!(!s.needs_login());
        assert!(s.error.as_deref().unwrap().contains("not found"));
        assert!(parse_callsign(NOT_FOUND).is_none());
    }

    #[test]
    fn successful_lookup_omits_session_block_so_no_relogin() {
        // The key HamQTH-vs-QRZ difference: a successful <search> carries no
        // <session>, so session_id is None — but that must NOT be read as expiry.
        let s = parse_session(LOOKUP_US);
        assert!(s.session_id.is_none());
        assert!(s.error.is_none());
        assert!(!s.needs_login(), "a hit must never trigger a re-login");
    }

    #[test]
    fn session_debug_redacts_id() {
        let dbg = format!("{:?}", parse_session(LOGIN_OK));
        assert!(dbg.contains("<redacted>") && !dbg.contains("09b0ae90"));
    }

    #[test]
    fn parses_full_us_record_with_state() {
        let r = parse_callsign(LOOKUP_US).unwrap();
        assert_eq!(r.call, "w1aw");
        assert_eq!(r.name.as_deref(), Some("ARRL Headquarters Operators Club")); // adr_name
        assert_eq!(r.qth.as_deref(), Some("Newington"));
        assert_eq!(r.grid.as_deref(), Some("FN31pr"));
        assert_eq!(r.state.as_deref(), Some("CT")); // <us_state>
        assert_eq!(r.country.as_deref(), Some("United States")); // adr_country
        assert_eq!(r.dxcc, Some(291)); // <adif>
        assert_eq!(r.cq_zone, Some(5));
        assert_eq!(r.itu_zone, Some(8));
    }

    #[test]
    fn parses_dx_record_name_from_nick_no_state() {
        let r = parse_callsign(LOOKUP_DX).unwrap();
        assert_eq!(r.call, "ok7an");
        assert_eq!(r.name.as_deref(), Some("Petr")); // falls back to <nick>
        assert!(r.state.is_none(), "non-US record has no us_state");
        assert_eq!(r.country.as_deref(), Some("Czech Republic"));
        assert_eq!(r.dxcc, Some(503));
        assert_eq!(r.cq_zone, Some(15));
        assert_eq!(r.itu_zone, Some(28));
    }

    #[test]
    fn no_search_block_is_none() {
        assert!(parse_callsign(LOGIN_OK).is_none());
        assert!(parse_callsign(EXPIRED).is_none());
    }

    #[test]
    fn is_hamqth_xml_accepts_hamqth_rejects_html() {
        assert!(is_hamqth_xml(LOGIN_OK));
        assert!(!is_hamqth_xml(
            "<!DOCTYPE html><html><title>HamQTH.com</title></html>"
        ));
    }

    #[test]
    fn tag_unescapes_entities() {
        let xml = "<HamQTH><search><callsign>X</callsign>\
<adr_name>Smith &amp; Jones</adr_name></search></HamQTH>";
        assert_eq!(
            parse_callsign(xml).unwrap().name.as_deref(),
            Some("Smith & Jones")
        );
    }

    #[test]
    fn adif_tag_does_not_collide_with_adr_adif() {
        // Guards the `>`-in-open-pattern prefix-collision defence: <adif> must read
        // the standalone element (291), never leak from <adr_adif> (999).
        let xml = "<HamQTH><search><callsign>X</callsign>\
<adr_adif>999</adr_adif><adif>291</adif></search></HamQTH>";
        assert_eq!(parse_callsign(xml).unwrap().dxcc, Some(291));
    }

    #[test]
    fn attributed_tag_yields_none_not_misbounded_value() {
        // A (hypothetical) attributed tag must not return a value mis-bounded at a
        // '>' inside the attribute — matches only the attribute-free form.
        let xml = "<HamQTH><search><callsign>X</callsign>\
<grid id=\"a>b\">JO70</grid></search></HamQTH>";
        assert!(parse_callsign(xml).unwrap().grid.is_none());
    }
}
