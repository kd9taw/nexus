! FT1 - 4-CPM turbo equalization mode for WSJT-X
! Copyright (C) 2026 Seth McCall, KD9TAW
!
! This file is part of WSJT-X.
!
! WSJT-X is free software: you can redistribute it and/or modify
! it under the terms of the GNU General Public License as published by
! the Free Software Foundation, either version 3 of the License, or
! (at your option) any later version.
!
! WSJT-X is distributed in the hope that it will be useful,
! but WITHOUT ANY WARRANTY; without even the implied warranty of
! MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
! GNU General Public License for more details.
!
! You should have received a copy of the GNU General Public License
! along with WSJT-X. If not, see <https://www.gnu.org/licenses/>.
!
program ft1_test

! FT1 AWGN loopback test -- exercises the turbo decoder directly.
!
! Pipeline: genft1 -> gen_ft1wave -> AWGN -> ft1_downsample -> turbo_decode_ft1
!
! Usage: ft1_test [snr_start snr_end ntrials]
!   Defaults: 0 -20 100
!
! Expected: threshold near -15 to -16 dB (50% decode rate)

  use packjt77
  include 'ft1_params.f90'

  parameter (NDMAX=NMAX/NDOWN)           !888

  character*37 msg37,msgsent37
  character*77 c77
  character arg*12
  integer itone(NN)
  integer*1 msgbits(77)
  integer*1 message91(91)
  real wave(NMAX)
  real dd(NMAX)
  real dd_noisy(NMAX)
  real dd_clean(NMAX)
  real llr_out(174)
  complex cd(0:NDMAX-1)
  complex cd_clean(0:NDMAX-1)
  real actual_sigma2, theoretical_sigma2
  logical newdata

  ! Timing variables
  real :: t_start, t_end, t_decode
  real :: t_min, t_max, t_sum
  integer :: noff_test, k
  real :: dt0_expected

  ! Test message
  msg37='CQ W9XYZ EN37'
  f0=1500.0

  ! Parse command-line arguments
  snr_start=0.0
  snr_end=-20.0
  ntrials=100
  nargs=iargc()
  if(nargs.ge.1) then
     call getarg(1,arg)
     read(arg,*) snr_start
  endif
  if(nargs.ge.2) then
     call getarg(2,arg)
     read(arg,*) snr_end
  endif
  if(nargs.ge.3) then
     call getarg(3,arg)
     read(arg,*) ntrials
  endif

  fs=12000.0
  dt=1.0/fs
  bandwidth_ratio=2500.0/(fs/2.0)

  ! Encode the test message
  call genft1(msg37,0,msgsent37,msgbits,itone)
  write(*,'(a,a37)') 'Test message: ',msgsent37
  write(*,'(a,f7.1,a)') 'Carrier freq: ',f0,' Hz'
  write(*,'(a,i3,a)') 'Trials:       ',ntrials,' per SNR point'
  write(*,*)

  ! Generate clean 4-CPM waveform at 12 kHz
  nwave=NMAX
  wave=0.
  call gen_ft1wave(itone,NN,NSPS_NUM,NSPS_DEN,fs,f0,wave,nwave)

  ! Signal with time offset (matching pipeline conditions)
  noff_test=3000                                  !0.25s at 12kHz
  dd=0.
  do i=1,NMAX
     k=i+noff_test
     if(k.ge.1 .and. k.le.NMAX) dd(k)=wave(i)
  enddo
  dt0_expected=real(noff_test)/real(NDOWN)         !Expected downsampled timing
  write(*,'(a,i5,a,f8.2)') 'Time offset: ',noff_test, &
       ' samples, expected dt0=',dt0_expected

  call sgran()

  ! SNR sweep
  nsnr=nint(snr_start-snr_end)+1
  if(nsnr.gt.50) nsnr=50

  write(*,'(a)') '   SNR   Decoded  Trials   Rate    Avg_nerr' &
       //'   t_avg(ms) t_min  t_max'
  write(*,'(a)') '   ---   -------  ------   ----    --------' &
       //'   --------- -----  -----'

  do isnr=1,nsnr
     snrdb=snr_start - real(isnr-1)
     sig=sqrt(2*bandwidth_ratio) * 10.0**(0.05*snrdb)

     ndec=0
     nerr_total=0
     t_min=1.0e30
     t_max=0.0
     t_sum=0.0

     do itrial=1,ntrials
        ! Add AWGN
        do i=1,NMAX
           dd_noisy(i)=sig*dd(i) + gran()
        enddo

        ! Downsample to baseband complex signal
        ! DEBUG: downsample at f0+5 Hz to simulate pipeline frequency error
        newdata=.true.
        call ft1_downsample(dd_noisy,newdata,f0,cd)

        ! Normalize signal to unit power per sample
        sum2=sum(real(cd*conjg(cd)))/real(NDMAX)
        if(sum2.gt.0.0) cd=cd/sqrt(sum2)

        ! Diagnostic: measure actual noise variance in downsampled signal
        if(isnr.eq.1 .and. itrial.eq.1) then
           ! Downsample the noiseless signal for comparison
           do i=1,NMAX
              dd_clean(i)=sig*dd(i)
           enddo
           newdata=.true.
           call ft1_downsample(dd_clean,newdata,f0,cd_clean)
           ! Normalize cd_clean to unit power (same as cd)
           sum2=sum(real(cd_clean*conjg(cd_clean)))/real(NDMAX)
           if(sum2.gt.0.0) cd_clean=cd_clean/sqrt(sum2)

           ! Measure actual noise variance after downsample+normalize
           actual_sigma2=sum(real((cd-cd_clean)*conjg(cd-cd_clean)))/real(NDMAX)

           ! Theoretical sigma2 = 1/(1+SNR_lin) where SNR_lin is in-band
           theoretical_sigma2=1.0/(1.0 + 10.0**(snrdb/10.0)*(2500.0/126.0))

           write(*,*)
           write(*,'(a)') 'Noise variance diagnostic (first trial):'
           write(*,'(a,f6.1,a)') '  SNR         = ', snrdb, ' dB'
           write(*,'(a,es12.4)') '  sig         = ', sig
           write(*,'(a,f10.6)')  '  actual_sigma2      = ', actual_sigma2
           write(*,'(a,f10.6)')  '  theoretical_sigma2 = ', theoretical_sigma2
           write(*,'(a,f10.4)')  '  ratio (actual/theo)= ', &
                actual_sigma2/max(theoretical_sigma2,1.0e-30)
        endif

        ! Diagnostic: for first trial, check matched filter quality
        if(isnr.eq.1 .and. itrial.eq.1) then
           call diag_branch_metrics(cd,NDMAX,itone)
           call diag_bcjr_quality(cd,NDMAX,itone,snrdb)
        endif

        ! Run turbo decoder
        npts=NDMAX
        dt0=dt0_expected
        snr_est=snrdb
        ntype=-1
        nharderror=-1
        dmin=0.0
        message91=0
        llr_out=0.0

        call cpu_time(t_start)
        call turbo_decode_ft1(cd,npts,f0,dt0,snr_est,llr_out, &
             message91,ntype,nharderror,dmin,0,ncheck_out)
        call cpu_time(t_end)
        t_decode = (t_end - t_start) * 1000.0  ! ms
        t_sum = t_sum + t_decode
        if(t_decode .lt. t_min) t_min = t_decode
        if(t_decode .gt. t_max) t_max = t_decode

        ! Diagnostic: print decode result for first trial of first SNR
        if(isnr.eq.1 .and. itrial.eq.1) then
           write(*,'(a,i3,a,i4)') 'First trial: ntype=',ntype, &
                ' nharderror=',nharderror
           write(*,'(a,5f8.2)') 'First 5 LLRs: ',llr_out(1:5)
        endif

        if(ntype.ge.0) then
           ndec=ndec+1
           if(nharderror.ge.0) nerr_total=nerr_total+nharderror
        endif
     enddo

     rate=real(ndec)/real(ntrials)
     if(ndec.gt.0) then
        avg_nerr=real(nerr_total)/real(ndec)
     else
        avg_nerr=-1.0
     endif
     write(*,'(f6.1,i9,i8,f8.3,f10.1,3f7.1)') snrdb,ndec,ntrials, &
          rate,avg_nerr,t_sum/real(ntrials),t_min,t_max
  enddo

  write(*,*)
  write(*,'(a)') 'Test complete.'

end program ft1_test


subroutine diag_branch_metrics(cd,npts,itone)
!
! Diagnostic: trace the correct trellis path from known transmitted
! symbols, then test matched filter correlations at the correct state.
!
  use cpm_trellis_mod
  use matched_filter_bank_mod
  implicit none
  integer, intent(in) :: npts
  complex, intent(in) :: cd(npts)
  integer, intent(in) :: itone(99)

  integer :: nd, i_n, k, idx, chan_pos, idx_start
  integer :: best_sym_r, best_sym_a, actual_sym
  integer :: j, s_cur
  complex :: corr(0:3)
  real :: metric_r(0:3), metric_a(0:3)
  real :: nsps_down_real
  integer :: ncorrect_r, ncorrect_a
  integer :: trellis_state(0:99)  ! state BEFORE each channel symbol (0-indexed)
  real :: corr_correct_r, corr_correct_a, corr_best_a
  real :: phase_angle

  call init_cpm_trellis()
  call init_matched_filters(8)

  nsps_down_real = 3000.0 / (7.0 * 54.0)

  ! Trace the correct trellis path through all 99 channel symbols
  ! Start at state 1: theta=0, sigma_1=0, sigma_2=0
  trellis_state(0) = 1
  do j = 1, 99
     s_cur = trellis_state(j-1)
     trellis_state(j) = next_state(s_cur, itone(j))
  enddo

  write(*,*)
  write(*,'(a)') 'Branch metric diagnostic (correct trellis path):'
  write(*,'(a)') 'DataSym ChanPos TXsym State  R(corr_tx)  |corr_tx|' &
       //'  Phase(deg)  BestR BestA'

  ncorrect_r = 0
  ncorrect_a = 0

  do nd = 1, 87
     if(nd .le. 43) then
        chan_pos = 3 + nd
        actual_sym = itone(4 + nd)
        ! State before this channel symbol (chan_pos is 0-indexed)
        s_cur = trellis_state(chan_pos)
     else
        chan_pos = 7 + nd
        actual_sym = itone(8 + nd)
        s_cur = trellis_state(chan_pos)
     endif
     idx_start = nint(chan_pos * nsps_down_real) + 1

     ! Compute correlation at the CORRECT state for all 4 input symbols
     do i_n = 0, 3
        corr(i_n) = cmplx(0.0, 0.0)
        do k = 1, 8
           idx = idx_start + k - 1
           if(idx .ge. 1 .and. idx .le. npts) then
              corr(i_n) = corr(i_n) + cd(idx) * conjg(mf_bank(k, s_cur, i_n))
           endif
        enddo
        metric_r(i_n) = real(corr(i_n))
        metric_a(i_n) = abs(corr(i_n))
     enddo

     ! Find best symbol by real and abs at the correct state
     best_sym_r = 0
     best_sym_a = 0
     do i_n = 1, 3
        if(metric_r(i_n) .gt. metric_r(best_sym_r)) best_sym_r = i_n
        if(metric_a(i_n) .gt. metric_a(best_sym_a)) best_sym_a = i_n
     enddo
     if(best_sym_r .eq. actual_sym) ncorrect_r = ncorrect_r + 1
     if(best_sym_a .eq. actual_sym) ncorrect_a = ncorrect_a + 1

     ! Print details for first 20 data symbols
     corr_correct_r = metric_r(actual_sym)
     corr_correct_a = metric_a(actual_sym)
     phase_angle = atan2(aimag(corr(actual_sym)), real(corr(actual_sym))) &
          * 180.0 / 3.14159265

     ! Print all symbols (mark wrong ones with *)
     if(best_sym_a .ne. actual_sym) then
        write(*,'(i5,i8,i6,i6,2f12.4,f10.1,2i6,a)') &
             nd, chan_pos, actual_sym, s_cur, &
             corr_correct_r, corr_correct_a, phase_angle, &
             best_sym_r, best_sym_a, ' ***WRONG***'
     else if(nd .le. 10 .or. nd .ge. 83) then
        write(*,'(i5,i8,i6,i6,2f12.4,f10.1,2i6)') &
             nd, chan_pos, actual_sym, s_cur, &
             corr_correct_r, corr_correct_a, phase_angle, &
             best_sym_r, best_sym_a
     endif

     ! For first data symbol, print sample-by-sample comparison
     if(nd .eq. 1) then
        write(*,*)
        write(*,'(a)') 'Sample-by-sample comparison for DataSym 1:'
        write(*,'(a)') 'k  idx    cd_r      cd_i     mf_r      mf_i    ' &
             //'  cd_phase  mf_phase  diff'
        do k = 1, 8
           idx = idx_start + k - 1
           if(idx .ge. 1 .and. idx .le. npts) then
              write(*,'(i2,i5,4f10.4,3f10.1)') k, idx, &
                   real(cd(idx)), aimag(cd(idx)), &
                   real(mf_bank(k,s_cur,actual_sym)), &
                   aimag(mf_bank(k,s_cur,actual_sym)), &
                   atan2(aimag(cd(idx)),real(cd(idx)))*180.0/3.14159265, &
                   atan2(aimag(mf_bank(k,s_cur,actual_sym)), &
                         real(mf_bank(k,s_cur,actual_sym)))*180.0/3.14159265, &
                   (atan2(aimag(cd(idx)),real(cd(idx))) - &
                    atan2(aimag(mf_bank(k,s_cur,actual_sym)), &
                          real(mf_bank(k,s_cur,actual_sym))))*180.0/3.14159265
           endif
        enddo
     endif
  enddo

  write(*,'(a,i3,a,i3,a)') &
       'At correct state: real=',ncorrect_r,' abs=',ncorrect_a,' of 87'

  ! Print correlation statistics
  block
    real :: corr_abs_sum, corr_abs_min, corr_abs_max
    real :: corr_r_sum, phase_sum_diag
    real :: ca
    integer :: nd2, chan_pos2, s_cur2

    corr_abs_sum = 0.0
    corr_abs_min = 1e30
    corr_abs_max = 0.0
    corr_r_sum = 0.0
    phase_sum_diag = 0.0

    do nd2 = 1, 87
       if(nd2 .le. 43) then
          chan_pos2 = 3 + nd2
          s_cur2 = trellis_state(chan_pos2)
          actual_sym = itone(4 + nd2)
       else
          chan_pos2 = 7 + nd2
          s_cur2 = trellis_state(chan_pos2)
          actual_sym = itone(8 + nd2)
       endif
       idx_start = nint(chan_pos2 * nsps_down_real) + 1
       corr(actual_sym) = cmplx(0.0, 0.0)
       do k = 1, 8
          idx = idx_start + k - 1
          if(idx .ge. 1 .and. idx .le. npts) then
             corr(actual_sym) = corr(actual_sym) + &
                  cd(idx) * conjg(mf_bank(k, s_cur2, actual_sym))
          endif
       enddo
       ca = abs(corr(actual_sym))
       corr_abs_sum = corr_abs_sum + ca
       if(ca .lt. corr_abs_min) corr_abs_min = ca
       if(ca .gt. corr_abs_max) corr_abs_max = ca
       corr_r_sum = corr_r_sum + real(corr(actual_sym))
       phase_sum_diag = phase_sum_diag + &
            atan2(aimag(corr(actual_sym)), real(corr(actual_sym)))
    enddo
    write(*,'(a,f8.4,a,f8.4,a,f8.4)') &
         'Corr |abs|: mean=', corr_abs_sum/87.0, &
         ' min=', corr_abs_min, ' max=', corr_abs_max
    write(*,'(a,f8.4,a,f8.1,a)') &
         'Corr real : mean=', corr_r_sum/87.0, &
         '  mean_phase=', phase_sum_diag/87.0*180.0/3.14159265, ' deg'
  end block

  return
end subroutine diag_branch_metrics


subroutine diag_bcjr_quality(cd, npts, itone, snrdb)
!
! Diagnostic: run the full-frame BCJR on the received signal and check
! the LLR signs against the known transmitted bits.
!
  use cpm_trellis_mod
  use matched_filter_bank_mod
  implicit none
  integer, intent(in) :: npts
  complex, intent(in) :: cd(npts)
  integer, intent(in) :: itone(99)
  real, intent(in)    :: snrdb

  integer, parameter :: NDATA = 87
  integer, parameter :: NBITS = 174
  integer, parameter :: NSYM = 99
  integer, parameter :: NSS = 8

  real    :: branch_metrics_all(NSTATES, 0:3, NSYM)
  real    :: apriori(NBITS), ext_llr(NBITS)
  integer :: tx_bits(NBITS)
  integer :: is_data(NSYM), data_idx_map(NSYM), sync_sym(NSYM)
  integer :: icos_rv0(4)
  complex :: cd_rot(npts)
  complex :: corr_sync, phase_sum
  real    :: phase_est, nsps_down_real, sigma2, snr_lin
  integer :: s_sync, j, k, idx, i, n, tone, nerr_sign

  call init_cpm_trellis()
  call init_matched_filters(NSS)

  ! Set up frame map (same as turbo decoder)
  icos_rv0 = (/0,2,3,1/)
  is_data = 0
  data_idx_map = 0
  sync_sym = -1
  sync_sym(1) = icos_rv0(1)
  sync_sym(2) = icos_rv0(2)
  sync_sym(3) = icos_rv0(3)
  sync_sym(4) = icos_rv0(4)
  do i = 5, 47
     is_data(i) = 1
     data_idx_map(i) = i - 4
  enddo
  sync_sym(48) = icos_rv0(1)
  sync_sym(49) = icos_rv0(2)
  sync_sym(50) = icos_rv0(3)
  sync_sym(51) = icos_rv0(4)
  do i = 52, 95
     is_data(i) = 1
     data_idx_map(i) = i - 8
  enddo
  sync_sym(96) = icos_rv0(1)
  sync_sym(97) = icos_rv0(2)
  sync_sym(98) = icos_rv0(3)
  sync_sym(99) = icos_rv0(4)

  ! Phase estimation (same as turbo decoder)
  nsps_down_real = 3000.0 / (7.0 * 54.0)
  phase_sum = cmplx(0.0, 0.0)
  s_sync = 1
  do j = 1, 4
     idx = nint(real(j-1) * nsps_down_real) + 1
     corr_sync = cmplx(0.0, 0.0)
     do k = 1, NSS
        if(idx+k-1 .ge. 1 .and. idx+k-1 .le. npts) then
           corr_sync = corr_sync + cd(idx+k-1) * &
                conjg(mf_bank(k, s_sync, icos_rv0(j)))
        endif
     enddo
     phase_sum = phase_sum + corr_sync
     s_sync = next_state(s_sync, icos_rv0(j))
  enddo
  phase_est = atan2(aimag(phase_sum), real(phase_sum))
  do i = 1, npts
     cd_rot(i) = cd(i) * cmplx(cos(phase_est), -sin(phase_est))
  enddo

  ! Compute branch metrics for ALL 99 positions
  snr_lin = 10.0**(snrdb / 10.0) * (2500.0 / 126.0)
  sigma2 = 1.0 / max(1.0 + snr_lin, 1.0e-6)
  call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
       branch_metrics_all, NSYM, 0)

  ! Force sync positions
  do n = 1, NSYM
     if(is_data(n) .eq. 0 .and. sync_sym(n) .ge. 0) then
        call force_sync_metrics(branch_metrics_all(:,:,n), sync_sym(n))
     endif
  enddo

  ! Run full-frame BCJR with no a priori
  apriori = 0.0
  call bcjr_cpm(branch_metrics_all, apriori, ext_llr, &
       NSYM, NDATA, is_data, data_idx_map)

  ! Extract interleaved bit values from known tones
  ! Gray code: bit_map(tone, 0)=MSB, bit_map(tone, 1)=LSB
  do n = 1, NDATA
     if(n .le. 43) then
        tone = itone(4 + n)
     else
        tone = itone(8 + n)
     endif
     tx_bits(2*n - 1) = bit_map(tone, 0)
     tx_bits(2*n)     = bit_map(tone, 1)
  enddo

  ! Check LLR sign correctness (positive = bit 1)
  nerr_sign = 0
  do i = 1, NBITS
     if(tx_bits(i) .eq. 1 .and. ext_llr(i) .lt. 0.0) nerr_sign = nerr_sign + 1
     if(tx_bits(i) .eq. 0 .and. ext_llr(i) .gt. 0.0) nerr_sign = nerr_sign + 1
  enddo

  write(*,*)
  write(*,'(a,f6.1,a)') 'BCJR diagnostic at SNR=', snrdb, ' dB:'
  write(*,'(a,f8.2,a)') '  Phase est: ', phase_est * 180.0 / 3.14159265, ' deg'
  write(*,'(a,f10.6)') '  sigma2   : ', sigma2
  write(*,'(a,i4,a,i4,a,f5.1,a)') '  BCJR pass 1 sign errors: ', &
       nerr_sign, ' / ', NBITS, ' (', 100.0*real(nerr_sign)/real(NBITS), '%)'

  ! Now run turbo iterations and track BER improvement
  block
    integer, parameter :: K_TURBO = 15
    real :: apriori_t(NBITS), ext_bcjr_t(NBITS)
    real :: ext_bcjr_dilv_t(NBITS), ext_ldpc_t(NBITS), ext_ldpc_ilv_t(NBITS)
    real :: llr_bp_t(NBITS), zn_bp_t(NBITS)
    integer*1 :: apmask_t(NBITS)
    integer :: k_turbo_iter, nhd_t, iter_t, ncheck_t, nerr_t
    integer :: bp_max_t
    real :: damp_t

    apmask_t = 0
    apriori_t = 0.0

    do k_turbo_iter = 1, K_TURBO
       ! BCJR pass (full frame)
       call bcjr_cpm(branch_metrics_all, apriori_t, ext_bcjr_t, &
            NSYM, NDATA, is_data, data_idx_map)

       ! Count sign errors in BCJR extrinsic
       nerr_t = 0
       do i = 1, NBITS
          if(tx_bits(i) .eq. 1 .and. ext_bcjr_t(i) .lt. 0.0) nerr_t = nerr_t + 1
          if(tx_bits(i) .eq. 0 .and. ext_bcjr_t(i) .gt. 0.0) nerr_t = nerr_t + 1
       enddo

       ! Deinterleave
       call ft1_interleave_real(ext_bcjr_t, ext_bcjr_dilv_t, -1)

       ! Progressive BP iteration limit (matching turbo_decode_ft1)
       if(k_turbo_iter .le. 2) then
          bp_max_t = 5
       else if(k_turbo_iter .le. 5) then
          bp_max_t = 15
       else
          bp_max_t = 30
       endif

       ! BP decode
       llr_bp_t = ext_bcjr_dilv_t
       call bpdecode174_91_beliefs(llr_bp_t, apmask_t, bp_max_t, zn_bp_t, &
            nhd_t, iter_t, ncheck_t)

       write(*,'(a,i2,a,i4,a,i4,a,i4,a,f6.2)') &
            '  Turbo iter ', k_turbo_iter, &
            ': BCJR_errs=', nerr_t, &
            ' BP_ncheck=', ncheck_t, &
            ' BP_nhd=', nhd_t, &
            ' LLR_mean=', sum(abs(ext_bcjr_t))/NBITS

       if(ncheck_t .eq. 0 .and. nhd_t .ge. 0) then
          write(*,'(a,i2,a)') '  -> Converged at turbo iter ', k_turbo_iter, '!'
          exit
       endif

       ! Compute LDPC extrinsic and feed back with ramped damping
       do i = 1, NBITS
          ext_ldpc_t(i) = zn_bp_t(i) - ext_bcjr_dilv_t(i)
          ext_ldpc_t(i) = max(-30.0, min(30.0, ext_ldpc_t(i)))
       enddo
       call ft1_interleave_real(ext_ldpc_t, ext_ldpc_ilv_t, 1)

       if(k_turbo_iter .le. 2) then
          damp_t = 0.2
       else if(k_turbo_iter .le. 5) then
          damp_t = 0.4
       else
          damp_t = 0.6
       endif
       apriori_t = damp_t * ext_ldpc_ilv_t
    enddo
  end block

  return
end subroutine diag_bcjr_quality
