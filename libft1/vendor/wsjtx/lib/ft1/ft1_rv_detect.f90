! FT1 - 4-CPM turbo equalization mode for WSJT-X
! Copyright (C) 2026 Seth McCall, KD9TAW
!
! This file is part of WSJT-X.  GPLv3 (see genft1.f90 header).
!
subroutine ft1_rv_detect(cd, npts, i0_hint, irv_det)

! Coherent IR-HARQ redundancy-version (RV) detector.
!
! Given a downconverted FT1 frame `cd` (complex baseband, ~8 samples/symbol,
! frame start near sample i0_hint), return irv_det in {0,1,2}: the redundancy
! version whose Costas sync arrays best match the received frame.
!
! WHY coherent: the coarse spectrogram (ft1_sync) cannot resolve the 14 Hz tone
! spacing (df=11.7 Hz/bin) so it cannot discriminate the RV Costas patterns
! (which are permutations of the same per-group tone set) -- its RV tag is wrong
! 40-70% of the time. A coherent CPM matched-filter correlation resolves the
! tones via the known phase trajectory.
!
! METRIC (per RV, at timing i0): G1 uses the known frame-start trellis state
! (clean, no search bias). G2/G3 entering states are data-dependent, handled by
! SUMMING |corr|^2 over the 16 theta=0 states (an incoherent combine). Summing
! -- not max -- is the key: a max-over-states GLRT adds a positive noise-peak
! bias that is larger for the WRONG RV, collapsing discrimination at low SNR;
! the sum's bias is common to all RVs and cancels in the comparison.
!
! DECISION: argmax over RV of the best metric in a +-0.5 symbol window around
! i0_hint, with a confidence margin -- accept the winner only if it exceeds the
! runner-up by RV_MARGIN, else fall back to RV0 (the safe default: store as a
! fresh RV0 rather than mis-combine).
!
! TIMING CONTRACT: the frame start must be within ~+-0.5 symbol of i0_hint.
! Coherent RV discrimination is timing-sensitive (a wider search lets a wrong RV
! find a spurious alignment). The IR-HARQ state machine supplies accurate timing
! by anchoring on the stored RV0 frame's timing -- RV0/RV1/RV2 of one QSO arrive
! at the same slot alignment.
!
! Validated: >99% accuracy and <1% false "RV0 tagged as RVk" for AWGN down to
! about -11 dB (ft1_rv_detect_test.f90). Requires init_cpm_trellis() and
! init_matched_filters(NSS) to have run (turbo_decode_ft1 does this before any
! HARQ path executes).

  use cpm_trellis_mod, only: next_state
  use matched_filter_bank_mod, only: mf_bank
  implicit none
  integer, intent(in)  :: npts, i0_hint
  complex, intent(in)  :: cd(0:npts-1)
  integer, intent(out) :: irv_det

  ! FT1 frame geometry (mirrors ft1_params.f90; declared explicitly because that
  ! include relies on implicit integer typing, incompatible with implicit none).
  integer, parameter :: NSS = 8           ! downsampled samples per symbol
  integer, parameter :: NSPS_NUM = 3000   ! samples/symbol numerator (12 kHz)
  integer, parameter :: NSPS_DEN = 7
  integer, parameter :: NDOWN = 54        ! downsample factor

  real, parameter :: RV_MARGIN = 1.10     ! winner must exceed runner-up by this
  real, parameter :: WIN_SYM   = 0.5      ! timing search half-window (symbols)
  integer :: icos_rv(0:3,0:2)
  real    :: smet(0:2), best, second, v, nsps_dn
  integer :: rv, i0, i0lo, i0hi, iwin

  icos_rv(0:3,0)=(/0,2,3,1/)
  icos_rv(0:3,1)=(/1,3,2,0/)
  icos_rv(0:3,2)=(/3,0,2,1/)

  nsps_dn = real(NSPS_NUM)/(real(NSPS_DEN)*real(NDOWN))
  i0lo = i0_hint - nint(WIN_SYM*nsps_dn)
  i0hi = i0_hint + nint(WIN_SYM*nsps_dn)

  do rv=0,2
     best=0.0
     do i0=i0lo,i0hi
        v = rv_metric(i0, icos_rv(0:3,rv))
        if(v.gt.best) best=v
     enddo
     smet(rv)=best
  enddo

  iwin=0
  if(smet(1).gt.smet(iwin)) iwin=1
  if(smet(2).gt.smet(iwin)) iwin=2
  second=-1.0
  do rv=0,2
     if(rv.ne.iwin .and. smet(rv).gt.second) second=smet(rv)
  enddo
  if(smet(iwin) .lt. RV_MARGIN*second) iwin=0    ! ambiguous -> RV0 (safe)
  irv_det=iwin

contains

  real function rv_metric(i0, icos)
  ! Coherent CPM Costas detection metric at timing i0 for Costas `icos`:
  ! |G1|^2 (known start state) + sum over 16 states of |corr|^2 for G2 and G3.
    integer, intent(in) :: i0, icos(0:3)
    complex :: z
    real    :: m, ssum, ndn
    integer :: i1, i2, i3, s, ig, grp
    ndn = real(NSPS_NUM)/(real(NSPS_DEN)*real(NDOWN))
    i1 = i0
    i2 = i0 + nint(47.0*ndn)
    i3 = i0 + nint(95.0*ndn)
    z = corr_costas(i1, icos, 1)              ! G1: known frame-start state
    m = real(z)**2 + aimag(z)**2
    do ig=1,2
       if(ig.eq.1) grp=i2
       if(ig.eq.2) grp=i3
       ssum=0.0
       do s=1,16                              ! theta=0 correlative states
          z=corr_costas(grp, icos, s)
          ssum=ssum + real(z)**2 + aimag(z)**2
       enddo
       m=m+ssum
    enddo
    rv_metric=m
  end function rv_metric

  complex function corr_costas(istart, icos, s0)
  ! Coherent correlation of 4 Costas sync symbols starting at trellis state s0.
    integer, intent(in) :: istart, icos(0:3), s0
    complex :: csync(4*NSS), z
    integer :: s_state, k, i, j
    real    :: scale_amp
    scale_amp = sqrt(real(NSS))
    s_state = s0; k = 1
    do i=0,3
       do j=1,NSS
          csync(k) = scale_amp*mf_bank(j, s_state, icos(i)); k = k+1
       enddo
       s_state = next_state(s_state, icos(i))
    enddo
    z = cmplx(0.0,0.0)
    if(istart.ge.0 .and. istart+4*NSS-1.le.npts-1) then
       z = sum(cd(istart:istart+4*NSS-1)*conjg(csync(1:4*NSS)))
    endif
    corr_costas = z
  end function corr_costas

end subroutine ft1_rv_detect
