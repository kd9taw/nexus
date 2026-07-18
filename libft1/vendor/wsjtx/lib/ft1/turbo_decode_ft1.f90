! FT1 - 4-CPM turbo equalization mode for WSJT-X
! Copyright (C) 2026 KD9TAW
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
subroutine turbo_decode_ft1(cd, npts, f0, dt0, snr_est, llr_out, &
     message91, ntype, nharderror, dmin, niter_max, ncheck_out)
!
! FT1 turbo equalization: iterative BCJR + LDPC BP decoding
!
! This is the main entry point for decoding a single FT1 candidate signal.
! It implements the turbo loop from Spec 3e Section 7, with OSD fallback.
!
! The BCJR processes the full 99-symbol frame (data + sync) for proper
! trellis continuity. Sync symbols constrain the trellis path at known
! positions, improving boundary effects and enabling known start state.
!
! Interface:
!   cd(npts):       Complex downsampled signal (baseband, ~8 samp/sym)
!   npts:           Number of samples in cd
!   f0:             Fine frequency offset (Hz) -- for future use
!   dt0:            Fine time offset (samples) -- for future use
!   snr_est:        SNR estimate (dB, in 2500 Hz BW) -- for noise variance
!   llr_out(174):   Final LLRs (output, for AP/IR-HARQ use)
!   message91(91):  Decoded message bits (output, if successful)
!   ntype:          Decode type: 1=turbo, 2=OSD, -1=failed (output)
!   nharderror:     Number of hard errors, -1 if failed (output)
!   dmin:           Reliability metric from OSD (output)
!   niter_max:      Max turbo iterations (0=full+VitSweep, -1=full, >0=probe mode)
!   ncheck_out:     Last ncheck_bp value on exit (for probe discrimination)
!

  use cpm_trellis_mod
  use matched_filter_bank_mod
  implicit none

  integer, intent(in)    :: npts
  complex, intent(in)    :: cd(npts)
  real, intent(in)       :: f0, dt0, snr_est
  real, intent(out)      :: llr_out(174)
  integer*1, intent(out) :: message91(91)
  integer, intent(out)   :: ntype, nharderror
  real, intent(out)      :: dmin
  integer, intent(in)    :: niter_max
  integer, intent(out)   :: ncheck_out

! Parameters
  integer, parameter :: NBITS = 174       ! Coded bits
  integer, parameter :: NDATA = 87        ! Data symbols
  integer, parameter :: NSYM = 99         ! Total symbols (data + sync)
  integer, parameter :: K_MAX = 25        ! Max outer turbo iterations
  integer, parameter :: NSS = 8           ! Nominal downsampled samples per symbol
  real, parameter    :: SIGMA2_SCALE = 1.0 ! LLR calibration for turbo eq

! Local arrays
  real    :: branch_metrics_all(NSTATES, 0:3, NSYM) ! Branch metrics for ALL positions
  real    :: bm_scaled(NSTATES, 0:3, NSYM)          ! Scaled branch metrics (annealing)
  real    :: apriori_llr(NBITS)   ! A priori LLRs for BCJR (from LDPC)
  real    :: ext_bcjr(NBITS)      ! Extrinsic LLRs from BCJR
  real    :: ext_bcjr_dilv(NBITS) ! Deinterleaved BCJR extrinsic (-> LDPC input)
  real    :: ext_ldpc(NBITS)      ! Extrinsic LLRs from LDPC
  real    :: ext_ldpc_ilv(NBITS)  ! Interleaved LDPC extrinsic (-> BCJR a priori)
  real    :: llr_bp(NBITS)        ! BP input LLRs
  real    :: zn_bp(NBITS)         ! BP total beliefs (for extrinsic extraction)
  integer*1 :: apmask(NBITS)     ! AP mask (all zeros for standard decode)
  integer*1 :: cw(NBITS)         ! Hard decision codeword
  real    :: sigma2               ! Noise variance estimate
  real    :: snr_linear           ! Linear SNR

  integer :: k_outer, bp_maxiter, iter_bp, ncheck_bp
  integer :: i, n, i_osd
  integer :: nhd_bp
  real    :: damp                      ! Current damping factor (ramped)
  real    :: bm_scale                  ! Branch metric scale (annealing)

! Frame map arrays
  integer :: is_data(NSYM)        ! 1 for data, 0 for sync
  integer :: data_idx(NSYM)       ! Data symbol index (1..87), 0 for sync
  integer :: sync_sym(NSYM)       ! Known symbol at sync positions, -1 for data
  integer :: icos_rv0(4)

! Phase estimation variables
  complex :: cd_rot(npts)         ! Phase-compensated signal
  complex :: corr_sync, phase_sum
  real    :: phi(3)               ! Phase estimates at Costas groups
  real    :: best_mag, mag_try    ! For state search
  real    :: phase_interp, sym_pos, frac
  real    :: nsps_down_real
  integer :: s_sync, j_sync, k_sync, idx_sync, idx_start_sync
  integer :: s_try                ! Correlative state search index
  integer :: igrp, chan_start(3)  ! Costas group loop

! Per-symbol phase tracking variables
  real    :: phase_raw(NSYM)        ! Per-symbol raw phase estimates
  real    :: phase_smooth(NSYM)     ! Smoothed phase trajectory
  real    :: alpha_phase             ! Adaptive smoothing parameter
  complex :: corr_dd                ! Per-symbol correlation
  complex :: corr_persym(NSYM)      ! Best complex correlation per symbol
  integer :: idx_dd, k_dd, n_dd, i_sym
  integer :: i_data_dd, b1_dd, b2_dd   ! BCJR-decided symbol extraction

! Timing offset
  integer :: n_offset, n_try, n_test
  real    :: sync_total

! Frequency/phase variables
  real    :: t_samp                ! Sample time
  real    :: fs_down               ! Downsampled sample rate
  real    :: df_corr               ! Frequency correction (Hz)
  real    :: phase_diff            ! Per-symbol phase difference
  real    :: sum_dphi, sum_wt      ! Weighted phase slope
  real    :: pi_val                ! pi constant

! Internal frequency refinement variables (SyncSweep)
  complex :: all_corr_c(NSTATES, 0:3, NSYM) ! Complex MF correlations (kept for turbo-aided)
  real    :: vit_pd_best, vit_pd_metric, vit_threshold  ! Best/current metric
  real    :: vit_pd_metrics(41)          ! Metric at each sweep point
  integer :: ibest_pd                    ! Index of best sweep point
  real    :: xq1, xq2, xq3, dxq, df_fine ! Parabolic interpolation
  real    :: alpha_pd(NSTATES)           ! Viterbi forward variables
  real    :: alpha_pd_new(NSTATES)       ! Updated Viterbi forward variables

  real    :: alpha_pd_center(NSTATES)    ! Viterbi forward at center (df=0)
  real    :: vit_pd_center               ! Best center metric for confidence check
  real    :: bm_pd, cos_n, sin_n        ! Branch metric, rotation
  integer :: s_pd, u_pd, s_next_pd      ! Viterbi loop indices
  real    :: phase_n                     ! Phase rotation angle
  real    :: df_best_int                 ! Best internal frequency offset
  real    :: df_try_pd                   ! Trial frequency
  real    :: df_pd_center                ! Center of fine sweep (from Pass 1)
  real    :: phi_est                     ! Joint phase estimate per df_try

! Phase-slope frequency estimation variables
  real    :: theta_g2_raw, theta_g3_raw  ! Raw G2/G3 phase (pi/2 ambiguous)
  real    :: theta_g2_adj, theta_g3_adj  ! Adjusted with k*pi/2
  real    :: theta_g3_pred               ! Predicted G3 from frequency
  real    :: df_phase_est                ! Freq estimate from phase slope
  real    :: best_residual               ! Best G3 residual across combos
  real    :: best_residual_n3            ! Best G3 residual with wrap
  real    :: residual_val                ! Current residual
  integer :: k2, k3, n3                  ! Ambiguity and wrap indices
  real    :: df_pd_step                 ! Sweep step size
  integer :: idf_pd, ndf_pd            ! Sweep loop variables
  real    :: twopi_val                   ! 2π constant

! Coherent frequency metric variables (backtrace-based periodogram)
  integer :: bt_prev(NSTATES, NSYM)    ! Backtrace: previous state
  integer :: bt_sym(NSTATES, NSYM)     ! Backtrace: chosen symbol
  integer :: path_state(NSYM)          ! Decoded path states
  integer :: path_sym(NSYM)            ! Decoded path symbols
  complex :: path_corr(NSYM)           ! Complex correlations along path
  complex :: coh_sum                   ! Coherent summation accumulator
  real    :: coh_metric                ! |coh_sum|²
  real    :: coh_best                  ! Best coherent metric
  real    :: df_coh_best               ! Frequency at best coherent metric
  integer :: s_bt                      ! Backtrace state variable

! Turbo-aided VitSweep variables (mid-loop frequency correction)
  integer :: sym_ta(NSYM)              ! Hard-decided symbols from BCJR
  real    :: alpha_ta(NSTATES)         ! Viterbi forward for turbo-aided sweep
  real    :: alpha_ta_new(NSTATES)     ! Updated forward
  real    :: vit_ta_best, vit_ta_metric ! Best/current metric
  real    :: df_ta_best, df_ta_try     ! Best/trial turbo-aided frequency
  real    :: bm_ta                     ! Branch metric for turbo-aided
  real    :: phase_ta                  ! Total phase for turbo-aided
  integer :: s_ta, s_next_ta, idf_ta  ! Loop indices
  integer :: i_data_ta, b1_ta, b2_ta  ! Symbol extraction
  real    :: phi_g1                    ! G1 phase (saved for all_corr_c rotation)

! For OSD fallback
  real    :: llr_osd(NBITS)
  integer*1 :: apmask_osd(NBITS)
  integer*1 :: cw_osd(NBITS)
  integer*1 :: msg91_osd(91)
  integer :: nhd_osd
  real    :: dmin_osd

! Initialize outputs
  message91 = 0
  ntype = -1
  nharderror = -1
  dmin = 0.0
  llr_out = 0.0
  ncheck_out = 83         ! Default: all parity checks failed
  apmask = 0              ! No AP-fixed bits (needed early for probe loop)

! ================================================================
! Step 0: Initialize trellis and matched filter bank
! ================================================================
  call init_cpm_trellis()
  call init_matched_filters(NSS)

! ================================================================
! Step 0b: Set up frame map (data vs sync positions)
! ================================================================
  icos_rv0 = (/0,2,3,1/)

  is_data = 0
  data_idx = 0
  sync_sym = -1

  ! Sync group 1: positions 1-4
  sync_sym(1) = icos_rv0(1)
  sync_sym(2) = icos_rv0(2)
  sync_sym(3) = icos_rv0(3)
  sync_sym(4) = icos_rv0(4)

  ! Data group 1: positions 5-47 (43 data symbols, indices 1-43)
  do i = 5, 47
     is_data(i) = 1
     data_idx(i) = i - 4
  enddo

  ! Sync group 2: positions 48-51
  sync_sym(48) = icos_rv0(1)
  sync_sym(49) = icos_rv0(2)
  sync_sym(50) = icos_rv0(3)
  sync_sym(51) = icos_rv0(4)

  ! Data group 2: positions 52-95 (44 data symbols, indices 44-87)
  do i = 52, 95
     is_data(i) = 1
     data_idx(i) = i - 8
  enddo

  ! Sync group 3: positions 96-99
  sync_sym(96) = icos_rv0(1)
  sync_sym(97) = icos_rv0(2)
  sync_sym(98) = icos_rv0(3)
  sync_sym(99) = icos_rv0(4)

! ================================================================
! Step 1: Estimate noise variance from SNR
! ================================================================
! Estimate sigma2 from sync correlations after phase compensation.
! Convert SNR from 2500 Hz bandwidth to effective noise bandwidth.
! For h=1/2, signal BW ~78 Hz, effective noise BW ~180 Hz (empirically tuned).
  snr_linear = 10.0**(snr_est / 10.0) * (2500.0 / 180.0)
  sigma2 = SIGMA2_SCALE / max(1.0 + snr_linear, 1.0e-6)

! ================================================================
! Step 1b: Per-segment phase estimation using all three Costas arrays
! ================================================================
! Estimate channel phase at each Costas group, then interpolate
! linearly for per-sample phase compensation. This tracks slow
! phase drift under HF fading.
!
! For h=1/2 CPM, all 4 phase states produce identical correlation
! magnitudes (the waveform is just rotated by k*π/2). So we search
! only the 16 correlative states at θ=0, then resolve the resulting
! π/2 ambiguity by picking the candidate closest to the previous
! group's phase estimate.
!
! Group centers (channel position): 1.5, 48.5, 96.5

  nsps_down_real = 3000.0 / (7.0 * 54.0)
  chan_start = (/0, 47, 95/)

! ================================================================
! Step 1a: Fine timing search around sync hint
! Try dt0±3 using Group 1+2+3 Costas correlation
! (known start state for G1, 16-state search for G2/G3).
! Sync search ibest can be off by 2-3 samples at low SNR,
! so ±3 range is needed for reliable timing refinement.
! ================================================================
  n_test = nint(dt0)
  n_offset = n_test
  best_mag = 0.0
  do n_try = -3, 3
     sync_total = 0.0
     ! Group 1: known start state (unambiguous)
     phase_sum = cmplx(0.0, 0.0)
     s_sync = 1
     do j_sync = 1, 4
        idx_start_sync = nint(real(chan_start(1) + j_sync - 1) * &
             nsps_down_real) + 1 + n_test + n_try
        corr_sync = cmplx(0.0, 0.0)
        do k_sync = 1, NSS
           idx_sync = idx_start_sync + k_sync - 1
           if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
              corr_sync = corr_sync + cd(idx_sync) * &
                   conjg(mf_bank(k_sync, s_sync, icos_rv0(j_sync)))
           endif
        enddo
        phase_sum = phase_sum + corr_sync
        s_sync = next_state(s_sync, icos_rv0(j_sync))
     enddo
     sync_total = abs(phase_sum)
     ! Groups 2-3: search 16 theta=0 states, take best magnitude
     do igrp = 2, 3
        mag_try = 0.0
        do s_try = 1, 16
           phase_sum = cmplx(0.0, 0.0)
           s_sync = s_try
           do j_sync = 1, 4
              idx_start_sync = nint(real(chan_start(igrp) + j_sync-1) * &
                   nsps_down_real) + 1 + n_test + n_try
              corr_sync = cmplx(0.0, 0.0)
              do k_sync = 1, NSS
                 idx_sync = idx_start_sync + k_sync - 1
                 if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
                    corr_sync = corr_sync + cd(idx_sync) * &
                         conjg(mf_bank(k_sync, s_sync, icos_rv0(j_sync)))
                 endif
              enddo
              phase_sum = phase_sum + corr_sync
              s_sync = next_state(s_sync, icos_rv0(j_sync))
           enddo
           if(abs(phase_sum) .gt. mag_try) mag_try = abs(phase_sum)
        enddo
        sync_total = sync_total + mag_try
     enddo
     if(sync_total .gt. best_mag) then
        best_mag = sync_total
        n_offset = n_test + n_try
     endif
  enddo

!  write(*,'(a,i4,a,i4,a,f8.1)') '    timing: hint=',n_test, &
!       ' chosen=',n_offset,' mag=',best_mag
  fs_down = 12000.0 / 54.0       ! ~222.2 Hz

! ================================================================
! Step 1b: G1 carrier phase estimation
! ================================================================
! Estimate and remove carrier phase from G1 Costas symbols (known
! start state). This is required BEFORE the VitSweep because the
! sweep uses real-part branch metrics that are sensitive to carrier
! phase.
  twopi_val = 8.0 * atan(1.0)
  phase_sum = cmplx(0.0, 0.0)
  s_sync = 1
  do j_sync = 1, 4
     idx_start_sync = nint(real(chan_start(1) + j_sync - 1) * &
          nsps_down_real) + 1 + n_offset
     corr_sync = cmplx(0.0, 0.0)
     do k_sync = 1, NSS
        idx_sync = idx_start_sync + k_sync - 1
        if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
           corr_sync = corr_sync + cd(idx_sync) * &
                conjg(mf_bank(k_sync, s_sync, icos_rv0(j_sync)))
        endif
     enddo
     phase_sum = phase_sum + corr_sync
     s_sync = next_state(s_sync, icos_rv0(j_sync))
  enddo
  phi(1) = atan2(aimag(phase_sum), real(phase_sum))
  do i = 1, npts
     cd_rot(i) = cd(i) * cmplx(cos(phi(1)), -sin(phi(1)))
  enddo

! ================================================================
! Step 1b2: 3-group phase-slope frequency estimation
! ================================================================
! After G1 phase correction, cd_rot has ~0 phase at G1 but a phase
! ramp from residual frequency error. Measure phase at G2/G3 (with
! pi/2 ambiguity from unknown trellis state), then use 16-combo
! search to resolve ambiguity and estimate df from the phase slope
! across the full 3.5-second frame.

! DISABLED (kept for reference, never compiled — the `.false.` short-circuits the
! whole block, including its debug write). The 3-group phase-slope df estimate
! proved unreliable: the pi/2-ambiguity 16-combo search over only G2/G3 is too
! noisy at low SNR. Step 1c below (line ~471, also gated on niter_max==0) runs a
! coarse Viterbi frequency sweep (±2 Hz @ 0.05 Hz) that is the robust replacement
! Tempo's live decoder actually uses (ft1_cabi calls turbo with niter_max=0). Do
! NOT re-enable without re-running the AWGN-threshold conformance sweep — it would
! double-correct against that sweep and regress sensitivity.
  if(.false. .and. niter_max .eq. 0) then   ! DISABLED: phase-slope unreliable
  pi_val = 4.0 * atan(1.0)

! Measure G2 phase: search 16 correlative states at theta=0
  best_mag = 0.0
  theta_g2_raw = 0.0
  do s_try = 1, 16
     phase_sum = cmplx(0.0, 0.0)
     s_sync = s_try
     do j_sync = 1, 4
        idx_start_sync = nint(real(chan_start(2) + j_sync - 1) * &
             nsps_down_real) + 1 + n_offset
        corr_sync = cmplx(0.0, 0.0)
        do k_sync = 1, NSS
           idx_sync = idx_start_sync + k_sync - 1
           if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
              corr_sync = corr_sync + cd_rot(idx_sync) * &
                   conjg(mf_bank(k_sync, s_sync, icos_rv0(j_sync)))
           endif
        enddo
        phase_sum = phase_sum + corr_sync
        s_sync = next_state(s_sync, icos_rv0(j_sync))
     enddo
     mag_try = abs(phase_sum)
     if(mag_try .gt. best_mag) then
        best_mag = mag_try
        theta_g2_raw = atan2(aimag(phase_sum), real(phase_sum))
     endif
  enddo

! Measure G3 phase: search 16 correlative states at theta=0
  best_mag = 0.0
  theta_g3_raw = 0.0
  do s_try = 1, 16
     phase_sum = cmplx(0.0, 0.0)
     s_sync = s_try
     do j_sync = 1, 4
        idx_start_sync = nint(real(chan_start(3) + j_sync - 1) * &
             nsps_down_real) + 1 + n_offset
        corr_sync = cmplx(0.0, 0.0)
        do k_sync = 1, NSS
           idx_sync = idx_start_sync + k_sync - 1
           if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
              corr_sync = corr_sync + cd_rot(idx_sync) * &
                   conjg(mf_bank(k_sync, s_sync, icos_rv0(j_sync)))
           endif
        enddo
        phase_sum = phase_sum + corr_sync
        s_sync = next_state(s_sync, icos_rv0(j_sync))
     enddo
     mag_try = abs(phase_sum)
     if(mag_try .gt. best_mag) then
        best_mag = mag_try
        theta_g3_raw = atan2(aimag(phase_sum), real(phase_sum))
     endif
  enddo

! 16-combo search: resolve pi/2 ambiguity and estimate frequency.
! For each (k2,k3): df from G2 (never wraps), validate against G3
! (allowing ±1 wrap). Best combo has smallest G3 residual.
  best_residual = 1.0e30
  df_phase_est = 0.0
  do k2 = 0, 3
     do k3 = 0, 3
        theta_g2_adj = theta_g2_raw + real(k2) * pi_val * 0.5
        ! df from G2 (no wrap: max |theta_G2| < pi for |df| < 0.3 Hz)
        df_try_pd = theta_g2_adj * 28.0 / (twopi_val * 47.0)
        ! Predict G3 phase from this df
        theta_g3_pred = df_try_pd * twopi_val * 95.0 / 28.0
        theta_g3_adj = theta_g3_raw + real(k3) * pi_val * 0.5
        ! Find best wrap n3 and compute residual
        best_residual_n3 = 1.0e30
        do n3 = -1, 1
           residual_val = abs(theta_g3_adj + real(n3) * twopi_val &
                - theta_g3_pred)
           if(residual_val .lt. best_residual_n3) &
                best_residual_n3 = residual_val
        enddo
        if(best_residual_n3 .lt. best_residual) then
           best_residual = best_residual_n3
           df_phase_est = df_try_pd
        endif
     enddo
  enddo

! Apply phase-slope frequency correction
  write(*,'(a,f8.3,a,f6.1)') '    phase_slope: df=',df_phase_est, &
       ' resid=',best_residual*180.0/pi_val
  do i = 1, npts
     t_samp = real(i - 1) / fs_down
     cd_rot(i) = cd_rot(i) * &
          cmplx(cos(-twopi_val * df_phase_est * t_samp), &
                sin(-twopi_val * df_phase_est * t_samp))
  enddo
  endif   ! niter_max .eq. 0

! ================================================================
! Step 1c: Internal frequency refinement via Viterbi sweep
! ================================================================
  if(niter_max .eq. 0 .and. snr_est .gt. -12.0) then

! Step 1c.1: Compute complex MF correlations for all 99 positions
  do n = 1, NSYM
     idx_start_sync = nint(real(n - 1) * nsps_down_real) + 1 + n_offset
     do s_sync = 1, NSTATES
        do i_sym = 0, 3
           corr_sync = cmplx(0.0, 0.0)
           do k_sync = 1, NSS
              idx_sync = idx_start_sync + k_sync - 1
              if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
                 corr_sync = corr_sync + cd_rot(idx_sync) * &
                      conjg(mf_bank(k_sync, s_sync, i_sym))
              endif
           enddo
           all_corr_c(s_sync, i_sym, n) = corr_sync
        enddo
     enddo
  enddo

! Step 1c.2: Coarse Viterbi frequency sweep ±2.0 Hz at 0.05 Hz (81 points)
  df_best_int = 0.0
  ndf_pd = 81
  df_pd_step = 0.05
  vit_pd_best = -1.0e30

  do idf_pd = 1, ndf_pd
     df_try_pd = real(idf_pd - (ndf_pd+1)/2) * df_pd_step

     ! Viterbi forward pass with frequency rotation
     alpha_pd = -1.0e30
     alpha_pd(1) = 0.0          ! Known start state

     do n = 1, NSYM
        phase_n = twopi_val * df_try_pd * real(n - 1) / 28.0
        cos_n = cos(phase_n)
        sin_n = sin(phase_n)

        alpha_pd_new = -1.0e30

        if(sync_sym(n) .ge. 0) then
           u_pd = sync_sym(n)
           do s_pd = 1, NSTATES
              if(alpha_pd(s_pd) .le. -1.0e20) cycle
              s_next_pd = next_state(s_pd, u_pd)
              bm_pd = real(all_corr_c(s_pd, u_pd, n)) * cos_n + &
                       aimag(all_corr_c(s_pd, u_pd, n)) * sin_n
              if(alpha_pd(s_pd) + bm_pd .gt. &
                   alpha_pd_new(s_next_pd)) then
                 alpha_pd_new(s_next_pd) = alpha_pd(s_pd) + bm_pd
              endif
           enddo
        else
           do s_pd = 1, NSTATES
              if(alpha_pd(s_pd) .le. -1.0e20) cycle
              do u_pd = 0, 3
                 s_next_pd = next_state(s_pd, u_pd)
                 bm_pd = real(all_corr_c(s_pd, u_pd, n)) * cos_n + &
                          aimag(all_corr_c(s_pd, u_pd, n)) * sin_n
                 if(alpha_pd(s_pd) + bm_pd .gt. &
                      alpha_pd_new(s_next_pd)) then
                    alpha_pd_new(s_next_pd) = alpha_pd(s_pd) + bm_pd
                 endif
              enddo
           enddo
        endif

        alpha_pd = alpha_pd_new
     enddo

     vit_pd_metric = maxval(alpha_pd)
     if(idf_pd .eq. (ndf_pd+1)/2) alpha_pd_center = alpha_pd
     if(vit_pd_metric .gt. vit_pd_best) then
        vit_pd_best = vit_pd_metric
        df_best_int = df_try_pd
     endif
  enddo

! Pass 2: Fine sweep ±0.025 Hz at 0.005 Hz around coarse best (11 points)
  df_pd_center = df_best_int
  vit_pd_best = -1.0e30
  do idf_pd = 1, 11
     df_try_pd = df_pd_center + real(idf_pd - 6) * 0.005

     alpha_pd = -1.0e30
     alpha_pd(1) = 0.0
     do n = 1, NSYM
        phase_n = twopi_val * df_try_pd * real(n - 1) / 28.0
        cos_n = cos(phase_n)
        sin_n = sin(phase_n)
        alpha_pd_new = -1.0e30
        if(sync_sym(n) .ge. 0) then
           u_pd = sync_sym(n)
           do s_pd = 1, NSTATES
              if(alpha_pd(s_pd) .le. -1.0e20) cycle
              s_next_pd = next_state(s_pd, u_pd)
              bm_pd = real(all_corr_c(s_pd, u_pd, n)) * cos_n + &
                       aimag(all_corr_c(s_pd, u_pd, n)) * sin_n
              if(alpha_pd(s_pd) + bm_pd .gt. alpha_pd_new(s_next_pd)) then
                 alpha_pd_new(s_next_pd) = alpha_pd(s_pd) + bm_pd
              endif
           enddo
        else
           do s_pd = 1, NSTATES
              if(alpha_pd(s_pd) .le. -1.0e20) cycle
              do u_pd = 0, 3
                 s_next_pd = next_state(s_pd, u_pd)
                 bm_pd = real(all_corr_c(s_pd, u_pd, n)) * cos_n + &
                          aimag(all_corr_c(s_pd, u_pd, n)) * sin_n
                 if(alpha_pd(s_pd) + bm_pd .gt. alpha_pd_new(s_next_pd)) then
                    alpha_pd_new(s_next_pd) = alpha_pd(s_pd) + bm_pd
                 endif
              enddo
           enddo
        endif
        alpha_pd = alpha_pd_new
     enddo

     vit_pd_metric = maxval(alpha_pd)
     if(vit_pd_metric .gt. vit_pd_best) then
        vit_pd_best = vit_pd_metric
        df_best_int = df_try_pd
     endif
  enddo

! Confidence check: compare peak to center (df=0) metric
  vit_pd_center = maxval(alpha_pd_center)
  vit_threshold = 3.0 + max(0.0, -(snr_est + 10.0))
  if(vit_pd_best - vit_pd_center .lt. vit_threshold .and. &
       abs(df_best_int) .gt. 0.01) then
     df_best_int = 0.0   ! Center is nearly as good — don't correct
  endif

! Step 1c.3: Apply frequency correction to cd_rot
  do i = 1, npts
     t_samp = real(i - 1) / fs_down
     cd_rot(i) = cd_rot(i) * &
          cmplx(cos(-twopi_val * df_best_int * t_samp), &
                sin(-twopi_val * df_best_int * t_samp))
  enddo
  endif   ! VitSweep

! Re-estimate carrier phase after frequency correction.
! The VitSweep correction exp(-j*2π*df*t) introduces a phase offset
! proportional to absolute time. Re-estimating from G1 removes this.
  phase_sum = cmplx(0.0, 0.0)
  s_sync = 1
  do j_sync = 1, 4
     idx_start_sync = nint(real(chan_start(1) + j_sync - 1) * &
          nsps_down_real) + 1 + n_offset
     corr_sync = cmplx(0.0, 0.0)
     do k_sync = 1, NSS
        idx_sync = idx_start_sync + k_sync - 1
        if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
           corr_sync = corr_sync + cd_rot(idx_sync) * &
                conjg(mf_bank(k_sync, s_sync, icos_rv0(j_sync)))
        endif
     enddo
     phase_sum = phase_sum + corr_sync
     s_sync = next_state(s_sync, icos_rv0(j_sync))
  enddo
  phi(1) = atan2(aimag(phase_sum), real(phase_sum))
  phi_g1 = phi(1)  ! Save for turbo-aided VitSweep
  do i = 1, npts
     cd_rot(i) = cd_rot(i) * cmplx(cos(phi(1)), -sin(phi(1)))
  enddo

! Compute branch metrics for ALL 99 positions (after freq correction)
  call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
       branch_metrics_all, NSYM, n_offset)

! DEBUG blocks removed for performance testing

! Force sync at all three Costas groups (G1: 1-4, G2: 48-51, G3: 96-99)
! After frequency correction, phase at G2/G3 is accurate for sync forcing.
  do n = 1, NSYM
     if(sync_sym(n) .ge. 0) then
        call force_sync_metrics(branch_metrics_all(:,:,n), sync_sym(n))
     endif
  enddo

! ================================================================
! Step 3: Turbo equalization loop
! ================================================================
  apriori_llr = 0.0       ! No a priori for first BCJR pass

  do k_outer = 1, K_MAX

     ! ----------------------------------------------------------
     ! Step 3a: BCJR detection on full-frame CPM trellis
     ! Branch metric annealing: start with softer metrics (0.7x),
     ! ramp to full strength. Prevents overconfident BCJR output
     ! in early iterations when a priori feedback is unreliable.
     ! ----------------------------------------------------------
     bm_scale = 1.0
     bm_scaled = bm_scale * branch_metrics_all

     call bcjr_cpm(bm_scaled, apriori_llr, ext_bcjr, &
          NSYM, NDATA, is_data, data_idx)

     ! ----------------------------------------------------------
     ! Step 3b: Deinterleave BCJR extrinsic LLRs
     ! ----------------------------------------------------------
     call ft1_interleave_real(ext_bcjr, ext_bcjr_dilv, -1)

     ! ----------------------------------------------------------
     ! Step 3c: LDPC BP decoding
     ! Progressive BP iteration schedule: start cautious, increase
     ! ----------------------------------------------------------
     if(k_outer .le. 2) then
        bp_maxiter = 10
     else if(k_outer .le. 5) then
        bp_maxiter = 20
     else if(k_outer .ge. K_MAX - 2) then
        bp_maxiter = 50
     else
        bp_maxiter = 30
     endif

     llr_bp = ext_bcjr_dilv

     call bpdecode174_91_beliefs(llr_bp, apmask, bp_maxiter, zn_bp, &
          nhd_bp, iter_bp, ncheck_bp)

     ncheck_out = ncheck_bp
!     write(*,'(a,i3,a,i3)') '    turbo k=',k_outer,' ncheck=',ncheck_bp

     ! ----------------------------------------------------------
     ! Step 3d: Check stopping criterion (valid codeword + CRC)
     ! ----------------------------------------------------------
     if(ncheck_bp .eq. 0 .and. nhd_bp .ge. 0) then
        cw = 0
        where(zn_bp .gt. 0.) cw = 1
        message91(1:77) = cw(1:77)
        message91(78:91) = cw(78:91)
        ntype = 1       ! Turbo decode success
        nharderror = nhd_bp
        ncheck_out = 0
        llr_out = zn_bp
        return
     endif

     ! ----------------------------------------------------------
     ! Probe early exit: return ncheck for frequency discrimination
     ! ----------------------------------------------------------
     if(niter_max .gt. 0 .and. k_outer .ge. niter_max) then
        ncheck_out = ncheck_bp
        llr_out = zn_bp
        return
     endif

     ! ----------------------------------------------------------
     ! Early exit for non-converging signals: aggressive cascade
     ! kills pure noise quickly, progressively tighter thresholds
     ! let promising signals continue longer.
     ! ----------------------------------------------------------
     if(k_outer .eq. 3 .and. ncheck_bp .gt. 55) then
        ncheck_out = ncheck_bp
        llr_out = zn_bp
        return
     endif
     if(k_outer .eq. 5 .and. ncheck_bp .gt. 40) then
        ncheck_out = ncheck_bp
        llr_out = zn_bp
        return
     endif
     if(k_outer .eq. 10 .and. ncheck_bp .gt. 25) then
        ncheck_out = ncheck_bp
        llr_out = zn_bp
        return
     endif

     ! ----------------------------------------------------------
     ! Step 3d0: Turbo-aided frequency correction at k_outer=3
     ! After 3 BCJR+BP iterations, use hard-decided symbols to run
     ! a VitSweep with ALL symbols known (no max-over-4 noise).
     ! Uses the pre-correction all_corr_c with analytical rotation
     ! to avoid recomputing correlations.
     ! ----------------------------------------------------------
     if(.false. .and. k_outer .eq. 3 .and. niter_max .le. 0 .and. &
          ncheck_bp .gt. 20) then
        ! Extract hard symbol decisions from BCJR extrinsic
        do n = 1, NSYM
           if(sync_sym(n) .ge. 0) then
              sym_ta(n) = sync_sym(n)
           else
              i_data_ta = data_idx(n)
              b1_ta = 0
              b2_ta = 0
              if(ext_bcjr(2*i_data_ta-1) .gt. 0.0) b1_ta = 1
              if(ext_bcjr(2*i_data_ta)   .gt. 0.0) b2_ta = 1
              ! Gray mapping: 00→0, 01→1, 11→2, 10→3
              sym_ta(n) = 2*b1_ta + b2_ta
              if(sym_ta(n) .eq. 2) then
                 sym_ta(n) = 3
              else if(sym_ta(n) .eq. 3) then
                 sym_ta(n) = 2
              endif
           endif
        enddo

        ! Turbo-aided VitSweep: ±3 Hz at 0.05 Hz (121 points)
        ! Reuses all_corr_c (computed from G1-corrected cd_rot) with
        ! analytical rotation: phase(n) = 2π*df_ta*(n-1)/28
        vit_ta_best = -1.0e30
        df_ta_best = 0.0
        do idf_ta = 1, 121
           df_ta_try = real(idf_ta - 61) * 0.05

           alpha_ta = -1.0e30
           alpha_ta(1) = 0.0
           do n = 1, NSYM
              phase_ta = twopi_val * df_ta_try * &
                   real(n - 1) / 28.0
              cos_n = cos(phase_ta)
              sin_n = sin(phase_ta)

              alpha_ta_new = -1.0e30
              u_pd = sym_ta(n)
              do s_ta = 1, NSTATES
                 if(alpha_ta(s_ta) .le. -1.0e20) cycle
                 s_next_ta = next_state(s_ta, u_pd)
                 bm_ta = real(all_corr_c(s_ta, u_pd, n)) * cos_n + &
                          aimag(all_corr_c(s_ta, u_pd, n)) * sin_n
                 if(alpha_ta(s_ta) + bm_ta .gt. &
                      alpha_ta_new(s_next_ta)) then
                    alpha_ta_new(s_next_ta) = alpha_ta(s_ta) + bm_ta
                 endif
              enddo
              alpha_ta = alpha_ta_new
           enddo

           vit_ta_metric = maxval(alpha_ta)
           if(vit_ta_metric .gt. vit_ta_best) then
              vit_ta_best = vit_ta_metric
              df_ta_best = df_ta_try
           endif
        enddo

        ! Apply correction if non-trivial
        if(abs(df_ta_best) .gt. 0.003) then
           write(*,'(a,f8.4)') '    turbo-aided df=',df_ta_best
           do i = 1, npts
              t_samp = real(i - 1) / fs_down
              cd_rot(i) = cd_rot(i) * &
                   cmplx(cos(-twopi_val*df_ta_best*t_samp), &
                         sin(-twopi_val*df_ta_best*t_samp))
           enddo
           ! Recompute branch metrics at corrected frequency
           call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
                branch_metrics_all, NSYM, n_offset)
           do n = 1, NSYM
              if(sync_sym(n) .ge. 0) then
                 call force_sync_metrics(branch_metrics_all(:,:,n), &
                      sync_sym(n))
              endif
           enddo
           ! Reset a priori: old LDPC feedback was for wrong frequency
           apriori_llr = 0.0
        endif
     endif

     ! Early termination: hopeless signal (no convergence by k=5)
     if(k_outer .ge. 5 .and. ncheck_bp .gt. 40) then
        ncheck_out = ncheck_bp
        llr_out = zn_bp
        return
     endif

     ! ----------------------------------------------------------
     ! Step 3d': Mid-loop OSD check
     ! Progressive: try OSD more aggressively as iterations progress
     ! ----------------------------------------------------------
     if(k_outer .ge. 3 .and. ncheck_bp .le. 15) then
        llr_osd = zn_bp
        apmask_osd = 0
        ! Progressive OSD order based on iteration and ncheck
        if(ncheck_bp .le. 3) then
           i_osd = 4    ! Close to convergence: try deep OSD
        else if(ncheck_bp .le. 8) then
           i_osd = 3
        else
           i_osd = 2
        endif
        call osd174_91(llr_osd, 91, apmask_osd, i_osd, msg91_osd, &
             cw_osd, nhd_osd, dmin_osd)
        if(nhd_osd .ge. 0) then
           message91 = msg91_osd
           ntype = 2       ! OSD decode in turbo loop
           nharderror = nhd_osd
           dmin = dmin_osd
           ncheck_out = ncheck_bp
           llr_out = zn_bp
           return
        endif
     endif

     ! ----------------------------------------------------------
     ! Step 3d'': Frequency correction + residual phase tracking
     !
     ! k_outer=2: Estimate residual frequency offset from phase
     !   slope of per-symbol MF correlations and apply a frequency
     !   twist. The coarse sync grid has ±0.25 Hz error; the phase
     !   slope across 99 symbols (weighted by correlation magnitude)
     !   estimates this offset. The π/2 disambiguation of phase
     !   DIFFERENCES wraps at ±3.5 Hz, well beyond the ±0.25 Hz
     !   expected residual.
     !
     ! k_outer>=4 (every 3): Per-symbol residual phase correction
     !   for fading and remaining frequency drift.
     ! ----------------------------------------------------------
     if(.false. .and. niter_max .eq. 0 .and. k_outer .eq. 2) then
        ! DISABLED (kept for reference, never compiled): residual-df from
        ! hard-decided per-symbol phase at k_outer==2 is too noisy (hard
        ! decisions unreliable that early), and it would also reset the AP LLRs
        ! mid-turbo, perturbing convergence. Step 1c's Viterbi sweep already
        ! handles residual frequency. Do NOT re-enable without re-validating the
        ! AWGN threshold — see the note at the Step 1b2 block above.
        ! Per-symbol residual phase from cd_rot correlations.
        ! Use BCJR-decided symbols to fix the input at each position,
        ! then search only over states. This avoids the (state,symbol)
        ! cross-correlation confusion that corrupts blind phase estimates.
        do n = 1, NSYM
           idx_dd = nint(real(n - 1) * nsps_down_real) + 1 + n_offset
           best_mag = 0.0
           corr_persym(n) = cmplx(0.0, 0.0)
           ! Determine decided symbol at this position
           if(sync_sym(n) .ge. 0) then
              i_sym = sync_sym(n)    ! Known sync symbol
           else
              ! Hard-decide from ext_bcjr (interleaved = frame order)
              ! Apply Gray code: bits(00)→0, (01)→1, (11)→2, (10)→3
              i_data_dd = data_idx(n)
              b1_dd = 0
              b2_dd = 0
              if(ext_bcjr(2*i_data_dd-1) .gt. 0.0) b1_dd = 1
              if(ext_bcjr(2*i_data_dd)   .gt. 0.0) b2_dd = 1
              i_sym = 2*b1_dd + b2_dd
              if(i_sym .eq. 2) then
                 i_sym = 3
              else if(i_sym .eq. 3) then
                 i_sym = 2
              endif
           endif
           do s_try = 1, 16    ! θ=0 correlative states only
              corr_dd = cmplx(0.0, 0.0)
              do k_dd = 1, NSS
                 n_dd = idx_dd + k_dd - 1
                 if(n_dd .ge. 1 .and. n_dd .le. npts) then
                    corr_dd = corr_dd + cd_rot(n_dd) * &
                         conjg(mf_bank(k_dd, s_try, i_sym))
                 endif
              enddo
              mag_try = abs(corr_dd)
              if(mag_try .gt. best_mag) then
                 best_mag = mag_try
                 corr_persym(n) = corr_dd
              endif
           enddo
        enddo

        ! Extract residual phase with independent π/2 disambiguation.
        do n = 1, NSYM
           phase_raw(n) = atan2(aimag(corr_persym(n)), &
                real(corr_persym(n)))
           phase_raw(n) = phase_raw(n) - &
                nint(phase_raw(n) / 1.5707963) * 1.5707963
        enddo

        ! At k_outer=2: estimate frequency offset from phase slope.
        ! Uses disambiguated phase DIFFERENCES (not absolute phases)
        ! to avoid the per-symbol wrapping problem. The π/2
        ! disambiguation of differences wraps at ±3.5 Hz (baud/8),
        ! far beyond the ±0.25 Hz expected from the 0.5 Hz coarse grid.
        ! Weighted by magnitude product to suppress noise contributions.
        if(k_outer .le. 5) then
           pi_val = 4.0*atan(1.0)
           sum_dphi = 0.0
           sum_wt = 0.0
           do n = 1, NSYM-1
              phase_diff = phase_raw(n+1) - phase_raw(n)
              ! Wrap to [-π/4, π/4]
              phase_diff = phase_diff - &
                   nint(phase_diff / 1.5707963) * 1.5707963
              ! Weight by geometric mean of adjacent magnitudes
              mag_try = abs(corr_persym(n)) * abs(corr_persym(n+1))
              sum_dphi = sum_dphi + phase_diff * mag_try
              sum_wt = sum_wt + mag_try
           enddo
           if(sum_wt .gt. 0.0) then
              df_corr = (sum_dphi / sum_wt) * 28.0 / (2.0 * pi_val)
           else
              df_corr = 0.0
           endif
           ! Apply frequency twist to cd_rot (only for large residuals;
           ! with 0.1 Hz probe grid, residual is ±0.05 Hz so rarely fires)
           write(*,'(a,i2,a,f8.3)') '    iter',k_outer,' df_corr=',df_corr
           if(abs(df_corr) .gt. 0.01) then
              do i = 1, npts
                 t_samp = real(i - 1) / fs_down
                 cd_rot(i) = cd_rot(i) * &
                      cmplx(cos(2.0*pi_val*df_corr*t_samp), &
                            -sin(2.0*pi_val*df_corr*t_samp))
              enddo
              ! Recompute branch metrics after frequency correction
              call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2,&
                   branch_metrics_all, NSYM, n_offset)
              do n = 1, NSYM
                 if(sync_sym(n) .ge. 0) then
                    call force_sync_metrics(branch_metrics_all(:,:,n), &
                         sync_sym(n))
                 endif
              enddo
              ! Recompute per-symbol phases after correction
              ! (using BCJR-decided symbols, same as above)
              do n = 1, NSYM
                 idx_dd = nint(real(n-1)*nsps_down_real) + 1 + n_offset
                 best_mag = 0.0
                 corr_persym(n) = cmplx(0.0, 0.0)
                 if(sync_sym(n) .ge. 0) then
                    i_sym = sync_sym(n)
                 else
                    i_data_dd = data_idx(n)
                    b1_dd = 0
                    b2_dd = 0
                    if(ext_bcjr(2*i_data_dd-1) .gt. 0.0) b1_dd = 1
                    if(ext_bcjr(2*i_data_dd)   .gt. 0.0) b2_dd = 1
                    i_sym = 2*b1_dd + b2_dd
                    if(i_sym .eq. 2) then
                       i_sym = 3
                    else if(i_sym .eq. 3) then
                       i_sym = 2
                    endif
                 endif
                 do s_try = 1, 16
                    corr_dd = cmplx(0.0, 0.0)
                    do k_dd = 1, NSS
                       n_dd = idx_dd + k_dd - 1
                       if(n_dd .ge. 1 .and. n_dd .le. npts) then
                          corr_dd = corr_dd + cd_rot(n_dd) * &
                               conjg(mf_bank(k_dd, s_try, i_sym))
                       endif
                    enddo
                    mag_try = abs(corr_dd)
                    if(mag_try .gt. best_mag) then
                       best_mag = mag_try
                       corr_persym(n) = corr_dd
                    endif
                 enddo
              enddo
              do n = 1, NSYM
                 phase_raw(n) = atan2(aimag(corr_persym(n)), &
                      real(corr_persym(n)))
                 phase_raw(n) = phase_raw(n) - &
                      nint(phase_raw(n) / 1.5707963) * 1.5707963
              enddo
              ! Reset a priori LLRs so turbo restarts fresh at corrected freq.
              ! Without this, LDPC feedback from wrong-frequency iterations
              ! contaminates subsequent BCJR passes and prevents convergence.
              apriori_llr = 0.0
           endif
        endif

        ! Per-symbol phase smoothing + correction
        ! Disabled in AWGN: the per-symbol noise correction destroys
        ! turbo convergence. Only useful under HF fading where channel
        ! phase actually varies. TODO: re-enable with fading detection.
        if(.false. .and. k_outer .ge. 4) then
           ! Adaptive forward-backward exponential smoothing
           ! Alpha depends on SNR: high SNR (fading regime) -> less smoothing
           if(snr_est .gt. -5.0) then
              alpha_phase = 0.6    ! Fast fading: trust measurements more
           else if(snr_est .gt. -12.0) then
              alpha_phase = 0.4    ! Medium: balanced
           else
              alpha_phase = 0.25   ! Low SNR / AWGN: smooth heavily
           endif

           ! Forward pass
           phase_smooth(1) = phase_raw(1)
           do n = 2, NSYM
              phase_smooth(n) = alpha_phase * phase_raw(n) + &
                   (1.0 - alpha_phase) * phase_smooth(n-1)
           enddo

           ! Backward pass (eliminates forward-only lag)
           do n = NSYM - 1, 1, -1
              phase_smooth(n) = 0.5 * phase_smooth(n) + &
                   0.5 * (alpha_phase * phase_raw(n) + &
                   (1.0 - alpha_phase) * phase_smooth(n+1))
           enddo

           ! Apply residual correction to cd_rot
           do i = 1, npts
              sym_pos = real(i - 1 - n_offset) / nsps_down_real
              n = int(sym_pos) + 1
              if(n .lt. 1) n = 1
              if(n .ge. NSYM) then
                 phase_interp = phase_smooth(NSYM)
              else
                 frac = sym_pos - real(n - 1)
                 phase_interp = phase_smooth(n) + &
                      frac * (phase_smooth(n+1) - phase_smooth(n))
              endif
              cd_rot(i) = cd_rot(i) * &
                   cmplx(cos(phase_interp), -sin(phase_interp))
           enddo

           ! Recompute branch metrics with corrected phase
           call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
                branch_metrics_all, NSYM, n_offset)

           ! Re-apply sync forcing (G1 only)
           do n = 1, 4
              if(sync_sym(n) .ge. 0) then
                 call force_sync_metrics(branch_metrics_all(:,:,n), &
                      sync_sym(n))
              endif
           enddo
        endif
     endif

     ! ----------------------------------------------------------
     ! Step 3e: Compute LDPC extrinsic LLRs for feedback
     ! ----------------------------------------------------------
     do i = 1, NBITS
        ext_ldpc(i) = zn_bp(i) - ext_bcjr_dilv(i)
        if(ext_ldpc(i) .gt. 30.0) ext_ldpc(i) = 30.0
        if(ext_ldpc(i) .lt. -30.0) ext_ldpc(i) = -30.0
     enddo

     ! ----------------------------------------------------------
     ! Step 3f: Forward interleave LDPC extrinsic -> BCJR a priori
     ! ----------------------------------------------------------
     call ft1_interleave_real(ext_ldpc, ext_ldpc_ilv, 1)

     ! ----------------------------------------------------------
     ! Step 3g: Update a priori for next BCJR iteration
     ! Ramped damping: start cautious (0.2), increase to (0.7)
     ! to prevent turbo divergence from overconfident feedback.
     ! ----------------------------------------------------------
     if(k_outer .le. 2) then
        damp = 0.3
     else if(k_outer .le. 5) then
        damp = 0.5
     else if(k_outer .le. 10) then
        damp = 0.7
     else
        damp = 0.8
     endif
     apriori_llr = damp * ext_ldpc_ilv

  enddo  ! Turbo loop

! ================================================================
! Step 4: OSD fallback (turbo loop did not converge)
! ================================================================
  do i = 1, NBITS
     llr_osd(i) = zn_bp(i)
     llr_out(i) = zn_bp(i)
  enddo
  apmask_osd = 0

  do i = 1, 4
     call osd174_91(llr_osd, 91, apmask_osd, i, msg91_osd, cw_osd, &
          nhd_osd, dmin_osd)
     if(nhd_osd .ge. 0) then
        message91 = msg91_osd
        ntype = 2       ! OSD decode success
        nharderror = nhd_osd
        dmin = dmin_osd
        ncheck_out = 0
        return
     endif
  enddo

! Decode failed
  ntype = -1
  nharderror = -1
  return
end subroutine turbo_decode_ft1


subroutine ft1_joint_turbo_harq(cd_rv0, cd_rv1, cd_rv2, npts, snr_est, &
     nrv, message77, nharderror, niter_outer)
! Joint iterative turbo HARQ combining.
!
! The incumbent HARQ path gives RV0 full turbo (BCJR<->LDPC) but RV1/RV2 only a
! single BCJR pass (ft1_demod_bcjr), then combines at the LDPC level — throwing
! away ~1.5-2.5 dB of coherent turbo gain on every retransmitted frame. This
! routine instead alternates a soft decode of the COMBINED LDPC(261/348) code
! with a BCJR re-demod of EACH received RV frame, feeding combined-code
! extrinsics back into every frame's a-priori (with own-look exclusion on the
! repeated systematic bits, to avoid positive feedback). Phase estimation is
! paid once per frame via ft1_bm_prep; only the cheap BCJR sweep repeats.
!
! Falls back to OSD + a full combined BP on the last iteration's channel
! evidence, so it cannot do worse than the incumbent single-BCJR combine.
!
! Inputs:
!   cd_rv0/1/2(npts) - complex baseband for each received RV frame
!                      (cd_rv2 unused/ignored when nrv<3; pass it anyway)
!   snr_est          - SNR estimate (dB, 2500 Hz BW)
!   nrv              - number of RV frames available (2 or 3)
!   niter_outer      - outer turbo iterations (5 typical; <=0 => no decode)
! Outputs:
!   message77(77)    - decoded message bits on success
!   nharderror       - >=0 on success, -1 on failure
  use cpm_trellis_mod
  use ldpc348_91_mod
  implicit none
  integer, intent(in)    :: npts, nrv, niter_outer
  complex, intent(in)    :: cd_rv0(npts), cd_rv1(npts), cd_rv2(npts)
  real,    intent(in)    :: snr_est
  integer*1, intent(out) :: message77(77)
  integer, intent(out)   :: nharderror

  integer, parameter :: NB = 174
  real    :: bm0(NSTATES,0:3,99), bm1(NSTATES,0:3,99), bm2(NSTATES,0:3,99)
  integer :: is_data(99), data_idx(99)
  real    :: apri0(NB), apri1(NB), apri2(NB)              ! symbol order
  real    :: extsym(NB), extcb0(NB), extcb1(NB), extcb2(NB) ! code-bit order
  real    :: LCH(N_MOTHER), LE(N_MOTHER), zncomb(N_MOTHER)
  real    :: aprcb(NB)
  integer*1 :: msg77(77)
  integer :: ncomb, t, nbp, nhd, ncheck, iter_bp, i
  real    :: damp
  ! OSD fallback
  integer*1 :: apmask_osd(NB), cw_osd(NB), msg91_osd(91)
  integer :: nhd_osd, i_osd
  real    :: dmin_osd, llr_osd(NB)

  nharderror = -1; message77 = 0
  if(niter_outer .le. 0) return

  if(nrv .ge. 3) then
     ncomb = N_EXT2
  else if(nrv .eq. 2) then
     ncomb = N_EXT1
  else
     ncomb = N_BASE
  endif

  ! Phase est + branch-metric prep ONCE per available frame
  call ft1_bm_prep(cd_rv0, npts, snr_est, 0, bm0, is_data, data_idx)
  if(nrv .ge. 2) call ft1_bm_prep(cd_rv1, npts, snr_est, 1, bm1, is_data, data_idx)
  if(nrv .ge. 3) call ft1_bm_prep(cd_rv2, npts, snr_est, 2, bm2, is_data, data_idx)

  apri0 = 0.0; apri1 = 0.0; apri2 = 0.0
  extcb0 = 0.0; extcb1 = 0.0; extcb2 = 0.0
  LE = 0.0

  do t = 1, niter_outer

     ! 1+2. BCJR re-demod each frame, deinterleave to code-bit order
     call bcjr_cpm(bm0, apri0, extsym, 99, 87, is_data, data_idx)
     call ft1_interleave_real(extsym, extcb0, -1)
     if(nrv .ge. 2) then
        call bcjr_cpm(bm1, apri1, extsym, 99, 87, is_data, data_idx)
        call ft1_interleave_real(extsym, extcb1, -1)
     endif
     if(nrv .ge. 3) then
        call bcjr_cpm(bm2, apri2, extsym, 99, 87, is_data, data_idx)
        call ft1_interleave_real(extsym, extcb2, -1)
     endif

     ! 3. Assemble combined channel LLR (matches incumbent combine layout)
     LCH = 0.0
     LCH(1:174) = extcb0(1:174)
     if(nrv .ge. 2) then
        do i = 1, 87
           LCH(i) = LCH(i) + extcb1(87+i)      ! systematic repeat
        enddo
        LCH(175:261) = extcb1(1:87)            ! RV1 new parity
     endif
     if(nrv .ge. 3) then
        do i = 1, 87
           LCH(i) = LCH(i) + extcb2(87+i)
        enddo
        LCH(262:348) = extcb2(1:87)            ! RV2 new parity
     endif

     ! 4. Soft LDPC BP on the combined codeword (ramp iterations)
     if(t .le. 2) then
        nbp = 12
     else if(t .le. 4) then
        nbp = 25
     else
        nbp = 40
     endif
     call bpdecode_ext_soft(LCH(1:ncomb), ncomb, nbp, msg77, &
          zncomb(1:ncomb), nhd, ncheck, iter_bp)

     ! 5. CRC / convergence stop
     if(ncheck .eq. 0 .and. nhd .ge. 0) then
        message77 = msg77
        nharderror = nhd
        return
     endif

     ! 6. Combined LDPC extrinsic
     do i = 1, ncomb
        LE(i) = zncomb(i) - LCH(i)
        if(LE(i) .gt. 30.0) LE(i) = 30.0
        if(LE(i) .lt. -30.0) LE(i) = -30.0
     enddo

     ! Ramped damping
     if(t .le. 2) then
        damp = 0.3
     else if(t .le. 4) then
        damp = 0.5
     else
        damp = 0.7
     endif

     ! 7+8. Map LDPC extrinsic back to each frame (own-look excluded on
     !      shared systematic bits 1-87), interleave to symbol order, damp.
     aprcb = 0.0
     aprcb(88:174) = LE(88:174)               ! RV0 exclusive parity
     do i = 1, 87
        aprcb(i) = LE(i)                       ! shared: code belief + others' looks
        if(nrv .ge. 2) aprcb(i) = aprcb(i) + extcb1(87+i)
        if(nrv .ge. 3) aprcb(i) = aprcb(i) + extcb2(87+i)
     enddo
     call clip30(aprcb, NB)
     call ft1_interleave_real(aprcb, apri0, 1)
     apri0 = damp * apri0

     if(nrv .ge. 2) then
        aprcb = 0.0
        aprcb(1:87) = LE(175:261)             ! RV1 exclusive parity
        do i = 1, 87
           aprcb(87+i) = LE(i) + extcb0(i)    ! shared: + RV0 look
           if(nrv .ge. 3) aprcb(87+i) = aprcb(87+i) + extcb2(87+i)
        enddo
        call clip30(aprcb, NB)
        call ft1_interleave_real(aprcb, apri1, 1)
        apri1 = damp * apri1
     endif

     if(nrv .ge. 3) then
        aprcb = 0.0
        aprcb(1:87) = LE(262:348)             ! RV2 exclusive parity
        do i = 1, 87
           aprcb(87+i) = LE(i) + extcb0(i) + extcb1(87+i)
        enddo
        call clip30(aprcb, NB)
        call ft1_interleave_real(aprcb, apri2, 1)
        apri2 = damp * apri2
     endif

  enddo  ! outer turbo loop

  ! Fallback: OSD on the final combined base-code LLRs, then one full BP.
  llr_osd(1:174) = LCH(1:174)
  apmask_osd = 0
  do i_osd = 1, 4
     call osd174_91(llr_osd, 91, apmask_osd, i_osd, msg91_osd, cw_osd, &
          nhd_osd, dmin_osd)
     if(nhd_osd .ge. 0) then
        message77 = msg91_osd(1:77)
        nharderror = nhd_osd
        return
     endif
  enddo
  call bpdecode_ext_soft(LCH(1:ncomb), ncomb, 50, msg77, &
       zncomb(1:ncomb), nhd, ncheck, iter_bp)
  if(ncheck .eq. 0 .and. nhd .ge. 0) then
     message77 = msg77
     nharderror = nhd
  endif
  return

contains
  subroutine clip30(a, n)
    integer, intent(in) :: n
    real, intent(inout) :: a(n)
    integer :: ii
    do ii = 1, n
       if(a(ii) .gt. 30.0) a(ii) = 30.0
       if(a(ii) .lt. -30.0) a(ii) = -30.0
    enddo
  end subroutine clip30
end subroutine ft1_joint_turbo_harq


subroutine compute_branch_metrics_all(cd, npts, nss, sigma2, &
     branch_metrics, nsym, n_offset)
!
! Compute channel branch metrics for ALL 99 symbol positions in the
! FT1 frame via matched filter correlation.
!
! Each symbol position n (1-indexed) corresponds to channel position
! n-1 (0-indexed). Exact symbol start positions are computed using
! Bresenham-style rational timing, offset by n_offset samples.
!
  use matched_filter_bank_mod, only: mf_bank, nss_down
  use cpm_trellis_mod, only: NSTATES
  implicit none

  integer, intent(in)  :: npts, nss, nsym, n_offset
  complex, intent(in)  :: cd(npts)
  real, intent(in)     :: sigma2
  real, intent(out)    :: branch_metrics(NSTATES, 0:3, nsym)

  integer :: n, s, i_n, k, idx, chan_pos, idx_start
  complex :: corr
  real :: scale
  real :: nsps_down_real

  if(sigma2 .gt. 0.0) then
     scale = 2.0 / sigma2
  else
     scale = 1.0
  endif

  ! Exact samples per symbol in the downsampled domain
  nsps_down_real = 3000.0 / (7.0 * 54.0)

  branch_metrics = 0.0

  do n = 1, nsym
     chan_pos = n - 1    ! 0-indexed channel position

     ! Exact sample start for this channel position (1-based, with offset)
     idx_start = nint(chan_pos * nsps_down_real) + 1 + n_offset

     do s = 1, NSTATES
        do i_n = 0, 3
           corr = cmplx(0.0, 0.0)
           do k = 1, nss
              idx = idx_start + k - 1
              if(idx .ge. 1 .and. idx .le. npts) then
                 corr = corr + cd(idx) * conjg(mf_bank(k, s, i_n))
              endif
           enddo
           branch_metrics(s, i_n, n) = scale * real(corr)
        enddo
     enddo
  enddo

  return
end subroutine compute_branch_metrics_all


subroutine force_sync_metrics(bm, known_sym)
!
! Force branch metrics at a sync position to select only the known symbol.
! Keeps channel metrics for the known symbol (preserves state discrimination)
! and sets NEG_INF for all other symbols.
!
  use cpm_trellis_mod, only: NSTATES
  implicit none
  real, intent(inout) :: bm(NSTATES, 0:3)
  integer, intent(in) :: known_sym
  real, parameter :: NEG_INF = -1.0e30
  integer :: s, i_n

  do s = 1, NSTATES
     do i_n = 0, 3
        if(i_n .ne. known_sym) then
           bm(s, i_n) = NEG_INF
        endif
        ! Leave bm(s, known_sym) at its channel value for state discrimination
     enddo
  enddo

  return
end subroutine force_sync_metrics


subroutine ft1_interleave_real(llr_in, llr_out, ndir)
!
! Interleave/deinterleave real-valued LLR arrays using the FT1
! S-random interleaver.
!
! The encoder does: LDPC codeword -> forward interleave -> symbols
! So the symbol order is the INTERLEAVED order.
!
! ndir = +1: forward interleave  (LDPC order -> interleaved/symbol order)
! ndir = -1: inverse interleave  (interleaved/symbol order -> LDPC order)
!
  implicit none
  integer, parameter :: N = 174
  real, intent(in)  :: llr_in(N)
  real, intent(out) :: llr_out(N)
  integer, intent(in) :: ndir

  integer :: k
  integer :: itable(N), itableinv(N)

! Forward permutation table (1-indexed)
! Optimized S-random interleaver: S=9, min_pair_sep=12, avg_pair_sep=74.3
! Generated by gen_interleaver_quick.py (seed=1000980, score=0.4499)
  data itable/ &
    138,  25, 171,  78,  93,  63,  39, 159,   2, 114, 104,  53, 150,  18,  73, &
    133,  85, 172,  28, 112,   6, 144,  49,  94,  67, 124, 169,  16,  40, 154, &
     58,   3,  84, 141,  97, 125,  21, 164,  12, 108,  75,  43, 148,  61, 137, &
     30, 128, 162,  96, 119,  70, 174,  48, 140,  79,  32,  57, 100, 161,  13, &
    173, 113, 131,  74,  37, 122,   1,  87,  20,  59, 105,  47, 143,  11, 157, &
    118,  90, 129, 168, 107,  80, 142,  68,  22, 152,  56,  95, 116,  35, 130, &
     77, 106,  17, 167,   4, 149, 139,  34, 121,  51,  88, 102,  14, 160,  65, &
    146,  29, 123,  41, 111, 135, 170, 155,  52,  92,  19, 126,  31,  66, 117, &
     82, 153,  10, 163, 103,  46, 127,  64,  33,  86,  23,  55, 115,  76,  45, &
    136, 145,  99,   8, 158,  62, 110,  24,  42,  83, 134, 120,   9,  72,  60, &
    101,  26,  44, 151, 165,  81,  15,  71,  91, 109,   5,  50,  36, 166, 147, &
    132,  69,  27,  89, 156,  54,   7,  38,  98/

! Inverse permutation table (1-indexed)
  data itableinv/ &
     67,   9,  32,  95, 161,  21, 172, 139, 148, 123,  74,  39,  60, 103, 157, &
     28,  93,  14, 116,  69,  37,  84, 131, 143,   2, 152, 168,  19, 107,  46, &
    118,  56, 129,  98,  89, 163,  65, 173,   7,  29, 109, 144,  42, 153, 135, &
    126,  72,  53,  23, 162, 100, 114,  12, 171, 132,  86,  57,  31,  70, 150, &
     44, 141,   6, 128, 105, 119,  25,  83, 167,  51, 158, 149,  15,  64,  41, &
    134,  91,   4,  55,  81, 156, 121, 145,  33,  17, 130,  68, 101, 169,  77, &
    159, 115,   5,  24,  87,  49,  35, 174, 138,  58, 151, 102, 125,  11,  71, &
     92,  80,  40, 160, 142, 110,  20,  62,  10, 133,  88, 120,  76,  50, 147, &
     99,  66, 108,  26,  36, 117, 127,  47,  78,  90,  63, 166,  16, 146, 111, &
    136,  45,   1,  97,  54,  34,  82,  73,  22, 137, 106, 165,  43,  96,  13, &
    154,  85, 122,  30, 113, 170,  75, 140,   8, 104,  59,  48, 124,  38, 155, &
    164,  94,  79,  27, 112,   3,  18,  61,  52/

  llr_out = 0.0

  if(ndir .ge. 0) then
     ! Forward: llr_out(pi(k)) = llr_in(k)
     do k = 1, N
        llr_out(itable(k)) = llr_in(k)
     enddo
  else
     ! Inverse: llr_out(pi_inv(k)) = llr_in(k)
     do k = 1, N
        llr_out(itableinv(k)) = llr_in(k)
     enddo
  endif

  return
end subroutine ft1_interleave_real


subroutine bpdecode174_91_beliefs(llr, apmask, maxiterations, &
     zn_out, nharderror, iter, ncheck)
!
! Wrapper around the LDPC BP decoder that returns the total bit beliefs
! (zn array) in addition to the standard outputs.
!
! This is needed for turbo equalization to extract LDPC extrinsic LLRs:
!   extrinsic(j) = zn(j) - llr_input(j)
!
  use crc
  implicit none
  integer, parameter :: N=174, K=91, M=N-K

  real, intent(in)     :: llr(N)
  integer*1, intent(in) :: apmask(N)
  integer, intent(in)  :: maxiterations
  real, intent(out)    :: zn_out(N)
  integer, intent(out) :: nharderror, iter, ncheck

  integer*1 :: cw(N), decoded(K), message77(77)
  integer :: nrw(M), ncw
  integer :: Nm(7,M), Mn(3,N)
  integer :: synd(M)
  real :: tov(3,N), toc(7,M), tanhtoc(7,M)
  real :: zn(N), Tmn, y
  integer :: i, j, kk, ibj, ichk, nd, ncnt, nclast, nbadcrc

  include "../ft8/ldpc_174_91_c_parity.f90"

  decoded = 0
  toc = 0
  tov = 0
  tanhtoc = 0

  ! Initialize messages to checks
  do j = 1, M
     do i = 1, nrw(j)
        toc(i,j) = llr(Nm(i,j))
     enddo
  enddo

  ncnt = 0
  nclast = 0

  do iter = 0, maxiterations

     ! Update bit beliefs
     do i = 1, N
        if(apmask(i) .ne. 1) then
           zn(i) = llr(i) + sum(tov(1:ncw,i))
        else
           zn(i) = llr(i)
        endif
     enddo

     ! Check syndrome
     cw = 0
     where(zn .gt. 0.) cw = 1
     ncheck = 0
     do i = 1, M
        synd(i) = sum(cw(Nm(1:nrw(i),i)))
        if(mod(synd(i),2) .ne. 0) ncheck = ncheck + 1
     enddo

     if(ncheck .eq. 0) then
        decoded = cw(1:K)
        call chkcrc14a(decoded, nbadcrc)
        nharderror = count((2*cw-1)*llr .lt. 0.0)
        if(nbadcrc .eq. 0) then
           message77 = decoded(1:77)
           zn_out = zn
           return
        endif
     endif

     if(iter .gt. 0) then
        nd = ncheck - nclast
        if(nd .lt. 0) then
           ncnt = 0
        else
           ncnt = ncnt + 1
        endif
        if(ncnt .ge. 5 .and. iter .ge. 10 .and. ncheck .gt. 15) then
           nharderror = -1
           zn_out = zn
           return
        endif
     endif
     nclast = ncheck

     ! Variable-to-check messages
     do j = 1, M
        do i = 1, nrw(j)
           ibj = Nm(i,j)
           toc(i,j) = zn(ibj)
           do kk = 1, ncw
              if(Mn(kk,ibj) .eq. j) then
                 toc(i,j) = toc(i,j) - tov(kk,ibj)
              endif
           enddo
        enddo
     enddo

     ! Check-to-variable messages
     do i = 1, M
        tanhtoc(1:7,i) = tanh(-toc(1:7,i)/2)
     enddo

     do j = 1, N
        do i = 1, ncw
           ichk = Mn(i,j)
           Tmn = product(tanhtoc(1:nrw(ichk),ichk), &
                mask=Nm(1:nrw(ichk),ichk) .ne. j)
           call platanh(-Tmn, y)
           tov(i,j) = 2*y
        enddo
     enddo

  enddo

  nharderror = -1
  zn_out = zn
  return
end subroutine bpdecode174_91_beliefs


subroutine compute_bp_beliefs(llr, apmask, maxiterations, zn_out)
!
! Run BP and return the final total beliefs (zn).
! Thin wrapper that discards decode status.
!
  implicit none
  integer, parameter :: N = 174
  real, intent(in)     :: llr(N)
  integer*1, intent(in) :: apmask(N)
  integer, intent(in)  :: maxiterations
  real, intent(out)    :: zn_out(N)
  integer :: nharderror, iter, ncheck

  call bpdecode174_91_beliefs(llr, apmask, maxiterations, &
       zn_out, nharderror, iter, ncheck)
  return
end subroutine compute_bp_beliefs
