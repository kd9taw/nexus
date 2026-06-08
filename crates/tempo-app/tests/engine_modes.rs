//! The live engine drives the QSO and Field Day auto-sequencers end to end:
//! two engines over a virtual channel complete a ragchew QSO and a Field Day
//! exchange, and the results surface in each engine's snapshot (the UI contract).

use tempo_app::dto::Tier;
use tempo_app::engine::Engine;
use tempo_core::channel::{VirtualAir, ON_TIME_OFFSET};
use tempo_core::ft1;

/// Shuttle frames between two engines over the channel for `slots` slots.
/// Engine `a` transmits on even slots, `b` on odd. Returns after `done(a,b)`.
fn run(a: &mut Engine, b: &mut Engine, slots: u64, done: impl Fn(&Engine, &Engine) -> bool) {
    let mut a2b = VirtualAir::new(ft1::SAMPLE_RATE, 11);
    let mut b2a = VirtualAir::new(ft1::SAMPLE_RATE, 22);
    for slot in 0..slots {
        if slot % 2 == 0 {
            for w in a.poll_tx(slot) {
                let rx = a2b.receive(&w, ON_TIME_OFFSET, 15.0);
                b.ingest(&rx, slot);
            }
        } else {
            for w in b.poll_tx(slot) {
                let rx = b2a.receive(&w, ON_TIME_OFFSET, 15.0);
                a.ingest(&rx, slot);
            }
        }
        if done(a, b) {
            break;
        }
    }
}

#[test]
fn qso_mode_completes_through_the_engine() {
    let mut a = Engine::new("W9XYZ", "EN37", 0);
    let mut b = Engine::new("K2DEF", "FN31", 1);
    a.set_tier(Tier::Ft1); // FT1-modem loopback (default tier is now FT8)
    b.set_tier(Tier::Ft1);
    a.set_mode("qso-run").unwrap(); // A RUNS (calls CQ)
    b.set_mode("qso-monitor").unwrap();

    let b_done = |e: &Engine| e.snapshot().qso.map(|q| q.state == "Done").unwrap_or(false);
    let a_logged = |e: &Engine| e.get_log().iter().any(|q| q.call == "K2DEF");
    run(&mut a, &mut b, 60, |a, b| a_logged(a) && b_done(b));

    // The answerer (monitor) reaches Done with the runner as its DX.
    assert!(b_done(&b), "B qso: {:?}", b.snapshot().qso);
    assert_eq!(b.snapshot().qso.unwrap().dxcall.as_deref(), Some("W9XYZ"));
    // The runner logged the contact...
    assert!(a_logged(&a), "A logged K2DEF: {:?}", a.get_log());
    // ...and, because it was RUNNING, returns to calling CQ to work the next caller
    // (WSJT-X run workflow) — give it a few more periods to process its own RR73.
    run(&mut a, &mut b, 12, |a, _| {
        a.snapshot().qso.map(|q| q.state == "CallingCq").unwrap_or(false)
    });
    assert_eq!(
        a.snapshot().qso.unwrap().state,
        "CallingCq",
        "A resumed calling CQ after the QSO"
    );
}

#[test]
fn field_day_mode_logs_through_the_engine() {
    let mut run_st = Engine::new("W9XYZ", "EN37", 0);
    let mut sp = Engine::new("K2DEF", "FN31", 1);
    run_st.set_tier(Tier::Ft1); // FT1-modem loopback (default tier is now FT8)
    sp.set_tier(Tier::Ft1);
    // Configure exchanges via settings, then enter Field Day mode.
    {
        let mut s = run_st.settings().clone();
        s.fd_class = "3A".into();
        s.fd_section = "WI".into();
        run_st.apply_settings(s);
        let mut s = sp.settings().clone();
        s.fd_class = "2A".into();
        s.fd_section = "IL".into();
        sp.apply_settings(s);
    }
    run_st.set_mode("fieldday-run").unwrap();
    sp.set_mode("fieldday-sp").unwrap();

    let logged = |e: &Engine| {
        e.snapshot()
            .field_day
            .map(|f| f.qso_count >= 1)
            .unwrap_or(false)
    };
    run(&mut run_st, &mut sp, 50, |a, b| logged(a) && logged(b));

    let fr = run_st.snapshot().field_day.expect("run field day status");
    let fs = sp.snapshot().field_day.expect("sp field day status");
    assert_eq!(fr.qso_count, 1, "runner log: {:?}", fr.log);
    assert_eq!(fs.qso_count, 1, "sp log: {:?}", fs.log);
    // Runner logged the S&P's exchange and vice versa.
    assert_eq!(fr.log[0].call, "K2DEF");
    assert_eq!(
        (fr.log[0].class.as_str(), fr.log[0].section.as_str()),
        ("2A", "IL")
    );
    assert_eq!(fs.log[0].call, "W9XYZ");
    assert_eq!(
        (fs.log[0].class.as_str(), fs.log[0].section.as_str()),
        ("3A", "WI")
    );
    assert_eq!(fr.points, 2); // one digital QSO = 2 points
}
