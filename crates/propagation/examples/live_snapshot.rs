//! Verify the live adapters against the REAL NOAA SWPC + PSK Reporter services.
//!
//!   cargo run -p propagation --example live_snapshot --features live -- KD9TAW EN52
//!
//! Respect PSK Reporter's ≥5-minute-per-dataset query limit when re-running.

fn main() {
    let mut args = std::env::args().skip(1);
    let call = args.next().unwrap_or_else(|| "KD9TAW".to_string());
    let grid = args.next().unwrap_or_else(|| "EN52".to_string());

    eprintln!("Fetching live propagation nowcast for {call} ({grid})…");
    // No logbook here → empty needs → active DXpeditions show as ATNO candidates.
    let needs = propagation::LogNeeds::new();
    let s = match propagation::live::snapshot(&call, &grid, 1800, &needs) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERROR: {e}");
            std::process::exit(1);
        }
    };

    let wx = &s.space_wx;
    println!(
        "Space weather (live): SFI {:.0} | Kp {:.1} | A {:.0} | X-ray {}{}",
        wx.sfi,
        wx.kp,
        wx.a_index,
        wx.xray_class,
        if wx.flare { " [FLARE]" } else { "" }
    );
    println!("Headline: {}", s.advisory.headline);
    if !s.advisory.banners.is_empty() {
        for b in &s.advisory.banners {
            println!("  ! {b}");
        }
    }
    println!("Bands (top 6 by score):");
    for b in s.advisory.bands.iter().take(6) {
        let region = b
            .best_region
            .as_ref()
            .map(|r| r.region.as_str())
            .unwrap_or("-");
        println!(
            "  {:<4} {:<8} score={:.2}  {}↓ {}↑  {:<14} {}",
            b.band,
            format!("{:?}", b.tier),
            b.score,
            b.n_hear_me,
            b.n_i_hear,
            region,
            b.reason
        );
    }
    println!("Openings: {}", s.openings.len());
    for o in &s.openings {
        println!(
            "  ⚡ {} {} point {} ~{} km p={:.2} {}",
            o.band, o.mode, o.octant, o.max_km as i32, o.probability, o.note
        );
    }

    let dx = &s.dxpeditions;
    println!("\nDXpeditions — on the air now: {}", dx.workable_now.len());
    for c in dx.workable_now.iter().take(10) {
        println!(
            "  📡 {:<8} {:<22} {:<3} {:?}  {}",
            c.call, c.entity, c.octant, c.status, c.how_to_call
        );
    }

    println!(
        "\nDXpedition calendar — upcoming/announced: {}",
        dx.upcoming.len()
    );
    for c in dx.upcoming.iter().take(15) {
        let days = ((c.start_unix - now_unix()) / 86_400).max(0);
        let bands = if c.bands.is_empty() {
            "?".to_string()
        } else {
            c.bands.join(",")
        };
        let modes = if c.modes.is_empty() {
            "?".to_string()
        } else {
            c.modes.join("/")
        };
        println!(
            "  🗓  T-{:>3}d  {:<8} {:<22} {:<10} {:<6} {:<14} {} {:.0}°",
            days, c.call, c.entity, bands, modes, c.region, c.octant, c.bearing_deg
        );
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
