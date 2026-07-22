//! SCRATCH — exploration only, delete before finishing.

use tempo_fast_sys::DecoderCtx;

const NMAX: usize = ft8::NMAX;

fn frame(msg: &str, f0: f32, mut seed: u32) -> Vec<i16> {
    let tones = ft8::encode(msg);
    assert_eq!(tones.len(), ft8::NN, "{msg} encodes");
    let wave = ft8::gen_wave(&tones, ft8::SAMPLE_RATE, f0);
    let mut iwave = vec![0i16; NMAX];
    for s in iwave.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *s = ((seed >> 8) % 31) as i16 - 15;
    }
    for (i, &v) in wave.iter().enumerate() {
        if 6000 + i < NMAX {
            iwave[6000 + i] = iwave[6000 + i].saturating_add((v * 1000.0) as i16);
        }
    }
    iwave
}

fn dec(iwave: &[i16], nutc: i32) -> Vec<String> {
    ft8::decode_frame_a7(iwave, 200, 2900, 3, "", "", 0, 0, nutc, true)
        .into_iter()
        .map(|d| format!("{:?}|nap={}", d.message, d.nap))
        .collect()
}

// gfortran name-mangled module variables — scratch probe only.
extern "C" {
    #[link_name = "__packjt77_MOD_calls12"]
    static CALLS12: [u8; 4096 * 13];
    #[link_name = "__packjt77_MOD_nzhash"]
    static NZHASH: i32;
    #[link_name = "__packjt77_MOD_ihash22"]
    static IHASH22: [i32; 1000];
}

fn probe(tag: &str) {
    unsafe {
        let c = std::slice::from_raw_parts(std::ptr::addr_of!(CALLS12) as *const u8, 13);
        let ih = std::slice::from_raw_parts(std::ptr::addr_of!(IHASH22) as *const i32, 3);
        eprintln!(
            "{tag}: calls12[0..13]={:?} nzhash={} ihash22[0..3]={:?}",
            c,
            *(std::ptr::addr_of!(NZHASH)),
            ih
        );
    }
}

#[test]
fn scratch() {
    probe("virgin");
    let seed_a = frame("CQ PJ4/K1ABC", 1500.0, 0x1111_1111);
    let ref_a = frame("<PJ4/K1ABC> W9XYZ RR73", 1700.0, 0x2222_2222);
    let i34 = frame("<W9XYZ> KD9TAW/P RRR", 1900.0, 0x7777_7777);
    probe("after encode");

    let mut ctx = DecoderCtx::new();
    ctx.scoped(|| {});
    probe("after fresh-ctx restore");

    let mut ctx2 = DecoderCtx::new();
    ctx2.scoped(|| {
        eprintln!("ref alone: {:?}", dec(&ref_a, 15));
        eprintln!("i3=4 alone: {:?}", dec(&i34, 15));
    });
    probe("after decode");
    let _ = seed_a;
}
