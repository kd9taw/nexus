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
module cpm_trellis_mod
!
! CPM trellis definition and precomputed tables for FT1
! 4-CPM h=1/2 BT=0.3 L=3
!
! Trellis has 64 states = 4 phase states x 16 correlative states
! State encoding: s = theta*16 + sigma_1*4 + sigma_2 + 1  (1-based)
!   theta   in {0,...,3} -- tilted phase state (actual phase = theta * pi/2)
!   sigma_1 in {0,1,2,3} -- most recent symbol (integer convention)
!   sigma_2 in {0,1,2,3} -- second most recent symbol (integer convention)
!
! Symbol convention: integer {0,1,2,3} matching the transmitter (gen_ft1wave).
! Phase change per completed symbol: pi * h * a = pi/2 * a
! In units of pi/2: delta_theta = a (where a in {0,1,2,3})
!
! Gray code bit mapping (for LLR computation):
!   symbol 0: bits (0,0)
!   symbol 1: bits (0,1)
!   symbol 2: bits (1,1)
!   symbol 3: bits (1,0)
!

  implicit none

  integer, parameter :: NSTATES = 64    ! Total trellis states (4 x 16)
  integer, parameter :: M_CPM = 4       ! Alphabet size
  integer, parameter :: P_PHASE = 4     ! Number of phase states
  integer, parameter :: NCORR = 16      ! Correlative states (M^(L-1) = 4^2)

! Gray code bit map: bit_map(symbol_index, bit_position)
!   bit_map(i,0) = MSB, bit_map(i,1) = LSB
  integer, parameter :: bit_map(0:3,0:1) = reshape( &
       (/ 0, 0, 1, 1,  &   ! bit 0 (MSB) for symbols 0,1,2,3
          0, 1, 1, 0 /), &  ! bit 1 (LSB) for symbols 0,1,2,3
       (/ 4, 2 /))

! Trellis connectivity tables (filled by init_cpm_trellis)
  integer :: next_state(NSTATES,0:3)    ! next_state(s, input_sym) = successor state
  integer :: prev_state(NSTATES,0:3)    ! prev_state(s, input_sym) = predecessor state

! Reverse lookup: for each state, which (state,symbol) pairs lead into it
  integer :: ntrans_to(NSTATES)         ! number of transitions into each state
  integer :: from_state(NSTATES,M_CPM)  ! which states lead into state s
  integer :: from_symbol(NSTATES,M_CPM) ! which input symbols cause those transitions

! Phase output for each transition (theta value for branch waveform)
  integer :: phase_out(NSTATES,0:3)     ! starting theta for branch waveform

  logical :: trellis_initialized = .false.

contains

subroutine init_cpm_trellis()
!
! Precompute all trellis connectivity tables.
! Must be called once before using the BCJR or matched filter bank.
!
  integer :: s, i_n, theta, sigma_1, sigma_2
  integer :: theta_next, s_next

  if(trellis_initialized) return

! Initialize reverse lookup counts
  ntrans_to = 0

! Build forward and reverse connectivity
  do s = 1, NSTATES
     ! Decode state components (1-based state index, 0-based components)
     theta   = (s - 1) / 16           ! 0..7
     sigma_1 = mod((s - 1) / 4, 4)   ! 0..3
     sigma_2 = mod(s - 1, 4)          ! 0..3

     ! Phase update: absorb sigma_2 (oldest correlative symbol) into tilted phase
     ! Phase change = pi/2 * sigma_2 (integer convention, h=1/2)
     ! In theta units (pi/2 each): delta = sigma_2
     theta_next = mod(theta + sigma_2, P_PHASE)

     do i_n = 0, 3
        ! Next state: (theta_next, i_n, sigma_1)
        ! sigma_1' = i_n, sigma_2' = sigma_1
        s_next = theta_next * 16 + i_n * 4 + sigma_1 + 1  ! +1 for 1-based

        next_state(s, i_n) = s_next

        ! Store the phase state for this transition's branch waveform
        phase_out(s, i_n) = theta

        ! Build reverse lookup
        ntrans_to(s_next) = ntrans_to(s_next) + 1
        from_state(s_next, ntrans_to(s_next)) = s
        from_symbol(s_next, ntrans_to(s_next)) = i_n
     enddo
  enddo

! Build prev_state by inverting next_state
  prev_state = 0
  do s = 1, NSTATES
     do i_n = 0, 3
        s_next = next_state(s, i_n)
        prev_state(s_next, i_n) = s
     enddo
  enddo

  trellis_initialized = .true.
  return
end subroutine init_cpm_trellis

function get_theta(s) result(theta)
! Extract phase state from state index (1-based)
  integer, intent(in) :: s
  integer :: theta
  theta = (s - 1) / 16
end function get_theta

function get_sigma1(s) result(sigma_1)
! Extract most recent correlative symbol index from state index (1-based)
  integer, intent(in) :: s
  integer :: sigma_1
  sigma_1 = mod((s - 1) / 4, 4)
end function get_sigma1

function get_sigma2(s) result(sigma_2)
! Extract second most recent correlative symbol index from state index (1-based)
  integer, intent(in) :: s
  integer :: sigma_2
  sigma_2 = mod(s - 1, 4)
end function get_sigma2

end module cpm_trellis_mod
