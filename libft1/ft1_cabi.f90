! Tempo: C ABI wrappers for the standalone FT1 modem (libft1).
!
! Exposes a clean iso_c_binding interface so the FT1 4-CPM turbo modem can be
! driven headlessly from C (and later from Rust via FFI). No Qt, no GUI.
!
! Underlying Fortran routines wrapped here:
!   genft1            (genft1.f90)            - message -> 99 channel symbols
!   gen_ft1wave       (gen_ft1wave.f90)       - symbols -> real audio waveform
!   ft1_downsample    (ft1_downsample.f90)    - audio -> complex baseband
!   turbo_decode_ft1  (turbo_decode_ft1.f90)  - baseband -> 91 message bits
!
! Frame / array constants (from ft1/ft1_params.f90):
!   NN    = 99     total channel symbols
!   NMAX  = 48000  raw audio samples (4.0 s @ 12 kHz)
!   NDOWN = 54     downsample factor
!   NDMAX = 888    downsampled complex samples (NMAX/NDOWN)

module ft1_cabi
  use iso_c_binding
  use packjt77, only: unpack77
  use ft1_decode, only: ft1_decoder
  use ir_harq_combine_mod, only: harq_init
  use dx1_params, only: DX1_NFRAME, DX1_NMAX
  implicit none

  ! Mirror the relevant ft1_params.f90 values as module parameters so the
  ! wrappers can size local arrays without pulling in the include from C.
  integer, parameter :: FT1_NN    = 99
  integer, parameter :: FT1_NMAX  = 48000
  integer, parameter :: FT1_NDOWN = 54
  integer, parameter :: FT1_NDMAX = FT1_NMAX / FT1_NDOWN   ! 888

  ! ------------------------------------------------------------------------
  ! Interop result struct for the full RX acquisition decoder. Layout MUST
  ! match ft1_decode_t in libft1.h (see header for the exact byte layout).
  ! ------------------------------------------------------------------------
  type, bind(C) :: ft1_decode_t
     real(c_float)         :: sync         ! sync metric (smax)
     integer(c_int)        :: snr          ! SNR estimate, dB (rounded)
     real(c_float)         :: dt           ! time offset, seconds
     real(c_float)         :: freq         ! audio frequency, Hz
     character(kind=c_char):: message(38)  ! NUL-terminated decoded text
     integer(c_int)        :: nap          ! AP type used (0 = no AP)
     real(c_float)         :: qual         ! decode quality metric
     integer(c_int)        :: rv           ! redundancy version, or -1 if N/A
  end type ft1_decode_t

  ! ------------------------------------------------------------------------
  ! Interop result struct for the DX1 full-passband decoder (dx1_decode_band).
  ! Layout MUST match dx1_decode_t in libft1.h:
  !   offset 0  float freq;  4 float sync;  8 int snr;  12 char message[38];
  !   total 52 bytes, 4-byte aligned (2-byte tail pad after message[38]).
  ! (DX1 has no dt/AP/RV, so the struct is leaner than ft1_decode_t.)
  ! ------------------------------------------------------------------------
  type, bind(C) :: dx1_decode_t
     real(c_float)         :: freq         ! resolved carrier, Hz
     real(c_float)         :: sync         ! chirp sync metric
     integer(c_int)        :: snr          ! SNR estimate, dB (rounded)
     character(kind=c_char):: message(38)  ! NUL-terminated decoded text
  end type dx1_decode_t

  ! ------------------------------------------------------------------------
  ! Module-level results buffer populated by the decode callback.
  ! NOT thread-safe / not reentrant: ft1_decode_frame() must not be called
  ! concurrently (the FT1 pipeline also uses process-global SAVE state).
  ! ------------------------------------------------------------------------
  integer, parameter :: FT1_MAXDEC = 100        ! decoder caps at 100 decodes
  type :: ft1_result_rec
     real    :: sync
     integer :: snr
     real    :: dt
     real    :: freq
     character(len=37) :: message
     integer :: nap
     real    :: qual
     integer :: rv
  end type ft1_result_rec

  type(ft1_result_rec), save :: g_results(FT1_MAXDEC)
  integer,              save :: g_count = 0

contains

  !-------------------------------------------------------------------------
  ! ft1_encode
  !   msg       : NUL- or space-terminated C string (the FT1 message)
  !   msg_len   : number of valid chars in msg (<= 37)
  !   itone_out : output array of 99 quaternary channel symbols {0,1,2,3}
  !   nsym_out  : number of symbols written (always 99 on success)
  !-------------------------------------------------------------------------
  subroutine ft1_encode(msg, msg_len, itone_out, nsym_out) bind(C, name="ft1_encode")
    character(kind=c_char), intent(in)  :: msg(*)
    integer(c_int), value,  intent(in)  :: msg_len
    integer(c_int),         intent(out) :: itone_out(FT1_NN)
    integer(c_int),         intent(out) :: nsym_out

    character(len=37) :: msg37, msgsent37
    integer(c_int)    :: itone(FT1_NN)
    integer(kind=1)   :: msgbits(77)
    integer           :: i, n

    ! Marshal the C string into a blank-padded character(37).
    msg37 = ' '
    n = min(msg_len, 37)
    do i = 1, n
       if (msg(i) == c_null_char) exit
       msg37(i:i) = msg(i)
    end do

    call genft1(msg37, 0, msgsent37, msgbits, itone)

    itone_out(1:FT1_NN) = itone(1:FT1_NN)
    nsym_out = FT1_NN
  end subroutine ft1_encode

  !-------------------------------------------------------------------------
  ! ft1_encode_rv
  !   Encode a message into 99 channel symbols for a specific IR-HARQ
  !   redundancy version. irv=0 is byte-identical to ft1_encode (RV0); irv=1/2
  !   emit the punctured retransmission frames (new LDPC(348,91) parity + a
  !   repeated systematic block) with the RV-specific Costas sync arrays.
  !   msg       : NUL- or space-terminated C string (the FT1 message)
  !   msg_len   : number of valid chars in msg (<= 37)
  !   irv       : redundancy version 0, 1, or 2 (out-of-range clamps to 0)
  !   itone_out : output array of 99 quaternary channel symbols {0,1,2,3}
  !   nsym_out  : number of symbols written (always 99 on success)
  !-------------------------------------------------------------------------
  subroutine ft1_encode_rv(msg, msg_len, irv, itone_out, nsym_out) &
       bind(C, name="ft1_encode_rv")
    character(kind=c_char), intent(in)  :: msg(*)
    integer(c_int), value,  intent(in)  :: msg_len
    integer(c_int), value,  intent(in)  :: irv
    integer(c_int),         intent(out) :: itone_out(FT1_NN)
    integer(c_int),         intent(out) :: nsym_out

    character(len=37) :: msg37, msgsent37
    integer(c_int)    :: itone(FT1_NN)
    integer(kind=1)   :: msgbits(77)
    integer           :: i, n, irv_l

    ! Marshal the C string into a blank-padded character(37).
    msg37 = ' '
    n = min(msg_len, 37)
    do i = 1, n
       if (msg(i) == c_null_char) exit
       msg37(i:i) = msg(i)
    end do

    irv_l = irv
    if (irv_l < 0 .or. irv_l > 2) irv_l = 0
    call genft1_rv(msg37, 0, irv_l, msgsent37, msgbits, itone)

    itone_out(1:FT1_NN) = itone(1:FT1_NN)
    nsym_out = FT1_NN
  end subroutine ft1_encode_rv

  !-------------------------------------------------------------------------
  ! ft1_gen_wave
  !   itone     : input channel symbols (length nsym)
  !   nsym      : number of symbols (99)
  !   nsps_num  : samples-per-symbol numerator   (3000)
  !   nsps_den  : samples-per-symbol denominator (7)
  !   fsample   : output sample rate (Hz), e.g. 12000.0
  !   f0        : audio carrier frequency (Hz), e.g. 1500.0
  !   wave_out  : caller-allocated output buffer of length nwave_out (in/out)
  !   nwave_out : in = buffer capacity; out = samples actually produced
  !-------------------------------------------------------------------------
  subroutine ft1_gen_wave(itone, nsym, nsps_num, nsps_den, fsample, f0, &
                          wave_out, nwave_out) bind(C, name="ft1_gen_wave")
    integer(c_int),        intent(in)    :: itone(*)
    integer(c_int), value, intent(in)    :: nsym, nsps_num, nsps_den
    real(c_float),  value, intent(in)    :: fsample, f0
    real(c_float),         intent(inout) :: wave_out(*)
    integer(c_int),        intent(inout) :: nwave_out

    integer        :: nwave
    integer        :: itone_l(nsym)
    real           :: f0_l, fs_l

    itone_l(1:nsym) = itone(1:nsym)
    nwave = nwave_out
    f0_l  = f0
    fs_l  = fsample

    call gen_ft1wave(itone_l, nsym, nsps_num, nsps_den, fs_l, f0_l, &
                     wave_out, nwave)

    nwave_out = nwave
  end subroutine ft1_gen_wave

  !-------------------------------------------------------------------------
  ! ft1_decode_rt
  !   Mirrors ft1_test's RX path: ft1_downsample -> normalize ->
  !   turbo_decode_ft1 (niter_max = 0 : full decode + Viterbi sweep).
  !
  !   wave           : NMAX (48000) raw audio samples @ 12 kHz
  !   f0             : candidate carrier frequency (Hz)
  !   snr_est        : SNR estimate (dB in 2500 Hz BW) for noise variance
  !   message91_out  : output 91 decoded message bits (int8 0/1)
  !   ntype_out      : decode type: 1=turbo, 2=OSD, -1=failed
  !   nharderror_out : number of hard errors, -1 if failed
  !-------------------------------------------------------------------------
  subroutine ft1_decode_rt(wave, f0, snr_est, message91_out, &
                           ntype_out, nharderror_out) bind(C, name="ft1_decode_rt")
    real(c_float),    intent(in)  :: wave(FT1_NMAX)
    real(c_float),    value, intent(in) :: f0, snr_est
    integer(c_int8_t),intent(out) :: message91_out(91)
    integer(c_int),   intent(out) :: ntype_out, nharderror_out

    real            :: dd(FT1_NMAX)
    complex         :: cd(0:FT1_NDMAX-1)
    real            :: llr_out(174)
    integer(kind=1) :: message91(91)
    integer         :: npts, ntype, nharderror, ncheck_out
    real            :: dt0, dmin, sum2, f0_l, snr_l
    logical         :: newdata

    dd(1:FT1_NMAX) = wave(1:FT1_NMAX)
    f0_l  = f0
    snr_l = snr_est

    ! Downsample to complex baseband (~8 samples/symbol).
    newdata = .true.
    call ft1_downsample(dd, newdata, f0_l, cd)

    ! Normalize to unit power per sample (matching ft1_test).
    sum2 = sum(real(cd*conjg(cd))) / real(FT1_NDMAX)
    if (sum2 > 0.0) cd = cd / sqrt(sum2)

    ! Turbo-decode. niter_max=0 => full decode + Viterbi sweep (ft1_test mode).
    npts       = FT1_NDMAX
    dt0        = 0.0
    ntype      = -1
    nharderror = -1
    dmin       = 0.0
    ncheck_out = -1
    message91  = 0
    llr_out    = 0.0

    call turbo_decode_ft1(cd, npts, f0_l, dt0, snr_l, llr_out, &
                          message91, ntype, nharderror, dmin, 0, ncheck_out)

    message91_out(1:91) = message91(1:91)
    ntype_out           = ntype
    nharderror_out      = nharderror
  end subroutine ft1_decode_rt

  !-------------------------------------------------------------------------
  ! ft1_unpack
  !   Convert the 77 message bits (message91(1:77)) back to readable text.
  !   bits77   : input 77 bits (int8 0/1) -- typically message91_out[0:77]
  !   msg_out  : caller-allocated C string buffer of >= 38 bytes
  !   msg_cap  : capacity of msg_out in bytes (incl. NUL terminator)
  !   success  : 1 if unpack succeeded, 0 otherwise
  !-------------------------------------------------------------------------
  subroutine ft1_unpack(bits77, msg_out, msg_cap, success) bind(C, name="ft1_unpack")
    integer(c_int8_t),     intent(in)    :: bits77(77)
    character(kind=c_char),intent(out)   :: msg_out(*)
    integer(c_int), value, intent(in)    :: msg_cap
    integer(c_int),        intent(out)   :: success

    character(len=77) :: c77
    character(len=37) :: msg
    logical           :: ok
    integer           :: i, n

    do i = 1, 77
       if (bits77(i) == 0_c_int8_t) then
          c77(i:i) = '0'
       else
          c77(i:i) = '1'
       end if
    end do

    call unpack77(c77, 1, msg, ok)

    msg_out(1:msg_cap) = c_null_char
    if (ok) then
       n = min(len_trim(msg), msg_cap - 1)
       do i = 1, n
          msg_out(i) = msg(i:i)
       end do
       msg_out(n+1) = c_null_char
       success = 1
    else
       success = 0
    end if
  end subroutine ft1_unpack

  !-------------------------------------------------------------------------
  ! ft1_collect_cb
  !   Internal Fortran callback matching the ft1_decode_callback abstract
  !   interface. Appends each decode into the module-level results buffer.
  !   The abstract interface is unchanged; the detected redundancy version is
  !   read from the decoder object's this%cur_rv field (set by decode() before
  !   each callback), so other callback implementers are unaffected.
  !-------------------------------------------------------------------------
  subroutine ft1_collect_cb(this, sync, snr, dt, freq, decoded, nap, qual)
    class(ft1_decoder), intent(inout) :: this
    real,              intent(in) :: sync
    integer,           intent(in) :: snr
    real,              intent(in) :: dt
    real,              intent(in) :: freq
    character(len=37), intent(in) :: decoded
    integer,           intent(in) :: nap
    real,              intent(in) :: qual

    if (g_count >= FT1_MAXDEC) return
    g_count = g_count + 1
    g_results(g_count)%sync    = sync
    g_results(g_count)%snr     = snr
    g_results(g_count)%dt      = dt
    g_results(g_count)%freq    = freq
    g_results(g_count)%message = decoded
    g_results(g_count)%nap     = nap
    g_results(g_count)%qual    = qual
    g_results(g_count)%rv      = this%cur_rv
  end subroutine ft1_collect_cb

  !-------------------------------------------------------------------------
  ! ft1_decode_frame
  !   Run the FULL FT1 RX acquisition pipeline on a 4-second frame:
  !   Costas sync candidate search (time + frequency) -> downsample ->
  !   turbo decode -> OSD/AP fallback -> SIC -> IR-HARQ. Returns ALL decodes.
  !
  !   iwave         : FT1_NMAX (48000) int16 audio samples @ 12 kHz
  !   nfa, nfb      : frequency search band edges (Hz)
  !   ndepth        : decode depth (3 = full; <=0 defaults to 3)
  !   mycall,hiscall: NUL/space-terminated callsigns for AP (may be empty)
  !   nqso_progress : QSO progress index (selects AP pass schedule)
  !   frame_time_ms : monotonic millisecond timestamp of THIS frame (ms since a
  !                   session epoch; need not be wall-clock, only monotonic and
  !                   consistent across frames). Keys cross-frame IR-HARQ slot
  !                   matching + 30 s expiry. Call ft1_harq_reset() on band/QSO
  !                   change to clear stale buffers.
  !   out           : caller array of ft1_decode_t (capacity max_out)
  !   max_out       : capacity of out
  !
  !   Returns the number of decodes written (>= 0), or -1 on error.
  !-------------------------------------------------------------------------
  function ft1_decode_frame(iwave, nfa, nfb, ndepth, mycall, hiscall, &
       nqso_progress, frame_time_ms, out, max_out) result(ndec) &
       bind(C, name="ft1_decode_frame")
    integer(c_int16_t),     intent(in)    :: iwave(FT1_NMAX)
    integer(c_int), value,  intent(in)    :: nfa, nfb, ndepth
    character(kind=c_char), intent(in)    :: mycall(*)
    character(kind=c_char), intent(in)    :: hiscall(*)
    integer(c_int), value,  intent(in)    :: nqso_progress
    integer(c_int), value,  intent(in)    :: frame_time_ms
    type(ft1_decode_t),     intent(out)   :: out(*)
    integer(c_int), value,  intent(in)    :: max_out
    integer(c_int)                        :: ndec

    type(ft1_decoder)        :: decoder
    integer(kind=2)          :: iwave_l(FT1_NMAX)
    character(len=12)        :: mycall_f, hiscall_f
    integer                  :: nfqso, ndepth_l, ncontest, i, j, n, ncopy
    logical                  :: lapcqonly

    ! Reset the results buffer for this frame.
    g_count = 0

    ! Marshal inputs.
    iwave_l(1:FT1_NMAX) = int(iwave(1:FT1_NMAX), kind=2)
    call c_to_fstr12(mycall,  mycall_f)
    call c_to_fstr12(hiscall, hiscall_f)

    ndepth_l = ndepth
    if (ndepth_l <= 0) ndepth_l = 3
    nfqso     = (nfa + nfb) / 2
    lapcqonly = .false.
    ncontest  = 0

    ! IR-HARQ buffers PERSIST across frames so genuine RV0/RV1/RV2
    ! retransmissions (same freq +-10 Hz, within 30 s) combine via
    ! ft1_joint_turbo_harq. They are NOT reset per frame; the decoder keys/expires
    ! slots on frame_time_ms, and the caller invokes ft1_harq_reset() on band/QSO
    ! change. (Previously harq_init() ran every frame, disabling cross-frame HARQ.)
    decoder%frame_time_ms = frame_time_ms

    ! Wire up the collector callback and run the full pipeline.
    decoder%callback => ft1_collect_cb
    call decoder%decode(ft1_collect_cb, iwave_l, nqso_progress, nfqso, &
         nfa, nfb, ndepth_l, lapcqonly, ncontest, mycall_f, hiscall_f)

    ! Copy results into the C struct array (up to max_out).
    ncopy = min(g_count, max_out)
    do i = 1, ncopy
       out(i)%sync = g_results(i)%sync
       out(i)%snr  = g_results(i)%snr
       out(i)%dt   = g_results(i)%dt
       out(i)%freq = g_results(i)%freq
       out(i)%nap  = g_results(i)%nap
       out(i)%qual = g_results(i)%qual
       out(i)%rv   = g_results(i)%rv    ! detected redundancy version (0/1/2, or -1)

       ! Marshal message -> NUL-terminated C string, trimming trailing spaces.
       n = len_trim(g_results(i)%message)
       if (n > 37) n = 37
       do j = 1, 38
          out(i)%message(j) = c_null_char
       end do
       do j = 1, n
          out(i)%message(j) = g_results(i)%message(j:j)
       end do
    end do

    ndec = g_count
  end function ft1_decode_frame

  !=========================================================================
  ! DX1 — non-coherent M-FSK robust tier (lib/dx1).
  !
  ! DX1 reuses the 77-bit message + LDPC(174,91) FEC but transmits with 8-FSK
  ! and decodes non-coherently (energy detection + soft LDPC), so it survives
  ! fading that breaks coherent FT1/FT8. Underlying Fortran (external):
  !   dx1_encode_msg (gen_dx1wave.f90) - text -> 77 bits -> 174-bit codeword
  !   gen_dx1wave    (gen_dx1wave.f90) - codeword -> chirp sync + 8-FSK audio
  !   dx1_decode     (dx1_decode.f90)  - audio -> sync -> detect -> LLR -> text
  !=========================================================================

  ! DX1 transmit-waveform length (samples @ 12 kHz): chirp sync + 58 symbols.
  function dx1_frame_len() result(n) bind(C, name="dx1_frame_len")
    integer(c_int) :: n
    n = DX1_NFRAME
  end function dx1_frame_len

  ! DX1 receive capture-window length (samples): a full 15 s T/R slot.
  function dx1_capture_len() result(n) bind(C, name="dx1_capture_len")
    integer(c_int) :: n
    n = DX1_NMAX
  end function dx1_capture_len

  !-------------------------------------------------------------------------
  ! dx1_encode_wave
  !   msg      : NUL/space-terminated message (<= 37 chars)
  !   msg_len  : valid chars in msg
  !   f0       : audio carrier (Hz), e.g. 1500.0
  !   fsample  : sample rate (Hz), e.g. 12000.0
  !   wave_out : caller buffer (capacity max_out >= dx1_frame_len())
  !   max_out  : capacity of wave_out
  !   returns  : samples written (> 0), or -1 on pack failure
  !-------------------------------------------------------------------------
  function dx1_encode_wave(msg, msg_len, f0, fsample, wave_out, max_out) &
       result(nwave) bind(C, name="dx1_encode_wave")
    character(kind=c_char), intent(in)  :: msg(*)
    integer(c_int), value,  intent(in)  :: msg_len, max_out
    real(c_float),  value,  intent(in)  :: f0, fsample
    real(c_float),          intent(out) :: wave_out(*)
    integer(c_int)                      :: nwave

    character(len=37) :: msg37, msgsent
    integer(kind=1)   :: msgbits(77)
    integer(kind=1)   :: codeword(174)
    logical           :: ok
    integer           :: i, n, nw

    msg37 = ' '
    n = min(msg_len, 37)
    do i = 1, n
       if (msg(i) == c_null_char) exit
       msg37(i:i) = msg(i)
    end do

    call dx1_encode_msg(msg37, msgbits, codeword, msgsent, ok)
    if (.not. ok) then
       nwave = -1
       return
    end if

    ! gen_dx1wave writes DX1_NFRAME samples; the caller must size >= that.
    if (max_out < DX1_NFRAME) then
       nwave = -1
       return
    end if
    call gen_dx1wave(codeword, f0, fsample, wave_out, nw)
    nwave = nw
  end function dx1_encode_wave

  !-------------------------------------------------------------------------
  ! dx1_decode_buf
  !   wave    : nwave real audio samples @ fsample (a capture window)
  !   nwave   : number of samples
  !   f0      : audio carrier to demodulate at (Hz)
  !   fsample : sample rate (Hz)
  !   idt_lo  : sync time-search window low edge (samples)
  !   idt_hi  : sync time-search window high edge (samples)
  !   msg_out : caller C string buffer (>= 38 bytes)
  !   msg_cap : capacity of msg_out
  !   snr_out : SNR estimate (dB)
  !   sync_out: chirp-sync quality metric
  !   returns : nharderr (< 0 => decode/CRC failed)
  !-------------------------------------------------------------------------
  function dx1_decode_buf(wave, nwave, f0, fsample, idt_lo, idt_hi, &
       msg_out, msg_cap, snr_out, sync_out) result(nharderr) &
       bind(C, name="dx1_decode_buf")
    real(c_float),          intent(in)  :: wave(*)
    integer(c_int), value,  intent(in)  :: nwave, idt_lo, idt_hi, msg_cap
    real(c_float),  value,  intent(in)  :: f0, fsample
    character(kind=c_char), intent(out) :: msg_out(*)
    real(c_float),          intent(out) :: snr_out, sync_out
    integer(c_int)                      :: nharderr

    character(len=37) :: msgout
    integer(kind=1)   :: msgbits(77)
    integer           :: nh, i, n
    real              :: snr_l, sync_l

    msgout = ' '
    nh     = -1
    snr_l  = 0.0
    sync_l = 0.0

    ! c_float == default real(4); pass the buffer straight through (no copy).
    call dx1_decode(wave, nwave, f0, fsample, idt_lo, idt_hi, &
                    msgout, msgbits, nh, snr_l, sync_l)

    do i = 1, msg_cap
       msg_out(i) = c_null_char
    end do
    if (nh >= 0) then
       n = min(len_trim(msgout), msg_cap - 1)
       do i = 1, n
          msg_out(i) = msgout(i:i)
       end do
    end if
    snr_out  = snr_l
    sync_out = sync_l
    nharderr = nh
  end function dx1_decode_buf

  !-------------------------------------------------------------------------
  ! dx1_decode_band : decode EVERY DX1 signal in the audio passband (full-band
  ! acquisition, like ft1_decode_frame for FT1) -- vs dx1_decode_buf, which
  ! decodes one known carrier.  Wraps dx1_decode_all (Stage A coarse carrier
  ! scan -> Stage B peak-pick -> Stage C full decode per survivor, CRC-gated).
  !
  !   wave    : nwave real audio samples @ fsample (one capture window)
  !   nwave   : number of samples
  !   f_lo    : low edge of the carrier (lower-comb-edge) scan range, Hz
  !   f_hi    : high edge of the carrier scan range, Hz
  !   fsample : sample rate (Hz)
  !   out     : caller array of dx1_decode_t (capacity max_out)
  !   max_out : capacity of out
  !   returns : number of decodes found (>= 0); up to min(found,max_out) written
  !
  ! NOT thread-safe (the modem keeps process-global SAVE state + FFTW plans).
  !-------------------------------------------------------------------------
  function dx1_decode_band(wave, nwave, f_lo, f_hi, fsample, out, max_out) &
       result(ndec) bind(C, name="dx1_decode_band")
    real(c_float),          intent(in)  :: wave(*)
    integer(c_int), value,  intent(in)  :: nwave, max_out
    real(c_float),  value,  intent(in)  :: f_lo, f_hi, fsample
    type(dx1_decode_t),     intent(out) :: out(*)
    integer(c_int)                      :: ndec

    integer, parameter :: CAP = 64
    character(len=37) :: msgs(CAP)
    real    :: freqs(CAP), snrs(CAP), syncs(CAP)
    integer :: nd, idt_lo, idt_hi, i, k, ncopy, lim

    ndec = 0
    if (max_out <= 0) return
    if (nwave < DX1_NFRAME) return
    lim = min(max_out, CAP)
    idt_lo = 1
    idt_hi = nwave - DX1_NFRAME + 1

    call dx1_decode_all(wave, nwave, f_lo, f_hi, fsample, idt_lo, idt_hi, &
                        lim, msgs, freqs, snrs, syncs, nd)
    if (nd < 0)   nd = 0
    if (nd > lim) nd = lim

    do i = 1, nd
       out(i)%freq = freqs(i)
       out(i)%sync = syncs(i)
       out(i)%snr  = nint(snrs(i))
       do k = 1, 38
          out(i)%message(k) = c_null_char
       end do
       ncopy = min(len_trim(msgs(i)), 37)
       do k = 1, ncopy
          out(i)%message(k) = msgs(i)(k:k)
       end do
    end do
    ndec = nd
  end function dx1_decode_band

  !-------------------------------------------------------------------------
  ! c_to_fstr12 : marshal a NUL/space-terminated C string into character(12),
  !               space-padded (as the FT1 decoder expects callsigns).
  !-------------------------------------------------------------------------
  subroutine c_to_fstr12(cstr, fstr)
    character(kind=c_char), intent(in)  :: cstr(*)
    character(len=12),      intent(out) :: fstr
    integer :: i
    fstr = ' '
    do i = 1, 12
       if (cstr(i) == c_null_char) exit
       fstr(i:i) = cstr(i)
    end do
  end subroutine c_to_fstr12

  !-------------------------------------------------------------------------
  ! ft1_harq_reset
  !   Clear all IR-HARQ soft-combining buffers. Call this on band change, QSO
  !   change, or an intentional QSY so a new exchange does not combine with
  !   stale RV frames from a previous one. (Buffers otherwise persist across
  !   frames and self-expire after 30 s; see ft1_decode_frame's frame_time_ms.)
  !-------------------------------------------------------------------------
  subroutine ft1_harq_reset() bind(C, name="ft1_harq_reset")
    call harq_init()
  end subroutine ft1_harq_reset

end module ft1_cabi
