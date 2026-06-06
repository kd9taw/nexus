//! Pure QRZ.com XML callsign-lookup helpers — the offline, unit-testable core of
//! the QRZ enrichment connector (mirrors [`crate::lotw`]/[`crate::eqsl`]). No I/O.
//!
//! QRZ's XML API is a **two-step session-key** flow (unlike LoTW/eQSL's
//! per-request credentials): log in once with the account username+password to
//! get an opaque session `<Key>`, then look up callsigns with `?s=<key>`. This
//! module builds both URLs, parses the `<Session>` block (key / quota / errors)
//! and the `<Callsign>` data block, and detects session expiry. The thin HTTPS
//! transport lives behind the `live` feature elsewhere.
//!
//! ⚠️ Both the login URL (password) and the lookup URL (session key) are
//! secret-bearing; the transport must redact errors. The session `<Key>` is a
//! bearer secret, so [`QrzSession`]'s `Debug` redacts it (and [`QrzLogin`]'s
//! redacts the password).
//!
//! ⚠️ Subscription reality: a FREE QRZ account's XML returns only name/address/
//! country — **grid and state are subscriber-only**. So `grid`/`state` are
//! `Option` and routinely `None` for non-subscribers (the expected case).

/// The QRZ XML endpoint. Host + scheme are a hard-coded https constant so the
/// secret-bearing query strings can only ever go to QRZ over TLS.
pub const QRZ_XML_URL: &str = "https://xmldata.qrz.com/xml/current/";

/// Login inputs (exchanged once for a session key). `Debug` redacts the password.
#[derive(Clone)]
pub struct QrzLogin {
    pub username: String,
    pub password: String,
    /// Product identifier sent as `agent=` (e.g. "nexus/0.1"); aids QRZ support.
    pub agent: String,
}

impl std::fmt::Debug for QrzLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QrzLogin")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("agent", &self.agent)
            .finish()
    }
}

/// The parsed `<Session>` block. `key == None` ⇒ no valid session ⇒ the caller
/// must (re)login. `Debug` redacts the key (a bearer secret).
#[derive(Clone, Default, PartialEq)]
pub struct QrzSession {
    pub key: Option<String>,
    /// Subscription expiry, or the literal `non-subscriber`.
    pub sub_exp: Option<String>,
    /// Lookups used in the current 24 h period.
    pub count: Option<u32>,
    pub message: Option<String>,
    pub error: Option<String>,
}

impl std::fmt::Debug for QrzSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QrzSession")
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .field("sub_exp", &self.sub_exp)
            .field("count", &self.count)
            .field("message", &self.message)
            .field("error", &self.error)
            .finish()
    }
}

impl QrzSession {
    /// True iff QRZ reported the session expired/invalid (no key, or an explicit
    /// timeout error) — the caller should re-login and retry once.
    pub fn needs_login(&self) -> bool {
        // Any session-level error (Timeout / "Invalid session key" / "Session does
        // not exist") means re-login; a lookup-level "Not found" does NOT (no
        // "session"), so it won't trigger a needless re-login.
        self.key.is_none()
            || self
                .error
                .as_deref()
                .is_some_and(|e| e.to_ascii_lowercase().contains("session"))
    }
}

/// A parsed QRZ callsign record. **Pure** (no serde — the serde DTO lives in
/// tempo-app, mirroring `ReconcileSummary`→`LotwSyncResult`). `grid`/`state` are
/// subscriber-only and routinely `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QrzLookup {
    pub call: String,
    pub name: Option<String>,
    /// City (QRZ `addr2`).
    pub qth: Option<String>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub dxcc: Option<u32>,
    pub cq_zone: Option<u32>,
    pub itu_zone: Option<u32>,
}

/// Percent-encode a query value (RFC 3986 unreserved set). Same encoder as the
/// sibling connectors; kept local. Encodes `;`/`&`/`=`/space etc. so a password
/// or callsign can't break the query string.
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

/// Build the login URL (carries the password — secret). QRZ accepts `&` or `;`
/// separators; we use `&` (standard) with percent-encoded values.
pub fn build_login_url(l: &QrzLogin) -> String {
    format!(
        "{QRZ_XML_URL}?username={}&password={}&agent={}",
        pct(&l.username),
        pct(&l.password),
        pct(&l.agent),
    )
}

/// Build a lookup URL (carries the session key — secret).
pub fn build_lookup_url(session_key: &str, callsign: &str) -> String {
    format!(
        "{QRZ_XML_URL}?s={}&callsign={}",
        pct(session_key.trim()),
        pct(callsign.trim()),
    )
}

/// True iff `body` is a QRZ XML response (not an HTML/error page). The
/// `<QRZDatabase` root is QRZ-specific, so a `contains` check is safe here (an
/// HTML page never carries it).
pub fn is_qrz_xml(body: &str) -> bool {
    body.to_ascii_lowercase().contains("<qrzdatabase")
}

/// Parse the `<Session>` block. Missing fields ⇒ `None`.
pub fn parse_session(xml: &str) -> QrzSession {
    QrzSession {
        key: tag(xml, "Key"),
        sub_exp: tag(xml, "SubExp"),
        count: tag(xml, "Count").and_then(|c| c.parse().ok()),
        message: tag(xml, "Message"),
        error: tag(xml, "Error"),
    }
}

/// Parse the `<Callsign>` data block. `None` if there is no callsign record
/// (e.g. a login-only or error response).
pub fn parse_callsign(xml: &str) -> Option<QrzLookup> {
    let call = tag(xml, "call")?;
    let name = tag(xml, "name_fmt").or_else(|| match (tag(xml, "fname"), tag(xml, "name")) {
        (Some(f), Some(l)) => Some(format!("{f} {l}")),
        (Some(f), None) => Some(f),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    });
    Some(QrzLookup {
        call,
        name,
        qth: tag(xml, "addr2"),
        grid: tag(xml, "grid"),
        state: tag(xml, "state"),
        country: tag(xml, "country"),
        dxcc: tag(xml, "dxcc").and_then(|d| d.parse().ok()),
        cq_zone: tag(xml, "cqzone").and_then(|d| d.parse().ok()),
        itu_zone: tag(xml, "ituzone").and_then(|d| d.parse().ok()),
    })
}

/// Extract the text content of the first `<name>…</name>` element (case-insensitive
/// tag name), XML-unescaped. `None` if absent/empty. Matches only the attribute-free
/// `<name>` form — QRZ data elements never carry attributes, so an attributed tag
/// safely yields `None` rather than a mis-bounded value (no `>`-in-attribute hazard).
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
    fn login_url_encodes_password() {
        let url = build_login_url(&QrzLogin {
            username: "AA7BQ".into(),
            password: "p@ss;w&rd".into(),
            agent: "nexus/0.1".into(),
        });
        assert!(url.starts_with("https://xmldata.qrz.com/xml/current/?username=AA7BQ&password="));
        assert!(url.contains("password=p%40ss%3Bw%26rd")); // ; and & encoded
        assert!(!url.contains("p@ss;w&rd"));
        assert!(url.contains("agent=nexus%2F0.1"));
    }

    #[test]
    fn lookup_url_encodes_key_and_call() {
        let url = build_lookup_url("KEY 123", "dl1abc");
        assert_eq!(
            url,
            "https://xmldata.qrz.com/xml/current/?s=KEY%20123&callsign=dl1abc"
        );
    }

    #[test]
    fn login_debug_redacts_password() {
        let dbg = format!(
            "{:?}",
            QrzLogin {
                username: "AA7BQ".into(),
                password: "sekret".into(),
                agent: "x".into()
            }
        );
        assert!(dbg.contains("<redacted>") && !dbg.contains("sekret"));
    }

    const LOGIN_OK: &str = "<?xml version=\"1.0\" ?>\n\
<QRZDatabase version=\"1.34\" xmlns=\"http://xmldata.qrz.com\">\n\
<Session><Key>3b1fc0de</Key><Count>12</Count><SubExp>Wed Jan 1 2031</SubExp></Session>\n\
</QRZDatabase>";

    const EXPIRED: &str = "<?xml version=\"1.0\" ?><QRZDatabase version=\"1.34\">\
<Session><Error>Session Timeout</Error></Session></QRZDatabase>";

    const NOT_FOUND: &str = "<QRZDatabase version=\"1.34\"><Session><Key>abc</Key>\
<Error>Not found: g1srdd</Error></Session></QRZDatabase>";

    const LOOKUP_FULL: &str = "<?xml version=\"1.0\" ?>\n\
<QRZDatabase version=\"1.34\" xmlns=\"http://xmldata.qrz.com\">\n\
<Callsign><call>AA7BQ</call><fname>Fred</fname><name>Lloyd</name><addr2>Scottsdale</addr2>\
<state>AZ</state><country>United States</country><grid>DM43bp</grid><dxcc>291</dxcc>\
<cqzone>3</cqzone><ituzone>6</ituzone></Callsign>\n\
<Session><Key>abc</Key><Count>13</Count></Session>\n</QRZDatabase>";

    // A free (non-subscriber) account: name/country only, NO grid/state.
    const LOOKUP_FREE: &str = "<QRZDatabase version=\"1.34\"><Callsign><call>AA7BQ</call>\
<name_fmt>Fred Lloyd</name_fmt><country>United States</country></Callsign>\
<Session><Key>abc</Key><SubExp>non-subscriber</SubExp>\
<Message>A subscription is required to obtain the complete data</Message></Session></QRZDatabase>";

    #[test]
    fn session_login_ok_has_key_no_relogin() {
        let s = parse_session(LOGIN_OK);
        assert_eq!(s.key.as_deref(), Some("3b1fc0de"));
        assert_eq!(s.count, Some(12));
        assert!(!s.needs_login());
    }

    #[test]
    fn session_expired_needs_login() {
        let s = parse_session(EXPIRED);
        assert!(s.key.is_none());
        assert!(s.needs_login());
    }

    #[test]
    fn session_not_found_keeps_key_no_relogin() {
        // Not-found is a valid session with an error — must NOT trigger re-login.
        let s = parse_session(NOT_FOUND);
        assert_eq!(s.key.as_deref(), Some("abc"));
        assert!(!s.needs_login());
        assert!(s.error.as_deref().unwrap().contains("Not found"));
    }

    #[test]
    fn session_debug_redacts_key() {
        let dbg = format!("{:?}", parse_session(LOGIN_OK));
        assert!(dbg.contains("<redacted>") && !dbg.contains("3b1fc0de"));
    }

    #[test]
    fn parses_full_subscriber_record() {
        let r = parse_callsign(LOOKUP_FULL).unwrap();
        assert_eq!(r.call, "AA7BQ");
        assert_eq!(r.name.as_deref(), Some("Fred Lloyd"));
        assert_eq!(r.qth.as_deref(), Some("Scottsdale"));
        assert_eq!(r.grid.as_deref(), Some("DM43bp"));
        assert_eq!(r.state.as_deref(), Some("AZ"));
        assert_eq!(r.dxcc, Some(291));
        assert_eq!(r.cq_zone, Some(3));
        assert_eq!(r.itu_zone, Some(6));
    }

    #[test]
    fn parses_free_record_without_grid_state() {
        let r = parse_callsign(LOOKUP_FREE).unwrap();
        assert_eq!(r.call, "AA7BQ");
        assert_eq!(r.name.as_deref(), Some("Fred Lloyd")); // from name_fmt
        assert!(r.grid.is_none(), "free tier has no grid");
        assert!(r.state.is_none(), "free tier has no state");
        assert_eq!(r.country.as_deref(), Some("United States"));
    }

    #[test]
    fn no_callsign_block_is_none() {
        assert!(parse_callsign(LOGIN_OK).is_none());
        assert!(parse_callsign(EXPIRED).is_none());
    }

    #[test]
    fn is_qrz_xml_accepts_qrz_rejects_html() {
        assert!(is_qrz_xml(LOGIN_OK));
        assert!(!is_qrz_xml(
            "<!DOCTYPE html><html><title>QRZ.com</title></html>"
        ));
    }

    #[test]
    fn tag_unescapes_entities() {
        let xml = "<QRZDatabase><Callsign><call>X</call><name>Smith &amp; Jones</name></Callsign></QRZDatabase>";
        assert_eq!(
            parse_callsign(xml).unwrap().name.as_deref(),
            Some("Smith & Jones")
        );
    }

    #[test]
    fn key_present_but_invalid_session_error_needs_login() {
        // QRZ can return a present-but-dead key with an "Invalid session key" error;
        // must re-login, not keep reusing the dead key.
        let xml = "<QRZDatabase><Session><Key>stale</Key>\
<Error>Invalid session key</Error></Session></QRZDatabase>";
        assert!(parse_session(xml).needs_login());
    }

    #[test]
    fn attributed_tag_yields_none_not_misbounded_value() {
        // Defends the no-open_attr fix: a (non-QRZ) attributed tag must not return a
        // value mis-bounded at a '>' inside the attribute.
        let xml = "<QRZDatabase><Callsign><call>X</call>\
<grid id=\"a>b\">DM43</grid></Callsign></QRZDatabase>";
        assert!(parse_callsign(xml).unwrap().grid.is_none());
    }
}
