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
subroutine bcjr_cpm(branch_metrics, apriori_llr, extrinsic_llr, &
     nsym, ndata, is_data, data_idx)
!
! BCJR algorithm on the full 64-state CPM trellis for FT1.
!
! Processes ALL nsym=99 trellis sections (data + sync) for proper
! trellis continuity. The sync symbols constrain the trellis path
! through forced branch metrics, acting as pilot-aided anchoring.
!
! Uses known start state (state 1: theta=0, sigma_1=0, sigma_2=0)
! for forward initialization instead of uniform prior.
!
! Extrinsic LLRs are extracted only at data positions (87 symbols,
! 174 coded bits).
!
! Interface:
!   branch_metrics(64, 0:3, nsym): channel branch metrics for all positions
!     Sync positions should have 0.0 for known symbol, NEG_INF for others
!   apriori_llr(2*ndata):   a priori LLRs on coded bits (174)
!   extrinsic_llr(2*ndata): output extrinsic LLRs (174 bits)
!   nsym:                   total symbols (99)
!   ndata:                  data symbols (87)
!   is_data(nsym):          1 for data positions, 0 for sync
!   data_idx(nsym):         data symbol index (1..87) at data positions, 0 at sync
!

  use cpm_trellis_mod
  implicit none

  integer, intent(in) :: nsym, ndata
  real, intent(in)    :: branch_metrics(NSTATES, 0:3, nsym)
  real, intent(in)    :: apriori_llr(2*ndata)
  integer, intent(in) :: is_data(nsym)
  integer, intent(in) :: data_idx(nsym)
  real, intent(out)   :: extrinsic_llr(2*ndata)

! Local arrays
  real :: alpha_m(NSTATES, 0:nsym)    ! Forward state metrics (log domain)
  real :: beta_m(NSTATES)              ! Backward state metrics (current section)
  real :: beta_next(NSTATES)           ! Backward state metrics (next section)
  real :: gamma_m(NSTATES, 0:3)       ! Branch metrics with a priori (one section)
  real :: metric                       ! Temporary metric value
  real :: amax, bmax                   ! For normalization
  real :: num, den                     ! Numerator/denominator for LLR
  real :: l_app, log_pa               ! APP LLR, a priori log-probability
  real, parameter :: LMAX = 30.0       ! LLR clipping threshold
  real, parameter :: NEG_INF = -1.0e30 ! Log-domain minus infinity

  integer :: n, s, i_n, j, k, s_next, bit_val, di

! ================================================================
! Forward recursion (alpha)
! ================================================================

! Known start state: state 1 (theta=0, sigma_1=0, sigma_2=0)
  do s = 1, NSTATES
     alpha_m(s, 0) = NEG_INF
  enddo
  alpha_m(1, 0) = 0.0

! Forward pass: n = 1..nsym
  do n = 1, nsym

     ! Build branch metrics with a priori for section n
     do s = 1, NSTATES
        do i_n = 0, 3
           gamma_m(s, i_n) = branch_metrics(s, i_n, n)

           ! Add a priori contribution only at data positions
           if(is_data(n) .eq. 1) then
              di = data_idx(n)
              log_pa = 0.0
              do j = 0, 1
                 bit_val = bit_map(i_n, j)
                 k = 2*(di-1) + j + 1    ! 1-based bit index
                 log_pa = log_pa + (2.0*bit_val - 1.0) * apriori_llr(k) * 0.5
              enddo
              gamma_m(s, i_n) = gamma_m(s, i_n) + log_pa
           endif
        enddo
     enddo

     ! Compute alpha[n][s'] = max*_{(s, i_n) -> s'} (alpha[n-1][s] + gamma[s][i_n])
     do s = 1, NSTATES
        alpha_m(s, n) = NEG_INF
     enddo

     do s = 1, NSTATES
        do i_n = 0, 3
           s_next = next_state(s, i_n)
           metric = alpha_m(s, n-1) + gamma_m(s, i_n)
           alpha_m(s_next, n) = maxstar(alpha_m(s_next, n), metric)
        enddo
     enddo

     ! Normalize to prevent overflow
     amax = alpha_m(1, n)
     do s = 2, NSTATES
        if(alpha_m(s, n) .gt. amax) amax = alpha_m(s, n)
     enddo
     do s = 1, NSTATES
        alpha_m(s, n) = alpha_m(s, n) - amax
     enddo

  enddo  ! Forward pass

! ================================================================
! Backward recursion (beta) + LLR computation
! Combined: compute beta backward and extract LLRs at data positions
! ================================================================

! Initialization: uniform prior (unknown final state)
  do s = 1, NSTATES
     beta_next(s) = 0.0
  enddo

! Backward pass: n = nsym down to 1
  do n = nsym, 1, -1

     ! Rebuild branch metrics with a priori for section n
     do s = 1, NSTATES
        do i_n = 0, 3
           gamma_m(s, i_n) = branch_metrics(s, i_n, n)
           if(is_data(n) .eq. 1) then
              di = data_idx(n)
              log_pa = 0.0
              do j = 0, 1
                 bit_val = bit_map(i_n, j)
                 k = 2*(di-1) + j + 1
                 log_pa = log_pa + (2.0*bit_val - 1.0) * apriori_llr(k) * 0.5
              enddo
              gamma_m(s, i_n) = gamma_m(s, i_n) + log_pa
           endif
        enddo
     enddo

     ! Compute beta[n-1][s] = max*_{i_n=0..3} (gamma[s][i_n] + beta[n][next_state(s,i_n)])
     do s = 1, NSTATES
        beta_m(s) = NEG_INF
        do i_n = 0, 3
           s_next = next_state(s, i_n)
           metric = gamma_m(s, i_n) + beta_next(s_next)
           beta_m(s) = maxstar(beta_m(s), metric)
        enddo
     enddo

     ! Normalize
     bmax = beta_m(1)
     do s = 2, NSTATES
        if(beta_m(s) .gt. bmax) bmax = beta_m(s)
     enddo
     do s = 1, NSTATES
        beta_m(s) = beta_m(s) - bmax
     enddo

     ! ============================================================
     ! Compute extrinsic LLRs only at DATA positions
     ! ============================================================
     if(is_data(n) .eq. 1) then
        di = data_idx(n)

        do j = 0, 1
           k = 2*(di-1) + j + 1    ! 1-based coded bit index
           num = NEG_INF           ! log P(bit_j = 0)
           den = NEG_INF           ! log P(bit_j = 1)

           do s = 1, NSTATES
              do i_n = 0, 3
                 s_next = next_state(s, i_n)
                 metric = alpha_m(s, n-1) + gamma_m(s, i_n) + beta_next(s_next)

                 if(bit_map(i_n, j) .eq. 0) then
                    num = maxstar(num, metric)
                 else
                    den = maxstar(den, metric)
                 endif
              enddo
           enddo

           ! APP LLR (positive means bit=1 more likely, WSJT-X convention)
           l_app = den - num

           ! Extrinsic = APP - a priori (remove a priori to prevent positive feedback)
           extrinsic_llr(k) = l_app - apriori_llr(k)

           ! Clip to prevent numerical issues
           if(extrinsic_llr(k) .gt. LMAX) extrinsic_llr(k) = LMAX
           if(extrinsic_llr(k) .lt. -LMAX) extrinsic_llr(k) = -LMAX
        enddo
     endif

     ! Shift beta for next iteration
     beta_next = beta_m

  enddo  ! Backward pass

  return

contains

  function maxstar(a, b) result(c)
  ! Jacobian logarithm: log(exp(a) + exp(b))
  ! = max(a, b) + log(1 + exp(-|a - b|))
    real, intent(in) :: a, b
    real :: c
    real :: diff

    if(a .gt. b) then
       diff = a - b
       c = a + log_correction(diff)
    else
       diff = b - a
       c = b + log_correction(diff)
    endif
  end function maxstar

  function log_correction(delta) result(corr)
  ! Compute log(1 + exp(-delta)) for delta >= 0
    real, intent(in) :: delta
    real :: corr

    if(delta .ge. 9.0) then
       corr = 0.0
    else if(delta .ge. 6.0) then
       corr = 0.00248 * (9.0 - delta) / 3.0
    else
       corr = log(1.0 + exp(-delta))
    endif
  end function log_correction

end subroutine bcjr_cpm
