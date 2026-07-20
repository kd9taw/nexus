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
subroutine ft1_demod_bcjr(cd, npts, snr_est, rv, llr_out)
!
! Single-pass BCJR demodulator for HARQ retransmissions (RV1/RV2).
!
! Unlike turbo_decode_ft1, this performs NO turbo iteration (no LDPC feedback).
! RV1/RV2 transmit 174 bits that don't form a valid LDPC(174,91) codeword,
! so LDPC-based turbo feedback would be wrong. A single BCJR pass with zero
! a-priori gives clean channel LLRs suitable for HARQ combining.
!
! Pipeline:
!   1. Phase estimation from RV-specific Costas arrays
!   2. Phase-compensate signal
!   3. Compute branch metrics (matched filter correlations)
!   4. Force sync metrics at known positions
!   5. Single BCJR pass with zero a-priori
!   6. Deinterleave extrinsic LLRs to code-bit order
!
! Input:
!   cd(npts)    - complex downsampled baseband signal (~8 samp/sym)
!   npts        - number of samples
!   snr_est     - SNR estimate (dB, 2500 Hz BW)
!   rv          - redundancy version (0, 1, or 2)
!
! Output:
!   llr_out(174) - LLRs in punctured code-bit order (deinterleaved)

  use cpm_trellis_mod
  use matched_filter_bank_mod
  implicit none

  integer, intent(in)  :: npts
  complex, intent(in)  :: cd(npts)
  real, intent(in)     :: snr_est
  integer, intent(in)  :: rv
  real, intent(out)    :: llr_out(174)

! Parameters
  integer, parameter :: NBITS = 174
  integer, parameter :: NDATA = 87
  integer, parameter :: NSYM = 99
  integer, parameter :: NSS = 8

! Local arrays
  real    :: branch_metrics_all(NSTATES, 0:3, NSYM)
  real    :: apriori_llr(NBITS)
  real    :: ext_bcjr(NBITS)

! Frame map
  integer :: is_data(NSYM)
  integer :: data_idx(NSYM)
  integer :: sync_sym(NSYM)
  integer :: icos(4)
  integer :: icos_rv0(4), icos_rv1(4), icos_rv2(4)

! Phase estimation variables
  complex :: cd_rot(npts)
  complex :: corr_sync, phase_sum, best_sum
  complex :: c_ref, c_derot
  real    :: phi(3)
  real    :: best_mag, mag_try, r_align, best_real
  real    :: phase_interp, sym_pos, frac, dphi
  real    :: nsps_down_real, sigma2, snr_linear
  integer :: s_sync, j_sync, k_sync, idx_sync, idx_start_sync
  integer :: s_try, igrp, chan_start(3), k_phase
  integer :: i, n

! Residual phase correction variables
  complex :: corr_dd, corr_persym(NSYM)
  real    :: phase_raw(NSYM), phase_smooth(NSYM), alpha_phase
  integer :: idx_dd, k_dd, n_dd, i_sym

  data icos_rv0/0,2,3,1/
  data icos_rv1/1,3,2,0/
  data icos_rv2/3,0,2,1/

! Select Costas array for this RV
  select case(rv)
  case(0); icos = icos_rv0
  case(1); icos = icos_rv1
  case(2); icos = icos_rv2
  case default; icos = icos_rv0
  end select

! Initialize trellis and matched filters
  call init_cpm_trellis()
  call init_matched_filters(NSS)

! Set up frame map
  is_data = 0
  data_idx = 0
  sync_sym = -1

  sync_sym(1)  = icos(1)
  sync_sym(2)  = icos(2)
  sync_sym(3)  = icos(3)
  sync_sym(4)  = icos(4)

  do i = 5, 47
     is_data(i) = 1
     data_idx(i) = i - 4
  enddo

  sync_sym(48) = icos(1)
  sync_sym(49) = icos(2)
  sync_sym(50) = icos(3)
  sync_sym(51) = icos(4)

  do i = 52, 95
     is_data(i) = 1
     data_idx(i) = i - 8
  enddo

  sync_sym(96) = icos(1)
  sync_sym(97) = icos(2)
  sync_sym(98) = icos(3)
  sync_sym(99) = icos(4)

! Noise variance estimate
  snr_linear = 10.0**(snr_est / 10.0) * (2500.0 / 180.0)
  sigma2 = 1.0 / max(1.0 + snr_linear, 1.0e-6)

! Per-segment phase estimation using three Costas arrays
  nsps_down_real = 3000.0 / (7.0 * 54.0)
  chan_start = (/0, 47, 95/)

! Group 1: known start state (state 1, theta=0)
  phase_sum = cmplx(0.0, 0.0)
  s_sync = 1
  do j_sync = 1, 4
     idx_start_sync = nint(real(chan_start(1) + j_sync - 1) * &
          nsps_down_real) + 1
     corr_sync = cmplx(0.0, 0.0)
     do k_sync = 1, NSS
        idx_sync = idx_start_sync + k_sync - 1
        if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
           corr_sync = corr_sync + cd(idx_sync) * &
                conjg(mf_bank(k_sync, s_sync, icos(j_sync)))
        endif
     enddo
     phase_sum = phase_sum + corr_sync
     s_sync = next_state(s_sync, icos(j_sync))
  enddo
  phi(1) = atan2(aimag(phase_sum), real(phase_sum))

! Groups 2-3: search correlative states, resolve pi/2 ambiguity
  do igrp = 2, 3
     best_mag = 0.0
     best_sum = cmplx(0.0, 0.0)
     do s_try = 1, 16
        phase_sum = cmplx(0.0, 0.0)
        s_sync = s_try
        do j_sync = 1, 4
           idx_start_sync = nint(real(chan_start(igrp) + j_sync - 1) * &
                nsps_down_real) + 1
           corr_sync = cmplx(0.0, 0.0)
           do k_sync = 1, NSS
              idx_sync = idx_start_sync + k_sync - 1
              if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
                 corr_sync = corr_sync + cd(idx_sync) * &
                      conjg(mf_bank(k_sync, s_sync, icos(j_sync)))
              endif
           enddo
           phase_sum = phase_sum + corr_sync
           s_sync = next_state(s_sync, icos(j_sync))
        enddo
        mag_try = abs(phase_sum)
        if(mag_try .gt. best_mag) then
           best_mag = mag_try
           best_sum = phase_sum
        endif
     enddo

     c_ref = cmplx(cos(phi(igrp-1)), sin(phi(igrp-1)))
     best_real = -1.0e30
     phi(igrp) = phi(igrp-1)
     do k_phase = 0, 3
        select case(k_phase)
           case(0); c_derot = best_sum
           case(1); c_derot = cmplx(aimag(best_sum), -real(best_sum))
           case(2); c_derot = cmplx(-real(best_sum), -aimag(best_sum))
           case(3); c_derot = cmplx(-aimag(best_sum), real(best_sum))
        end select
        r_align = real(c_derot * conjg(c_ref))
        if(r_align .gt. best_real) then
           best_real = r_align
           phi(igrp) = atan2(aimag(c_derot), real(c_derot))
        endif
     enddo
  enddo

! Unwrap phase
  dphi = phi(2) - phi(1)
  if(dphi .gt. 3.14159265) phi(2) = phi(2) - 6.28318530
  if(dphi .lt. -3.14159265) phi(2) = phi(2) + 6.28318530
  dphi = phi(3) - phi(2)
  if(dphi .gt. 3.14159265) phi(3) = phi(3) - 6.28318530
  if(dphi .lt. -3.14159265) phi(3) = phi(3) + 6.28318530

! Piecewise-linear phase interpolation
  do i = 1, npts
     sym_pos = real(i - 1) / nsps_down_real
     if(sym_pos .le. 48.5) then
        frac = (sym_pos - 1.5) / (48.5 - 1.5)
        phase_interp = phi(1) + frac * (phi(2) - phi(1))
     else
        frac = (sym_pos - 48.5) / (96.5 - 48.5)
        phase_interp = phi(2) + frac * (phi(3) - phi(2))
     endif
     cd_rot(i) = cd(i) * cmplx(cos(phase_interp), -sin(phase_interp))
  enddo

! Compute branch metrics
  call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
       branch_metrics_all, NSYM, 0)

! Force sync positions
  do n = 1, NSYM
     if(is_data(n) .eq. 0 .and. sync_sym(n) .ge. 0) then
        call force_sync_metrics(branch_metrics_all(:,:,n), sync_sym(n))
     endif
  enddo

! First BCJR pass with zero a-priori
  apriori_llr = 0.0
  call bcjr_cpm(branch_metrics_all, apriori_llr, ext_bcjr, &
       NSYM, NDATA, is_data, data_idx)

! ================================================================
! Residual phase correction: use soft decisions from first BCJR to
! estimate per-symbol residual phase, correct cd_rot, re-run BCJR.
! This "demod-correct-demod" pattern improves LLR quality under fading.
! ================================================================

  ! Per-symbol residual phase from cd_rot correlations
  do n = 1, NSYM
     idx_dd = nint(real(n - 1) * nsps_down_real) + 1
     best_mag = 0.0
     corr_persym(n) = cmplx(0.0, 0.0)
     do s_try = 1, 16    ! theta=0 correlative states only
        do i_sym = 0, 3
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
  enddo

  ! Extract residual phase with independent pi/2 disambiguation
  do n = 1, NSYM
     phase_raw(n) = atan2(aimag(corr_persym(n)), &
          real(corr_persym(n)))
     phase_raw(n) = phase_raw(n) - &
          nint(phase_raw(n) / 1.5707963) * 1.5707963
  enddo

  ! Adaptive forward-backward exponential smoothing
  if(snr_est .gt. -5.0) then
     alpha_phase = 0.6
  else if(snr_est .gt. -12.0) then
     alpha_phase = 0.4
  else
     alpha_phase = 0.25
  endif

  phase_smooth(1) = phase_raw(1)
  do n = 2, NSYM
     phase_smooth(n) = alpha_phase * phase_raw(n) + &
          (1.0 - alpha_phase) * phase_smooth(n-1)
  enddo
  do n = NSYM - 1, 1, -1
     phase_smooth(n) = 0.5 * phase_smooth(n) + &
          0.5 * (alpha_phase * phase_raw(n) + &
          (1.0 - alpha_phase) * phase_smooth(n+1))
  enddo

  ! Apply residual correction to cd_rot
  do i = 1, npts
     sym_pos = real(i - 1) / nsps_down_real
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
       branch_metrics_all, NSYM, 0)

  ! Re-apply sync forcing
  do n = 1, NSYM
     if(is_data(n) .eq. 0 .and. sync_sym(n) .ge. 0) then
        call force_sync_metrics(branch_metrics_all(:,:,n), sync_sym(n))
     endif
  enddo

  ! Second BCJR pass with corrected branch metrics
  call bcjr_cpm(branch_metrics_all, apriori_llr, ext_bcjr, &
       NSYM, NDATA, is_data, data_idx)

! Deinterleave to LDPC code-bit order
  call ft1_interleave_real(ext_bcjr, llr_out, -1)

  return
end subroutine ft1_demod_bcjr


subroutine ft1_bm_prep(cd, npts, snr_est, rv, bm_out, is_data, data_idx)
!
! Branch-metric preparation for joint turbo HARQ combining.
!
! Factors out the phase-estimation + demod-correct-demod front end of
! ft1_demod_bcjr, returning the *final* (residual-phase-corrected, sync-forced)
! CPM branch metrics and the frame map, WITHOUT running BCJR. A joint-turbo
! caller can then run bcjr_cpm(bm_out, apriori, ...) repeatedly with evolving
! a-priori across outer iterations, paying the (expensive) phase estimation
! only once per received frame.
!
! Equivalence: ft1_bm_prep(...) followed by a single
!   call bcjr_cpm(bm_out, 0.0, ext, 99,87, is_data, data_idx)
!   call ft1_interleave_real(ext, llr_out, -1)
! reproduces ft1_demod_bcjr byte-for-byte (its first BCJR pass is unused — the
! residual-phase estimate is taken from cd_rot correlations, not soft bits).
!
! Input:
!   cd(npts) - complex downsampled baseband (~8 samp/sym); npts samples
!   snr_est  - SNR estimate (dB, 2500 Hz BW)
!   rv       - redundancy version (0,1,2) -> selects Costas variant
! Output:
!   bm_out(NSTATES,0:3,99) - corrected, sync-forced branch metrics
!   is_data(99), data_idx(99) - frame map (data vs sync, payload index)

  use cpm_trellis_mod
  use matched_filter_bank_mod
  implicit none

  integer, intent(in)  :: npts
  complex, intent(in)  :: cd(npts)
  real, intent(in)     :: snr_est
  integer, intent(in)  :: rv
  real, intent(out)    :: bm_out(NSTATES, 0:3, 99)
  integer, intent(out) :: is_data(99)
  integer, intent(out) :: data_idx(99)

  integer, parameter :: NSYM = 99
  integer, parameter :: NSS = 8

  integer :: sync_sym(NSYM)
  integer :: icos(4)
  integer :: icos_rv0(4), icos_rv1(4), icos_rv2(4)

  complex :: cd_rot(npts)
  complex :: corr_sync, phase_sum, best_sum
  complex :: c_ref, c_derot
  real    :: phi(3)
  real    :: best_mag, mag_try, r_align, best_real
  real    :: phase_interp, sym_pos, frac, dphi
  real    :: nsps_down_real, sigma2, snr_linear
  integer :: s_sync, j_sync, k_sync, idx_sync, idx_start_sync
  integer :: s_try, igrp, chan_start(3), k_phase
  integer :: i, n

  complex :: corr_dd, corr_persym(NSYM)
  real    :: phase_raw(NSYM), phase_smooth(NSYM), alpha_phase
  integer :: idx_dd, k_dd, n_dd, i_sym

  data icos_rv0/0,2,3,1/
  data icos_rv1/1,3,2,0/
  data icos_rv2/3,0,2,1/

  select case(rv)
  case(0); icos = icos_rv0
  case(1); icos = icos_rv1
  case(2); icos = icos_rv2
  case default; icos = icos_rv0
  end select

  call init_cpm_trellis()
  call init_matched_filters(NSS)

  is_data = 0
  data_idx = 0
  sync_sym = -1

  sync_sym(1)  = icos(1)
  sync_sym(2)  = icos(2)
  sync_sym(3)  = icos(3)
  sync_sym(4)  = icos(4)
  do i = 5, 47
     is_data(i) = 1
     data_idx(i) = i - 4
  enddo
  sync_sym(48) = icos(1)
  sync_sym(49) = icos(2)
  sync_sym(50) = icos(3)
  sync_sym(51) = icos(4)
  do i = 52, 95
     is_data(i) = 1
     data_idx(i) = i - 8
  enddo
  sync_sym(96) = icos(1)
  sync_sym(97) = icos(2)
  sync_sym(98) = icos(3)
  sync_sym(99) = icos(4)

  snr_linear = 10.0**(snr_est / 10.0) * (2500.0 / 180.0)
  sigma2 = 1.0 / max(1.0 + snr_linear, 1.0e-6)

  nsps_down_real = 3000.0 / (7.0 * 54.0)
  chan_start = (/0, 47, 95/)

! Group 1: known start state (state 1, theta=0)
  phase_sum = cmplx(0.0, 0.0)
  s_sync = 1
  do j_sync = 1, 4
     idx_start_sync = nint(real(chan_start(1) + j_sync - 1) * &
          nsps_down_real) + 1
     corr_sync = cmplx(0.0, 0.0)
     do k_sync = 1, NSS
        idx_sync = idx_start_sync + k_sync - 1
        if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
           corr_sync = corr_sync + cd(idx_sync) * &
                conjg(mf_bank(k_sync, s_sync, icos(j_sync)))
        endif
     enddo
     phase_sum = phase_sum + corr_sync
     s_sync = next_state(s_sync, icos(j_sync))
  enddo
  phi(1) = atan2(aimag(phase_sum), real(phase_sum))

! Groups 2-3: search correlative states, resolve pi/2 ambiguity
  do igrp = 2, 3
     best_mag = 0.0
     best_sum = cmplx(0.0, 0.0)
     do s_try = 1, 16
        phase_sum = cmplx(0.0, 0.0)
        s_sync = s_try
        do j_sync = 1, 4
           idx_start_sync = nint(real(chan_start(igrp) + j_sync - 1) * &
                nsps_down_real) + 1
           corr_sync = cmplx(0.0, 0.0)
           do k_sync = 1, NSS
              idx_sync = idx_start_sync + k_sync - 1
              if(idx_sync .ge. 1 .and. idx_sync .le. npts) then
                 corr_sync = corr_sync + cd(idx_sync) * &
                      conjg(mf_bank(k_sync, s_sync, icos(j_sync)))
              endif
           enddo
           phase_sum = phase_sum + corr_sync
           s_sync = next_state(s_sync, icos(j_sync))
        enddo
        mag_try = abs(phase_sum)
        if(mag_try .gt. best_mag) then
           best_mag = mag_try
           best_sum = phase_sum
        endif
     enddo

     c_ref = cmplx(cos(phi(igrp-1)), sin(phi(igrp-1)))
     best_real = -1.0e30
     phi(igrp) = phi(igrp-1)
     do k_phase = 0, 3
        select case(k_phase)
           case(0); c_derot = best_sum
           case(1); c_derot = cmplx(aimag(best_sum), -real(best_sum))
           case(2); c_derot = cmplx(-real(best_sum), -aimag(best_sum))
           case(3); c_derot = cmplx(-aimag(best_sum), real(best_sum))
        end select
        r_align = real(c_derot * conjg(c_ref))
        if(r_align .gt. best_real) then
           best_real = r_align
           phi(igrp) = atan2(aimag(c_derot), real(c_derot))
        endif
     enddo
  enddo

! Unwrap phase
  dphi = phi(2) - phi(1)
  if(dphi .gt. 3.14159265) phi(2) = phi(2) - 6.28318530
  if(dphi .lt. -3.14159265) phi(2) = phi(2) + 6.28318530
  dphi = phi(3) - phi(2)
  if(dphi .gt. 3.14159265) phi(3) = phi(3) - 6.28318530
  if(dphi .lt. -3.14159265) phi(3) = phi(3) + 6.28318530

! Piecewise-linear phase interpolation
  do i = 1, npts
     sym_pos = real(i - 1) / nsps_down_real
     if(sym_pos .le. 48.5) then
        frac = (sym_pos - 1.5) / (48.5 - 1.5)
        phase_interp = phi(1) + frac * (phi(2) - phi(1))
     else
        frac = (sym_pos - 48.5) / (96.5 - 48.5)
        phase_interp = phi(2) + frac * (phi(3) - phi(2))
     endif
     cd_rot(i) = cd(i) * cmplx(cos(phase_interp), -sin(phase_interp))
  enddo

! Branch metrics (first pass)
  call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
       bm_out, NSYM, 0)
  do n = 1, NSYM
     if(is_data(n) .eq. 0 .and. sync_sym(n) .ge. 0) then
        call force_sync_metrics(bm_out(:,:,n), sync_sym(n))
     endif
  enddo

! Residual per-symbol phase from cd_rot correlations (no soft bits needed)
  do n = 1, NSYM
     idx_dd = nint(real(n - 1) * nsps_down_real) + 1
     best_mag = 0.0
     corr_persym(n) = cmplx(0.0, 0.0)
     do s_try = 1, 16
        do i_sym = 0, 3
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
  enddo

  do n = 1, NSYM
     phase_raw(n) = atan2(aimag(corr_persym(n)), real(corr_persym(n)))
     phase_raw(n) = phase_raw(n) - &
          nint(phase_raw(n) / 1.5707963) * 1.5707963
  enddo

  if(snr_est .gt. -5.0) then
     alpha_phase = 0.6
  else if(snr_est .gt. -12.0) then
     alpha_phase = 0.4
  else
     alpha_phase = 0.25
  endif

  phase_smooth(1) = phase_raw(1)
  do n = 2, NSYM
     phase_smooth(n) = alpha_phase * phase_raw(n) + &
          (1.0 - alpha_phase) * phase_smooth(n-1)
  enddo
  do n = NSYM - 1, 1, -1
     phase_smooth(n) = 0.5 * phase_smooth(n) + &
          0.5 * (alpha_phase * phase_raw(n) + &
          (1.0 - alpha_phase) * phase_smooth(n+1))
  enddo

  do i = 1, npts
     sym_pos = real(i - 1) / nsps_down_real
     n = int(sym_pos) + 1
     if(n .lt. 1) n = 1
     if(n .ge. NSYM) then
        phase_interp = phase_smooth(NSYM)
     else
        frac = sym_pos - real(n - 1)
        phase_interp = phase_smooth(n) + &
             frac * (phase_smooth(n+1) - phase_smooth(n))
     endif
     cd_rot(i) = cd_rot(i) * cmplx(cos(phase_interp), -sin(phase_interp))
  enddo

! Branch metrics (corrected) + re-force sync -> final bm_out
  call compute_branch_metrics_all(cd_rot, npts, NSS, sigma2, &
       bm_out, NSYM, 0)
  do n = 1, NSYM
     if(is_data(n) .eq. 0 .and. sync_sym(n) .ge. 0) then
        call force_sync_metrics(bm_out(:,:,n), sync_sym(n))
     endif
  enddo

  return
end subroutine ft1_bm_prep
