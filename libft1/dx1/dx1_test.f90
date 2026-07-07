! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! DX1-S simulation harness.
!
! Pipeline: dx1_encode_msg -> gen_dx1wave -> (+AWGN [+Rayleigh fade]) ->
!           dx1_decode (sync -> detect -> LLR -> LDPC -> unpack77).
!
! Two sweeps:
!   1. AWGN: SNR 0 .. -24 dB, N trials/point, report 50% decode threshold.
!   2. Per-symbol flat Rayleigh fading (independent draw per symbol, with an
!      optional slow ~1 Hz component) to demonstrate the small fading penalty.
!
! SNR convention: identical to ft1_test.f90 / WSJT-X.  Noise is unit-variance
! per sample (gran()); the signal is scaled by
!     sig = sqrt(2 * bandwidth_ratio) * 10^(0.05 * snrdb),
!     bandwidth_ratio = 2500 / (fs/2),
! i.e. SNR referenced to a 2500 Hz noise bandwidth -- directly comparable to
! FT8's -21 dB figure.  (The 2500 Hz reference is the standard; DX1 occupies
! only ~50 Hz, so the in-band SNR is ~17 dB higher than the reported number.)
! ---------------------------------------------------------------------------
program dx1_test
  use dx1_params
  implicit none

  real, external :: gran            ! C Gaussian-noise generator (gran.c)

  real, allocatable :: wave(:)      ! clean frame
  real, allocatable :: dd(:)        ! placed + noisy receive buffer
  real, allocatable :: ddclean(:)   ! placed clean buffer (faded, no noise)
  integer*1 :: msgbits(77), codeword(174), dmsgbits(77)
  character(len=37) :: msgsent, msgrx
  logical   :: ok
  integer   :: nwave, i, k, isnr, itrial, nsnr, ndec, nharderr
  integer   :: noff_test, idt_lo, idt_hi
  real      :: f0, fs, dt, bandwidth_ratio, sig, snrdb
  real      :: snr_est, sync_metric
  real      :: snr_start, snr_end
  integer   :: ntrials
  character(len=64) :: arg
  integer   :: nargs

  ! fading
  real      :: fade_amp(DX1_NSYM)
  real      :: thr_awgn, thr_fade
  real      :: prev_rate

  fs = DX1_FS
  dt = 1.0/fs
  f0 = DX1_F0
  bandwidth_ratio = 2500.0/(fs/2.0)

  snr_start = 0.0
  snr_end   = -24.0
  ntrials   = 200
  nargs = iargc()
  if(nargs.ge.1) then
     call getarg(1,arg); read(arg,*) snr_start
  endif
  if(nargs.ge.2) then
     call getarg(2,arg); read(arg,*) snr_end
  endif
  if(nargs.ge.3) then
     call getarg(3,arg); read(arg,*) ntrials
  endif

  allocate(wave(DX1_NFRAME))
  allocate(dd(DX1_NMAX))
  allocate(ddclean(DX1_NMAX))

  ! --- Encode a test message -> 174 coded bits ---
  call dx1_encode_msg('CQ W9XYZ EN37', msgbits, codeword, msgsent, ok)
  if(.not.ok) then
     write(*,'(a)') 'FATAL: message did not pack.'
     stop 1
  endif

  ! --- Generate the clean DX1-S frame ---
  call gen_dx1wave(codeword, f0, fs, wave, nwave)

  write(*,'(a)')        '============================================================'
  write(*,'(a)')        ' DX1-S simulation harness (M=8 FSK, baud=6.25, LDPC 174,91)'
  write(*,'(a)')        '============================================================'
  write(*,'(a,a37)')    ' Test message : ', msgsent
  write(*,'(a,f7.1,a)') ' Carrier f0   : ', f0, ' Hz'
  write(*,'(a,f6.2,a,i6,a)') ' Baud         : ', DX1_BAUD, ' Hz  NSPS=', DX1_NSPS, &
                             '  (1920-pt FFT, 6.25 Hz/bin)'
  write(*,'(a,i3,a,i3,a)')   ' Symbols      : ', DX1_NSYM, ' data + ', DX1_NSYNC, &
                             ' chirp-sync'
  write(*,'(a,f6.1,a,f6.2,a)') ' Occupied BW  : ', DX1_BW, ' Hz   data dur=', &
                             real(DX1_NDATA_SAMP)/fs, ' s'
  write(*,'(a,i3,a)')   ' Trials/point : ', ntrials, ''
  write(*,'(a)')        ' SNR ref      : 2500 Hz noise BW (comparable to FT8 -21 dB)'
  write(*,*)

  ! --- Time offset: place the frame 0.25 s into the buffer ---
  noff_test = nint(0.25*fs)
  idt_lo = noff_test - nint(0.20*fs)
  idt_hi = noff_test + nint(0.20*fs)
  if(idt_lo.lt.1) idt_lo = 1

  call sgran()

  ! --- High-SNR round-trip sanity check (GATE 1) ---
  call build_clean(wave, nwave, noff_test, ddclean, DX1_NMAX)
  sig = sqrt(2.0*bandwidth_ratio)*10.0**(0.05*10.0)   ! +10 dB
  do i=1,DX1_NMAX
     dd(i) = sig*ddclean(i) + gran()
  enddo
  call dx1_decode(dd, DX1_NMAX, f0, fs, idt_lo, idt_hi, msgrx, dmsgbits, &
                  nharderr, snr_est, sync_metric)
  write(*,'(a)') ' --- Round-trip sanity check (+10 dB AWGN) ---'
  write(*,'(a,a37)') '   decoded      : ', msgrx
  write(*,'(a,i4,a,es11.3)') '   nharderr     : ', nharderr, &
                           '   sync_metric=', sync_metric
  if(nharderr.ge.0 .and. trim(msgrx).eq.trim(msgsent)) then
     write(*,'(a)') '   RESULT       : PASS (exact round-trip)'
  else
     write(*,'(a)') '   RESULT       : FAIL'
  endif
  write(*,*)

  nsnr = nint(snr_start - snr_end) + 1
  if(nsnr.gt.60) nsnr = 60

  ! ====================== AWGN SWEEP ======================
  write(*,'(a)') ' ================== AWGN ==================='
  write(*,'(a)') '    SNR(dB)   Decoded/Trials      Rate'
  write(*,'(a)') '    -------   --------------      ----'
  call build_clean(wave, nwave, noff_test, ddclean, DX1_NMAX)
  thr_awgn = -999.0
  prev_rate = 1.0
  do isnr=1,nsnr
     snrdb = snr_start - real(isnr-1)
     sig = sqrt(2.0*bandwidth_ratio)*10.0**(0.05*snrdb)
     ndec = 0
     do itrial=1,ntrials
        do i=1,DX1_NMAX
           dd(i) = sig*ddclean(i) + gran()
        enddo
        call dx1_decode(dd, DX1_NMAX, f0, fs, idt_lo, idt_hi, msgrx, &
                        dmsgbits, nharderr, snr_est, sync_metric)
        if(nharderr.ge.0 .and. trim(msgrx).eq.trim(msgsent)) ndec = ndec + 1
     enddo
     call report_point(snrdb, ndec, ntrials)
     call update_threshold(snrdb, ndec, ntrials, prev_rate, thr_awgn)
  enddo
  write(*,*)

  ! ====================== RAYLEIGH FADING SWEEP ======================
  ! Each symbol's amplitude is multiplied by an independent Rayleigh draw
  ! (mean power normalised to 1), plus a slow ~1 Hz sinusoidal component to
  ! model a non-static channel.  The chirp sync amplitude is left unfaded-ish
  ! (it spans the whole preamble) -- realistic since the chirp's long
  ! integration averages fading.
  write(*,'(a)') ' ============ RAYLEIGH FADING (per-symbol) ============'
  write(*,'(a)') '    SNR(dB)   Decoded/Trials      Rate'
  write(*,'(a)') '    -------   --------------      ----'
  thr_fade = -999.0
  prev_rate = 1.0
  do isnr=1,nsnr
     snrdb = snr_start - real(isnr-1)
     sig = sqrt(2.0*bandwidth_ratio)*10.0**(0.05*snrdb)
     ndec = 0
     do itrial=1,ntrials
        call draw_rayleigh(fade_amp, DX1_NSYM)
        call build_faded(wave, nwave, noff_test, fade_amp, ddclean, DX1_NMAX)
        do i=1,DX1_NMAX
           dd(i) = sig*ddclean(i) + gran()
        enddo
        call dx1_decode(dd, DX1_NMAX, f0, fs, idt_lo, idt_hi, msgrx, &
                        dmsgbits, nharderr, snr_est, sync_metric)
        if(nharderr.ge.0 .and. trim(msgrx).eq.trim(msgsent)) ndec = ndec + 1
     enddo
     call report_point(snrdb, ndec, ntrials)
     call update_threshold(snrdb, ndec, ntrials, prev_rate, thr_fade)
  enddo
  write(*,*)

  ! ====================== SUMMARY ======================
  write(*,'(a)') ' ==================== SUMMARY ===================='
  if(thr_awgn.gt.-900.0) then
     write(*,'(a,f6.1,a)') ' AWGN    50% decode threshold : ', thr_awgn, ' dB'
  else
     write(*,'(a)')        ' AWGN    50% decode threshold : not reached in sweep'
  endif
  if(thr_fade.gt.-900.0) then
     write(*,'(a,f6.1,a)') ' Fading  50% decode threshold : ', thr_fade, ' dB'
  else
     write(*,'(a)')        ' Fading  50% decode threshold : not reached in sweep'
  endif
  if(thr_awgn.gt.-900.0 .and. thr_fade.gt.-900.0) then
     write(*,'(a,f6.1,a)') ' Fading penalty               : ', &
                           thr_awgn - thr_fade, ' dB'
  endif

  deallocate(wave, dd, ddclean)

contains

  ! Place a clean (unfaded) frame into the receive buffer at offset noff.
  subroutine build_clean(w, nw, noff, buf, nbuf)
    integer, intent(in)  :: nw, noff, nbuf
    real,    intent(in)  :: w(nw)
    real,    intent(out) :: buf(nbuf)
    integer :: ii, kk
    buf = 0.0
    do ii=1,nw
       kk = ii + noff
       if(kk.ge.1 .and. kk.le.nbuf) buf(kk) = w(ii)
    enddo
  end subroutine build_clean

  ! Place a per-symbol-faded frame.  Sync preamble unfaded; each data symbol
  ! scaled by fade_amp(isym).
  subroutine build_faded(w, nw, noff, famp, buf, nbuf)
    integer, intent(in)  :: nw, noff, nbuf
    real,    intent(in)  :: w(nw), famp(DX1_NSYM)
    real,    intent(out) :: buf(nbuf)
    integer :: ii, kk, isym, base, sidx
    buf = 0.0
    ! sync part (slowly faded by the mean to be mildly realistic = unfaded)
    do ii=1,DX1_NSPS_SYNC
       kk = ii + noff
       if(kk.ge.1 .and. kk.le.nbuf) buf(kk) = w(ii)
    enddo
    ! data part
    do isym=1,DX1_NSYM
       base = DX1_NSPS_SYNC + (isym-1)*DX1_NSPS
       do sidx=1,DX1_NSPS
          ii = base + sidx
          kk = ii + noff
          if(ii.le.nw .and. kk.ge.1 .and. kk.le.nbuf) &
               buf(kk) = w(ii)*famp(isym)
       enddo
    enddo
  end subroutine build_faded

  ! Independent per-symbol Rayleigh amplitude draws (unit mean power), times a
  ! slow ~1 Hz sinusoidal envelope (range 0.5..1.5) to model channel motion.
  subroutine draw_rayleigh(famp, n)
    integer, intent(in)  :: n
    real,    intent(out) :: famp(n)
    integer :: jj
    real    :: g1, g2, r, slow, twopi, tsec
    twopi = 6.2831853
    do jj=1,n
       g1 = gran()
       g2 = gran()
       ! Rayleigh amplitude with E[a^2]=1: a = sqrt((g1^2+g2^2)/2)
       r = sqrt((g1*g1 + g2*g2)*0.5)
       tsec = real(jj-1)*real(DX1_NSPS)/DX1_FS
       slow = 1.0 + 0.5*sin(twopi*1.0*tsec)   ! 1 Hz, +/-0.5
       famp(jj) = r*slow
    enddo
  end subroutine draw_rayleigh

  subroutine report_point(snrdb, ndec, ntr)
    real,    intent(in) :: snrdb
    integer, intent(in) :: ndec, ntr
    write(*,'(4x,f7.1,4x,i5,a,i5,4x,f7.1,a)') snrdb, ndec, ' / ', ntr, &
         100.0*real(ndec)/real(ntr), ' %'
  end subroutine report_point

  ! Linear-interpolate the SNR at which the rate crosses 50% (descending SNR).
  subroutine update_threshold(snrdb, ndec, ntr, prev_rate, thr)
    real,    intent(in)    :: snrdb
    integer, intent(in)    :: ndec, ntr
    real,    intent(inout) :: prev_rate
    real,    intent(inout) :: thr
    real :: rate
    rate = real(ndec)/real(ntr)
    if(thr.le.-900.0 .and. prev_rate.ge.0.5 .and. rate.lt.0.5) then
       ! crossing between (snrdb+1, prev_rate) and (snrdb, rate)
       if(prev_rate.gt.rate) then
          thr = snrdb + (0.5 - rate)/(prev_rate - rate)
       else
          thr = snrdb
       endif
    endif
    prev_rate = rate
  end subroutine update_threshold

end program dx1_test
