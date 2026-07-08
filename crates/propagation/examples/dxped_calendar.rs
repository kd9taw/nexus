//! Verify the DXpedition feed + contact-likelihood linkage on REAL data: NG3K
//! ADXO calendar + ClubLog active overlay + live NOAA space weather → modelled
//! "workable now" likelihood (active) and best daily windows (calendar).
//! No PSK Reporter call, so it's safe to re-run.
//!
//!   cargo run -p propagation --example dxped_calendar --features live -- KD9TAW EN52

use propagation::live::{dxped, swpc};
use propagation::{DxpeditionTracker, LogNeeds, PropAdvisor};

fn main() {
    let mut args = std::env::args().skip(1);
    let call = args.next().unwrap_or_else(|| "KD9TAW".to_string());
    let grid = args.next().unwrap_or_else(|| "EN52".to_string());

    eprintln!("Fetching NG3K calendar + ClubLog active + NOAA space weather for {call} ({grid})…");
    let plans = match dxped::fetch_plans() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR (dxped): {e}");
            std::process::exit(1);
        }
    };
    let wx = swpc::fetch_space_wx().unwrap_or_default();
    println!("Space weather: SFI {:.0} | Kp {:.1}\n", wx.sfi, wx.kp);

    let now = now_unix();
    // No live spots here → pure model (calendar planning + active model-only).
    let advisory = PropAdvisor::new(&call, &grid).advise(now, &[], &wx);
    // No logbook connected → empty LogNeeds → every active entity is an ATNO
    // candidate (the real app builds LogNeeds from the operator's ADIF log).
    let needs = LogNeeds::new();
    let dash = DxpeditionTracker::new(&grid).dashboard(now, &plans, &needs, &advisory, &wx);

    println!("ON THE AIR NOW — modelled workability (best needed bands):");
    // Group cards by call, show the best band per call.
    let mut seen = std::collections::HashSet::new();
    for c in &dash.workable_now {
        if !seen.insert(c.call.clone()) {
            continue;
        }
        let best: Vec<_> = dash
            .workable_now
            .iter()
            .filter(|x| x.call == c.call)
            .take(4)
            .map(|x| format!("{} {} ({})", x.band, x.likelihood, x.window_hint))
            .collect();
        println!(
            "  📡 {:<8} {:<22} {:<3}  {}",
            c.call,
            c.entity,
            c.octant,
            best.join(" · ")
        );
    }

    println!("\nDXPEDITION CALENDAR — when to plan your chase:");
    for c in dash.upcoming.iter().take(18) {
        let days = ((c.start_unix - now) / 86_400).max(0);
        println!(
            "  🗓  T-{:>3}d  {:<8} {:<22} {:<3}  ⇒ {}",
            days,
            c.call,
            c.entity,
            c.octant,
            if c.best.is_empty() {
                "—".to_string()
            } else {
                c.best.clone()
            }
        );
        for o in c.outlook.iter().take(3) {
            println!(
                "            {:<4} {:<10} {}",
                o.band, o.workability, o.window
            );
        }
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
