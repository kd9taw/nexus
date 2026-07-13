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
    /// QRZ `<nickname>` — the operator's preferred short/first name when they set one.
    /// Preferred over `name` for display when present (operators want to be greeted by it).
    pub nickname: Option<String>,
    /// City (QRZ `addr2`).
    pub qth: Option<String>,
    pub grid: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
    pub dxcc: Option<u32>,
    pub cq_zone: Option<u32>,
    pub itu_zone: Option<u32>,
    /// Profile photo URL (QRZ `<image>`). Subscriber-only + operator-supplied, so routinely `None`.
    pub image: Option<String>,
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
        nickname: tag(xml, "nickname"),
        qth: tag(xml, "addr2"),
        grid: tag(xml, "grid"),
        state: tag(xml, "state"),
        country: tag(xml, "country"),
        dxcc: tag(xml, "dxcc").and_then(|d| d.parse().ok()),
        cq_zone: tag(xml, "cqzone").and_then(|d| d.parse().ok()),
        itu_zone: tag(xml, "ituzone").and_then(|d| d.parse().ok()),
        image: tag(xml, "image"),
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

// ----- QRZ Logbook PUSH (a separate per-logbook API key; see `qrz-push.md`) -----

/// The QRZ Logbook API endpoint (POST, `name=value` form). Hard-coded https.
pub const QRZ_LOGBOOK_URL: &str = "https://logbook.qrz.com/api";

/// Outcome of a QRZ Logbook INSERT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QrzPushResult {
    /// Inserted (new `logid`).
    Ok,
    /// Overwrote a duplicate (only when `OPTION=REPLACE` was sent).
    Replace,
    /// A duplicate that was rejected — benign ("already in your QRZ logbook").
    Duplicate,
    /// The API key was missing/invalid/insufficient.
    AuthFail,
    /// Any other failure (see `reason`).
    Fail,
}

impl QrzPushResult {
    /// Map a QRZ push outcome to the generic per-QSO [`UploadOutcome`] for the
    /// logbook's `upload.qrz` cursor. `Ok`/`Replace` → on file (Accepted); a
    /// rejected duplicate is benign (Duplicate); `AuthFail`/`Fail` are bounces the
    /// diagnostics surface as R9. Always definitive (no transient/None case).
    pub fn to_upload_outcome(self) -> crate::logbook::UploadOutcome {
        use crate::logbook::UploadOutcome as U;
        match self {
            QrzPushResult::Ok | QrzPushResult::Replace => U::Accepted,
            QrzPushResult::Duplicate => U::Duplicate,
            QrzPushResult::AuthFail => U::AuthFail,
            QrzPushResult::Fail => U::Rejected,
        }
    }
}

/// Parsed QRZ Logbook INSERT response.
#[derive(Debug, Clone, PartialEq)]
pub struct QrzPush {
    pub result: QrzPushResult,
    pub logid: Option<String>,
    pub count: u32,
    pub reason: Option<String>,
}

/// Build the `name=value` POST body for an INSERT. The ADIF tags (`<…>`) and the
/// key are percent-encoded so they survive form-encoding. The body carries the API
/// key — never log it.
pub fn build_insert_body(api_key: &str, adif_record: &str, replace: bool) -> String {
    let mut body = format!(
        "KEY={}&ACTION=INSERT&ADIF={}",
        pct(api_key.trim()),
        pct(adif_record),
    );
    if replace {
        body.push_str("&OPTION=REPLACE");
    }
    body
}

/// Build the body of a QRZ Logbook **STATUS** request — validates the API key
/// with a real round-trip WITHOUT inserting anything (the Test button).
pub fn build_status_body(api_key: &str) -> String {
    format!("KEY={}&ACTION=STATUS", pct(api_key.trim()))
}

/// What a STATUS round-trip proved about the logbook the key unlocks.
#[derive(Debug, Clone, PartialEq)]
pub struct QrzStatus {
    pub ok: bool,
    /// Logbook owner callsign (QRZ `OWNER`), when reported.
    pub owner: Option<String>,
    /// Logbook name (QRZ `BOOK_NAME`/`BOOKID`), when reported.
    pub book: Option<String>,
    /// QSO count in the logbook (QRZ `COUNT`).
    pub count: u32,
    /// Failure reason (auth errors etc.).
    pub reason: Option<String>,
}

/// Parse a QRZ Logbook STATUS `name=value` response.
pub fn parse_status_response(body: &str) -> QrzStatus {
    let mut ok = false;
    let mut owner = None;
    let mut book = None;
    let mut count = 0u32;
    let mut reason = None;
    for pair in body.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let val = urldecode(v.trim());
        match k.trim().to_ascii_uppercase().as_str() {
            "RESULT" => ok = val.eq_ignore_ascii_case("OK"),
            "OWNER" if !val.is_empty() => owner = Some(val),
            "BOOK_NAME" if !val.is_empty() => book = Some(val),
            "BOOKID" if book.is_none() && !val.is_empty() => book = Some(format!("book {val}")),
            "COUNT" => count = val.parse().unwrap_or(0),
            "REASON" if !val.is_empty() => reason = Some(val),
            _ => {}
        }
    }
    QrzStatus {
        ok,
        owner,
        book,
        count,
        reason,
    }
}

/// Parse a QRZ Logbook `name=value` response. A `RESULT=FAIL` whose `REASON`
/// mentions "duplicate" maps to [`QrzPushResult::Duplicate`] (benign).
pub fn parse_push_response(body: &str) -> QrzPush {
    let mut result_raw: Option<String> = None;
    let mut logid = None;
    let mut count = 0u32;
    let mut reason = None;
    for pair in body.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let val = urldecode(v.trim());
        match k.trim().to_ascii_uppercase().as_str() {
            "RESULT" => result_raw = Some(val.to_ascii_uppercase()),
            "LOGID" if !val.is_empty() => logid = Some(val),
            "COUNT" => count = val.parse().unwrap_or(0),
            "REASON" if !val.is_empty() => reason = Some(val),
            _ => {}
        }
    }
    let is_dup = reason
        .as_deref()
        .is_some_and(|r| r.to_ascii_lowercase().contains("duplicate"));
    let result = match result_raw.as_deref() {
        Some("OK") => QrzPushResult::Ok,
        Some("REPLACE") => QrzPushResult::Replace,
        Some("AUTH") => QrzPushResult::AuthFail,
        Some("FAIL") if is_dup => QrzPushResult::Duplicate,
        _ => QrzPushResult::Fail,
    };
    QrzPush {
        result,
        logid,
        count,
        reason,
    }
}

/// Minimal `application/x-www-form-urlencoded` value decoder (`+`→space, `%XX`).
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                // Parse the two raw bytes as ASCII hex — NEVER slice `s` (a `%`
                // followed by a multibyte UTF-8 byte would split a char boundary
                // and panic). `from_utf8` rejects non-ASCII hex bytes safely.
                let hex = [bytes[i + 1], bytes[i + 2]];
                match std::str::from_utf8(&hex)
                    .ok()
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
<cqzone>3</cqzone><ituzone>6</ituzone><image>https://cdn-xml.qrz.com/q/aa7bq/aa7bq.jpg</image></Callsign>\n\
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
        assert_eq!(
            r.image.as_deref(),
            Some("https://cdn-xml.qrz.com/q/aa7bq/aa7bq.jpg")
        );
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
    fn parses_the_nickname_when_present() {
        let with = "<Callsign><call>W1XYZ</call><name_fmt>John Public</name_fmt>\
                    <nickname>Johnny</nickname></Callsign>";
        let r = parse_callsign(with).unwrap();
        assert_eq!(r.name.as_deref(), Some("John Public"));
        assert_eq!(r.nickname.as_deref(), Some("Johnny"), "nickname is parsed");
        // Absent → None, so the UI falls back to the full name.
        let without = parse_callsign(
            "<Callsign><call>W1XYZ</call><name_fmt>John Public</name_fmt></Callsign>",
        )
        .unwrap();
        assert!(without.nickname.is_none());
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

    #[test]
    fn insert_body_encodes_adif_and_key() {
        let body = build_insert_body("AB-12-CD", "<call:4>W1AW<eor>", false);
        assert!(body.starts_with("KEY=AB-12-CD&ACTION=INSERT&ADIF="));
        assert_eq!(
            build_status_body(" AB-12-CD "),
            "KEY=AB-12-CD&ACTION=STATUS"
        );
        // STATUS parse: the success shape QRZ actually returns.
        let st = parse_status_response(
            "RESULT=OK&OWNER=KD9TAW&BOOK_NAME=My+Logbook&COUNT=1234&ACTION=STATUS",
        );
        assert!(st.ok);
        assert_eq!(st.owner.as_deref(), Some("KD9TAW"));
        assert_eq!(st.book.as_deref(), Some("My Logbook"));
        assert_eq!(st.count, 1234);
        // Auth failure carries the reason through.
        let st = parse_status_response("RESULT=AUTH&REASON=invalid+api+key");
        assert!(!st.ok);
        assert_eq!(st.reason.as_deref(), Some("invalid api key"));
        // ADIF tag delimiters must be percent-encoded into the form value.
        assert!(body.contains("ADIF=%3Ccall%3A4%3EW1AW%3Ceor%3E"));
        assert!(!body.contains("&OPTION="));
        assert!(build_insert_body("k", "<eor>", true).ends_with("&OPTION=REPLACE"));
    }

    #[test]
    fn push_response_ok_with_logid() {
        let p = parse_push_response("RESULT=OK&COUNT=1&LOGID=123456");
        assert_eq!(p.result, QrzPushResult::Ok);
        assert_eq!(p.logid.as_deref(), Some("123456"));
        assert_eq!(p.count, 1);
    }

    #[test]
    fn push_response_duplicate_is_benign() {
        // A duplicate is RESULT=FAIL + a "duplicate" reason + COUNT=0 → Duplicate.
        let p = parse_push_response("RESULT=FAIL&COUNT=0&REASON=Unable+to+add+QSO%3A+duplicate");
        assert_eq!(p.result, QrzPushResult::Duplicate);
        assert_eq!(p.count, 0);
        assert!(p.reason.as_deref().unwrap().contains("duplicate"));
    }

    #[test]
    fn push_response_auth_and_plain_fail() {
        assert_eq!(
            parse_push_response("RESULT=AUTH").result,
            QrzPushResult::AuthFail
        );
        assert_eq!(
            parse_push_response("RESULT=FAIL&REASON=bad+ADIF").result,
            QrzPushResult::Fail
        );
    }

    #[test]
    fn push_response_urldecode_does_not_panic_on_multibyte_after_percent() {
        // A network REASON with '%' before a multibyte UTF-8 byte must not panic
        // (the old str-slice decoder split a char boundary).
        let p = parse_push_response("RESULT=FAIL&REASON=oops %€ at end");
        assert_eq!(p.result, QrzPushResult::Fail);
        assert!(p.reason.is_some());
        // A trailing bare '%' is also safe.
        assert_eq!(
            parse_push_response("RESULT=OK&REASON=100%").result,
            QrzPushResult::Ok
        );
    }

    #[test]
    fn push_result_maps_to_upload_outcome() {
        use crate::logbook::UploadOutcome as U;
        assert_eq!(QrzPushResult::Ok.to_upload_outcome(), U::Accepted);
        assert_eq!(QrzPushResult::Replace.to_upload_outcome(), U::Accepted);
        assert_eq!(QrzPushResult::Duplicate.to_upload_outcome(), U::Duplicate);
        assert_eq!(QrzPushResult::AuthFail.to_upload_outcome(), U::AuthFail);
        assert_eq!(QrzPushResult::Fail.to_upload_outcome(), U::Rejected);
    }
}
