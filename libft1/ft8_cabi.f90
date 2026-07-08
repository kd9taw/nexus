! Nexus: C ABI wrappers for native FT8 decode/encode, built on the vendored
! WSJT-X GPL modem sources (lib/ft8). Exposes a clean iso_c_binding interface so
! the FT8 mode can be driven headlessly from C / Rust, mirroring ft1_cabi.f90.
! No Qt, no GUI, and NONE of WSJT-X's nzhsym streaming / a7 cross-cycle / shmem
! machinery: this decodes one complete 15 s frame by calling the core decode
! primitives directly (ft8apset -> sync8 -> ft8b), exactly as the inner loop of
! ft8_decode::decode does (ft8_decode.f90 lines ~172-239). ft8b performs its own
! signal subtraction (lsubtract), so multi-pass weak-signal recovery works.
!
! Underlying Fortran routines wrapped here:
!   genft8       (ft8/genft8.f90)      - message -> 79 channel tones
!   gen_ft8wave  (ft8/gen_ft8wave.f90) - tones -> real audio waveform
!   ft8apset     (ft8/ft8apset.f90)    - a-priori symbol setup from callsigns
!   sync8        (ft8/sync8.f90)       - Costas sync candidate search
!   ft8b         (ft8/ft8b.f90)        - per-candidate decode (+ internal subtract)
!
! Frame / array constants (from ft8/ft8_params.f90):
!   NN   = 79      total channel symbols
!   NSPS = 1920    samples per symbol @ 12 kHz
!   NMAX = 180000  raw audio samples (15.0 s @ 12 kHz)
!   NH1  = 1920    half-FFT length (sbase sizing)
!   NZ   = 151680  samples in the full 12.64 s waveform (NSPS*NN)

module ft8_cabi
  use iso_c_binding
  implicit none

  integer, parameter :: F8_NN      = 79
  integer, parameter :: F8_NSPS    = 1920
  integer, parameter :: F8_NMAX    = 180000          ! 15 * 12000
  integer, parameter :: F8_NH1     = 1920            ! NFFT1/2 = (2*1920)/2
  integer, parameter :: F8_NZ      = F8_NSPS * F8_NN ! 151680
  integer, parameter :: F8_MAXCAND = 600
  integer, parameter :: F8_MAXDEC  = 100

  ! Interop result struct. Layout MUST match ft8_decode_t in libft1.h.
  !   off 0  float sync; 4 int snr; 8 float dt; 12 float freq;
  !   16 char message[38]; 54 (pad 2) int nap; 60 float qual; total 64.
  type, bind(C) :: ft8_decode_t
     real(c_float)          :: sync         ! sync metric
     integer(c_int)         :: snr          ! SNR estimate, dB (rounded)
     real(c_float)          :: dt           ! time offset, seconds (xdt-0.5)
     real(c_float)          :: freq         ! audio frequency, Hz
     character(kind=c_char) :: message(38)  ! NUL-terminated decoded text
     integer(c_int)         :: nap          ! AP type used (iaptype; 0 = none)
     real(c_float)          :: qual         ! decode quality metric [0,1]
  end type ft8_decode_t

contains

  !-------------------------------------------------------------------------
  ! ft8_encode : message text -> 79 channel tones {0..7}
  !   msg       : NUL/space-terminated C string (<= 37 chars)
  !   msg_len   : valid chars in msg
  !   itone_out : output 79 tones
  !   nsym_out  : symbols written (79), or -1 on bad message
  !-------------------------------------------------------------------------
  subroutine ft8_encode(msg, msg_len, itone_out, nsym_out) bind(C, name="ft8_encode")
    character(kind=c_char), intent(in)  :: msg(*)
    integer(c_int), value,  intent(in)  :: msg_len
    integer(c_int),         intent(out) :: itone_out(F8_NN)
    integer(c_int),         intent(out) :: nsym_out

    character(len=37) :: msg37, msgsent37
    integer(kind=1)   :: msgbits(77)
    integer           :: itone(F8_NN)
    integer           :: i, n, i3, n3

    msg37 = ' '
    n = min(msg_len, 37)
    do i = 1, n
       if (msg(i) == c_null_char) exit
       msg37(i:i) = msg(i)
    end do

    call genft8(msg37, i3, n3, msgsent37, msgbits, itone)
    if (i3 < 0 .and. n3 < 0) then
       nsym_out = -1
       return
    end if
    itone_out(1:F8_NN) = itone(1:F8_NN)
    nsym_out = F8_NN
  end subroutine ft8_encode

  !-------------------------------------------------------------------------
  ! ft8_gen_wave : tones -> real audio waveform
  !   itone     : input tones (length nsym)
  !   nsym      : number of tones (79)
  !   fsample   : output sample rate (Hz), e.g. 12000.0
  !   f0        : audio carrier (Hz), e.g. 1500.0
  !   wave_out  : caller buffer (capacity nwave_out)
  !   nwave_out : in = capacity; out = samples produced (nsym*NSPS)
  !-------------------------------------------------------------------------
  subroutine ft8_gen_wave(itone, nsym, fsample, f0, wave_out, nwave_out) &
       bind(C, name="ft8_gen_wave")
    integer(c_int),        intent(in)    :: itone(*)
    integer(c_int), value, intent(in)    :: nsym
    real(c_float),  value, intent(in)    :: fsample, f0
    real(c_float),         intent(inout) :: wave_out(*)
    integer(c_int),        intent(inout) :: nwave_out

    integer :: nwave, itone_l(nsym)
    real    :: f0_l, fs_l, bt
    complex :: cwave(nsym*F8_NSPS)

    nwave = nsym * F8_NSPS
    if (nwave_out < nwave) then
       nwave_out = -1
       return
    end if
    itone_l(1:nsym) = itone(1:nsym)
    f0_l = f0
    fs_l = fsample
    bt   = 2.0   ! FT8 Gaussian BT
    call gen_ft8wave(itone_l, nsym, F8_NSPS, bt, fs_l, f0_l, cwave, &
                     wave_out, 0, nwave)
    nwave_out = nwave
  end subroutine ft8_gen_wave

  !-------------------------------------------------------------------------
  ! ft8_decode_frame : decode EVERY FT8 signal in a complete 15 s frame.
  !
  !   iwave         : F8_NMAX (180000) int16 audio samples @ 12 kHz
  !   nfa, nfb      : frequency search band edges (Hz)
  !   ndepth        : 1..3 (3 = full bp+osd, 3 passes; <=0 defaults to 3)
  !   mycall,hiscall: NUL/space-terminated callsigns for AP (may be empty)
  !   nqso_progress : QSO progress index (AP pass schedule)
  !   nfqso_in      : QSO/RX audio freq (Hz) the operator is working — WSJT-X's
  !                   nfqso. The deep a-priori passes (iaptype>=3, the MyCall+
  !                   DxCall masks) only fire within napwid of this, and sync8
  !                   prioritizes near it. Pass 0 / out-of-band ⇒ band center.
  !   out           : caller array of ft8_decode_t (capacity max_out)
  !   max_out       : capacity of out
  !
  !   Returns the number of decodes written (>= 0), or -1 on error.
  !
  ! NOT thread-safe (the modem keeps process-global SAVE state + FFTW plans).
  !-------------------------------------------------------------------------
  function ft8_decode_frame(iwave, nfa, nfb, ndepth, mycall, hiscall, &
       nqso_progress, nfqso_in, out, max_out) result(ndec) bind(C, name="ft8_decode_frame")
    integer(c_int16_t),     intent(in)  :: iwave(F8_NMAX)
    integer(c_int), value,  intent(in)  :: nfa, nfb, ndepth, nqso_progress, nfqso_in, max_out
    character(kind=c_char), intent(in)  :: mycall(*)
    character(kind=c_char), intent(in)  :: hiscall(*)
    type(ft8_decode_t),     intent(out) :: out(*)
    integer(c_int)                      :: ndec

    real              :: dd(F8_NMAX)
    real              :: candidate(3, F8_MAXCAND)
    real              :: sbase(F8_NH1)
    integer           :: apsym(58), aph10(10)
    integer           :: itone(F8_NN)
    character(len=37) :: allmessages(F8_MAXDEC)
    character(len=37) :: msg37
    character(len=12) :: mycall12, hiscall12
    integer :: ncontest, nfqso, nftx, ndepth_l, ndeep, npass, ipass
    integer :: maxc, ncand, icand, id, n, j, ib
    integer :: iaptype, nharderrors, nbadcrc, iappass
    integer :: nsnr, ndecodes, n2, napwid
    real    :: sync, f1, xdt, xbase, dmin, xsnr, syncmin
    logical :: newdat, lsubtract, ldupe, lft8apon, lapcqonly, nagain

    ndec = 0
    if (max_out <= 0) return

    dd(1:F8_NMAX) = real(iwave(1:F8_NMAX))
    call c_to_fstr12(mycall,  mycall12)
    call c_to_fstr12(hiscall, hiscall12)

    ndepth_l = ndepth
    if (ndepth_l <= 0) ndepth_l = 3
    ncontest  = 0
    ! Center deep AP + sync on the operator's QSO/RX freq when supplied; else the
    ! band midpoint (legacy behavior). nftx mirrors it (no separate TX freq here).
    if (nfqso_in >= nfa .and. nfqso_in <= nfb) then
       nfqso = nfqso_in
    else
       nfqso = (nfa + nfb) / 2
    end if
    nftx      = nfqso
    lft8apon  = .true.
    lapcqonly = .false.
    nagain    = .false.
    napwid    = 75

    call ft8apset(mycall12, hiscall12, ncontest, apsym, aph10)

    ndecodes = 0
    allmessages = ' '
    n2 = 0

    ! Mirror ft8_decode::decode pass logic. ft8b subtracts decoded signals from
    ! dd internally when lsubtract is set, so later passes find weaker stations.
    npass = 3
    if (ndepth_l == 1) npass = 2
    do ipass = 1, npass
       newdat  = .true.
       syncmin = 1.3
       if (ndepth_l <= 2) syncmin = 1.6
       if (ipass == 1) then
          lsubtract = .true.
          ndeep = ndepth_l
          if (ndepth_l == 3) ndeep = 2
       else if (ipass == 2) then
          n2 = ndecodes
          if (ndecodes == 0) cycle
          lsubtract = .true.
          ndeep = ndepth_l
       else
          if ((ndecodes - n2) == 0) cycle
          lsubtract = .true.
          ndeep = ndepth_l
       end if

       maxc = F8_MAXCAND
       call sync8(dd, F8_NMAX, nfa, nfb, syncmin, nfqso, maxc, candidate, ncand, sbase)

       do icand = 1, ncand
          sync  = candidate(3, icand)
          f1    = candidate(1, icand)
          xdt   = candidate(2, icand)
          ib    = max(1, nint(f1 / 3.125))
          if (ib > F8_NH1) ib = F8_NH1
          xbase = 10.0 ** (0.1 * (sbase(ib) - 40.0))
          msg37 = ' '
          call ft8b(dd, newdat, nqso_progress, nfqso, nftx, ndeep, 50, lft8apon, &
               lapcqonly, napwid, lsubtract, nagain, ncontest, iaptype, mycall12, &
               hiscall12, f1, xdt, xbase, apsym, aph10, nharderrors, dmin, &
               nbadcrc, iappass, msg37, xsnr, itone)
          nsnr = nint(xsnr)
          xdt  = xdt - 0.5
          if (nbadcrc == 0) then
             ldupe = .false.
             do id = 1, ndecodes
                if (msg37 == allmessages(id)) ldupe = .true.
             end do
             if (.not. ldupe) then
                if (ndecodes >= F8_MAXDEC) cycle
                ndecodes = ndecodes + 1
                allmessages(ndecodes) = msg37
                if (ndecodes <= max_out) then
                   out(ndecodes)%sync = sync
                   out(ndecodes)%snr  = nsnr
                   out(ndecodes)%dt   = xdt
                   out(ndecodes)%freq = f1
                   out(ndecodes)%nap  = iaptype
                   out(ndecodes)%qual = 1.0 - (nharderrors + dmin) / 60.0
                   do j = 1, 38
                      out(ndecodes)%message(j) = c_null_char
                   end do
                   n = min(len_trim(msg37), 37)
                   do j = 1, n
                      out(ndecodes)%message(j) = msg37(j:j)
                   end do
                end if
             end if
          end if
       end do
    end do

    ndec = min(ndecodes, max_out)
  end function ft8_decode_frame

  !-------------------------------------------------------------------------
  ! c_to_fstr12 : marshal a NUL/space-terminated C string into character(12),
  !               space-padded (as the FT8 decoder expects callsigns).
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

end module ft8_cabi
