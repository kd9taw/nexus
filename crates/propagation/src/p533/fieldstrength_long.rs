//! Median sky-wave field strength for LONG paths — the port of ITU-R P.533 §5.3
//! ("Paths longer than 7000 km", reference `MedianSkywaveFieldStrengthLong.c`)
//! and §5.4 ("Paths between 7000 and 9000 km", reference
//! `Between7000kmand9000km.c`). The method is Dambolt–Suessmann's (patterned on
//! REC533's `FTZ()`).
//!
//! VERBATIM numerical port: every constant is copied digit-for-digit, including
//! the reference's truncated `D2R = 0.0174532925` / `R2D = 57.2957795` (NOT
//! `PI/180`) where the C uses those literals — required for bit-parity with the
//! reference build we gate against.
//!
//! The C reads/writes a `struct PathData`; this port makes the consumed fields
//! explicit function parameters instead. Fields consumed: `L_tx`/`L_rx`
//! (tx/rx), `CP[MP].L` (path midpoint — for the bearing + winter anomaly),
//! `distance`, `frequency`, `month`, `hour` (the reference hour SLOT = UT−1),
//! `SSN` (used RAW in the fL formula, which the reference notes "can be greater
//! than MAXSSN"; control-point building clamps internally where the reference
//! does), `txpower` (dB(1 kW)). Field-strength outputs land in [`LongPath`].
//!
//! Antenna gain: the reference `AntennaGain08()` returns the largest gain over
//! 0–8° elevation. Our engine is ISOTROPIC-only, so that maximum reduces to the
//! constant isotropic gain; it is taken as the `gtl` parameter (default 0.0).
//!
//! INTEGRATION NOTE — this module calls four helpers that currently exist in
//! [`super::muf`] but are private there. To compile, bump them to `pub` in
//! `muf.rs` (all validated verbatim ports already):
//!   `pub fn calc_b`, `pub fn calc_dmax`, `pub fn calc_f2dmuf`, `pub fn find_fof2var`.
//! (`incidence_angle`, `elevation_angle`, `what_season` are already `pub`.)

use super::cp::{self, ControlPt};
use super::geometry::{self, Location};
use super::muf;

/// Reference `PI` (equals `std::f64::consts::PI` as an f64).
const PI: f64 = std::f64::consts::PI;
/// Earth radius (km) — P.533 `R0`.
const R0: f64 = geometry::R0;
/// Reference `Common.h` truncated conversions (NOT `PI/180` — kept for parity).
const D2R: f64 = 0.0174532925;
const R2D: f64 = 57.2957795;
/// "Not otherwise included" loss for the long model (`NOIL`).
const NOIL: f64 = -0.17;
/// Minimum elevation angle (degrees) for the long model (`MINELEANGLEL`).
const MIN_ELE_ANGLE_L: f64 = 3.0;
/// Gyrofrequency height index for 300 km (`HR300km`).
const HR300KM: usize = 1;
/// Reference-time "no time found" sentinel (`NOTIME`).
const NOTIME: i32 = 99;
/// Control-point array indices for T+dM/2 and R−dM/2 (`TdM2` / `RdM2`).
const TDM2: usize = 26;
const RDM2: usize = 27;
/// Potential control points: 26 penetration points + 2 Table-1a control points.
const MAXCP: usize = 28;
/// Winter-anomaly hemisphere indices (`NORTH` / `SOUTH`).
const NORTH: usize = 0;
const SOUTH: usize = 1;
/// Decile flags (`DL` / `DU`).
const DL: usize = 0;
const DU: usize = 1;

/// Everything the long-path model + §5.4 interpolation write back onto the path.
#[derive(Debug, Clone, Default)]
pub struct LongPath {
    /// Overall resultant median field strength (dB(1 µV/m)) — `path->El`.
    pub el: f64,
    /// Free-space field strength for 3 MW EIRP — `path->E0`.
    pub e0: f64,
    /// Largest antenna gain in 0–8° elevation — `path->Gtl`.
    pub gtl: f64,
    /// Long-distance focusing gain (dB), limited to 15 — `path->Gap`.
    pub gap: f64,
    /// "Not otherwise included" loss — `path->Ly`.
    pub ly: f64,
    /// Mean gyrofrequency (MHz) — `path->fH`.
    pub fh: f64,
    /// Upper reference frequency (MHz) — `path->fM`.
    pub fm: f64,
    /// Lower reference frequency (MHz) — `path->fL`.
    pub fl: f64,
    /// `f(f, fH, fL, fM)` term (eqn 28) — `path->F`.
    pub f: f64,
    /// Slant range — `path->ptick`.
    pub ptick: f64,
    /// Correction factors at the two control points — `path->K[2]`.
    pub k: [f64; 2],

    // The following are set only when distance > 9000 km (long model exclusive).
    /// Composite elevation angle — `path->ele`.
    pub ele: f64,
    /// `path->dmax` (fixed at 4000 km for the long model).
    pub dmax: f64,
    /// Basic MUF (MHz) — `path->BMUF`.
    pub bmuf: f64,
    pub muf50: f64,
    pub muf10: f64,
    pub muf90: f64,
    pub opmuf: f64,
    pub opmuf10: f64,
    pub opmuf90: f64,
    /// Control-point copies the reference stows into `path->CP[Td02/Rd02/T1k/R1k]`.
    pub cp_td02: ControlPt,
    pub cp_rd02: ControlPt,
    pub cp_t1k: ControlPt,
    pub cp_r1k: ControlPt,
}

/// Place a control point on the great circle and fill its parameters — the
/// reference `ZeroCP()` + `GreatCirclePoint()` + `CalculateCPParameters()`
/// sequence (a fresh `ControlPt::default()` supplies the `ZeroCP`).
fn set_cp(
    cp: &mut ControlPt,
    tx: Location,
    rx: Location,
    distance: f64,
    fracd: f64,
    month0: usize,
    hour_slot: i32,
    ssn: f64,
) {
    let (loc, dist) = geometry::great_circle_point(tx, rx, distance, fracd);
    cp.lat = loc.lat;
    cp.lng = loc.lng;
    cp.distance = dist;
    cp::calculate_cp_parameters(cp, month0, hour_slot, ssn);
}

/// Port of `MedianSkywaveFieldStrengthLong()` — §5.3 field strength for paths
/// ≥ 7000 km (the ONLY method used beyond 9000 km). `mp` is the path midpoint
/// (`path->CP[MP].L`); `hour_slot` is the reference hour slot (UT−1); `ssn` is
/// the raw SSN; `gtl` is the isotropic 0–8° antenna gain (0.0 default).
#[allow(clippy::too_many_arguments)]
pub fn median_skywave_field_strength_long(
    tx: Location,
    rx: Location,
    mp: Location,
    distance: f64,
    frequency: f64,
    month0: usize,
    hour_slot: i32,
    ssn: f64,
    txpower: f64,
    gtl: f64,
) -> LongPath {
    let mut out = LongPath::default();

    // (reference line 115) This procedure applies only to paths ≥ 7000 km.
    if distance < 7000.0 {
        return out;
    }

    // (reference line 118) For both fM and fL the reflection height is 300 km.
    let hr = 300.0;

    // (reference lines 120-133) fL geometry: equal hops ≤ 3000 km.
    let mut n = 0i32;
    while distance / (n as f64 + 1.0) > 3000.0 {
        n += 1;
    }
    let n_l = n;
    let d_l = distance / (n_l as f64 + 1.0);
    let delta_l = muf::elevation_angle(d_l, hr);

    // (reference lines 135-162) fM geometry: equal hops ≤ 4000 km, add a hop if
    // the elevation angle falls below the long-model minimum.
    let mut n = 0i32;
    while distance / (n as f64 + 1.0) > 4000.0 {
        n += 1;
    }
    let mut n_m = n;
    let mut d_m = distance / (n_m as f64 + 1.0);
    let mut delta_m = muf::elevation_angle(d_m, hr);
    if delta_m < MIN_ELE_ANGLE_L * D2R {
        n_m += 1;
        d_m = distance / (n_m as f64 + 1.0);
        delta_m = muf::elevation_angle(d_m, hr);
    }

    // (reference lines 179-186) 90-km penetration geometry.
    let i90 = muf::incidence_angle(delta_l, 90.0);
    let phi = PI / 2.0 - delta_l - i90;
    let dh90 = R0 * phi;

    // (reference lines 188-242) 24 hours of penetration + Table-1a control points.
    let mut cp: Vec<Vec<ControlPt>> = vec![vec![ControlPt::default(); 24]; MAXCP];
    for j in 0..24usize {
        let slot = j as i32;
        for i in 0..=(n_l as usize) {
            // (reference lines 199-217) Two penetration points per hop.
            // End nearest the tx.
            let fracd = (i as f64 * d_l + dh90) / distance;
            set_cp(
                &mut cp[2 * i][j],
                tx,
                rx,
                distance,
                fracd,
                month0,
                slot,
                ssn,
            );
            cp[2 * i][j].hr = 90.0;
            // End nearest the rx.
            let fracd = ((i as f64 + 1.0) * d_l - dh90) / distance;
            set_cp(
                &mut cp[2 * i + 1][j],
                tx,
                rx,
                distance,
                fracd,
                month0,
                slot,
                ssn,
            );
            cp[2 * i + 1][j].hr = 90.0;
        }

        // (reference lines 219-237) T+dM/2 and R−dM/2 (Table 1a) as the last two.
        let fracd = 1.0 / (2.0 * (n_m as f64 + 1.0));
        set_cp(&mut cp[TDM2][j], tx, rx, distance, fracd, month0, slot, ssn);
        let fracd = 1.0 - (1.0 / (2.0 * (n_m as f64 + 1.0)));
        set_cp(&mut cp[RDM2][j], tx, rx, distance, fracd, month0, slot, ssn);
        cp[TDM2][j].x = 0.0;
        cp[TDM2][j].foe = 0.0;
        cp[TDM2][j].hr = 300.0; // reflection height fixed at 300 km
        cp[RDM2][j].x = 0.0;
        cp[RDM2][j].foe = 0.0;
        cp[RDM2][j].hr = 300.0;
    }
    let hour = hour_slot as usize;

    // (reference lines 249-254) Virtual slant range (eqn 19) + free-space field.
    let psi = d_m / (2.0 * R0);
    let ptick = (2.0 * R0 * (psi.sin() / (delta_m + psi).cos())).abs() * (n_m as f64 + 1.0);
    let e0 = 139.6 - 20.0 * ptick.log10();

    // (reference line 256) Isotropic antenna: the 0–8° maximum is the constant
    // isotropic gain, supplied as `gtl`.
    let gtl_out = gtl;

    // (reference lines 259-266) Focusing gain (≤ 15 dB), NOIL loss, tx power.
    let d = distance;
    let gap = (10.0 * (d / (R0 * (d / R0).sin().abs())).log10()).min(15.0);
    let ly = NOIL;
    let pt = txpower;

    // (reference line 269) Mean gyrofrequency: 300-km fH averaged over the CPs.
    let fh = (cp[TDM2][hour].fh[HR300KM] + cp[RDM2][hour].fh[HR300KM]) / 2.0;
    out.fh = fh;

    // (reference line 272) Upper reference frequency fM (+ MUFs beyond 9000 km).
    find_mufs_and_fm(&mut out, &cp, d_m, distance, hour_slot, mp, rx, month0, ssn);

    // (reference line 275) Lower reference frequency fL.
    out.fl = find_fl(
        &cp, n_l, ptick, fh, i90, distance, month0, ssn, mp, hour_slot,
    );

    // (reference lines 277-287) Assemble Etl.
    let f = frequency;
    let fm = out.fm;
    let fl = out.fl;
    let mut etl = ((fl + fh).powi(2) / (f + fh).powi(2)) + ((f + fh).powi(2) / (fm + fh).powi(2));
    etl *= (fm + fh).powi(2) / ((fm + fh).powi(2) + (fl + fh).powi(2));
    out.f = 1.0 - etl;
    etl = e0 * (1.0 - etl);
    etl = etl - 30.0 + pt + gtl_out + gap - ly;
    out.el = etl;

    out.e0 = e0;
    out.ptick = ptick;
    out.gtl = gtl_out;
    out.gap = gap;
    out.ly = ly;

    // (reference lines 293-310) Beyond 9000 km this is the sole method, so set
    // the composite elevation, copy the control points, and pin dmax.
    if distance > 9000.0 {
        out.ele = delta_m;
        out.cp_rd02 = cp[RDM2][hour].clone();
        out.cp_td02 = cp[TDM2][hour].clone();
        out.cp_t1k = cp[0][hour].clone();
        out.cp_r1k = cp[2 * n_l as usize][hour].clone();
        out.dmax = 4000.0;
    }

    out
}

/// Port of `FindMUFsandfM()` — upper reference frequency fM from 24 hours of
/// basic MUFs at the Table-1a control points, plus (beyond 9000 km) the basic /
/// operational MUFs and their deciles. Fills `out.fm`, `out.k`, and the MUF set.
#[allow(clippy::too_many_arguments)]
fn find_mufs_and_fm(
    out: &mut LongPath,
    cp: &[Vec<ControlPt>],
    dm: f64,
    distance: f64,
    hour_slot: i32,
    mp: Location,
    rx: Location,
    month0: usize,
    ssn: f64,
) {
    let hour = hour_slot as usize;

    // (reference lines 397-399) W, X, Y interpolation tables for K.
    let w = [0.1, 0.2];
    let x = [1.2, 0.2];
    let y = [0.6, 0.4];

    // (reference lines 413-419) Distance reduction factor fD — NBS Report 7619,
    // D. Lucas 1963, coefficients rescaled for km hop distances.
    let f_d = ((((((-2.40074637494790e-24 * dm + 25.8520201885984e-21) * dm
        - 92.4986988833091e-18)
        * dm
        + 102.342990689362e-15)
        * dm
        + 22.0776941764705e-12)
        * dm
        + 87.4376851991085e-9)
        * dm
        + 29.1996868566837e-6)
        * dm;

    // (reference lines 424-429) Local-noon (UTC) indices at the two CPs.
    let mut noon = [0i32; 2];
    noon[0] = ((12.0 - cp[TDM2][1].lng / (15.0 * D2R)) as i32) - 1;
    noon[1] = ((12.0 - cp[RDM2][1].lng / (15.0 * D2R)) as i32) - 1;
    noon[0] = (noon[0] + 24) % 24;
    noon[1] = (noon[1] + 24) % 24;

    // (reference lines 432-452) 24-hour basic MUFs fBM + their 24-hour minima.
    let mut fbm = [[0.0f64; 24]; 2];
    let mut fbmmin = [100.0f64; 2];
    for t in 0..24usize {
        // Control point T + dM/2.
        let f4 = 1.1 * cp[TDM2][t].fof2 * cp[TDM2][t].m3kf2;
        let fz = cp[TDM2][t].fof2 + 0.5 * cp[TDM2][t].fh[HR300KM];
        fbm[0][t] = fz + (f4 - fz) * f_d;
        fbmmin[0] = fbm[0][t].min(fbmmin[0]);
        // Control point R − dM/2.
        let f4 = 1.1 * cp[RDM2][t].fof2 * cp[RDM2][t].m3kf2;
        let fz = cp[RDM2][t].fof2 + 0.5 * cp[RDM2][t].fh[HR300KM];
        fbm[1][t] = fz + (f4 - fz) * f_d;
        fbmmin[1] = fbm[1][t].min(fbmmin[1]);
    }

    // (reference lines 456-472) Forward azimuth at the midpoint → interp W/X/Y.
    let mut a = geometry::bearing(mp, rx, false); // SHORTPATH
    if a > PI {
        a -= PI;
    }
    if a >= PI / 2.0 {
        a -= PI / 2.0;
    } else {
        a = PI / 2.0 - a;
    }
    let ew = a / (PI / 2.0);
    let iw = w[0] * (1.0 - ew) + w[1] * ew;
    let iy = y[0] * (1.0 - ew) + y[1] * ew;
    let ix = x[0] * (1.0 - ew) + x[1] * ew;

    // (reference lines 475-510) K at each CP, then fM = min over the two CPs.
    for nn in 0..2usize {
        out.k[nn] = 1.2
            + iw * (fbm[nn][hour] / fbm[nn][noon[nn] as usize])
            + ix * ((fbm[nn][noon[nn] as usize] / fbm[nn][hour]).powf(1.0 / 3.0) - 1.0)
            + iy * (fbmmin[nn] / fbm[nn][noon[nn] as usize]).powi(2);
    }
    out.fm = (out.k[0] * fbm[0][hour]).min(out.k[1] * fbm[1][hour]);

    // (reference lines 513-547) Beyond 9000 km set the basic/operational MUFs.
    if distance > 9000.0 {
        // Reference sets smallerCP to 26 (TdM2) or 27 (RdM2).
        let (smaller_cp, bmuf) = if fbm[0][hour] < fbm[1][hour] {
            (TDM2, fbm[0][hour])
        } else {
            (RDM2, fbm[1][hour])
        };
        out.bmuf = bmuf;

        let ltime = cp[smaller_cp][hour].ltime;
        let lat = cp[smaller_cp][hour].lat;
        let season = muf::what_season(mp.lat, month0);
        let deltal = muf::find_fof2var(season, ltime, lat, ssn, DL);
        let deltau = muf::find_fof2var(season, ltime, lat, ssn, DU);

        out.muf50 = out.bmuf;
        out.muf10 = out.muf50 * deltau;
        out.muf90 = out.muf50 * deltal;
        out.opmuf = out.fm;
        out.opmuf10 = out.opmuf * deltau;
        out.opmuf90 = out.opmuf * deltal;
    }
}

/// Port of `FindfL()` — lower reference frequency from 24 hours of solar zenith
/// angles at the 90-km penetration points. `hops` is nL; `ssn` is used RAW (the
/// reference notes it may exceed MAXSSN here). Returns fL for the present hour.
#[allow(clippy::too_many_arguments)]
fn find_fl(
    cp: &[Vec<ControlPt>],
    hops: i32,
    ptick: f64,
    fh: f64,
    i90: f64,
    distance: f64,
    month0: usize,
    ssn: f64,
    mp: Location,
    hour_slot: i32,
) -> f64 {
    // (reference lines 595-605) Σ √cos(χ) over the penetration points, per hour.
    let mut sum_cos_chi = [0.0f64; 24];
    for t in 0..24usize {
        for i in 0..(2 * (hops as usize + 1)) {
            let chi = cp[i][t].sun.sza;
            if chi > 0.0 && chi < PI / 2.0 {
                sum_cos_chi[t] += chi.cos().sqrt();
            }
        }
    }

    // (reference lines 608-610) Winter-anomaly factor at the midpoint; fLN.
    let aw = winter_anomaly(mp.lat, month0);
    let fln = (distance / 3000.0).sqrt();

    // (reference lines 616-618) 24 hourly fL values (SSN may exceed MAXSSN).
    let mut fl = [0.0f64; 24];
    for i in 0..24usize {
        let arg = ((1.0 + 0.009 * ssn) * sum_cos_chi[i]) / (i90.cos() * (9.5e6 / ptick).ln());
        fl[i] = ((5.3 * arg.sqrt() - fh) * (aw + 1.0)).max(fln);
    }

    // (reference lines 620-642) First day→night LUF transition; ease fL down.
    let mut tr = NOTIME;
    for now in 0..24usize {
        let prev = ((now as i32 - 1 + 24) % 24) as usize;
        if tr == NOTIME && fl[prev] >= 2.0 * fln && fl[now] <= 2.0 * fln {
            tr = now as i32;
            let dt = (2.0 * fln - fl[now]) / (fl[prev] - fl[now]);
            fl[now] = 0.7945 * fl[prev] * (dt * (1.0 - 0.7945) + 0.7945);
            // (reference lines 637-639) `fL[now] < fL[tr]` is `fL[now] < fL[now]`
            // here (tr == now) → always false, a no-op; omitted.
        }
    }

    // (reference lines 644-652) Continue the night decay for three more hours.
    if tr != NOTIME {
        for i in 1..4i32 {
            let now = ((tr + i + 24) % 24) as usize;
            let prev = ((now as i32 - 1 + 24) % 24) as usize;
            fl[now] = (fl[prev] * 0.7945).max(fl[now]);
        }
    }

    // (reference lines 668-680) The "present hour" is slot+1 (FTZ 1-based origin).
    let now = ((hour_slot + 1 + 24) % 24) as usize;
    fl[now]
}

/// Port of `WinterAnomaly()` — Table 5 P.533-12 winter-anomaly factor Aw for any
/// latitude (radians) and 0-based month, interpolated to the 60° peak.
fn winter_anomaly(lat: f64, month0: usize) -> f64 {
    // (reference lines 723-734) Winter-anomaly factor [month][N, S].
    const AW: [[f64; 2]; 12] = [
        [0.30, 0.00], // January
        [0.15, 0.00], // February
        [0.03, 0.00], // March
        [0.00, 0.03], // April
        [0.00, 0.15], // May
        [0.00, 0.30], // June
        [0.00, 0.30], // July
        [0.00, 0.15], // August
        [0.00, 0.03], // September
        [0.03, 0.00], // October
        [0.15, 0.00], // November
        [0.30, 0.00], // December
    ];

    let ins = if lat < 0.0 { SOUTH } else { NORTH };
    let lat = lat.abs(); // all latitudes positive

    if (lat >= 0.0 && lat <= 30.0 * D2R) || lat >= 90.0 * D2R {
        0.0
    } else if lat < 60.0 * D2R {
        AW[month0][ins] * (lat * R2D - 30.0) / 30.0
    } else {
        AW[month0][ins] * (90.0 - lat * R2D) / 30.0
    }
}

/// Port of `Between7000kmand9000km()` — §5.4 interpolation. Interpolates the
/// field strength (in the linear domain) between the short model's `es` at
/// 7000 km and the long model's `el` at 9000 km, and computes the basic MUF
/// from the two Table-1a control points. Returns `Some((Ei, BMUF))` only for
/// 7000 < distance < 9000 km (`path->Ei`, `path->BMUF`); `None` otherwise.
///
/// `n0_f2` is the lowest-order F2 mode index (`path->n0_F2`); `td02`/`rd02` are
/// `path->CP[Td02]`/`path->CP[Rd02]` (taken `&mut` because `calc_b` writes their
/// `x`, mirroring the reference `CalcB()` side effect).
pub fn between_7000_and_9000(
    distance: f64,
    es: f64,
    el: f64,
    n0_f2: usize,
    td02: &mut ControlPt,
    rd02: &mut ControlPt,
) -> Option<(f64, f64)> {
    if 7000.0 < distance && distance < 9000.0 {
        // (reference lines 40-45) Interpolate in the linear domain; eqn (42).
        let xl = 10f64.powf(el / 100.0);
        let xs = 10f64.powf(es / 100.0);
        let xi = xs + ((distance - 7000.0) / 2000.0) * (xl - xs);
        let ei = 100.0 * xi.log10();

        // (reference lines 47-56) Basic MUF at each Table-1a control point.
        let b = muf::calc_b(td02);
        let dmax = muf::calc_dmax(td02).min(4000.0);
        let bmuf0 = muf::calc_f2dmuf(td02, distance / (n0_f2 as f64 + 1.0), dmax, b);

        let b = muf::calc_b(rd02);
        let dmax = muf::calc_dmax(rd02).min(4000.0);
        let bmuf1 = muf::calc_f2dmuf(rd02, distance / (n0_f2 as f64 + 1.0), dmax, b);

        Some((ei, bmuf0.min(bmuf1)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(lat: f64, lng: f64) -> Location {
        Location::new(lat.to_radians(), lng.to_radians())
    }

    /// The long model (> 9000 km) runs end to end and produces physically sane
    /// numbers: a finite field strength, a plausible mean gyrofrequency, ordered
    /// reference frequencies, and the beyond-9000-km path parameters populated.
    #[test]
    fn long_model_is_physically_sane() {
        // Johannesburg → Tokyo ≈ 13.5 Mm.
        let tx = loc(-26.2, 28.0);
        let rx = loc(35.7, 139.7);
        let cps = geometry::control_points(tx, rx);
        assert!(cps.distance > 9000.0, "{}", cps.distance);
        let mp = cps.mp.0;

        let p = median_skywave_field_strength_long(
            tx,
            rx,
            mp,
            cps.distance,
            14.0,
            4,
            11,
            50.0,
            0.0,
            0.0,
        );

        assert!(p.el.is_finite(), "El {}", p.el);
        assert!((0.3..3.0).contains(&p.fh), "fH {}", p.fh);
        assert!(p.fm > 0.0 && p.fm < 60.0, "fM {}", p.fm);
        assert!(p.fl >= 0.0 && p.fl.is_finite(), "fL {}", p.fl);
        assert!(p.e0 > 0.0, "E0 {}", p.e0);
        assert!(p.gap > 0.0 && p.gap <= 15.0, "Gap {}", p.gap);
        assert!((p.ly - (-0.17)).abs() < 1e-12);
        // Beyond-9000-km outputs are populated.
        assert!((p.dmax - 4000.0).abs() < 1e-12);
        assert!(p.ele > 0.0, "ele {}", p.ele);
        assert!(p.bmuf > 0.0 && p.muf10 >= p.muf90, "MUF {p:?}");
        assert!(p.opmuf > 0.0);
        // F = 1 - Etl term must be a fraction.
        assert!(p.f.is_finite());
    }

    /// Below 7000 km the reference returns without touching the path; our port
    /// returns the default.
    #[test]
    fn short_paths_are_untouched() {
        let tx = loc(40.0, -88.0);
        let rx = loc(42.0, -80.0);
        let p = median_skywave_field_strength_long(tx, rx, tx, 700.0, 14.0, 4, 11, 50.0, 0.0, 0.0);
        assert_eq!(p.el, 0.0);
        assert_eq!(p.dmax, 0.0);
    }

    /// Winter-anomaly Table 5 behavior: zero inside ±30°, the month/hemisphere
    /// column selection, and the 60°-peak interpolation.
    #[test]
    fn winter_anomaly_tracks_table_5() {
        // Inside ±30° → 0 regardless of month/hemisphere.
        assert_eq!(winter_anomaly(20f64.to_radians(), 0), 0.0);
        assert_eq!(winter_anomaly((-25f64).to_radians(), 5), 0.0);
        // Northern January peaks at 60°N (Aw = 0.30).
        assert!((winter_anomaly(60f64.to_radians(), 0) - 0.30).abs() < 1e-3);
        // Halfway 30°→60° in the north gives half the peak.
        assert!((winter_anomaly(45f64.to_radians(), 0) - 0.15).abs() < 2e-3);
        // Southern hemisphere uses the S column: January S = 0.00, June S = 0.30.
        assert_eq!(winter_anomaly((-45f64).to_radians(), 0), 0.0);
        assert!((winter_anomaly((-60f64).to_radians(), 5) - 0.30).abs() < 1e-3);
        // Poleward of 90° → 0.
        assert_eq!(winter_anomaly(90f64.to_radians(), 0), 0.0);
    }

    /// §5.4 interpolation: at the midpoint (8000 km) Ei sits between Es and El
    /// (linear-domain midpoint), a basic MUF comes out finite, and the range
    /// gate returns `None` outside 7000–9000 km.
    #[test]
    fn between_interpolates_and_gates() {
        let mut td = ControlPt {
            lat: 40f64.to_radians(),
            lng: (-30f64).to_radians(),
            distance: 3500.0,
            ..Default::default()
        };
        cp::calculate_cp_parameters(&mut td, 4, 11, 50.0);
        let mut rd = ControlPt {
            lat: 48f64.to_radians(),
            lng: 5f64.to_radians(),
            distance: 5000.0,
            ..Default::default()
        };
        cp::calculate_cp_parameters(&mut rd, 4, 11, 50.0);

        // 8000 km → fraction 0.5, so Ei is the linear-domain midpoint of Es/El.
        let (ei, bmuf) = between_7000_and_9000(8000.0, 20.0, 15.0, 2, &mut td, &mut rd).unwrap();
        assert!(ei < 20.0 && ei > 15.0, "Ei {ei} not between El and Es");
        let xs = 10f64.powf(20.0 / 100.0);
        let xl = 10f64.powf(15.0 / 100.0);
        let want = 100.0 * (xs + 0.5 * (xl - xs)).log10();
        assert!((ei - want).abs() < 1e-9, "Ei {ei} vs {want}");
        assert!(bmuf > 0.0 && bmuf.is_finite(), "BMUF {bmuf}");

        // Outside the band → None.
        assert!(between_7000_and_9000(6500.0, 20.0, 15.0, 2, &mut td, &mut rd).is_none());
        assert!(between_7000_and_9000(9500.0, 20.0, 15.0, 2, &mut td, &mut rd).is_none());
    }
}
