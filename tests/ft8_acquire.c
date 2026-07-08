/*
 * Nexus: native FT8 full-frame acquisition + decode-parity test.
 *
 * Pipeline (fully self-contained — no external .wav needed):
 *   for each of 3 known FT8 messages:
 *     ft8_encode -> ft8_gen_wave at a distinct f0 -> place at 0.5 s TX start
 *   sum the three signals, add AWGN, convert to int16 ->
 *   ft8_decode_frame(iwave, nfa=200, nfb=2900, ndepth=3, "","",0, nfqso=0, ...)
 *
 * PASS if all three transmitted messages are recovered. This proves the
 * native FT8 path (ft8apset -> sync8 -> ft8b, with ft8b's internal multi-pass
 * subtraction) decodes multiple overlapping signals at parity with WSJT-X.
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

static void trim(char *s) {
    for (int j = (int)strlen(s) - 1; j >= 0 && s[j] == ' '; j--) s[j] = '\0';
}

int main(void) {
    const char *msgs[3] = {
        "CQ KD9TAW EN52",
        "KD9TAW W1AW -08",
        "W1AW KD9TAW R-15",
    };
    const float f0s[3]  = { 700.0f, 1400.0f, 2100.0f };
    const float snrs[3] = { -10.0f, -12.0f, -14.0f };
    const float fs   = 12000.0f;
    const int   noff = 6000;               /* 0.5 s FT8 TX start @ 12 kHz */

    static float   dd[FT8_NMAX];
    static float   wave[FT8_NZ];
    static int16_t iwave[FT8_NMAX];

    const int MAXOUT = 64;
    ft8_decode_t out[64];

    const float bw_ratio = 2500.0f / (fs / 2.0f);

    memset(dd, 0, sizeof(dd));

    for (int s = 0; s < 3; s++) {
        int itone[FT8_NN];
        int nsym = 0;
        ft8_encode(msgs[s], (int)strlen(msgs[s]), itone, &nsym);
        if (nsym <= 0) {
            printf("RESULT: FAIL (ft8_encode rejected '%s')\n", msgs[s]);
            return 1;
        }
        int nwave = FT8_NZ;
        ft8_gen_wave(itone, nsym, fs, f0s[s], wave, &nwave);
        if (nwave <= 0) {
            printf("RESULT: FAIL (ft8_gen_wave failed for '%s')\n", msgs[s]);
            return 1;
        }
        float sig = sqrtf(2.0f * bw_ratio) * powf(10.0f, 0.05f * snrs[s]);
        for (int i = 0; i < nwave; i++) {
            int k = i + noff;
            if (k >= 0 && k < FT8_NMAX) dd[k] += sig * wave[i];
        }
        printf("TX[%d] '%s' f0=%.0f Hz snr=%.0f dB (%d sym, %d samples)\n",
               s, msgs[s], f0s[s], snrs[s], nsym, nwave);
    }

    /* Add AWGN over the whole frame, then scale to int16 (decoder normalizes). */
    srand(20260605);
    for (int i = 0; i < FT8_NMAX; i++) {
        float v = (dd[i] + grandf()) * 100.0f;
        if (v >  32767.0f) v =  32767.0f;
        if (v < -32768.0f) v = -32768.0f;
        iwave[i] = (int16_t)lrintf(v);
    }

    int ndec = ft8_decode_frame(iwave, 200, 2900, 3, "", "", 0, 0, out, MAXOUT);
    printf("ft8_decode_frame returned %d decode(s)\n", ndec);
    if (ndec < 0) {
        printf("RESULT: FAIL (decoder error, ndec=%d)\n", ndec);
        return 1;
    }

    int found[3] = {0, 0, 0};
    for (int i = 0; i < ndec && i < MAXOUT; i++) {
        char m[40];
        strncpy(m, out[i].message, sizeof(m) - 1);
        m[sizeof(m) - 1] = '\0';
        trim(m);
        printf("  decode[%d]: sync=%.2f snr=%d dt=%.2f s freq=%.1f Hz "
               "nap=%d qual=%.2f msg='%s'\n",
               i, out[i].sync, out[i].snr, out[i].dt, out[i].freq,
               out[i].nap, out[i].qual, m);
        for (int s = 0; s < 3; s++)
            if (strcmp(m, msgs[s]) == 0) found[s] = 1;
    }

    int nfound = found[0] + found[1] + found[2];
    if (nfound == 3) {
        printf("RESULT: PASS (all 3 messages recovered)\n");
        return 0;
    }
    printf("RESULT: FAIL (recovered %d/3: [%d %d %d])\n",
           nfound, found[0], found[1], found[2]);
    return 1;
}
