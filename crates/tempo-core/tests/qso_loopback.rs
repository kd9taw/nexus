//! End-to-end auto-sequenced FT1 QSO between two stations over the virtual air.
//!
//! Proves the sequencer + message layer + modem complete a real exchange
//! (CQ → grid reply → report → R-report → RR73 → 73) with no human in the loop,
//! entirely headless.

use tempo_core::message::Msg;
use tempo_core::qso::{run_loopback_qso, State, Station};

#[test]
fn two_stations_complete_a_qso() {
    let mut a = Station::calling_cq("W9XYZ", "EN37"); // initiator
                                                      // Responder answers A explicitly (Monitor is now passive — it never auto-keys;
                                                      // the operator works a station by double-clicking, i.e. an `answering` station).
    let mut b = Station::answering("K2DEF", "FN31", "W9XYZ");

    let air = run_loopback_qso(&mut a, &mut b, 15.0, 20);

    // Both sides reached completion.
    assert_eq!(a.state, State::Done, "A transcript: {:?}", a.transcript);
    assert_eq!(b.state, State::Done, "B transcript: {:?}", b.transcript);
    assert!(a.done() && b.done());

    // They identified each other.
    assert_eq!(a.dxcall.as_deref(), Some("K2DEF"));
    assert_eq!(b.dxcall.as_deref(), Some("W9XYZ"));

    // Each received a signal report about its own signal.
    assert!(a.rx_report.is_some(), "A never got a report");
    assert!(b.rx_report.is_some(), "B never got a report");

    // The on-air exchange contains the full canonical sequence, in order.
    let texts: Vec<String> = air.iter().map(|e| e.text.clone()).collect();
    let kinds: Vec<Msg> = texts.iter().map(|t| Msg::parse(t)).collect();

    let pos = |pred: &dyn Fn(&Msg) -> bool| kinds.iter().position(pred);
    let cq = pos(&|m| matches!(m, Msg::Cq { .. })).expect("a CQ");
    let grid = pos(&|m| matches!(m, Msg::Grid { .. })).expect("a grid reply");
    let report = pos(&|m| matches!(m, Msg::Report { .. })).expect("a report");
    let rreport = pos(&|m| matches!(m, Msg::RReport { .. })).expect("an R-report");
    let rr73 = pos(&|m| matches!(m, Msg::Rr73 { .. })).expect("an RR73");
    let bye = pos(&|m| matches!(m, Msg::Bye73 { .. })).expect("a 73");

    assert!(
        cq < grid && grid < report && report < rreport && rreport < rr73 && rr73 <= bye,
        "sequence out of order: {texts:?}"
    );

    // Sanity: the whole QSO fit in a handful of slots.
    assert!(air.len() <= 8, "QSO took too many overs: {texts:?}");

    eprintln!("--- QSO transcript ---");
    for e in &air {
        eprintln!("slot {:>2}  {}: {}", e.slot, e.from, e.text);
    }
}
