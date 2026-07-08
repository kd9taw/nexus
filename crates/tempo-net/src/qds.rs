//! Minimal Qt `QDataStream`-compatible serializer (big-endian).
//!
//! WSJT-X frames its UDP datagrams with a `QDataStream` (Qt's portable binary
//! format). `QDataStream` is **big-endian** by default and writes fixed-width
//! integers/floats in network byte order. Strings are `QString`s: a `quint32`
//! byte-length prefix followed by the UTF-8 bytes (no NUL terminator); a length
//! of `0xFFFFFFFF` denotes a null/`None` string.
//!
//! WSJT-X serializes a few Qt value types we reproduce here:
//! - `QTime` is sent as a single `quint32` of milliseconds since midnight.
//! - `QDateTime` is `qint64` Julian day, `quint32` ms-of-day, `qint8` timespec
//!   (`1` = UTC), matching `QDataStream` version >= Qt 5.2 serialization.
//!
//! Everything here is a pure byte builder — no I/O — so the encodings are
//! exhaustively unit-tested against known byte layouts.

/// A growable big-endian byte buffer mirroring `QDataStream`'s write side.
#[derive(Debug, Default, Clone)]
pub struct QdsWriter {
    buf: Vec<u8>,
}

impl QdsWriter {
    /// A fresh, empty writer.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Consume the writer, yielding the encoded bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Borrow the encoded bytes so far.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    /// Number of bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True if nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// `quint8` — a single byte.
    /// Write a big-endian `quint16`.
    pub fn put_u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    pub fn put_u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    /// `qint8` — a single signed byte.
    pub fn put_i8(&mut self, v: i8) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    /// `quint32`, big-endian.
    pub fn put_u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `qint32`, big-endian.
    pub fn put_i32(&mut self, v: i32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `quint64`, big-endian.
    pub fn put_u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `qint64`, big-endian.
    pub fn put_i64(&mut self, v: i64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `double` (IEEE-754 64-bit), big-endian.
    pub fn put_f64(&mut self, v: f64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_be_bytes());
        self
    }

    /// `bool` — one byte, `0x01` true / `0x00` false (as `QDataStream` writes it).
    pub fn put_bool(&mut self, v: bool) -> &mut Self {
        self.buf.push(v as u8);
        self
    }

    /// `QString`/`QByteArray`-style UTF-8 string: a `quint32` byte-length prefix
    /// then the raw UTF-8 bytes. `None` is the null sentinel (`0xFFFFFFFF`, no
    /// payload). An empty (non-null) string is length `0` with no payload.
    ///
    /// WSJT-X transmits its `QString` fields as UTF-8 byte arrays in exactly
    /// this layout (the C++ side uses `out << string.toUtf8()` semantics for the
    /// telemetry messages).
    pub fn put_utf8(&mut self, s: Option<&str>) -> &mut Self {
        match s {
            None => self.put_u32(0xFFFF_FFFF),
            Some(s) => {
                let bytes = s.as_bytes();
                self.put_u32(bytes.len() as u32);
                self.buf.extend_from_slice(bytes);
                self
            }
        }
    }

    /// `QTime`: milliseconds since midnight as a `quint32`.
    pub fn put_qtime(&mut self, ms_since_midnight: u32) -> &mut Self {
        self.put_u32(ms_since_midnight)
    }

    /// `QDateTime` (UTC) from a Unix timestamp in seconds.
    ///
    /// Serialized the way `QDataStream` (Qt >= 5.2) writes it:
    /// - `qint64` Julian day number of the date,
    /// - `quint32` milliseconds since midnight,
    /// - `qint8` time-spec (`1` = UTC).
    pub fn put_qdatetime(&mut self, unix_secs: i64) -> &mut Self {
        let (jd, ms_of_day) = unix_to_julian_day(unix_secs);
        self.put_i64(jd);
        self.put_u32(ms_of_day);
        self.put_i8(1); // Qt::TimeSpec::UTC
        self
    }
}

/// Convert a Unix timestamp (UTC seconds) to a `(Julian day number,
/// milliseconds-of-day)` pair as Qt's `QDate`/`QTime` would represent it.
///
/// The Unix epoch (1970-01-01) is Julian day **2440588**. We split the seconds
/// into whole days (flooring toward negative infinity so pre-epoch instants
/// stay correct) and a within-day remainder.
fn unix_to_julian_day(unix_secs: i64) -> (i64, u32) {
    const JD_UNIX_EPOCH: i64 = 2_440_588;
    const SECS_PER_DAY: i64 = 86_400;
    let days = unix_secs.div_euclid(SECS_PER_DAY);
    let secs_in_day = unix_secs.rem_euclid(SECS_PER_DAY);
    let jd = JD_UNIX_EPOCH + days;
    let ms_of_day = (secs_in_day * 1000) as u32;
    (jd, ms_of_day)
}

/// A cursor over a `QDataStream`-encoded byte slice (big-endian read side).
///
/// All readers return `None` on truncation so a malformed/short datagram can
/// never panic the inbound poll loop.
#[derive(Debug, Clone)]
pub struct QdsReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> QdsReader<'a> {
    /// Wrap a byte slice for reading.
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Bytes not yet consumed.
    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.buf.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    /// Read a `quint8`.
    pub fn read_u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    /// Read a `qint8`.
    pub fn read_i8(&mut self) -> Option<i8> {
        self.read_u8().map(|b| b as i8)
    }

    /// Read a big-endian `quint16`.
    pub fn read_u16(&mut self) -> Option<u16> {
        self.take(2)
            .map(|b| u16::from_be_bytes(b.try_into().unwrap()))
    }

    /// Read a serialized `QColor` and reduce it to a CSS `#rrggbb` hex (or
    /// `None` for an Invalid color — the "clear this highlight" sentinel).
    /// QDataStream layout: `qint8` spec (0 = Invalid, 1 = Rgb, …), then five
    /// `quint16`s: alpha, red, green, blue, pad. Non-RGB specs are read fully
    /// (stream stays aligned) but reported as `None`.
    pub fn read_qcolor(&mut self) -> Option<Option<String>> {
        let spec = self.read_i8()?;
        let _alpha = self.read_u16()?;
        let r = self.read_u16()?;
        let g = self.read_u16()?;
        let b = self.read_u16()?;
        let _pad = self.read_u16()?;
        if spec != 1 {
            return Some(None); // Invalid/unsupported spec = clear
        }
        // Qt stores 16-bit channels; CSS wants 8-bit.
        Some(Some(format!(
            "#{:02x}{:02x}{:02x}",
            (r >> 8) as u8,
            (g >> 8) as u8,
            (b >> 8) as u8
        )))
    }

    /// Read a big-endian `quint32`.
    pub fn read_u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    }

    /// Read a big-endian `qint32`.
    pub fn read_i32(&mut self) -> Option<i32> {
        self.take(4)
            .map(|b| i32::from_be_bytes(b.try_into().unwrap()))
    }

    /// Read a big-endian `quint64`.
    pub fn read_u64(&mut self) -> Option<u64> {
        self.take(8)
            .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
    }

    /// Read a big-endian `qint64`.
    pub fn read_i64(&mut self) -> Option<i64> {
        self.take(8)
            .map(|b| i64::from_be_bytes(b.try_into().unwrap()))
    }

    /// Read a `double`.
    pub fn read_f64(&mut self) -> Option<f64> {
        self.take(8)
            .map(|b| f64::from_be_bytes(b.try_into().unwrap()))
    }

    /// Read a `bool` (one byte; non-zero is true).
    pub fn read_bool(&mut self) -> Option<bool> {
        self.read_u8().map(|b| b != 0)
    }

    /// Read a `QString`/UTF-8 byte array: `quint32` length then the bytes.
    /// Returns `Ok(None)` for the null sentinel (`0xFFFFFFFF`), `Ok(Some(_))`
    /// for present strings (lossily decoded), or `None` on truncation.
    pub fn read_utf8(&mut self) -> Option<Option<String>> {
        let len = self.read_u32()?;
        if len == 0xFFFF_FFFF {
            return Some(None);
        }
        let bytes = self.take(len as usize)?;
        Some(Some(String::from_utf8_lossy(bytes).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_big_endian() {
        let mut w = QdsWriter::new();
        w.put_u8(0x12)
            .put_u32(0x01020304)
            .put_i32(-1)
            .put_u64(0x0102030405060708);
        assert_eq!(
            w.as_bytes(),
            &[
                0x12, // u8
                0x01, 0x02, 0x03, 0x04, // u32 BE
                0xFF, 0xFF, 0xFF, 0xFF, // i32 -1
                0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // u64 BE
            ]
        );
    }

    #[test]
    fn f64_is_big_endian_ieee754() {
        let mut w = QdsWriter::new();
        w.put_f64(1.0);
        // 1.0 == 0x3FF0000000000000, big-endian.
        assert_eq!(
            w.as_bytes(),
            &[0x3F, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn bool_is_one_byte() {
        let mut w = QdsWriter::new();
        w.put_bool(true).put_bool(false);
        assert_eq!(w.as_bytes(), &[0x01, 0x00]);
    }

    #[test]
    fn utf8_string_has_u32_length_prefix() {
        let mut w = QdsWriter::new();
        w.put_utf8(Some("Tempo"));
        assert_eq!(
            w.as_bytes(),
            &[
                0x00, 0x00, 0x00, 0x05, // length 5
                b'T', b'e', b'm', b'p', b'o',
            ]
        );
    }

    #[test]
    fn utf8_none_is_null_sentinel() {
        let mut w = QdsWriter::new();
        w.put_utf8(None);
        assert_eq!(w.as_bytes(), &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn utf8_empty_string_is_zero_length() {
        let mut w = QdsWriter::new();
        w.put_utf8(Some(""));
        assert_eq!(w.as_bytes(), &[0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn qtime_is_u32_millis() {
        let mut w = QdsWriter::new();
        // 12:34:56.000 -> ((12*3600)+(34*60)+56)*1000 = 45_296_000 ms.
        w.put_qtime(45_296_000);
        assert_eq!(w.as_bytes(), &45_296_000u32.to_be_bytes());
    }

    #[test]
    fn julian_day_of_unix_epoch() {
        let (jd, ms) = unix_to_julian_day(0);
        assert_eq!(jd, 2_440_588);
        assert_eq!(ms, 0);
    }

    #[test]
    fn qdatetime_layout_known_instant() {
        // 2020-01-01T00:00:00Z = 1577836800 unix secs.
        // Julian day of 2020-01-01 is 2458850.
        let mut w = QdsWriter::new();
        w.put_qdatetime(1_577_836_800);
        let mut expect = Vec::new();
        expect.extend_from_slice(&2_458_850i64.to_be_bytes()); // JD
        expect.extend_from_slice(&0u32.to_be_bytes()); // ms of day
        expect.push(1); // UTC
        assert_eq!(w.as_bytes(), &expect[..]);
    }

    #[test]
    fn reader_roundtrips_writer() {
        let mut w = QdsWriter::new();
        w.put_u32(0xADBCCBDA)
            .put_u8(42)
            .put_i32(-7)
            .put_u64(1234567890)
            .put_f64(3.5)
            .put_bool(true)
            .put_utf8(Some("CQ"))
            .put_utf8(None);
        let bytes = w.into_bytes();

        let mut r = QdsReader::new(&bytes);
        assert_eq!(r.read_u32(), Some(0xADBCCBDA));
        assert_eq!(r.read_u8(), Some(42));
        assert_eq!(r.read_i32(), Some(-7));
        assert_eq!(r.read_u64(), Some(1234567890));
        assert_eq!(r.read_f64(), Some(3.5));
        assert_eq!(r.read_bool(), Some(true));
        assert_eq!(r.read_utf8(), Some(Some("CQ".to_string())));
        assert_eq!(r.read_utf8(), Some(None));
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn reader_is_truncation_safe() {
        let mut r = QdsReader::new(&[0x00, 0x01]);
        assert_eq!(r.read_u32(), None); // only 2 bytes, no panic
        let mut r2 = QdsReader::new(&[0x00, 0x00, 0x00, 0x05, b'h', b'i']);
        assert_eq!(r2.read_utf8(), None); // claims len 5, only 2 present
    }

    #[test]
    fn qdatetime_carries_time_of_day() {
        // 2020-01-01T01:02:03Z -> 1577840523 unix secs.
        // ms of day = ((1*3600)+(2*60)+3)*1000 = 3_723_000.
        let mut w = QdsWriter::new();
        w.put_qdatetime(1_577_840_523);
        let mut expect = Vec::new();
        expect.extend_from_slice(&2_458_850i64.to_be_bytes());
        expect.extend_from_slice(&3_723_000u32.to_be_bytes());
        expect.push(1);
        assert_eq!(w.as_bytes(), &expect[..]);
    }
}
