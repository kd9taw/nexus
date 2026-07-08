/*
 * Tempo: real-RX acquisition test for the full FT1 decode pipeline.
 *
 * Pipeline:
 *   ft1_encode -> ft1_gen_wave (f0=1500) -> place at a NONZERO time offset
 *   (~0.4 s) -> add AWGN (~ -8 dB) -> convert to int16 ->
 *   ft1_decode_frame(iwave, nfa=200, nfb=2900, ndepth=3, "","",0, ...)
 *
 * PASS if >= 1 decode is returned whose message == "CQ W9XYZ EN37".
 * This proves the Costas sync acquisition path finds the signal WITHOUT a
 * known dt0 (unlike ft1_decode_rt, which assumes dt0 = 0).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

#include "libft1.h"

/* Box-Muller Gaussian, unit variance. */
static float grandf(void) {
    double u1 = (rand() + 1.0) / (RAND_MAX + 2.0);
    double u2 = (rand() + 1.0) / (RAND_MAX + 2.0);
    return (float)(sqrt(-2.0 * log(u1)) * cos(2.0 * M_PI * u2));
}

int main(void) {
    const char *msg = "CQ W9XYZ EN37";
    const float f0 = 1500.0f;
    const float fs = 12000.0f;
    const float snr_db = -8.0f;            /* weak signal */
    const int   noff = 4800;               /* ~0.4 s @ 12 kHz time offset */

    int   itone[FT1_NN];
    int   nsym = 0;

    static float   wave[FT1_NMAX];
    static float   dd[FT1_NMAX];
    static int16_t iwave[FT1_NMAX];
    int   nwave = FT1_NMAX;

    const int MAXOUT = 32;
    ft1_decode_t out[32];

    /* --- Encode + generate waveform --- */
    ft1_encode(msg, (int)strlen(msg), itone, &nsym);
    nwave = FT1_NMAX;
    ft1_gen_wave(itone, nsym, FT1_NSPS_NUM, FT1_NSPS_DEN, fs, f0, wave, &nwave);
    printf("Encoded '%s' -> %d symbols, %d-sample waveform\n", msg, nsym, nwave);
    printf("Injected time offset: %d samples (%.3f s), f0=%.0f Hz, SNR=%.0f dB\n",
           noff, (float)noff / fs, f0, snr_db);

    /* --- Place at nonzero offset, add AWGN (scale like ft1_test/roundtrip) --- */
    float bw_ratio = 2500.0f / (fs / 2.0f);
    float sig = sqrtf(2.0f * bw_ratio) * powf(10.0f, 0.05f * snr_db);

    memset(dd, 0, sizeof(dd));
    for (int i = 0; i < FT1_NMAX; i++) {
        int k = i + noff;
        if (k >= 0 && k < FT1_NMAX) dd[k] = wave[i];
    }
    srand(20260531);
    for (int i = 0; i < FT1_NMAX; i++) {
        dd[i] = sig * dd[i] + grandf();
    }

    /* --- Convert to int16 @ 12 kHz. The decoder reads int16 audio; scale by
     *     a typical WSJT-X-ish gain so samples occupy a healthy int16 range
     *     without clipping. The decoder normalizes internally. --- */
    for (int i = 0; i < FT1_NMAX; i++) {
        float v = dd[i] * 100.0f;
        if (v >  32767.0f) v =  32767.0f;
        if (v < -32768.0f) v = -32768.0f;
        iwave[i] = (int16_t)lrintf(v);
    }

    /* --- Full acquisition decode. frame_time_ms keys cross-frame IR-HARQ;
     *     this single-frame smoke test passes 0 (no retransmission to combine). --- */
    int ndec = ft1_decode_frame(iwave, 200, 2900, 3, "", "", 0, 0, out, MAXOUT);
    printf("ft1_decode_frame returned %d decode(s)\n", ndec);

    if (ndec < 0) {
        printf("RESULT: FAIL (decoder error, ndec=%d)\n", ndec);
        return 1;
    }

    int found = 0;
    for (int i = 0; i < ndec && i < MAXOUT; i++) {
        /* trim trailing spaces just in case */
        char m[40];
        strncpy(m, out[i].message, sizeof(m) - 1);
        m[sizeof(m) - 1] = '\0';
        for (int j = (int)strlen(m) - 1; j >= 0 && m[j] == ' '; j--) m[j] = '\0';

        printf("  decode[%d]: sync=%.2f snr=%d dt=%.3f s freq=%.1f Hz "
               "nap=%d qual=%.2f rv=%d msg='%s'\n",
               i, out[i].sync, out[i].snr, out[i].dt, out[i].freq,
               out[i].nap, out[i].qual, out[i].rv, m);

        if (strcmp(m, msg) == 0) found = 1;
    }

    if (found) {
        printf("RESULT: PASS (acquisition recovered '%s' without known dt0)\n", msg);
        return 0;
    }
    printf("RESULT: FAIL (message '%s' not among %d decode(s))\n", msg, ndec);
    return 1;
}
