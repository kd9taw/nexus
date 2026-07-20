! Nexus: C ABI wrappers for native FT4 decode/encode, built on the vendored
! WSJT-X GPL modem sources (lib/ft4 + ft4_decode.f90). Mirrors ft8_cabi.f90 /
! ft1_cabi.f90. Unlike FT8, the WSJT-X FT4 decoder (ft4_decode::decode) is a
! clean, self-contained OO decoder with a callback (no nzhsym/a7/shmem), so we
! drive it directly via a collector callback exactly like ft1_cabi does for FT1.
!
! Underlying Fortran:
!   genft4       (ft4/genft4.f90)      - message -> 103 channel tones {0..3}
!   gen_ft4wave  (ft4/gen_ft4wave.f90) - tones -> full-length real audio frame
!   ft4_decode   (ft4_decode.f90)      - OO decoder: getcandidates4 -> sync4d ->
!                                        get_ft4_bitmetrics -> decode174_91 -> subtract
!
! Frame / array constants (from ft4/ft4_params.f90):
!   NN   = 103     sync + data channel symbols (16 sync + 87 data)
!   NSPS = 576     samples per symbol @ 12 kHz
!   NMAX = 72576   samples in iwave (21*3456, ~6.05 s window of the 7.5 s slot)

module ft4_cabi
  use iso_c_binding
  use ft4_decode, only: ft4_decoder
  implicit none

  integer, parameter :: F4_NN     = 103
  integer, parameter :: F4_NSPS   = 576
  integer, parameter :: F4_NMAX   = 72576           ! 21 * 3456
  integer, parameter :: F4_MAXDEC = 200   ! stock WSJT-X per-period cap (MAXDEC)

  ! Interop result struct. Layout MUST match ft4_decode_t in libft1.h
  ! (identical layout to ft8_decode_t: 64 bytes).
  type, bind(C) :: ft4_decode_t
     real(c_float)          :: sync
     integer(c_int)         :: snr
     real(c_float)          :: dt
     real(c_float)          :: freq
     character(kind=c_char) :: message(38)
     integer(c_int)         :: nap
     real(c_float)          :: qual
  end type ft4_decode_t

  ! Module-level results buffer populated by the collector callback.
  ! NOT thread-safe: ft4_decode_frame() must not be called concurrently.
  type :: ft4_result_rec
     real    :: sync
     integer :: snr
     real    :: dt
     real    :: freq
     character(len=37) :: message
     integer :: nap
     real    :: qual
  end type ft4_result_rec

  type(ft4_result_rec), save :: g4_results(F4_MAXDEC)
  integer,              save :: g4_count = 0

contains

  !-------------------------------------------------------------------------
  ! ft4_encode : message text -> 103 channel tones {0..3}
  !-------------------------------------------------------------------------
  subroutine ft4_encode(msg, msg_len, itone_out, nsym_out) bind(C, name="ft4_encode")
    character(kind=c_char), intent(in)  :: msg(*)
    integer(c_int), value,  intent(in)  :: msg_len
    integer(c_int),         intent(out) :: itone_out(F4_NN)
    integer(c_int),         intent(out) :: nsym_out

    character(len=37) :: msg37, msgsent37
    integer(kind=1)   :: msgbits(77)
    integer           :: i4tone(F4_NN)
    integer           :: i, n

    msg37 = ' '
    n = min(msg_len, 37)
    do i = 1, n
       if (msg(i) == c_null_char) exit
       msg37(i:i) = msg(i)
    end do

    call genft4(msg37, 0, msgsent37, msgbits, i4tone)
    if (index(msgsent37, '*** bad') > 0) then
       nsym_out = -1
       return
    end if
    itone_out(1:F4_NN) = i4tone(1:F4_NN)
    nsym_out = F4_NN
  end subroutine ft4_encode

  !-------------------------------------------------------------------------
  ! ft4_gen_wave : tones -> full-length real audio frame (NMAX samples),
  ! exactly as ft4sim does (gen_ft4wave positions the shaped/ramped signal).
  !-------------------------------------------------------------------------
  subroutine ft4_gen_wave(itone, nsym, fsample, f0, wave_out, nwave_out) &
       bind(C, name="ft4_gen_wave")
    integer(c_int),        intent(in)    :: itone(*)
    integer(c_int), value, intent(in)    :: nsym
    real(c_float),  value, intent(in)    :: fsample, f0
    real(c_float),         intent(inout) :: wave_out(*)
    integer(c_int),        intent(inout) :: nwave_out

    integer :: itone_l(nsym)
    real    :: f0_l, fs_l
    complex :: cwave(F4_NMAX)

    if (nwave_out < F4_NMAX) then
       nwave_out = -1
       return
    end if
    itone_l(1:nsym) = itone(1:nsym)
    f0_l = f0
    fs_l = fsample
    call gen_ft4wave(itone_l, nsym, F4_NSPS, fs_l, f0_l, cwave, &
                     wave_out, 0, F4_NMAX)
    nwave_out = F4_NMAX
  end subroutine ft4_gen_wave

  !-------------------------------------------------------------------------
  ! ft4_collect_cb : collector callback matching ft4_decode_callback.
  !-------------------------------------------------------------------------
  subroutine ft4_collect_cb(this, sync, snr, dt, freq, decoded, nap, qual)
    class(ft4_decoder), intent(inout) :: this
    real,              intent(in) :: sync
    integer,           intent(in) :: snr
    real,              intent(in) :: dt
    real,              intent(in) :: freq
    character(len=37), intent(in) :: decoded
    integer,           intent(in) :: nap
    real,              intent(in) :: qual

    if (g4_count >= F4_MAXDEC) return
    g4_count = g4_count + 1
    g4_results(g4_count)%sync    = sync
    g4_results(g4_count)%snr     = snr
    g4_results(g4_count)%dt      = dt
    g4_results(g4_count)%freq    = freq
    g4_results(g4_count)%message = decoded
    g4_results(g4_count)%nap     = nap
    g4_results(g4_count)%qual    = qual
  end subroutine ft4_collect_cb

  !-------------------------------------------------------------------------
  ! ft4_decode_frame : decode EVERY FT4 signal in a 72576-sample frame.
  !
  !   iwave         : F4_NMAX (72576) int16 audio samples @ 12 kHz
  !   nfa, nfb      : frequency search band edges (Hz)
  !   ndepth        : 1..3 (3 = full bp+osd; <=0 defaults to 3)
  !   mycall,hiscall: NUL/space-terminated callsigns for AP (may be empty)
  !   nqso_progress : QSO progress index (AP pass schedule)
  !   nfqso_in      : QSO/RX audio freq (Hz) the operator is working — WSJT-X's
  !                   nfqso. The deep a-priori passes fire near it. Pass 0 /
  !                   out-of-band ⇒ band center.
  !   out           : caller array of ft4_decode_t (capacity max_out)
  !   max_out       : capacity of out
  !
  !   Returns the number of decodes written (>= 0), or -1 on error.
  !
  ! NOT thread-safe (the modem keeps process-global SAVE state + FFTW plans).
  !-------------------------------------------------------------------------
  function ft4_decode_frame(iwave, nfa, nfb, ndepth, mycall, hiscall, &
       nqso_progress, nfqso_in, out, max_out) result(ndec) bind(C, name="ft4_decode_frame")
    integer(c_int16_t),     intent(in)  :: iwave(F4_NMAX)
    integer(c_int), value,  intent(in)  :: nfa, nfb, ndepth, nqso_progress, nfqso_in, max_out
    character(kind=c_char), intent(in)  :: mycall(*)
    character(kind=c_char), intent(in)  :: hiscall(*)
    type(ft4_decode_t),     intent(out) :: out(*)
    integer(c_int)                      :: ndec

    type(ft4_decoder) :: decoder
    integer(kind=2)   :: iwave_l(F4_NMAX)
    character(len=12) :: mycall_f, hiscall_f
    integer           :: nfqso, ndepth_l, ncontest, i, j, n, ncopy
    logical           :: lapcqonly

    ndec = 0
    if (max_out <= 0) return

    g4_count = 0
    iwave_l(1:F4_NMAX) = int(iwave(1:F4_NMAX), kind=2)
    call c_to_fstr12(mycall,  mycall_f)
    call c_to_fstr12(hiscall, hiscall_f)

    ndepth_l = ndepth
    if (ndepth_l <= 0) ndepth_l = 3
    ! Center deep AP on the operator's QSO/RX freq when supplied; else band mid.
    if (nfqso_in >= nfa .and. nfqso_in <= nfb) then
       nfqso = nfqso_in
    else
       nfqso = (nfa + nfb) / 2
    end if
    lapcqonly = .false.
    ncontest  = 0

    decoder%callback => ft4_collect_cb
    call decoder%decode(ft4_collect_cb, iwave_l, nqso_progress, nfqso, &
         nfa, nfb, ndepth_l, lapcqonly, ncontest, mycall_f, hiscall_f)

    ! ncopy == max_out here means the cap was hit and the weakest decodes were
    ! dropped: raise F4_MAXDEC and the Rust MAX_DECODES (crates/ft4) together.
    ncopy = min(g4_count, max_out)
    do i = 1, ncopy
       out(i)%sync = g4_results(i)%sync
       out(i)%snr  = g4_results(i)%snr
       out(i)%dt   = g4_results(i)%dt
       out(i)%freq = g4_results(i)%freq
       out(i)%nap  = g4_results(i)%nap
       out(i)%qual = g4_results(i)%qual
       do j = 1, 38
          out(i)%message(j) = c_null_char
       end do
       n = len_trim(g4_results(i)%message)
       if (n > 37) n = 37
       do j = 1, n
          out(i)%message(j) = g4_results(i)%message(j:j)
       end do
    end do

    ndec = g4_count
  end function ft4_decode_frame

  !-------------------------------------------------------------------------
  ! c_to_fstr12 : marshal a NUL/space-terminated C string into character(12).
  ! (Module-scoped: name-mangled per module, so no clash with ft8_cabi's copy.)
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

end module ft4_cabi
