! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! Non-coherent M-FSK detector.
!
! For each of the 58 data symbols, mix the real audio down to complex
! baseband at f0, take a 1920-pt FFT, and read the energy in each of the 8
! tone bins (bins 0..7, since tone spacing = 6.25 Hz = 1 FFT bin).  Also
! accumulate the energy in a set of off-tone "noise" bins for sigma^2
! estimation downstream.
!
! Output:
!   energy(8, NSYM) : per-symbol tone-bin energies |Y_k|^2
!   noisevar        : mean per-bin noise energy estimated from off-tone bins
! ---------------------------------------------------------------------------
subroutine dx1_detect(dd, ndd, istart, f0, fsample, energy, noisevar)
  use dx1_params
  implicit none
  integer, intent(in)  :: ndd
  real,    intent(in)  :: dd(ndd)          ! raw audio
  integer, intent(in)  :: istart           ! 1-based sample index of first DATA symbol
  real,    intent(in)  :: f0, fsample
  real,    intent(out) :: energy(DX1_M, DX1_NSYM)
  real,    intent(out) :: noisevar

  complex :: c(DX1_NSPS)
  real(8) :: twopi, dt, ph, dph
  integer :: isym, k, idx, kb, j
  real    :: e
  real(8) :: noisesum
  integer :: noisecnt
  integer, parameter :: NOFF = 16   ! number of off-tone bins sampled for noise

  twopi = 8.d0*atan(1.d0)
  dt = 1.d0/dble(fsample)
  dph = -twopi*dble(f0)*dt           ! down-mix by f0

  noisesum = 0.d0
  noisecnt = 0

  do isym=1,DX1_NSYM
     ! Build the down-mixed complex baseband for this symbol window.
     ph = dph*dble((isym-1)*DX1_NSPS)   ! continue the LO phase across symbols
     do k=1,DX1_NSPS
        idx = istart + (isym-1)*DX1_NSPS + (k-1)
        if(idx.ge.1 .and. idx.le.ndd) then
           c(k) = cmplx(dd(idx)*real(cos(ph)), dd(idx)*real(sin(ph)))
        else
           c(k) = (0.0,0.0)
        endif
        ph = ph + dph
     enddo

     ! In-place complex forward FFT (isign=-1, iform=1).
     call four2a(c, DX1_NSPS, 1, -1, 1)

     ! Tone k (0..7) -> FFT bin k (6.25 Hz/bin, baseband).
     do kb=0,DX1_M-1
        j = kb + 1                     ! bin index 1-based; bin1 = DC = tone0
        e = real(c(j))**2 + aimag(c(j))**2
        energy(kb+1, isym) = e
     enddo

     ! Noise estimate: sample bins well away from the 8 tone bins
     ! (positive-frequency side, bins 12..27 -> 68.75..168.75 Hz offset).
     do kb=12,12+NOFF-1
        j = kb + 1
        noisesum = noisesum + dble(real(c(j))**2 + aimag(c(j))**2)
        noisecnt = noisecnt + 1
     enddo
  enddo

  if(noisecnt.gt.0) then
     noisevar = real(noisesum/dble(noisecnt))
  else
     noisevar = 1.0
  endif
  if(noisevar.le.0.0) noisevar = 1.0e-6

  return
end subroutine dx1_detect
