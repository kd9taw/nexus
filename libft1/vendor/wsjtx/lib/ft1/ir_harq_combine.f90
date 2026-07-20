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
module ir_harq_combine_mod

! IR-HARQ joint-turbo combining for FT1.
!
! Manages per-signal RAW BASEBAND buffers across T/R periods. When a decode
! fails on RV0, the RV-aware-synced complex baseband (cd) of that frame is
! stored. When RV1 (or RV2) arrives at the same frequency, the buffered RV
! frames are combined by ft1_joint_turbo_harq (turbo_decode_ft1.f90), which
! re-demodulates each frame's cd jointly with the combined LDPC(261/348) code
! — recovering the coherent turbo gain the old single-BCJR LLR combine threw
! away (+1.3 dB AWGN / +3.2 dB fading on the 3-TX threshold; see WS-A0).
!
! Buffer key: (frequency_bin, T/R parity) with ±10 Hz matching tolerance.
! Expiry: 30 seconds (7-8 T/R periods at 4.0s).
!
! Reference: specs/3d_protocol.md Section 8.7
!            ir_harq_protocol_design.md Section 3.3-3.6

  use ldpc348_91_mod
  implicit none

  integer, parameter :: MAX_HARQ_SLOTS = 100    !Max simultaneous in-progress decodes
  integer, parameter :: FREQ_TOL_HZ = 10        !Frequency matching tolerance (Hz)
  integer, parameter :: EXPIRY_MS = 30000        !Buffer expiry (30 seconds)
  integer, parameter :: HARQ_NDMAX = 888         !Downsampled frame length (NMAX/NDOWN)
  integer, parameter :: HARQ_NITER = 5           !Joint turbo outer iters (compute sweet spot)

  type :: harq_slot
     logical :: active = .false.
     real :: freq = 0.0                          !Signal frequency (Hz) of the RV0 frame
     integer :: ibest = 0                        !RV0 frame timing (downsampled samples).
                                                 !Anchors RV1/RV2 detection+sync: later RVs
                                                 !of a QSO arrive at the same slot alignment.
     integer :: rv_count = -1                    !Highest RV stored (0, 1, or 2)
     integer :: timestamp_ms = 0                 !Timestamp of last update
     real :: snr_est = -99.0                     !SNR (dB) used for joint demod
     integer :: npts = HARQ_NDMAX                !Valid samples in the cd buffers
     complex :: cd_rv0(HARQ_NDMAX)               !RV0 frame baseband (offset-0 aligned)
     complex :: cd_rv1(HARQ_NDMAX)               !RV1 frame baseband
     complex :: cd_rv2(HARQ_NDMAX)               !RV2 frame baseband
  end type harq_slot

  type(harq_slot) :: slots(MAX_HARQ_SLOTS)
  logical :: harq_initialized = .false.

contains

  subroutine harq_init()
    implicit none
    integer :: i
    do i = 1, MAX_HARQ_SLOTS
       slots(i)%active = .false.
       slots(i)%rv_count = -1
    enddo
    harq_initialized = .true.
  end subroutine harq_init


  subroutine harq_store_rv0(freq, cd, npts, snr_est, ibest, timestamp_ms)
  ! Store the RV0 frame's complex baseband (offset-0 aligned) + its frequency and
  ! timing (ibest), for later joint-turbo combining when RV1/RV2 arrive. The
  ! stored (freq, ibest) anchor the RV-aware detection+sync of those later frames.

    implicit none
    real, intent(in) :: freq
    integer, intent(in) :: npts
    complex, intent(in) :: cd(npts)
    real, intent(in) :: snr_est
    integer, intent(in) :: ibest
    integer, intent(in) :: timestamp_ms
    integer :: islot, nc

    if(.not.harq_initialized) call harq_init()

    ! Find existing slot for this frequency or allocate new one
    islot = find_slot(freq, timestamp_ms)
    if(islot .le. 0) then
       islot = allocate_slot(timestamp_ms)
       if(islot .le. 0) return          !Buffer full
    endif

    nc = min(npts, HARQ_NDMAX)
    slots(islot)%active = .true.
    slots(islot)%freq = freq
    slots(islot)%ibest = ibest
    slots(islot)%rv_count = 0
    slots(islot)%timestamp_ms = timestamp_ms
    slots(islot)%snr_est = snr_est
    slots(islot)%npts = nc
    slots(islot)%cd_rv0 = (0.0, 0.0)
    slots(islot)%cd_rv0(1:nc) = cd(1:nc)

  end subroutine harq_store_rv0


  subroutine harq_lookup(freq, timestamp_ms, islot, freq_out, ibest_out, rvcount_out)
  ! Look up an active (unexpired) HARQ slot near `freq`. Returns its stored RV0
  ! frequency and timing so the caller can re-reference an incoming RV1/RV2 frame
  ! to the RV0 anchor before RV detection + joint combining. islot<=0 if none.

    implicit none
    real, intent(in)     :: freq
    integer, intent(in)  :: timestamp_ms
    integer, intent(out) :: islot, ibest_out, rvcount_out
    real, intent(out)    :: freq_out

    freq_out = 0.0; ibest_out = 0; rvcount_out = -1
    if(.not.harq_initialized) then
       islot = 0
       return
    endif
    islot = find_slot(freq, timestamp_ms)
    if(islot .gt. 0) then
       freq_out    = slots(islot)%freq
       ibest_out   = slots(islot)%ibest
       rvcount_out = slots(islot)%rv_count
    endif

  end subroutine harq_lookup


  subroutine harq_combine_rv1(freq, cd, npts, snr_est, timestamp_ms, &
       message77, nharderror, decode_ok)
  ! Buffer the RV1 frame's baseband and joint-turbo-combine it with the stored
  ! RV0 frame (LDPC(261,91)) via ft1_joint_turbo_harq.

    implicit none
    real, intent(in) :: freq
    integer, intent(in) :: npts
    complex, intent(in) :: cd(npts)
    real, intent(in) :: snr_est
    integer, intent(in) :: timestamp_ms
    integer*1, intent(out) :: message77(77)
    integer, intent(out) :: nharderror
    logical, intent(out) :: decode_ok

    complex :: cd_dummy(HARQ_NDMAX)
    integer :: islot, nc, np
    real :: snr_use

    decode_ok = .false.
    nharderror = -1
    message77 = 0

    if(.not.harq_initialized) call harq_init()

    islot = find_slot(freq, timestamp_ms)
    if(islot .le. 0) return             !No stored RV0 for this frequency
    if(slots(islot)%rv_count .lt. 0) return

    nc = min(npts, HARQ_NDMAX)
    slots(islot)%cd_rv1 = (0.0, 0.0)
    slots(islot)%cd_rv1(1:nc) = cd(1:nc)
    slots(islot)%rv_count = 1
    slots(islot)%timestamp_ms = timestamp_ms

    ! Joint iterative turbo HARQ: RV0 + RV1 (re-demod each frame's cd jointly).
    ! Use the more conservative (lower) SNR of the two frames for noise variance.
    snr_use = min(slots(islot)%snr_est, snr_est)
    np = min(slots(islot)%npts, nc)
    cd_dummy = (0.0, 0.0)
    call ft1_joint_turbo_harq(slots(islot)%cd_rv0, slots(islot)%cd_rv1, &
         cd_dummy, np, snr_use, 2, message77, nharderror, HARQ_NITER)

    if(nharderror .ge. 0) then
       decode_ok = .true.
       call harq_clear_slot(islot)
    endif

  end subroutine harq_combine_rv1


  subroutine harq_combine_rv2(freq, cd, npts, snr_est, timestamp_ms, &
       message77, nharderror, decode_ok)
  ! Buffer the RV2 frame's baseband and joint-turbo-combine RV0+RV1+RV2
  ! (LDPC(348,91)) via ft1_joint_turbo_harq.

    implicit none
    real, intent(in) :: freq
    integer, intent(in) :: npts
    complex, intent(in) :: cd(npts)
    real, intent(in) :: snr_est
    integer, intent(in) :: timestamp_ms
    integer*1, intent(out) :: message77(77)
    integer, intent(out) :: nharderror
    logical, intent(out) :: decode_ok

    integer :: islot, nc, np
    real :: snr_use

    decode_ok = .false.
    nharderror = -1
    message77 = 0

    if(.not.harq_initialized) call harq_init()

    islot = find_slot(freq, timestamp_ms)
    if(islot .le. 0) return
    if(slots(islot)%rv_count .lt. 1) return   !Need at least RV0+RV1 stored

    nc = min(npts, HARQ_NDMAX)
    slots(islot)%cd_rv2 = (0.0, 0.0)
    slots(islot)%cd_rv2(1:nc) = cd(1:nc)
    slots(islot)%rv_count = 2
    slots(islot)%timestamp_ms = timestamp_ms

    ! Joint iterative turbo HARQ: RV0 + RV1 + RV2.
    snr_use = min(slots(islot)%snr_est, snr_est)
    np = min(slots(islot)%npts, nc)
    call ft1_joint_turbo_harq(slots(islot)%cd_rv0, slots(islot)%cd_rv1, &
         slots(islot)%cd_rv2, np, snr_use, 3, message77, nharderror, HARQ_NITER)

    if(nharderror .ge. 0) decode_ok = .true.

    ! Always clear slot after RV2 (final attempt)
    call harq_clear_slot(islot)

  end subroutine harq_combine_rv2


  subroutine harq_expire(current_time_ms)
  ! Expire stale buffers older than EXPIRY_MS.

    implicit none
    integer, intent(in) :: current_time_ms
    integer :: i

    if(.not.harq_initialized) return

    do i = 1, MAX_HARQ_SLOTS
       if(slots(i)%active) then
          if(current_time_ms - slots(i)%timestamp_ms .gt. EXPIRY_MS) then
             call harq_clear_slot(i)
          endif
       endif
    enddo

  end subroutine harq_expire


  ! === Internal helper functions ===

  integer function find_slot(freq, timestamp_ms)
  ! Find an active slot matching the given frequency within tolerance.

    implicit none
    real, intent(in) :: freq
    integer, intent(in) :: timestamp_ms
    integer :: i

    find_slot = 0
    do i = 1, MAX_HARQ_SLOTS
       if(slots(i)%active) then
          if(abs(slots(i)%freq - freq) .le. real(FREQ_TOL_HZ)) then
             ! Check not expired
             if(timestamp_ms - slots(i)%timestamp_ms .le. EXPIRY_MS) then
                find_slot = i
                return
             else
                call harq_clear_slot(i)   !Expired, clear it
             endif
          endif
       endif
    enddo

  end function find_slot


  integer function allocate_slot(timestamp_ms)
  ! Find a free slot, or recycle the oldest expired one.

    implicit none
    integer, intent(in) :: timestamp_ms
    integer :: i, oldest_slot, oldest_time

    allocate_slot = 0

    ! First pass: find inactive slot
    do i = 1, MAX_HARQ_SLOTS
       if(.not.slots(i)%active) then
          allocate_slot = i
          return
       endif
    enddo

    ! Second pass: recycle oldest slot
    oldest_slot = 1
    oldest_time = slots(1)%timestamp_ms
    do i = 2, MAX_HARQ_SLOTS
       if(slots(i)%timestamp_ms .lt. oldest_time) then
          oldest_time = slots(i)%timestamp_ms
          oldest_slot = i
       endif
    enddo
    call harq_clear_slot(oldest_slot)
    allocate_slot = oldest_slot

  end function allocate_slot


  subroutine harq_clear_slot(islot)
    implicit none
    integer, intent(in) :: islot
    slots(islot)%active = .false.
    slots(islot)%rv_count = -1
    slots(islot)%freq = 0.0
    slots(islot)%ibest = 0
    slots(islot)%timestamp_ms = 0
    slots(islot)%snr_est = -99.0
    slots(islot)%npts = HARQ_NDMAX
    slots(islot)%cd_rv0 = (0.0, 0.0)
    slots(islot)%cd_rv1 = (0.0, 0.0)
    slots(islot)%cd_rv2 = (0.0, 0.0)
  end subroutine harq_clear_slot

end module ir_harq_combine_mod
