! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! Chirp sync detector.
!
! The TX prepends a linear-FM sweep (f0 .. f0+BW over DX1_NSPS_SYNC samples,
! generated in gen_dx1wave).  The RX correlates a complex chirp replica
! against the received audio over a coarse time x frequency grid.  The peak
! of |correlation| gives the start-of-sync sample (-> start-of-data) and the
! frequency offset df.
!
! Output:
!   istart_data : 1-based sample index of the FIRST DATA symbol.
!   df          : estimated carrier frequency offset (Hz), search granularity.
!   metric      : peak normalised correlation magnitude (detection quality).
! ---------------------------------------------------------------------------
subroutine dx1_sync(dd, ndd, f0, fsample, idt_search_lo, idt_search_hi, &
                    istart_data, df, metric)
  use dx1_params
  implicit none
  integer, intent(in)  :: ndd
  real,    intent(in)  :: dd(ndd)
  real,    intent(in)  :: f0, fsample
  integer, intent(in)  :: idt_search_lo, idt_search_hi   ! sample-offset search range
  integer, intent(out) :: istart_data
  real,    intent(out) :: df
  real,    intent(out) :: metric

  integer, parameter :: NSY = DX1_NSPS_SYNC
  integer, parameter :: TSTEP = 48        ! coarse time step (4 ms @ 12 kHz)
  real,    parameter :: FSTEP = 1.5625     ! coarse freq step (baud/4) Hz
  integer, parameter :: NFOFF = 9          ! +/- 4 freq bins -> +/- 6.25 Hz

  real(8) :: twopi, dt, t, ph, chirp_rate, fbase
  complex :: repl(NSY)                     ! reference chirp replica (df=0)
  complex :: acc
  real    :: bestmetric, mag, sigpow
  integer :: it, jf, k, idx, bestit, bestjf
  real(8) :: foff

  twopi = 8.d0*atan(1.d0)
  dt = 1.d0/dble(fsample)
  fbase = dble(f0)
  chirp_rate = dble(DX1_BW)/(dble(NSY)*dt)     ! Hz/s

  ! Precompute the df=0 complex chirp replica (conjugated for correlation).
  ph = 0.d0
  do k=1,NSY
     t = dble(k-1)*dt
     ph = ph + twopi*(fbase + chirp_rate*t)*dt
     repl(k) = cmplx(real(cos(ph)), -real(sin(ph)))   ! exp(-j phi)
  enddo

  bestmetric = -1.0
  bestit = idt_search_lo
  bestjf = 0

  do it = idt_search_lo, idt_search_hi, TSTEP
     do jf = -(NFOFF-1)/2, (NFOFF-1)/2
        foff = dble(jf)*dble(FSTEP)
        acc = (0.0,0.0)
        sigpow = 0.0
        do k=1,NSY
           idx = it + (k-1)
           if(idx.ge.1 .and. idx.le.ndd) then
              ! mix received audio by the freq-offset hypothesis and multiply
              ! by conjugate replica; freq offset folded into a rotating phasor.
              t = dble(k-1)*dt
              acc = acc + dd(idx)*repl(k)* &
                    cmplx(real(cos(-twopi*foff*t)), real(sin(-twopi*foff*t)))
              sigpow = sigpow + dd(idx)*dd(idx)
           endif
        enddo
        if(sigpow.gt.0.0) then
           mag = (real(acc)**2 + aimag(acc)**2)/sigpow
        else
           mag = 0.0
        endif
        if(mag.gt.bestmetric) then
           bestmetric = mag
           bestit = it
           bestjf = jf
        endif
     enddo
  enddo

  ! Refine time around the coarse winner at full resolution (df fixed).
  call refine_time(dd, ndd, repl, NSY, dble(bestjf)*dble(FSTEP), &
                   fsample, max(bestit-TSTEP,idt_search_lo), &
                   min(bestit+TSTEP,idt_search_hi), bestit, bestmetric)

  df = real(bestjf)*FSTEP
  metric = bestmetric
  ! Data starts immediately after the sync preamble.
  istart_data = bestit + NSY
  return

contains

  subroutine refine_time(dd, ndd, repl, nsy, foff, fsample, lo, hi, it_out, m_out)
    integer, intent(in)    :: ndd, nsy, lo, hi
    real,    intent(in)    :: dd(ndd), fsample
    complex, intent(in)    :: repl(nsy)
    real(8), intent(in)    :: foff
    integer, intent(inout) :: it_out
    real,    intent(inout) :: m_out
    integer :: it, k, idx
    real(8) :: twopi, dt, t
    complex :: acc
    real    :: mag, sigpow
    twopi = 8.d0*atan(1.d0)
    dt = 1.d0/dble(fsample)
    do it=lo,hi
       acc=(0.0,0.0)
       sigpow=0.0
       do k=1,nsy
          idx=it+(k-1)
          if(idx.ge.1 .and. idx.le.ndd) then
             t=dble(k-1)*dt
             acc=acc+dd(idx)*repl(k)* &
                 cmplx(real(cos(-twopi*foff*t)),real(sin(-twopi*foff*t)))
             sigpow=sigpow+dd(idx)*dd(idx)
          endif
       enddo
       if(sigpow.gt.0.0) then
          mag=(real(acc)**2+aimag(acc)**2)/sigpow
       else
          mag=0.0
       endif
       if(mag.gt.m_out) then
          m_out=mag
          it_out=it
       endif
    enddo
  end subroutine refine_time

end subroutine dx1_sync
