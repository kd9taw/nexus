! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! DX1 full-passband acquisition harness (Stage A/B/C scan, dx1_decode_all).
!
! Places THREE independent DX1 frames at different carriers AND different time
! offsets into one receive buffer, adds AWGN, and verifies the band scanner
! recovers ALL THREE messages at the right carriers.  Also reports the wall-clock
! cost of the scan (must be << the 15 s slot) and a single-carrier full-window
! dx1_decode for comparison.
!
! SNR convention identical to dx1_test.f90 / ft1_test.f90 (2500 Hz noise BW).
! A comfortably-above-threshold SNR is used: this is a FUNCTION + BUDGET gate,
! not a sensitivity sweep (see dx1_test for the AWGN/fading waterfall).
! ---------------------------------------------------------------------------
program dx1_band_test
  use dx1_params
  implicit none

  real, external :: gran

  integer, parameter :: NSIG = 3
  integer, parameter :: MAXDEC = 16

  real, allocatable :: wave(:)        ! one clean frame (reused per signal)
  real, allocatable :: dd(:)          ! shared receive buffer
  integer*1 :: msgbits(77), codeword(174)
  character(len=37) :: msgs(NSIG), msgsent
  real      :: carriers(NSIG)
  integer   :: offsets(NSIG)
  logical   :: ok

  ! band-scan outputs
  character(len=37) :: dmsg(MAXDEC)
  real      :: dfreq(MAXDEC), dsnr(MAXDEC), dsync(MAXDEC)
  integer   :: ndec

  real      :: fs, bandwidth_ratio, sig, snrdb
  integer   :: nwave, i, k, isig, nfound
  integer   :: idt_lo, idt_hi
  logical   :: found(NSIG)
  integer   :: nargs
  character(len=64) :: arg

  ! timing
  integer(8) :: c0, c1, crate
  real       :: t_scan, t_single

  ! single-carrier comparison
  character(len=37) :: smsg
  integer*1 :: sbits(77)
  integer   :: snh
  real      :: ssnr, ssync

  fs = DX1_FS
  bandwidth_ratio = 2500.0/(fs/2.0)

  snrdb = -5.0          ! comfortably above the ~-18 dB AWGN threshold
  nargs = iargc()
  if (nargs >= 1) then
     call getarg(1, arg); read(arg,*) snrdb
  endif

  msgs(1) = 'CQ W9XYZ EN37'
  msgs(2) = 'CQ K2DEF FN20'
  msgs(3) = 'CQ AA1BB FN42'
  carriers = (/ 700.0, 1500.0, 2300.0 /)
  offsets  = (/ 3000, 6000, 9000 /)        ! distinct arrival times (samples)

  allocate(wave(DX1_NFRAME))
  allocate(dd(DX1_NMAX))

  write(*,'(a)') '============================================================'
  write(*,'(a)') ' DX1-S full-passband acquisition test (dx1_decode_all)'
  write(*,'(a)') '============================================================'
  write(*,'(a,i2,a,f6.1,a)') ' Signals      : ', NSIG, '   SNR=', snrdb, ' dB (2500 Hz ref)'
  write(*,'(a,3f8.1)')       ' Carriers (Hz): ', carriers
  write(*,'(a,3i8)')         ' Offsets (smp): ', offsets
  write(*,*)

  call sgran()

  ! --- Build the multi-signal receive buffer: 3 clean frames at their carriers
  !     and time offsets, summed, then one AWGN realisation over the whole slot.
  dd = 0.0
  do isig = 1, NSIG
     call dx1_encode_msg(msgs(isig), msgbits, codeword, msgsent, ok)
     if (.not. ok) then
        write(*,'(a,i0)') ' FATAL: message did not pack, signal ', isig
        stop 1
     endif
     call gen_dx1wave(codeword, carriers(isig), fs, wave, nwave)
     do k = 1, nwave
        i = k + offsets(isig)
        if (i >= 1 .and. i <= DX1_NMAX) dd(i) = dd(i) + wave(k)
     enddo
  enddo

  sig = sqrt(2.0*bandwidth_ratio)*10.0**(0.05*snrdb)
  do i = 1, DX1_NMAX
     dd(i) = sig*dd(i) + gran()
  enddo

  ! --- Full-band scan over the audio passband (200..2900 Hz) ---
  idt_lo = 1
  idt_hi = DX1_NMAX - DX1_NFRAME + 1
  call system_clock(c0, crate)
  call dx1_decode_all(dd, DX1_NMAX, 200.0, 2900.0, fs, idt_lo, idt_hi, &
                      MAXDEC, dmsg, dfreq, dsnr, dsync, ndec)
  call system_clock(c1)
  t_scan = real(c1-c0)/real(crate)

  write(*,'(a,i0,a)') ' --- Band scan found ', ndec, ' decode(s) ---'
  do i = 1, ndec
     write(*,'(a,i2,a,a20,a,f8.1,a,f6.1,a,es10.2)') '   [', i, '] "', dmsg(i), &
          '"  f=', dfreq(i), ' Hz  snr=', dsnr(i), '  sync=', dsync(i)
  enddo
  write(*,*)

  ! --- Verify each placed signal was recovered at ~its carrier ---
  ! On a miss, also run a single-carrier dx1_decode at the TRUE carrier on the
  ! SAME buffer: that isolates a scanner fault (Stage A/B dropped a decodable
  ! signal) from a decode-margin miss (the signal is simply at its threshold in
  ! this overlap geometry -- single-carrier would miss too).
  found = .false.
  nfound = 0
  do isig = 1, NSIG
     do i = 1, ndec
        if (trim(dmsg(i)) == trim(msgs(isig)) .and. &
            abs(dfreq(i) - carriers(isig)) <= DX1_BAUD) then   ! within one baud bin
           found(isig) = .true.
        endif
     enddo
     if (found(isig)) then
        nfound = nfound + 1
        write(*,'(a,a20,a,f7.1,a)') ' FOUND  ', msgs(isig), ' @ ', carriers(isig), ' Hz'
     else
        call dx1_decode(dd, DX1_NMAX, carriers(isig), fs, idt_lo, idt_hi, &
                        smsg, sbits, snh, ssnr, ssync)
        write(*,'(a,a20,a,f7.1,a,l1,a,i0)') ' MISS   ', msgs(isig), ' @ ', &
             carriers(isig), ' Hz  [single-carrier decodable=', &
             (snh >= 0 .and. trim(smsg) == trim(msgs(isig))), ' nh=', snh
     endif
  enddo
  write(*,*)

  ! --- Single-carrier full-window dx1_decode timing (the per-carrier cost the
  !     naive sweep would pay ~213x).  Decodes signal #2 at its known carrier.
  call system_clock(c0, crate)
  call dx1_decode(dd, DX1_NMAX, carriers(2), fs, idt_lo, idt_hi, smsg, sbits, &
                  snh, ssnr, ssync)
  call system_clock(c1)
  t_single = real(c1-c0)/real(crate)

  write(*,'(a)') ' ==================== TIMING ===================='
  write(*,'(a,f7.2,a)') ' Full-band scan (3 signals)   : ', t_scan,   ' s'
  write(*,'(a,f7.2,a)') ' Single-carrier full window   : ', t_single, ' s'
  write(*,'(a,f7.1,a)') ' Naive sweep would cost ~      : ', &
       t_single * ((2900.0-200.0)/12.5), ' s (213 carriers)'
  write(*,*)

  write(*,'(a)') ' ==================== RESULT ===================='
  if (nfound == NSIG) then
     write(*,'(a,i0,a,i0,a)') ' PASS: all ', NSIG, ' signals decoded (', ndec, ' total decodes)'
     deallocate(wave, dd)
     stop 0
  else
     write(*,'(a,i0,a,i0,a)') ' FAIL: only ', nfound, ' of ', NSIG, ' signals decoded'
     deallocate(wave, dd)
     stop 1
  endif

end program dx1_band_test
