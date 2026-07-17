//! Serial DTR/RTS keyline CW keyer — the classic "PC keys the rig's KEY jack" method.
//!
//! The app toggles a serial control line (DTR or RTS) with Morse timing; a simple
//! transistor/optoisolator interface keys the rig's straight-key line, and the RIG (in CW
//! mode) shapes the actual RF envelope. This is what N1MM+, fldigi, Win-Test, and DXLab
//! call "serial CW keying" — the clean way to key a rig that lacks CAT CW (e.g. the
//! IC-756PRO III) without buying a WinKeyer, and without the soundcard-through-SSB
//! workaround.
//!
//! Unlike the WinKeyer (hardware owns the timing) or the soundcard keyer (the audio
//! callback plays a pre-rendered tone), this keyer must generate the element timing
//! itself, so it owns a dedicated keying thread. The service loop hands it one word at a
//! time; the thread walks `tempo_core::cw::morse_key_events` and toggles the line, keying
//! up immediately on abort. (PC-generated timing has some OS jitter — inherent to serial
//! keying; a WinKeyer is the jitter-free upgrade.)

/// Which control line keys the rig. **Dtr** is the CW convention (RTS = PTT); **Rts** is
/// offered because some interfaces wire it the other way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyLine {
    Dtr,
    Rts,
}

impl KeyLine {
    /// Parse `"dtr"`/`"rts"` (case-insensitive); anything else → `Dtr` (the default).
    pub fn parse(s: &str) -> Self {
        if s.eq_ignore_ascii_case("rts") {
            KeyLine::Rts
        } else {
            KeyLine::Dtr
        }
    }
}

#[cfg(feature = "serial")]
pub use imp::SerialKeyer;

#[cfg(feature = "serial")]
mod imp {
    use super::KeyLine;
    use serialport::SerialPort;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    fn set_key(sp: &mut dyn SerialPort, line: KeyLine, down: bool) {
        let _ = match line {
            KeyLine::Dtr => sp.write_data_terminal_ready(down),
            KeyLine::Rts => sp.write_request_to_send(down),
        };
    }

    /// The keying thread: block for a word, key its Morse elements on the line, key up on
    /// abort (checked every ~8 ms so Stop TX is snappy), and exit when the channel closes.
    fn keyer_loop(
        mut sp: Box<dyn SerialPort>,
        line: KeyLine,
        rx: mpsc::Receiver<(String, u32)>,
        abort: Arc<AtomicBool>,
    ) {
        set_key(&mut *sp, line, false); // idle: key up
        while let Ok((text, wpm)) = rx.recv() {
            abort.store(false, Ordering::Relaxed); // a fresh word consumes any prior abort
            let mut aborted = false;
            'word: for (down, ms) in tempo_core::cw::morse_key_events(&text, wpm) {
                set_key(&mut *sp, line, down);
                // Sleep the element in slices so an abort keys up within ~8 ms.
                let mut left = ms;
                while left > 0 {
                    if abort.load(Ordering::Relaxed) {
                        set_key(&mut *sp, line, false);
                        aborted = true;
                        break 'word;
                    }
                    let chunk = left.min(8);
                    thread::sleep(Duration::from_millis(chunk as u64));
                    left -= chunk;
                }
            }
            set_key(&mut *sp, line, false); // key up between words
            if aborted {
                while rx.try_recv().is_ok() {} // drop the rest of the aborted macro
            }
        }
        set_key(&mut *sp, line, false); // channel closed (keyer dropped) → key up + exit
    }

    /// An open serial keyline keyer with its own keying thread.
    pub struct SerialKeyer {
        tx: Option<mpsc::Sender<(String, u32)>>,
        abort: Arc<AtomicBool>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl SerialKeyer {
        /// Open `port` and spawn the keying thread. 1200 baud is arbitrary — only the DTR/RTS
        /// control line is toggled, no data bytes are sent. The line starts keyed up.
        pub fn open(port: &str, line: KeyLine) -> std::io::Result<Self> {
            let sp = serialport::new(port, 1200)
                .timeout(Duration::from_millis(200))
                .open()
                .map_err(|e| std::io::Error::other(e.to_string()))?;
            let (tx, rx) = mpsc::channel();
            let abort = Arc::new(AtomicBool::new(false));
            let abort_thread = abort.clone();
            let handle = thread::spawn(move || keyer_loop(sp, line, rx, abort_thread));
            Ok(Self {
                tx: Some(tx),
                abort,
                handle: Some(handle),
            })
        }

        /// Queue `text` to be keyed at `wpm` (non-blocking — the thread does the timing).
        pub fn send(&self, text: &str, wpm: u32) {
            if let Some(tx) = &self.tx {
                let _ = tx.send((text.to_string(), wpm));
            }
        }

        /// Abort NOW: key up and drop any queued words (Stop TX).
        pub fn clear(&self) {
            self.abort.store(true, Ordering::Relaxed);
        }
    }

    impl Drop for SerialKeyer {
        fn drop(&mut self) {
            self.abort.store(true, Ordering::Relaxed); // interrupt a word in progress
            self.tx = None; // close the channel → the thread keys up and exits
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyline_parses_case_insensitively_and_defaults_to_dtr() {
        assert_eq!(KeyLine::parse("dtr"), KeyLine::Dtr);
        assert_eq!(KeyLine::parse("DTR"), KeyLine::Dtr);
        assert_eq!(KeyLine::parse("rts"), KeyLine::Rts);
        assert_eq!(KeyLine::parse("RTS"), KeyLine::Rts);
        assert_eq!(KeyLine::parse(""), KeyLine::Dtr); // unknown → the CW convention
        assert_eq!(KeyLine::parse("garbage"), KeyLine::Dtr);
    }
}
