//! Ionospheric absorption + auroral loss for the short (≤ 7000 km) sky-wave
//! field-strength model — a VERBATIM numerical port of the helper functions of
//! the reference `MedianSkywaveFieldStrengthShort.c` (lines 512–1416):
//! `AbsorptionTerm`, `DiurnalAbsorptionExponent`, `AbsorptionFactor`,
//! `AbsorptionLayerPenetrationFactor`, `FindLh` (+ `WhatSeasonforLh`),
//! `SmallestCPfoF2` and `PenetrationPoints`. `AntennaGain`/`ZeroCP` are not
//! ported (ZeroCP is subsumed by `ControlPt::default()`).
//!
//! Every constant/table digit is copied from the reference; the P.533 Table 2
//! auroral-loss grids and the CONTP/CONTAT/PHIFUN coefficient tables are
//! reproduced exactly. Degree/radian conversions use the reference's own
//! truncated `D2R`/`R2D` (`Common.h`) so the arithmetic matches bit-for-bit.
//! Angles are radians unless a formula is explicitly in degrees; months are
//! 0-based (`JAN = 0`).

use super::cp::{self, ControlPt};
use super::geometry::{self, Location};
use super::muf::{self, MufPath};
use super::solar;
use std::f64::consts::PI;

// P.533 constants (`Common.h`/`P533.h`) — reference values, verbatim.
const D2R: f64 = 0.0174532925; // PI/180
const R2D: f64 = 57.2957795; // 180/PI
const R0: f64 = geometry::R0; // 6371.009 km

// Layer index into `ControlPt::dip`/`fh` (`P533.h`).
const HR100KM: usize = 0; // height = 100 km

// Month indices (`P533.h`, 0-based).
const JAN: usize = 0;
const FEB: usize = 1;
const MAR: usize = 2;
const APR: usize = 3;
const MAY: usize = 4;
const JUN: usize = 5;
const JUL: usize = 6;
const AUG: usize = 7;
const SEP: usize = 8;
const OCT: usize = 9;
const NOV: usize = 10;
const DEC: usize = 11;

// Season indices (`P533.h`).
const WINTER: usize = 0;
const EQUINOX: usize = 1;
const SUMMER: usize = 2;

// (reference line 512) AbsorptionTerm()
/// Determines the absorption term from its three factors (P.533-12 §5.2.2,
/// Figures 1–3): the absorption factor at local noon, the absorption-layer
/// penetration factor, and the diurnal absorption exponent. Returns the term
/// used to build `Li` in Eqn (20).
pub fn absorption_term(cp: &ControlPt, month0: usize, fv: f64) -> f64 {
    // (reference line 555) Diurnal absorption exponent, p.
    let p = diurnal_absorption_exponent(cp, month0);

    // (reference line 559) Solar zenith at the j-th control point, capped 102°.
    let chij = cp.sun.sza.min(102.0 * D2R);
    // (reference line 561) Eqn (21) with the chij argument.
    let fchij = (0.881 * chij).cos().powf(p).max(0.02);

    // (reference line 566–573) Solar zenith at LOCAL NOON. The reference zeros a
    // temp control point, copies CP into it (so only the location carries), then
    // calls SolarParameters() at the point's local solar noon (CP.Sun.lsn, a UTC
    // fractional hour already stored by CalculateCPParameters). We read back the
    // zenith only, so this is exactly `solar_parameters(loc, month, lsn).sza`.
    let sun_noon = solar::solar_parameters(cp.loc(), month0, cp.sun.lsn);
    let chijnoon = sun_noon.sza;
    // (reference line 578) Eqn (21) with the chijnoon argument.
    let fchijnoon = (0.881 * chijnoon).cos().powf(p).max(0.02);

    // (reference line 581) Absorption factor at local noon, R12 = 0.
    let atnoon = absorption_factor(cp, month0);

    // (reference line 583) Absorption-layer penetration factor, arg fv/foE.
    let phin = absorption_layer_penetration_factor(fv / cp.foe);

    // (reference line 596)
    atnoon * phin * fchij / fchijnoon
}

// (reference line 600) DiurnalAbsorptionExponent()
/// The diurnal absorption exponent, p — a function of the month and magnetic
/// dip (P.533-12 Figure 3). Based on `CONTP()` in REC533.
///
/// Reference test vector (its comment): latitude = 3.4464, magnetic dip = 2.89,
/// month = 4 → p = 0.689 (see the unit test; the comment's month is 1-based).
pub fn diurnal_absorption_exponent(cp: &ControlPt, month0: usize) -> f64 {
    // (reference line 625) Modified-dip breakpoints per month (degrees).
    const PPT: [f64; 12] = [
        30.0, 30.0, 30.0, 27.5, 32.5, 35.0, 37.5, 35.0, 32.5, 30.0, 30.0, 30.0,
    ];

    // (reference line 627) CONTP Chebyshev/polynomial coefficients, months 0–5.
    const PVAL1: [[[f64; 7]; 2]; 6] = [
        [
            [1.510, -0.353, -0.090, 0.191, 0.133, -0.067, -0.053],
            [1.400, -0.365, -1.212, -0.049, 1.187, 0.119, -0.400],
        ],
        [
            [1.490, -0.348, -0.055, 0.164, 0.160, -0.041, -0.080],
            [1.450, -0.119, -0.913, -0.640, 0.347, 0.458, 0.107],
        ],
        [
            [1.520, -0.410, -0.138, 0.308, 0.267, -0.113, -0.133],
            [1.500, -0.492, -0.958, 0.216, 0.267, -0.029, 0.187],
        ],
        [
            [1.580, -0.129, -0.228, -0.192, 0.200, 0.116, -0.027],
            [1.530, -0.468, -1.312, 0.096, 0.973, 0.057, -0.187],
        ],
        [
            [1.590, 0.002, -0.102, -0.579, -0.467, 0.522, 0.613],
            [1.490, -0.937, -1.622, 1.365, 1.720, -0.873, -0.453],
        ],
        [
            [1.600, -0.060, -0.175, -0.037, 0.147, -0.008, -0.027],
            [1.460, -0.881, -1.595, 0.901, 2.133, -0.395, -0.933],
        ],
    ];

    // (reference line 654) CONTP coefficients, months 6–11 (index month-6).
    const PVAL2: [[[f64; 7]; 2]; 6] = [
        [
            [1.60, -0.030, -0.135, -0.137, 0.053, 0.072, 0.027],
            [1.43, -0.902, -1.667, 0.905, 2.480, -0.383, -1.173],
        ],
        [
            [1.59, -0.032, -0.083, -0.119, 0.000, 0.031, 0.053],
            [1.46, -0.831, -1.653, 0.708, 2.320, -0.257, -1.067],
        ],
        [
            [1.59, -0.060, -0.180, -0.181, 0.267, 0.081, -0.107],
            [1.51, -0.809, -1.740, 0.750, 2.240, -0.301, -0.960],
        ],
        [
            [1.57, -0.189, -0.207, -0.005, 0.293, 0.004, -0.107],
            [1.52, -0.433, -1.015, -0.017, 0.440, 0.115, 0.080],
        ],
        [
            [1.55, -0.292, -0.275, 0.093, 0.427, -0.026, -0.187],
            [1.44, -0.279, -0.770, -0.266, 0.053, 0.245, 0.267],
        ],
        [
            [1.51, -0.347, -0.082, 0.160, 0.093, -0.048, -0.027],
            [1.40, -0.355, -1.212, -0.102, 1.187, 0.172, -0.400],
        ],
    ];

    // (reference line 691) Initialize the exponent p.
    let mut p = 0.0;

    // (reference line 694) Modified magnetic dip angle (radians here): the dip
    // is stored in `dip[HR100km]`, the latitude in radians.
    let mut moddip = cp.dip[HR100KM].atan2(cp.lat.cos().sqrt()).abs();

    // (reference line 696)
    if moddip > 70.0 * D2R {
        moddip = 70.0 * D2R;
    }

    // (reference line 700) Southern hemisphere: shift the month by six.
    let mut month = month0;
    if cp.lat < 0.0 {
        month += 6;
        if month > 11 {
            month -= 12;
        }
    }

    // (reference line 705)
    let pp = PPT[month] * D2R;

    // (reference line 707) Normalize moddip to [-1, 1] and pick the sub-table.
    let i;
    if moddip > pp {
        i = 1;
        moddip = -1.0 + 2.0 * (moddip - pp) / (70.0 * D2R - pp);
    } else {
        i = 0;
        moddip = -1.0 + 2.0 * moddip / pp;
    }

    // (reference line 716) Evaluate the polynomial in moddip.
    let mut sx = 1.0;
    for j in 0..7 {
        let a = if month <= 5 {
            PVAL1[month][i][j]
        } else {
            PVAL2[month - 6][i][j]
        };
        p += a * sx;
        sx *= moddip;
    }

    // (reference line 736)
    p
}

// (reference line 740) AbsorptionFactor()
/// The absorption factor `ATnoon` at local noon and R12 = 0 (P.533-12 Figure 1).
/// Based on `CONTAT()` in REC533.
pub fn absorption_factor(cp: &ControlPt, month0: usize) -> f64 {
    // (reference line 766) ATNO[season/month row][geomagnetic-latitude column].
    // Rows: 0 Win, 1 Feb, 2 Mar, 3 Apr, 4 Su_Eq, 5 Summ, 6 Sep, 7 Oct, 8 Nov.
    const ATNO: [[f64; 29]; 9] = [
        [
            323.9, 297.5, 274.5, 256.4, 244.2, 235.0, 229.5, 226.1, 226.8, // Win
            229.0, 232.5, 237.0, 243.4, 249.9, 258.1, 267.5, 277.5, 283.3, 283.2, // Win
            273.1, 257.0, 232.1, 201.4, 171.5, 146.0, 123.0, 103.1, 83.0, 66.6, // Win
        ],
        [
            312.1, 285.1, 263.1, 251.8, 249.5, 250.9, 254.5, 260.3, 266.7, 272.3, // Feb
            277.8, 280.3, 283.9, 284.5, 284.4, 283.0, 278.6, 273.0, 265.7, 256.3, // Feb
            244.8, 232.0, 218.1, 204.5, 189.9, 172.3, 155.3, 135.5, 116.2, // Feb
        ],
        [
            347.7, 321.9, 302.5, 293.8, 291.4, 289.3, 292.1, 296.6, 304.3, 313.0, // Mar
            321.7, 333.8, 342.6, 349.6, 355.2, 355.6, 352.2, 341.7, 327.3, 308.4, // Mar
            286.0, 265.0, 244.1, 223.8, 202.8, 181.8, 160.8, 141.6, 123.4, // Mar
        ],
        [
            338.0, 313.2, 297.0, 290.2, 292.1, 299.4, 308.0, 320.4, 331.6, 340.7, // Apr
            347.8, 353.8, 357.0, 360.0, 359.8, 358.3, 355.8, 350.8, 344.5, 332.7, // Apr
            316.4, 292.5, 266.1, 236.4, 214.0, 193.8, 177.5, 165.0, 155.9, // Apr
        ],
        [
            328.1, 303.8, 287.7, 282.5, 284.4, 289.4, 294.8, 303.6, 312.9, 322.7, // Su_Eq
            332.3, 343.8, 350.6, 358.7, 364.3, 365.8, 362.4, 356.0, 346.7, 333.0, // Su_Eq
            318.8, 299.7, 282.1, 260.5, 240.5, 220.6, 203.9, 186.3, 173.0, // Su_Eq
        ],
        [
            305.1, 288.5, 275.2, 273.7, 278.6, 288.9, 302.5, 319.3, 333.6, 346.3, // Summ
            356.3, 364.7, 371.7, 373.6, 374.2, 373.1, 370.5, 365.1, 358.5, 347.7, // Summ
            335.0, 320.3, 299.1, 276.6, 253.2, 230.7, 214.0, 196.6, 185.3, // Summ
        ],
        [
            345.4, 319.4, 298.7, 290.1, 290.0, 291.8, 296.3, 302.9, 312.1, 320.1, // Sep
            327.8, 334.1, 340.2, 343.3, 345.7, 346.5, 345.3, 341.1, 334.5, 321.7, // Sep
            304.2, 286.8, 265.9, 244.8, 224.1, 204.5, 183.6, 164.1, 145.2, // Sep
        ],
        [
            341.9, 314.8, 295.3, 277.9, 265.0, 258.2, 254.4, 255.8, 257.3, 262.9, // Oct
            268.5, 279.0, 287.5, 295.2, 299.6, 300.2, 298.9, 291.5, 279.0, 262.6, // Oct
            245.7, 227.0, 203.6, 182.3, 163.2, 147.1, 133.9, 119.9, 110.8, // Oct
        ],
        [
            318.8, 293.3, 268.3, 251.7, 240.4, 233.1, 229.4, 228.8, 230.5, 235.5, // Nov
            239.7, 242.6, 245.4, 247.5, 248.9, 249.9, 248.5, 244.4, 237.3, 225.6, // Nov
            213.5, 195.2, 172.7, 151.3, 131.1, 113.1, 100.1, 89.0, 80.0, // Nov
        ],
    ];

    // (reference line 815) Month → row index:
    //   Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec
    //    0   1   2   3   4   5   5   4   6   7   8   0
    let i = match month0 {
        JUL => 5,
        AUG => 4,
        SEP => 6,
        OCT => 7,
        NOV => 8,
        DEC => 0,
        _ => month0, // JAN..JUN keep i = month
    };

    // (reference line 842) Latitude (degrees) → fractional column, interpolate.
    let mut x = (cp.lat * R2D).abs();
    if x >= 70.0 {
        x = 69.99; // keep the (int) cast below from reaching column 28
    }
    x /= 2.5;
    let j = x as usize;
    x -= j as f64;

    // (reference line 847)
    ATNO[i][j + 1] * x + ATNO[i][j] * (1.0 - x)
}

// (reference line 852) AbsorptionLayerPenetrationFactor()
/// The absorption-layer penetration factor (P.533-12 Figure 2), argument `T`
/// the ratio of the vertical-incidence frequency to foE. Based on `PHIFUN()`.
pub fn absorption_layer_penetration_factor(t: f64) -> f64 {
    let x;
    let mut phi;

    // (reference line 875)
    if t <= 1.0 {
        if t < 0.0 {
            phi = 0.0;
        } else {
            x = (t - 0.475) / 0.475;
            phi = (((((-0.093 * x + 0.04) * x + 0.127) * x - 0.027) * x + 0.044) * x + 0.159) * x
                + 0.225;
            phi = phi.min(0.53);
        }
    } else if t <= 2.2 {
        x = (t - 1.65) / 0.55;
        phi =
            (((((0.043 * x - 0.07) * x - 0.027) * x + 0.034) * x + 0.054) * x - 0.049) * x + 0.375;
        phi = phi.min(0.53);
    } else if t <= 10.0 {
        x = t;
        phi = 0.34 + (((10.0 - x) * 0.02) / 7.8);
    } else {
        phi = 0.34;
    }

    // (reference line 903) Multiply by the scaling factor.
    phi /= 0.34;

    phi
}

// (reference line 909) FindLh()
/// The auroral / "other" signal loss `Lh` from P.533-12 Table 2, for a control
/// point, a hop distance `dh`, an `hour` (mid-path local time) and a 0-based
/// month.
pub fn find_lh(cp: &ControlPt, dh: f64, hour: i32, month0: usize) -> f64 {
    // (reference line 937) Lh[transmission range][season][geomagnetic latitude
    // row][mid-path local time column]. Upper block: ranges ≤ 2500 km; lower
    // block: ranges > 2500 km. Each [8][8]: 8 gmlat rows × 8 local-time columns.
    const LH: [[[[f64; 8]; 8]; 3]; 2] = [
        // a) Transmission ranges less than or equal to 2500 km
        [
            [
                // Winter
                [2.0, 6.6, 6.2, 1.5, 0.5, 1.4, 1.5, 1.0],
                [3.4, 8.3, 8.6, 0.9, 0.5, 2.5, 3.0, 3.0],
                [6.2, 15.6, 12.8, 2.3, 1.5, 4.6, 7.0, 5.0],
                [7.0, 16.0, 14.0, 3.6, 2.0, 6.8, 9.8, 6.6],
                [2.0, 4.5, 6.6, 1.4, 0.8, 2.7, 3.0, 2.0],
                [1.3, 1.0, 3.2, 0.3, 0.4, 1.8, 2.3, 0.9],
                [0.9, 0.6, 2.2, 0.2, 0.2, 1.2, 1.5, 0.6],
                [0.4, 0.3, 1.1, 0.1, 0.1, 0.6, 0.7, 0.3],
            ],
            [
                // Equinox
                [1.4, 2.5, 7.4, 3.8, 1.0, 2.4, 2.4, 3.3],
                [3.3, 11.0, 11.6, 5.1, 2.6, 4.0, 6.0, 7.0],
                [6.5, 12.0, 21.4, 8.5, 4.8, 6.0, 10.0, 13.7],
                [6.7, 11.2, 17.0, 9.0, 7.2, 9.0, 10.9, 15.0],
                [2.4, 4.4, 7.5, 5.0, 2.6, 4.8, 5.5, 6.1],
                [1.7, 2.0, 5.0, 3.0, 2.2, 4.0, 3.0, 4.0],
                [1.1, 1.3, 3.3, 2.0, 1.4, 2.6, 2.0, 2.6],
                [0.5, 0.6, 1.6, 1.0, 0.7, 1.3, 1.0, 1.3],
            ],
            [
                // Summer
                [2.2, 2.7, 1.2, 2.3, 2.2, 3.8, 4.2, 3.8],
                [2.4, 3.0, 2.8, 3.0, 2.7, 4.2, 4.8, 4.5],
                [4.9, 4.2, 6.2, 4.5, 3.8, 5.4, 7.7, 7.2],
                [6.5, 4.8, 9.0, 6.0, 4.8, 9.1, 9.5, 8.9],
                [3.2, 2.7, 4.0, 3.0, 3.0, 6.5, 6.7, 5.0],
                [2.5, 1.8, 2.4, 2.3, 2.6, 5.0, 4.6, 4.0],
                [1.6, 1.2, 1.6, 1.5, 1.7, 3.3, 3.1, 2.6],
                [0.8, 0.6, 0.8, 0.7, 0.8, 1.6, 1.5, 1.3],
            ],
        ],
        // b) Transmission ranges greater than 2500 km
        [
            [
                // Winter
                [1.5, 2.7, 2.5, 0.8, 0.0, 0.9, 0.8, 1.6],
                [2.5, 4.5, 4.3, 0.8, 0.3, 1.6, 2.0, 4.8],
                [5.5, 5.0, 7.0, 1.9, 0.5, 3.0, 4.5, 9.6],
                [5.3, 7.0, 5.9, 2.0, 0.7, 4.0, 4.5, 10.0],
                [1.6, 2.4, 2.7, 0.6, 0.4, 1.7, 1.8, 3.5],
                [0.9, 1.0, 1.3, 0.1, 0.1, 1.0, 1.5, 1.4],
                [0.6, 0.6, 0.8, 0.1, 0.1, 0.6, 1.0, 0.5],
                [0.3, 0.3, 0.4, 0.0, 0.0, 0.3, 0.5, 0.4],
            ],
            [
                // Equinox
                [1.0, 1.2, 2.7, 3.0, 0.6, 2.0, 2.3, 1.6],
                [1.8, 2.9, 4.1, 5.7, 1.5, 3.2, 5.6, 3.6],
                [3.7, 5.6, 7.7, 8.1, 3.5, 5.0, 9.5, 7.3],
                [3.9, 5.2, 7.6, 9.0, 5.0, 7.5, 10.0, 7.9],
                [1.4, 2.0, 3.2, 3.8, 1.8, 4.0, 5.4, 3.4],
                [0.9, 0.9, 1.8, 2.0, 1.3, 3.1, 2.7, 2.0],
                [0.6, 0.6, 1.2, 1.3, 0.8, 2.0, 1.8, 1.3],
                [0.3, 0.3, 0.6, 0.6, 0.4, 1.0, 0.9, 0.6],
            ],
            [
                // Summer
                [1.9, 3.8, 2.2, 1.1, 2.1, 1.2, 2.3, 2.4],
                [1.9, 4.6, 2.9, 1.3, 2.2, 1.3, 2.8, 2.7],
                [4.4, 6.3, 5.9, 1.9, 3.3, 1.7, 4.4, 4.5],
                [5.5, 8.5, 7.6, 2.6, 4.2, 3.2, 5.5, 5.7],
                [2.8, 3.8, 3.7, 1.4, 2.7, 1.6, 4.5, 3.2],
                [2.2, 2.4, 2.2, 1.0, 2.2, 1.2, 4.4, 2.5],
                [1.4, 1.6, 1.4, 0.6, 1.4, 0.8, 2.9, 1.6],
                [0.7, 0.8, 0.7, 0.3, 0.7, 0.4, 1.4, 0.8],
            ],
        ],
    ];

    // (reference line 1027) Geomagnetic coordinates of the control point.
    let gn = geometry::geomagnetic_coords(cp.loc());

    // (reference line 1030) Season index for the Lh array.
    let season = what_season_for_lh(cp.loc(), month0);

    // (reference line 1035) Transmit-range index.
    let txrange = if dh <= 2500.0 { 0 } else { 1 };

    // (reference line 1043) Mid-path local time index (3-hour bins).
    let mut mplt: usize = 0;
    if (1..4).contains(&hour) {
        mplt = 0;
    } else if (4..7).contains(&hour) {
        mplt = 1;
    } else if (7..10).contains(&hour) {
        mplt = 2;
    } else if (10..13).contains(&hour) {
        mplt = 3;
    } else if (13..16).contains(&hour) {
        mplt = 4;
    } else if (16..19).contains(&hour) {
        mplt = 5;
    } else if (19..22).contains(&hour) {
        mplt = 6;
    } else if hour >= 22 || hour < 1 {
        mplt = 7;
    }

    // (reference line 1068) Geomagnetic latitude index (|Gn.lat|, 5° bins).
    let gn_lat = gn.lat.abs();
    let gmlat: usize;
    if 77.5 * D2R <= gn_lat {
        gmlat = 0;
    } else if (72.5 * D2R <= gn_lat) && (gn_lat < 77.5 * D2R) {
        gmlat = 1;
    } else if (67.5 * D2R <= gn_lat) && (gn_lat < 72.5 * D2R) {
        gmlat = 2;
    } else if (62.5 * D2R <= gn_lat) && (gn_lat < 67.5 * D2R) {
        gmlat = 3;
    } else if (57.5 * D2R <= gn_lat) && (gn_lat < 62.5 * D2R) {
        gmlat = 4;
    } else if (52.5 * D2R <= gn_lat) && (gn_lat < 57.5 * D2R) {
        gmlat = 5;
    } else if (47.5 * D2R <= gn_lat) && (gn_lat < 52.5 * D2R) {
        gmlat = 6;
    } else if (42.5 * D2R <= gn_lat) && (gn_lat < 47.5 * D2R) {
        gmlat = 7;
    } else {
        return 0.0; // < 42.5°: nothing more to do, return 0.0 as Lh
    }

    // (reference line 1108)
    LH[txrange][season][gmlat][mplt]
}

// (reference line 1112) WhatSeasonforLh()
/// The Lh-table season index from month and hemisphere (differs from the
/// general `WhatSeason()`: here NOV is EQUINOX, not WINTER).
fn what_season_for_lh(loc: Location, month0: usize) -> usize {
    if loc.lat >= 0.0 {
        // Northern hemisphere and the equator
        match month0 {
            DEC | JAN | FEB => WINTER,
            MAR | APR | MAY | SEP | OCT | NOV => EQUINOX,
            JUN | JUL | AUG => SUMMER,
            _ => WINTER,
        }
    } else {
        // Southern hemisphere
        match month0 {
            JUN | JUL | AUG => WINTER,
            MAR | APR | MAY | SEP | OCT | NOV => EQUINOX,
            DEC | JAN | FEB => SUMMER,
            _ => WINTER,
        }
    }
}

// (reference line 1159) SmallestCPfoF2()
/// Index of the control point with the smallest foF2, over exactly five control
/// points (the reference `path.CP[0..4]`). The reference sorts the five indices
/// DESCENDING by foF2 via an all-pairs compare-and-swap, then returns the last
/// index that is non-zero — which is the smallest-foF2 index unless control
/// point 0 itself holds the smallest foF2, in which case the "skip zero" step
/// returns the SECOND-smallest index instead (a reference quirk, reproduced
/// verbatim). Ties follow the exact swap order below.
///
/// Not currently called by the field-strength stage (which uses its own
/// equivalent keyed on the optional control points); ported for completeness.
#[allow(dead_code)]
pub fn smallest_cp_fof2(cps: &[&ControlPt]) -> usize {
    let mut temp = 0usize;
    let mut idx = [0usize, 1, 2, 3, 4]; // This is what will change.

    // Sort by brute force.
    for i in 0..5 {
        for j in 0..5 {
            if cps[idx[i]].fof2 > cps[idx[j]].fof2 {
                idx.swap(i, j);
            }
        }
    }

    // Return the last non-zero index.
    for i in 0..5 {
        if idx[i] != 0 {
            temp = idx[i];
        }
    }

    temp
}

// (reference line 1337) PenetrationPoints()
/// Absorption averaged over the ray penetration points along the path (the
/// PEN = TRUE method): as in the long model, twice as many penetration points
/// as hops. Builds temp control points at the computed fractional distances via
/// `great_circle_point`, fills them with `calculate_cp_parameters`, and sums
/// their `absorption_term`. `noh` is the number of hops; `hr` the reflection
/// height; `fv` the vertical-incidence frequency. Month/hour-slot/SSN come from
/// the path (the reference reads them from `path->month`/`hour`/`SSN`).
pub fn penetration_points(path: &MufPath, noh: f64, hr: f64, fv: f64) -> f64 {
    // (reference line 1357) Hop distance.
    let dh = path.distance / (noh + 1.0);

    // (reference line 1360) Elevation angle.
    let delta = muf::elevation_angle(dh, hr);

    // (reference line 1363) Angle of incidence at the 90-km penetration points.
    let aoi90 = muf::incidence_angle(delta, 90.0);

    // (reference line 1367) 90-km half-hop distance.
    let phi = PI / 2.0 - delta - aoi90;
    let dh90 = R0 * phi;

    // (reference line 1375) Absorption-term sum over all penetration points.
    let mut at_sum = 0.0;

    // (reference line 1377) 90-km penetration points, `for(i=0; i <= noh; i++)`.
    let mut i = 0i32;
    while (i as f64) <= noh {
        let fi = i as f64;

        // (reference line 1387) End nearest the transmitter for this hop.
        let mut pp_tx = ControlPt::default(); // ZeroCP
        let fracd = (fi * dh + dh90) / path.distance;
        let (loc, dist) = geometry::great_circle_point(path.tx, path.rx, path.distance, fracd);
        pp_tx.lat = loc.lat;
        pp_tx.lng = loc.lng;
        pp_tx.distance = dist;
        cp::calculate_cp_parameters(&mut pp_tx, path.month0, path.hour_slot, path.ssn);
        pp_tx.hr = 90.0;
        at_sum += absorption_term(&pp_tx, path.month0, fv);

        // (reference line 1398) End nearest the receiver for this hop.
        let mut pp_rx = ControlPt::default(); // ZeroCP
        let fracd = ((fi + 1.0) * dh - dh90) / path.distance;
        let (loc, dist) = geometry::great_circle_point(path.tx, path.rx, path.distance, fracd);
        pp_rx.lat = loc.lat;
        pp_rx.lng = loc.lng;
        pp_rx.distance = dist;
        cp::calculate_cp_parameters(&mut pp_rx, path.month0, path.hour_slot, path.ssn);
        pp_rx.hr = 90.0;
        at_sum += absorption_term(&pp_rx, path.month0, fv);

        i += 1;
    }

    // (reference line 1413) Average of the absorption over all penetration points.
    at_sum / (2.0 * (noh + 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diurnal_absorption_exponent_matches_documented_vector() {
        // Reference comment (DiurnalAbsorptionExponent, ~line 620):
        //   "For latitude = 3.4464, magnetic dip = 2.89 and month = 4 the
        //    diurnal absorption exponent will be p = 0.689"
        // The comment's "month = 4" is 1-BASED (April) ⇒ 0-based month0 = 3.
        // The stored dip (2.89) drives the modified dip above 70° (atan2 →
        // ~70.9°), so moddip clamps to 70°, the normalized value is exactly +1,
        // and p collapses to the sum of the April high-latitude coefficient row
        // pval1[3][1] = 0.689 (month0 = 4 would give 0.690).
        let cp = ControlPt {
            lat: 3.4464 * D2R,
            dip: [2.89, 0.0],
            ..Default::default()
        };
        let p = diurnal_absorption_exponent(&cp, 3);
        assert!((p - 0.689).abs() < 1e-6, "p = {p}");
    }

    #[test]
    fn penetration_factor_scaling_and_bounds() {
        // Boundary checks (self-derived from PHIFUN): the T ≥ 10 plateau (0.34)
        // normalizes to exactly 1.0 after the /0.34 scaling; T < 0 → 0.0.
        assert!((absorption_layer_penetration_factor(10.0) - 1.0).abs() < 1e-12);
        assert!((absorption_layer_penetration_factor(50.0) - 1.0).abs() < 1e-12);
        assert!(absorption_layer_penetration_factor(-1.0).abs() < 1e-12);
    }
}
