/*
 * Tempo: C ABI for the standalone FT1 4-CPM turbo modem (libft1).
 *
 * Decoupled from Qt / WSJT-X. Links FFTW3 single precision. No GUI.
 *
 * Frame / array constants (from ft1/ft1_params.f90):
 *   FT1_NN    = 99     total channel symbols
 *   FT1_NMAX  = 48000  raw audio samples (4.0 s @ 12 kHz)
 *   FT1_NDOWN = 54     downsample factor
 *   FT1_NDMAX = 888    downsampled complex samples
 */
#ifndef LIBFT1_H
#define LIBFT1_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FT1_NN        99      /* total channel symbols                 */
#define FT1_NMAX      48000   /* raw audio samples @ 12 kHz (4.0 s)    */
#define FT1_NDOWN     54      /* downsample factor                     */
#define FT1_NDMAX     888     /* downsampled complex samples           */
#define FT1_NSPS_NUM  3000    /* samples-per-symbol numerator          */
#define FT1_NSPS_DEN  7       /* samples-per-symbol denominator        */
#define FT1_MSG91     91      /* decoded message bits (77 msg + 14 CRC)*/

/*
 * Encode an FT1 message into 99 quaternary channel symbols {0,1,2,3}.
 *   msg       : NUL- or space-terminated message string (<= 37 chars)
 *   msg_len   : number of valid chars in msg
 *   itone_out : caller buffer of FT1_NN (99) ints
 *   nsym_out  : number of symbols written (99 on success)
 */
void ft1_encode(const char *msg, int msg_len,
                int *itone_out /*[99]*/, int *nsym_out);

/*
 * Encode an FT1 message for a specific IR-HARQ redundancy version.
 *   irv = 0 : byte-identical to ft1_encode (initial transmission).
 *   irv = 1 : first retransmission  (87 new LDPC(348,91) parity + 87 systematic,
 *             RV1 Costas sync). Combine with RV0 at the receiver for coding gain.
 *   irv = 2 : second retransmission (RV2 parity + systematic, RV2 Costas sync).
 * Out-of-range irv clamps to 0. Other args as ft1_encode.
 */
void ft1_encode_rv(const char *msg, int msg_len, int irv,
                   int *itone_out /*[99]*/, int *nsym_out);

/*
 * Generate the real-valued 4-CPM audio waveform from channel symbols.
 *   itone     : nsym channel symbols
 *   nsym      : number of symbols (99)
 *   nsps_num  : samples-per-symbol numerator   (FT1_NSPS_NUM = 3000)
 *   nsps_den  : samples-per-symbol denominator (FT1_NSPS_DEN = 7)
 *   fsample   : output sample rate (Hz), e.g. 12000.0f
 *   f0        : audio carrier frequency (Hz), e.g. 1500.0f
 *   wave_out  : caller buffer (length *nwave_out on input)
 *   nwave_out : in = buffer capacity; out = samples produced
 */
void ft1_gen_wave(const int *itone, int nsym, int nsps_num, int nsps_den,
                  float fsample, float f0,
                  float *wave_out, int *nwave_out);

/*
 * Decode a received FT1 frame (real-time / single-candidate path).
 * Mirrors ft1_test: ft1_downsample -> normalize -> turbo_decode_ft1.
 *   wave           : FT1_NMAX (48000) raw audio samples @ 12 kHz
 *   f0             : candidate carrier frequency (Hz)
 *   snr_est        : SNR estimate (dB in 2500 Hz BW)
 *   message91_out  : caller buffer of FT1_MSG91 (91) int8 bits (0/1)
 *   ntype_out      : 1=turbo, 2=OSD, -1=failed
 *   nharderror_out : hard error count, -1 if failed
 */
void ft1_decode_rt(const float *wave /*[FT1_NMAX]*/, float f0, float snr_est,
                   int8_t *message91_out /*[91]*/,
                   int *ntype_out, int *nharderror_out);

/*
 * Unpack the 77 message bits (message91_out[0..76]) back to readable text.
 *   bits77   : 77 int8 bits (0/1)
 *   msg_out  : caller buffer (>= 38 bytes recommended)
 *   msg_cap  : capacity of msg_out in bytes (incl. NUL)
 *   success  : 1 if unpack succeeded, 0 otherwise
 */
void ft1_unpack(const int8_t *bits77 /*[77]*/,
                char *msg_out, int msg_cap, int *success);

/*
 * One decode result from the full RX acquisition pipeline.
 *
 * Layout (matches the Fortran bind(C) type ft1_decode_t; LP64, no padding
 * needed since all fields are naturally 4-byte aligned and message[38] is
 * followed by two 4-byte ints):
 *   offset  size  field
 *      0      4    float sync
 *      4      4    int   snr
 *      8      4    float dt
 *     12      4    float freq
 *     16     38    char  message[38]   (NUL-terminated)
 *     54      2    (padding to 4-byte boundary)
 *     56      4    int   nap
 *     60      4    float qual
 *     64      4    int   rv
 *   total: 68 bytes, 4-byte aligned.
 */
typedef struct {
    float sync;          /* sync metric                                  */
    int   snr;           /* SNR estimate, dB (rounded)                   */
    float dt;            /* time offset, seconds                         */
    float freq;          /* audio frequency, Hz                          */
    char  message[38];   /* NUL-terminated decoded message text          */
    int   nap;           /* AP type used (0 = none)                      */
    float qual;          /* decode quality metric                        */
    int   rv;            /* redundancy version 0/1/2 (rv>0 = recovered by */
                         /* joint-turbo combining that many RVs), or -1   */
} ft1_decode_t;

/*
 * Run the FULL FT1 receive acquisition pipeline on a 4-second frame:
 * Costas sync candidate search (time + frequency) -> downconvert ->
 * fine sync -> turbo decode -> OSD/AP fallback -> signal subtraction (SIC)
 * -> IR-HARQ combining. Finds signals WITHOUT a known time offset.
 *
 *   iwave         : FT1_NMAX (48000) int16 audio samples @ 12 kHz
 *   nfa, nfb      : frequency search band edges (Hz), e.g. 200 .. 2900
 *   ndepth        : decode depth (3 = full turbo+OSD+SIC; <=0 defaults to 3)
 *   mycall        : NUL/space-terminated callsign for AP (may be "")
 *   hiscall       : NUL/space-terminated callsign for AP (may be "")
 *   nqso_progress : QSO progress index (selects the AP pass schedule)
 *   frame_time_ms : monotonic millisecond timestamp for THIS frame (need not be
 *                   wall-clock; only monotonic + consistent across frames). Keys
 *                   cross-frame IR-HARQ: a failed RV0 frame is buffered and a
 *                   later RV1/RV2 at the same freq (+-10 Hz, within 30 s) is
 *                   joint-turbo-combined. Call ft1_harq_reset() on band/QSO
 *                   change. Only the low 32 bits / differences <= 30 s matter.
 *   out           : caller array of ft1_decode_t (capacity max_out)
 *   max_out       : capacity of out
 *
 * Returns the number of decodes found (>= 0), or -1 on error. Up to
 * min(found, max_out) entries are written to out.
 *
 * NOTE: not thread-safe / not reentrant (the FT1 pipeline keeps process-
 * global SAVE state and this call uses a module-level results buffer).
 */
int ft1_decode_frame(const int16_t *iwave /*[FT1_NMAX]*/,
                     int nfa, int nfb, int ndepth,
                     const char *mycall, const char *hiscall,
                     int nqso_progress, int frame_time_ms,
                     ft1_decode_t *out, int max_out);

/*
 * Clear all IR-HARQ soft-combining buffers. Call on band change, QSO change,
 * or an intentional QSY so a new exchange does not joint-combine with stale RV
 * frames from a previous one. (Buffers otherwise persist across decode calls
 * and self-expire 30 s after their last update.)
 */
void ft1_harq_reset(void);

/*===========================================================================
 * FT8: native decode/encode of the standard WSJT-X FT8 mode (15 s T/R),
 * built on the vendored WSJT-X GPL sources (lib/ft8). Full-frame decode via
 * the core primitives (ft8apset -> sync8 -> ft8b); no nzhsym/a7/shmem.
 *===========================================================================*/

#define FT8_NN     79       /* total channel symbols                        */
#define FT8_NSPS   1920     /* samples per symbol @ 12 kHz                   */
#define FT8_NMAX   180000   /* raw audio samples (15.0 s @ 12 kHz)          */
#define FT8_NZ     151680   /* samples in the full 12.64 s waveform (NSPS*NN)*/

/*
 * Encode an FT8 message into 79 channel tones {0..7}.
 *   msg/msg_len : message text (<= 37 chars) and its valid length
 *   itone_out   : caller buffer of FT8_NN (79) ints
 *   nsym_out    : symbols written (79), or -1 on bad message
 */
void ft8_encode(const char *msg, int msg_len,
                int *itone_out /*[79]*/, int *nsym_out);

/*
 * Generate the real FT8 audio waveform (Gaussian BT=2.0) from channel tones.
 *   itone     : nsym tones
 *   nsym      : number of tones (79)
 *   fsample   : sample rate (Hz), e.g. 12000.0f
 *   f0        : audio carrier (Hz), e.g. 1500.0f
 *   wave_out  : caller buffer (capacity *nwave_out on input)
 *   nwave_out : in = capacity; out = samples produced (nsym*FT8_NSPS), or -1
 */
void ft8_gen_wave(const int *itone, int nsym, float fsample, float f0,
                  float *wave_out, int *nwave_out);

/*
 * One decode result from the FT8 full-frame acquisition.
 *
 * Layout (matches the Fortran bind(C) type ft8_decode_t):
 *   offset  size  field
 *      0      4    float sync
 *      4      4    int   snr
 *      8      4    float dt    (xdt - 0.5, seconds)
 *     12      4    float freq
 *     16     38    char  message[38]  (NUL-terminated)
 *     54      2    (padding to 4-byte boundary)
 *     56      4    int   nap   (iaptype; 0 = none)
 *     60      4    float qual
 *   total: 64 bytes, 4-byte aligned.
 */
typedef struct {
    float sync;          /* sync metric                                  */
    int   snr;           /* SNR estimate, dB (rounded)                   */
    float dt;            /* time offset, seconds (xdt - 0.5)             */
    float freq;          /* audio frequency, Hz                          */
    char  message[38];   /* NUL-terminated decoded message text          */
    int   nap;           /* AP type used (iaptype; 0 = none)             */
    float qual;          /* decode quality metric [0,1]                  */
} ft8_decode_t;

/*
 * Decode EVERY FT8 signal in a complete 15 s frame.
 *   iwave         : FT8_NMAX (180000) int16 audio samples @ 12 kHz
 *   nfa, nfb      : frequency search band edges (Hz), e.g. 200 .. 2900
 *   ndepth        : 1..3 (3 = full bp+osd, 3 passes; <=0 defaults to 3)
 *   mycall/hiscall: NUL/space-terminated callsigns for AP (may be "")
 *   nqso_progress : QSO progress index (AP pass schedule)
 *   nfqso         : QSO/RX audio freq (Hz) being worked (WSJT-X nfqso); the deep
 *                   AP passes + sync center on it. 0 / out of [nfa,nfb] = band mid
 *   out           : caller array of ft8_decode_t (capacity max_out)
 *   max_out       : capacity of out
 *
 * Returns the number of decodes found (>= 0), or -1 on error. Up to
 * min(found, max_out) entries are written. NOT thread-safe / not reentrant.
 */
int ft8_decode_frame(const int16_t *iwave /*[FT8_NMAX]*/,
                     int nfa, int nfb, int ndepth,
                     const char *mycall, const char *hiscall,
                     int nqso_progress, int nfqso,
                     ft8_decode_t *out, int max_out);

/*===========================================================================
 * FT4: native decode/encode of the standard WSJT-X FT4 mode (7.5 s T/R,
 * 4-GFSK), built on the vendored WSJT-X GPL sources (lib/ft4 + ft4_decode.f90).
 * Driven via the OO ft4_decoder + a collector callback (no nzhsym/a7/shmem).
 *===========================================================================*/

#define FT4_NN     103      /* sync + data channel symbols (16 + 87)        */
#define FT4_NSPS   576      /* samples per symbol @ 12 kHz                  */
#define FT4_NMAX   72576    /* samples in iwave (21*3456, ~6.05 s window)   */

/*
 * Encode an FT4 message into 103 channel tones {0..3}.
 *   msg/msg_len : message text (<= 37 chars) and its valid length
 *   itone_out   : caller buffer of FT4_NN (103) ints
 *   nsym_out    : symbols written (103), or -1 on bad message
 */
void ft4_encode(const char *msg, int msg_len,
                int *itone_out /*[103]*/, int *nsym_out);

/*
 * Generate the full-length real FT4 audio frame (FT4_NMAX samples) from tones,
 * exactly as ft4sim does (gen_ft4wave positions the shaped/ramped signal).
 *   itone     : nsym tones (103)
 *   nsym      : number of tones (103)
 *   fsample   : sample rate (Hz), e.g. 12000.0f
 *   f0        : audio carrier (Hz)
 *   wave_out  : caller buffer (capacity *nwave_out on input, >= FT4_NMAX)
 *   nwave_out : in = capacity; out = samples produced (FT4_NMAX), or -1
 */
void ft4_gen_wave(const int *itone, int nsym, float fsample, float f0,
                  float *wave_out, int *nwave_out);

/* One decode result from FT4 full-frame acquisition (same layout as
 * ft8_decode_t: 64 bytes). */
typedef struct {
    float sync;          /* sync metric                                  */
    int   snr;           /* SNR estimate, dB (rounded)                   */
    float dt;            /* time offset, seconds                         */
    float freq;          /* audio frequency, Hz                          */
    char  message[38];   /* NUL-terminated decoded message text          */
    int   nap;           /* AP type used (iaptype; 0 = none)             */
    float qual;          /* decode quality metric [0,1]                  */
} ft4_decode_t;

/*
 * Decode EVERY FT4 signal in a complete frame.
 *   iwave         : FT4_NMAX (72576) int16 audio samples @ 12 kHz
 *   nfa, nfb      : frequency search band edges (Hz)
 *   ndepth        : 1..3 (3 = full bp+osd; <=0 defaults to 3)
 *   mycall/hiscall: NUL/space-terminated callsigns for AP (may be "")
 *   nqso_progress : QSO progress index (AP pass schedule)
 *   nfqso         : QSO/RX audio freq (Hz) being worked (WSJT-X nfqso); the deep
 *                   AP passes center on it. 0 / out of [nfa,nfb] = band mid
 *   out           : caller array of ft4_decode_t (capacity max_out)
 *   max_out       : capacity of out
 *
 * Returns the number of decodes found (>= 0), or -1 on error. Up to
 * min(found, max_out) entries are written. NOT thread-safe / not reentrant.
 */
int ft4_decode_frame(const int16_t *iwave /*[FT4_NMAX]*/,
                     int nfa, int nfb, int ndepth,
                     const char *mycall, const char *hiscall,
                     int nqso_progress, int nfqso,
                     ft4_decode_t *out, int max_out);

/*===========================================================================
 * DX1-S: non-coherent M-FSK + soft-LDPC robust tier (fading-resilient).
 *===========================================================================*/

/* DX1 transmit-waveform length (samples @ 12 kHz): chirp sync + 58 symbols. */
int dx1_frame_len(void);

/* DX1 receive capture-window length (samples): a full 15 s T/R slot. */
int dx1_capture_len(void);

/*
 * Encode text -> DX1 audio (chirp sync preamble + 8-FSK data).
 *   msg/msg_len : message text (<= 37 chars) and its length
 *   f0          : audio carrier (lower comb edge), Hz, e.g. 1500.0
 *   fsample     : sample rate, Hz, e.g. 12000.0
 *   wave_out    : caller buffer, capacity max_out >= dx1_frame_len()
 *   returns     : samples written (> 0), or -1 on pack failure / small buffer
 */
int dx1_encode_wave(const char *msg, int msg_len, float f0, float fsample,
                    float *wave_out, int max_out);

/*
 * Decode ONE known carrier (single-offset). The sync chirp is searched in time
 * over [idt_lo, idt_hi] and in frequency over only +-6.25 Hz of f0.
 *   returns : nharderr (< 0 => decode/CRC failed); msg_out NUL-filled on fail.
 */
int dx1_decode_buf(const float *wave, int nwave, float f0, float fsample,
                   int idt_lo, int idt_hi, char *msg_out, int msg_cap,
                   float *snr_out, float *sync_out);

/*
 * One decode from the DX1 full-passband scan.
 *
 * Layout (matches the Fortran bind(C) type dx1_decode_t; all fields naturally
 * 4-byte aligned, a 2-byte tail pad after message[38]):
 *   offset  size  field
 *      0      4    float freq      (resolved carrier, Hz)
 *      4      4    float sync      (chirp sync metric)
 *      8      4    int   snr       (SNR estimate, dB, rounded)
 *     12     38    char  message[38] (NUL-terminated)
 *     50      2    (padding to 4-byte boundary)
 *   total: 52 bytes, 4-byte aligned.
 */
typedef struct {
    float freq;          /* resolved carrier, Hz                         */
    float sync;          /* chirp sync metric                            */
    int   snr;           /* SNR estimate, dB (rounded)                   */
    char  message[38];   /* NUL-terminated decoded message text          */
} dx1_decode_t;

/*
 * Decode EVERY DX1 signal in the audio passband in one slot (full-band
 * acquisition, like ft1_decode_frame for FT1), vs dx1_decode_buf's single
 * carrier. Three stages: coarse chirp-correlation carrier scan on a 12.5 Hz
 * grid -> median-threshold peak-pick -> full decode per survivor (the CRC-14
 * inside the LDPC decoder rejects false peaks).
 *
 *   wave/nwave : audio samples @ fsample (one capture window)
 *   f_lo, f_hi : carrier (lower-comb-edge) scan range, Hz, e.g. 200 .. 2900
 *   fsample    : sample rate, Hz
 *   out        : caller array of dx1_decode_t (capacity max_out)
 *   max_out    : capacity of out (also caps decodes/slot)
 *
 * Returns the number of decodes found (>= 0); up to min(found, max_out) are
 * written to out.  NOT thread-safe / not reentrant.
 */
int dx1_decode_band(const float *wave, int nwave, float f_lo, float f_hi,
                    float fsample, dx1_decode_t *out, int max_out);

#ifdef __cplusplus
}
#endif

#endif /* LIBFT1_H */
