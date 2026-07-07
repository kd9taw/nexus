! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! Bit LLR computation for non-coherent M-FSK.
!
! For each symbol there are M=8 tone-bin energies E_m.  Under a square-law
! (non-coherent) detector the per-tone metric is E_m / sigma^2.  For each of
! the 3 bits carried by the symbol:
!
!   LLR_j = logsumexp_{tones with Gray-bit j = 0}(E_m/sigma^2)
!         - logsumexp_{tones with Gray-bit j = 1}(E_m/sigma^2)
!
! Sign convention matches bpdecode174_91: LLR > 0 favours bit = 1, so we
! return (lse1 - lse0) so that a stronger "bit=1" tone group yields a
! positive LLR (cw set where zn>0).
!
! Note on the energy/sigma^2 scaling: for a square-law detector the natural
! exponent argument is E/sigma^2 where sigma^2 here is the *per complex bin*
! noise energy (== mean off-tone bin energy).  A 0.5 factor (E/(2 sigma^2))
! is the textbook Rician/Rayleigh form for |Y|^2 with sigma^2 the per-
! quadrature variance; since our noisevar is the full |Y|^2 mean (2x per-
! quadrature), E/noisevar is the correct argument.
! ---------------------------------------------------------------------------
subroutine dx1_llr(energy, noisevar, llr)
  use dx1_params
  implicit none
  real, intent(in)  :: energy(DX1_M, DX1_NSYM)
  real, intent(in)  :: noisevar
  real, intent(out) :: llr(DX1_NBITS)

  integer :: isym, jbit, m, bb, ib
  integer :: tonebits(3)            ! Gray bits (MSB..LSB) of each tone
  real    :: x(DX1_M)               ! scaled metrics E/sigma^2 for this symbol
  real    :: acc0, acc1
  real    :: inv
  integer :: bitsmap(3,0:DX1_M-1)   ! precomputed tone -> 3 Gray bits

  ! Precompute Gray bit pattern for every tone.
  do m=0,DX1_M-1
     call dx1_tone_to_bits(m, tonebits)
     bitsmap(1,m) = tonebits(1)
     bitsmap(2,m) = tonebits(2)
     bitsmap(3,m) = tonebits(3)
  enddo

  inv = 1.0/noisevar

  do isym=1,DX1_NSYM
     do m=1,DX1_M
        x(m) = energy(m,isym)*inv
     enddo

     ! For each of the 3 bits this symbol carries.
     do jbit=1,3
        ! group tones by Gray bit value, log-sum-exp each group.
        call lse_group(x, bitsmap, jbit, 0, acc0)
        call lse_group(x, bitsmap, jbit, 1, acc1)
        ib = (isym-1)*DX1_BPS + jbit
        llr(ib) = acc1 - acc0
     enddo
  enddo

  return
contains

  ! log-sum-exp over the tones whose Gray bit `jbit` equals `want`.
  subroutine lse_group(x, bitsmap, jbit, want, out)
    real,    intent(in)  :: x(DX1_M)
    integer, intent(in)  :: bitsmap(3,0:DX1_M-1)
    integer, intent(in)  :: jbit, want
    real,    intent(out) :: out
    integer :: mm
    real    :: xmax, s
    logical :: any
    xmax = -1.0e30
    any = .false.
    do mm=1,DX1_M
       if(bitsmap(jbit,mm-1).eq.want) then
          any = .true.
          if(x(mm).gt.xmax) xmax = x(mm)
       endif
    enddo
    if(.not.any) then
       out = -1.0e30
       return
    endif
    s = 0.0
    do mm=1,DX1_M
       if(bitsmap(jbit,mm-1).eq.want) then
          s = s + exp(x(mm)-xmax)
       endif
    enddo
    out = xmax + log(s)
  end subroutine lse_group

end subroutine dx1_llr
