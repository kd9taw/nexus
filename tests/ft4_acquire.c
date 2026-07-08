/*
 * Nexus: native FT4 full-frame acquisition + decode-parity test.
 *
 * Pipeline (self-contained):
 *   for each of 3 known FT4 messages:
 *     ft4_encode -> ft4_gen_wave at a distinct f0 (full NMAX frame) -> scale
 *   sum the signals, add AWGN, convert to int16 ->
 *   ft4_decode_frame(iwave, nfa=200, nfb=2900, ndepth=3, "","",0, nfqso=0, ...)
 *
 * PASS if all three transmitted messages are recovered.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

#include "libft1.h"

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
    const float snrs[3] = { -8.0f, -10.0f, -12.0f };
    const float fs = 12000.0f;

    static float   dd[FT4_NMAX];
    static float   wave[FT4_NMAX];
    static int16_t iwave[FT4_NMAX];

    const int MAXOUT = 64;
    ft4_decode_t out[64];

    const float bw_ratio = 2500.0f / (fs / 2.0f);

    memset(dd, 0, sizeof(dd));

    for (int s = 0; s < 3; s++) {
        int itone[FT4_NN];
        int nsym = 0;
        ft4_encode(msgs[s], (int)strlen(msgs[s]), itone, &nsym);
        if (nsym <= 0) {
            printf("RESULT: FAIL (ft4_encode rejected '%s')\n", msgs[s]);
            return 1;
        }
        int nwave = FT4_NMAX;
        ft4_gen_wave(itone, nsym, fs, f0s[s], wave, &nwave);
        if (nwave <= 0) {
            printf("RESULT: FAIL (ft4_gen_wave failed for '%s')\n", msgs[s]);
            return 1;
        }
        float sig = sqrtf(2.0f * bw_ratio) * powf(10.0f, 0.05f * snrs[s]);
        for (int i = 0; i < nwave && i < FT4_NMAX; i++) dd[i] += sig * wave[i];
        printf("TX[%d] '%s' f0=%.0f Hz snr=%.0f dB (%d sym, %d samples)\n",
               s, msgs[s], f0s[s], snrs[s], nsym, nwave);
    }

    srand(20260605);
    for (int i = 0; i < FT4_NMAX; i++) {
        float v = (dd[i] + grandf()) * 100.0f;
        if (v >  32767.0f) v =  32767.0f;
        if (v < -32768.0f) v = -32768.0f;
        iwave[i] = (int16_t)lrintf(v);
    }

    int ndec = ft4_decode_frame(iwave, 200, 2900, 3, "", "", 0, 0, out, MAXOUT);
    printf("ft4_decode_frame returned %d decode(s)\n", ndec);
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
