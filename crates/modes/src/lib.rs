//! Nexus mode + signal-source abstractions — the architectural spine of the
//! digital-ops nerve center.
//!
//! Two pluggable seams let the rest of the app stay mode- and source-agnostic:
//!
//! - [`Mode`] — everything mode-specific (T/R timing, frame size, waveform,
//!   decode, passband, capabilities). FT8/FT4/FT1 ship today; a future mode is a
//!   new `impl Mode` with no other changes. ([`Ft8Mode`], [`Ft4Mode`], [`Ft1Mode`].)
//! - [`SignalSource`] — the user-selectable "native engine vs companion" switch:
//!   [`NativeSource`] decodes locally captured audio with a [`Mode`], while
//!   [`WsjtxUdpSource`] consumes an upstream WSJT-X/JTDX/MSHV decode stream over
//!   UDP. Both yield the unified [`Decode`].
//!
//! Every decode, whatever its mode or source, is normalized to one [`Decode`].

pub mod decode;
pub mod mode;
pub mod source;

pub use decode::Decode;
pub use mode::{make_mode, Capabilities, Ft1Mode, Ft4Mode, Ft8Mode, Mode, ModeKind};
pub use source::{DecodeRequest, NativeSource, SignalSource, WsjtxUdpSource};

#[cfg(test)]
mod tests {
    use super::*;

    const FS: f32 = 12_000.0;

    fn to_i16_frame(wave: &[f32], frame_len: usize, off: usize, gain: f32) -> Vec<i16> {
        let mut iwave = vec![0i16; frame_len];
        for (i, &s) in wave.iter().enumerate() {
            let k = off + i;
            if k < frame_len {
                iwave[k] = (s * gain).clamp(-32768.0, 32767.0) as i16;
            }
        }
        iwave
    }

    /// A clean (noise-free) full-frame buffer carrying `msg` at `f0`, built via
    /// the mode's own `encode`/`gen_wave` — so it exercises the trait, not the
    /// concrete crate. FT8 starts at the 0.5 s TX point; FT4 self-positions; FT1
    /// is placed at ~0.4 s (matching the proven acquisition harness).
    fn native_frame(mode: &dyn Mode, msg: &str, f0: f32) -> Vec<i16> {
        let tones = mode.encode(msg);
        assert!(!tones.is_empty(), "{} encode failed", mode.name());
        let wave = mode.gen_wave(&tones, FS, f0);
        // FT8 and FT4 self-position (Mode::gen_wave includes the 0.5 s lead-in); FT1's
        // bare wave is placed at the proven ~0.4 s acquisition point by this harness.
        let off = match mode.kind() {
            ModeKind::Ft8 => 0,
            ModeKind::Ft4 => 0,
            ModeKind::Ft1 => 4_800,
        };
        to_i16_frame(&wave, mode.frame_samples(), off, 1000.0)
    }

    #[test]
    fn mode_metadata() {
        let m8 = make_mode(ModeKind::Ft8);
        assert_eq!(m8.name(), "FT8");
        assert_eq!(m8.slot_secs(), 15.0);
        assert_eq!(m8.frame_samples(), ft8::NMAX);
        assert!(m8.capabilities().fox_hound);
        assert!(!m8.capabilities().ir_harq);

        let m4 = make_mode(ModeKind::Ft4);
        assert_eq!(m4.slot_secs(), 7.5);
        assert_eq!(m4.frame_samples(), ft4::NMAX);
        assert!(!m4.capabilities().fox_hound);

        let m1 = make_mode(ModeKind::Ft1);
        assert_eq!(m1.slot_secs(), 4.0);
        assert_eq!(m1.frame_samples(), ft1::NMAX);
        assert!(m1.capabilities().ir_harq);

        assert_eq!(ModeKind::ALL.len(), 3);
    }

    /// Each native mode decodes its own clean signal through a `Box<dyn
    /// SignalSource>` — proving the Mode + SignalSource dispatch end to end.
    fn native_roundtrip(kind: ModeKind) {
        let msg = "CQ KD9TAW EN52";
        let mode = make_mode(kind);
        let frame = native_frame(mode.as_ref(), msg, 1500.0);

        let mut src: Box<dyn SignalSource> = Box::new(NativeSource::from_kind(kind));
        assert_eq!(src.mode_kind(), Some(kind));
        let decs = src.decode(&DecodeRequest::full_band(&frame));
        assert!(
            decs.iter().any(|d| d.message == msg),
            "{} native source must decode its own signal; got {decs:?}",
            kind.as_str()
        );
        // Every native decode is tagged with the source's mode (so the feed can
        // label it truly, even after a mode switch).
        assert!(
            decs.iter().all(|d| d.mode == Some(kind)),
            "{} native decodes must carry their mode",
            kind.as_str()
        );
    }

    #[test]
    fn native_ft8_through_trait() {
        native_roundtrip(ModeKind::Ft8);
    }

    #[test]
    fn native_ft4_through_trait() {
        native_roundtrip(ModeKind::Ft4);
    }

    #[test]
    fn ft8_gen_wave_is_slot_positioned_with_lead_in() {
        // Mode::gen_wave for FT8 must include the 0.5 s lead-in (slot-positioned), so the
        // radio loop plays it at the slot boundary without going on the air 0.5 s early.
        let m = make_mode(ModeKind::Ft8);
        let tones = m.encode("CQ KD9TAW EN52");
        let wave = m.gen_wave(&tones, FS, 1500.0);
        let lead = (0.5 * FS).round() as usize;
        let bare = ft8::gen_wave(&tones, FS, 1500.0);
        assert_eq!(
            wave.len(),
            lead + bare.len(),
            "FT8 wave includes the 0.5 s lead-in"
        );
        assert!(wave[..lead].iter().all(|&s| s == 0.0), "lead-in is silence");
        assert!(
            wave[lead..].iter().any(|&s| s != 0.0),
            "tones follow the lead-in"
        );
    }

    #[test]
    fn native_ft1_through_trait() {
        native_roundtrip(ModeKind::Ft1);
    }

    /// The companion source maps an upstream WSJT-X `Decode` datagram to a
    /// unified [`Decode`], drained at the next interval.
    #[test]
    fn wsjtx_udp_source_ingests_decode() {
        use tempo_net::wsjtx::{encode_decode, Decode as WxDecode};

        let bytes = encode_decode(
            "WSJT-X",
            &WxDecode {
                new: true,
                time_ms: 1000,
                snr: -7,
                delta_time: 0.1,
                delta_freq: 1200,
                mode: "~",
                message: "CQ W1AW FN31",
                low_confidence: false,
                off_air: false,
            },
        );

        let mut src = WsjtxUdpSource::new();
        let no_audio: Vec<i16> = Vec::new();
        // Nothing queued yet.
        assert!(src.decode(&DecodeRequest::full_band(&no_audio)).is_empty());

        assert!(
            src.ingest_datagram(&bytes),
            "Decode datagram should be queued"
        );
        let decs = src.decode(&DecodeRequest::full_band(&no_audio));
        assert_eq!(decs.len(), 1);
        assert_eq!(decs[0].message, "CQ W1AW FN31");
        assert_eq!(decs[0].snr, -7);
        assert_eq!(decs[0].freq, 1200.0);
        assert_eq!(decs[0].rv, None);
        // The WSJT-X mode ("~" = FT8) is carried through, not our selected tier.
        assert_eq!(decs[0].mode, Some(ModeKind::Ft8));

        // A non-Decode datagram is ignored.
        let close = tempo_net::wsjtx::encode_close("WSJT-X");
        assert!(!src.ingest_datagram(&close));
    }

    /// The companion source receives a real WSJT-X `Decode` datagram over a bound
    /// UDP socket and surfaces it via `decode()` — proving the live network path,
    /// not just `ingest_datagram`.
    #[test]
    fn wsjtx_udp_source_receives_over_socket() {
        use std::net::UdpSocket;
        use tempo_net::wsjtx::{encode_decode, Decode as WxDecode};

        let mut src = WsjtxUdpSource::bind("127.0.0.1:0").expect("bind ephemeral");
        let addr = src.local_addr().expect("bound addr");

        let bytes = encode_decode(
            "WSJT-X",
            &WxDecode {
                new: true,
                time_ms: 2000,
                snr: -12,
                delta_time: -0.2,
                delta_freq: 1500,
                mode: "~",
                message: "K1JT W1AW -15",
                low_confidence: false,
                off_air: false,
            },
        );
        UdpSocket::bind("127.0.0.1:0")
            .expect("tx socket")
            .send_to(&bytes, addr)
            .expect("send datagram");

        // Loopback delivery is near-instant but async; poll briefly via the trait.
        let no_audio: Vec<i16> = Vec::new();
        let mut decs = Vec::new();
        for _ in 0..50 {
            decs = src.decode(&DecodeRequest::full_band(&no_audio));
            if !decs.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(decs.len(), 1, "a decode should arrive over the UDP socket");
        assert_eq!(decs[0].message, "K1JT W1AW -15");
        assert_eq!(decs[0].snr, -12);
        assert_eq!(decs[0].freq, 1500.0);
        assert_eq!(
            decs[0].mode,
            Some(ModeKind::Ft8),
            "WSJT-X mode carried through"
        );
    }

    /// Native and companion sources are interchangeable behind the trait object.
    #[test]
    fn sources_are_polymorphic() {
        let sources: Vec<Box<dyn SignalSource>> = vec![
            Box::new(NativeSource::from_kind(ModeKind::Ft4)),
            Box::new(WsjtxUdpSource::new()),
        ];
        assert_eq!(sources[0].label(), "Native (FT4)");
        assert_eq!(sources[1].label(), "WSJT-X UDP");
        assert_eq!(sources[0].mode_kind(), Some(ModeKind::Ft4));
        assert_eq!(sources[1].mode_kind(), None);
    }
}
