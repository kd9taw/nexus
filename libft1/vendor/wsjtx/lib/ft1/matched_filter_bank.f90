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
module matched_filter_bank_mod
!
! Precomputed matched filter bank for FT1 CPM branch metric computation.
!
! For each of the 256 trellis transitions (64 states x 4 input symbols),
! there is a unique complex baseband waveform spanning one symbol period.
! The waveform depends on the current input symbol plus the L-1=2 previous
! symbols (from the correlative state) and the starting phase (from the
! phase state).
!
! Uses INTEGER symbol convention {0,1,2,3} matching the transmitter.
! Phase states at pi/4 spacing (8 states).
!
! Each filter has NSS_DOWN samples (downsampled rate, nominally 8 samples
! per symbol at ~222 Hz effective rate).
!

  use cpm_trellis_mod
  implicit none

  integer, parameter :: NSS_MAX = 16       ! Max samples per symbol (downsampled)
  integer, parameter :: NPULSE_PER_SYM = 8 ! Default downsampled samples per symbol

! Complex matched filter bank: mf_bank(sample, state, input_symbol)
  complex :: mf_bank(NSS_MAX, NSTATES, 0:3)

  integer :: nss_down = NPULSE_PER_SYM     ! Actual samples per symbol (set by init)

  logical :: mf_initialized = .false.

contains

subroutine init_matched_filters(nsps_down)
!
! Precompute the 512 matched filter waveforms for the CPM trellis.
!
! nsps_down: number of samples per symbol in the downsampled signal
!            (nominally 8 for ~222 Hz effective rate)
!
! For each (state, input_symbol) pair:
!   1. Determine the L=3 symbol sequence [sigma_2, sigma_1, current]
!   2. Generate the CPM phase trajectory for this sequence
!   3. Store the one-symbol-period portion as the matched filter
!
! Uses integer symbol convention: symbols are {0,1,2,3} directly.
! Phase formula: phi(t) = theta*pi/4 + 2*pi*h*(a_n*q(t) + a_{n-1}*q(t+T) + a_{n-2}*q(t+2T))
!
  integer, intent(in) :: nsps_down
  integer :: s, i_n, k
  integer :: theta, sigma_1, sigma_2
  real :: phi_start, phi_k, t_norm
  real, parameter :: PI = 3.141592653589793
  real, parameter :: H_MOD = 0.5         ! Modulation index h = 1/2
  real, parameter :: BT = 0.3            ! Gaussian bandwidth-time product
  real :: energy

  if(mf_initialized) return
  if(.not.trellis_initialized) call init_cpm_trellis()

  nss_down = nsps_down
  if(nss_down .gt. NSS_MAX) then
     write(*,*) 'ERROR: nsps_down exceeds NSS_MAX', nss_down, NSS_MAX
     return
  endif

  mf_bank = cmplx(0.0, 0.0)

  do s = 1, NSTATES
     ! Extract state components
     theta   = get_theta(s)
     sigma_1 = get_sigma1(s)
     sigma_2 = get_sigma2(s)

     ! Starting phase from tilted phase state (pi/2 spacing)
     phi_start = theta * PI / 2.0

     do i_n = 0, 3
        ! Generate one symbol period of the CPM waveform
        ! phi(t) = phi_start + 2*pi*h * [i_n*q(t) + sigma_1*q(t+T) + sigma_2*q(t+2T)]
        ! where q is the phase pulse and t is in symbol periods [0, 1)
        ! Integer symbols used directly (matching transmitter convention)

        do k = 0, nss_down - 1
           ! Normalized time within the symbol: 0 to ~0.875 (in symbol periods)
           t_norm = real(k) / real(nss_down)

           ! Phase from overlapping L=3 pulses (integer symbol values)
           phi_k = phi_start + 2.0*PI*H_MOD * ( &
                i_n     * phase_pulse(t_norm, BT) + &
                sigma_1 * phase_pulse(t_norm + 1.0, BT) + &
                sigma_2 * phase_pulse(t_norm + 2.0, BT))

           ! Complex baseband waveform sample
           mf_bank(k+1, s, i_n) = cmplx(cos(phi_k), sin(phi_k))
        enddo

        ! Normalize matched filter to unit energy
        energy = 0.0
        do k = 1, nss_down
           energy = energy + real(mf_bank(k,s,i_n))**2 + aimag(mf_bank(k,s,i_n))**2
        enddo
        if(energy .gt. 0.0) then
           energy = sqrt(energy)
           do k = 1, nss_down
              mf_bank(k, s, i_n) = mf_bank(k, s, i_n) / energy
           enddo
        endif

     enddo  ! i_n
  enddo  ! s

  mf_initialized = .true.
  return
end subroutine init_matched_filters


function phase_pulse(t, bt) result(q)
!
! Compute the CPM phase pulse q(t) for Gaussian frequency pulse.
!
! q(t) = integral from -L/2 to (t - L/2) of g(tau) d(tau), normalized
!        so that q(L=3) = 0.5 (standard CPM convention).
!
! WSJT-X's gfsk_pulse integrates to ~1.0 (not 0.5), so we compute
! the full integral once and scale all results by 0.5/I_full.
!
  real, intent(in) :: t    ! Time in symbol periods from start of pulse (0 to L=3)
  real, intent(in) :: bt   ! Gaussian BT product
  real :: q
  real, parameter :: PI = 3.141592653589793
  real :: c_const, u, v, dv
  integer :: i, nsteps
  real :: sum_val, gval
  real :: scale_factor

  real, save :: saved_scale = 0.0
  logical, save :: scale_computed = .false.

  c_const = PI * sqrt(2.0 / log(2.0))

  ! Compute the normalization scale factor once (0.5 / I_full)
  if(.not. scale_computed) then
     nsteps = 400
     dv = 3.0 / real(nsteps)
     sum_val = 0.0
     do i = 0, nsteps
        v = -1.5 + i * dv
        gval = gfsk_pulse_eval(bt, c_const, v)
        if(i .eq. 0 .or. i .eq. nsteps) then
           sum_val = sum_val + gval
        else if(mod(i,2) .eq. 1) then
           sum_val = sum_val + 4.0 * gval
        else
           sum_val = sum_val + 2.0 * gval
        endif
     enddo
     saved_scale = 0.5 / (sum_val * dv / 3.0)
     scale_computed = .true.
  endif

  ! Boundary conditions
  if(t .le. 0.0) then
     q = 0.0
     return
  endif
  if(t .ge. 3.0) then
     q = 0.5
     return
  endif

  ! Numerical integration using Simpson's rule
  ! Integrate gfsk_pulse from -1.5 to (t - 1.5)
  nsteps = 200
  u = t - 1.5
  dv = (u - (-1.5)) / real(nsteps)
  if(abs(dv) .lt. 1.0e-10) then
     q = 0.0
     return
  endif

  sum_val = 0.0
  do i = 0, nsteps
     v = -1.5 + i * dv
     gval = gfsk_pulse_eval(bt, c_const, v)
     if(i .eq. 0 .or. i .eq. nsteps) then
        sum_val = sum_val + gval
     else if(mod(i,2) .eq. 1) then
        sum_val = sum_val + 4.0 * gval
     else
        sum_val = sum_val + 2.0 * gval
     endif
  enddo
  q = sum_val * dv / 3.0 * saved_scale

  return
end function phase_pulse


function gfsk_pulse_eval(bt, c_const, t) result(g)
!
! Evaluate the Gaussian frequency pulse at normalized time t.
! g(t) = 0.5 * (erf(c*BT*(t+0.5)) - erf(c*BT*(t-0.5)))
!
  real, intent(in) :: bt, c_const, t
  real :: g

  g = 0.5 * (erf_approx(c_const * bt * (t + 0.5)) - &
              erf_approx(c_const * bt * (t - 0.5)))
  return
end function gfsk_pulse_eval


function erf_approx(x) result(y)
!
! Error function approximation (Abramowitz & Stegun 7.1.26, max error < 1.5e-7)
!
  real, intent(in) :: x
  real :: y
  real :: t, ax, poly
  real, parameter :: p  =  0.3275911
  real, parameter :: a1 =  0.254829592
  real, parameter :: a2 = -0.284496736
  real, parameter :: a3 =  1.421413741
  real, parameter :: a4 = -1.453152027
  real, parameter :: a5 =  1.061405429

  ax = abs(x)
  t = 1.0 / (1.0 + p * ax)
  poly = t * (a1 + t * (a2 + t * (a3 + t * (a4 + t * a5))))
  y = 1.0 - poly * exp(-ax * ax)
  if(x .lt. 0.0) y = -y
  return
end function erf_approx


subroutine compute_branch_metrics(cd, npts, n_data_start, sigma2, &
     branch_metrics, ndata)
!
! Compute channel branch metrics from received signal via matched filtering.
!
  integer, intent(in)  :: npts, n_data_start, ndata
  complex, intent(in)  :: cd(npts)
  real, intent(in)     :: sigma2
  real, intent(out)    :: branch_metrics(NSTATES, 0:3, ndata)

  integer :: n, s, i_n, k, idx
  complex :: corr
  real :: scale

  if(sigma2 .gt. 0.0) then
     scale = 2.0 / sigma2
  else
     scale = 1.0
  endif

  branch_metrics = 0.0

  do n = 1, ndata
     do s = 1, NSTATES
        do i_n = 0, 3
           corr = cmplx(0.0, 0.0)
           do k = 1, nss_down
              idx = n_data_start + (n-1) * nss_down + k
              if(idx .ge. 1 .and. idx .le. npts) then
                 corr = corr + cd(idx) * conjg(mf_bank(k, s, i_n))
              endif
           enddo
           branch_metrics(s, i_n, n) = scale * real(corr)
        enddo
     enddo
  enddo

  return
end subroutine compute_branch_metrics

end module matched_filter_bank_mod
