//! End-to-end ARRL Field Day exchange between two stations over the real modem +
//! virtual channel: a running station and a search-and-pounce station complete
//! the Class+Section exchange, both log the contact, and exports are valid.

use tempo_core::fieldday::{run_loopback_fieldday, Exchange, FieldDayStation};

#[test]
fn two_stations_complete_field_day_exchange() {
    let mut running = FieldDayStation::running("W9XYZ", "EN37", Exchange::new("3A", "WI"), "20M");
    let mut sp =
        FieldDayStation::search_and_pounce("K2DEF", "FN31", Exchange::new("2A", "IL"), "20M");

    run_loopback_fieldday(&mut running, &mut sp, 15.0, 40);

    // Both stations logged the contact.
    assert_eq!(
        running.log.qso_count(),
        1,
        "running: {:?}",
        running.transcript
    );
    assert_eq!(sp.log.qso_count(), 1, "sp: {:?}", sp.transcript);

    // Running logged the caller's exchange (K2DEF 2A IL).
    let r = &running.log.qsos()[0];
    assert_eq!(
        (r.call.as_str(), r.class.as_str(), r.section.as_str()),
        ("K2DEF", "2A", "IL")
    );

    // S&P logged the runner's exchange (W9XYZ 3A WI).
    let s = &sp.log.qsos()[0];
    assert_eq!(
        (s.call.as_str(), s.class.as_str(), s.section.as_str()),
        ("W9XYZ", "3A", "WI")
    );

    // Exports are well-formed.
    let cab = running.log.cabrillo(14_074);
    assert_eq!(cab.matches("QSO:").count(), 1);
    assert!(cab.contains("W9XYZ 3A WI K2DEF 2A IL"), "cabrillo: {cab}");
    assert!(running.log.adif().contains("K2DEF") && running.log.adif().contains("ARRL_SECT"));

    eprintln!("--- Field Day ---");
    eprintln!("running W9XYZ log: {:?}", running.log.qsos());
    eprintln!("S&P K2DEF log:     {:?}", sp.log.qsos());
}
