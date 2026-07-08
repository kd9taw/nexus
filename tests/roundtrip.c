/*
 * Tempo: headless FT1 encode -> decode round-trip via the libft1 C ABI.
 *
 * Pipeline:
 *   ft1_encode -> ft1_gen_wave -> [add AWGN @ high SNR] -> ft1_decode_rt
 *   -> ft1_unpack -> compare against "CQ W9XYZ EN37".
 *
 * PASS if the recovered message equals the transmitted message.
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
    const float snr_db = 10.0f;     /* high SNR */

    int   itone[FT1_NN];
    int   nsym = 0;

    /* Buffers sized to the raw audio length. */
    static float wave[FT1_NMAX];
    static float dd[FT1_NMAX];
    int   nwave = FT1_NMAX;

    int8_t message91[FT1_MSG91];
    int    ntype = -1, nharderror = -1;

    /* --- Encode --- */
    ft1_encode(msg, (int)strlen(msg), itone, &nsym);
    printf("Encoded '%s' -> %d channel symbols\n", msg, nsym);
    if (nsym != FT1_NN) {
        fprintf(stderr, "FAIL: expected %d symbols, got %d\n", FT1_NN, nsym);
        return 1;
    }

    /* --- Generate waveform --- */
    nwave = FT1_NMAX;
    ft1_gen_wave(itone, nsym, FT1_NSPS_NUM, FT1_NSPS_DEN, fs, f0, wave, &nwave);
    printf("Generated waveform: %d samples\n", nwave);

    /* --- Scale the signal and add AWGN at high SNR (mirror ft1_test) --- */
    /* sig scaling for 2500 Hz BW SNR, matching ft1_test convention:
     *   sig = sqrt(2*bw_ratio) * 10^(0.05*snr), bw_ratio = 2500/(fs/2) */
    float bw_ratio = 2500.0f / (fs / 2.0f);
    float sig = sqrtf(2.0f * bw_ratio) * powf(10.0f, 0.05f * snr_db);

    /* The signal is placed at sample 0 (no time offset): the ft1_decode_rt
     * ABI assumes the fine timing offset dt0 = 0 and the turbo decoder only
     * searches dt0 +/- 3 downsampled samples. (ft1_test injects a 3000-sample
     * offset and passes the matching dt0 internally; the C ABI does not expose
     * dt0, so we keep the frame aligned to the start of the buffer.) */
    srand(12345);
    for (int i = 0; i < FT1_NMAX; i++) {
        dd[i] = sig * wave[i] + grandf();
    }

    /* --- Decode --- */
    ft1_decode_rt(dd, f0, snr_db, message91, &ntype, &nharderror);
    printf("Decode: ntype=%d nharderror=%d\n", ntype, nharderror);

    if (ntype < 0) {
        printf("RESULT: FAIL (decode failed, ntype=%d)\n", ntype);
        return 1;
    }

    /* --- Unpack the 77 message bits to text --- */
    char recovered[64];
    int  ok = 0;
    ft1_unpack(message91, recovered, (int)sizeof(recovered), &ok);
    if (!ok) {
        printf("RESULT: FAIL (unpack failed)\n");
        return 1;
    }

    /* Trim trailing whitespace from the recovered message. */
    for (int i = (int)strlen(recovered) - 1; i >= 0 && recovered[i] == ' '; i--)
        recovered[i] = '\0';

    printf("Recovered message: '%s'\n", recovered);

    if (strcmp(recovered, msg) == 0) {
        printf("RESULT: PASS (recovered == '%s')\n", msg);
        return 0;
    } else {
        printf("RESULT: FAIL (recovered '%s' != '%s')\n", recovered, msg);
        return 1;
    }
}
