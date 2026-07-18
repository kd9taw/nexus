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
subroutine ft1_sync(dd,fa,fb,syncmin,nfqso,maxcand0,savg,candidate,   &
     ncand,sbase)

! Sync detection for FT1 signals: find candidate frequencies.
!
! Stage 1 (this routine): Find spectral peaks in the averaged spectrum
!   that could contain FT1 signals. For each peak, correlate the
!   spectrogram against all 3 Costas RV patterns to estimate the
!   redundancy version. Returns (freq, sync_quality, rv_index).
!
! Stage 2 (sync1d_ft1, called from ft1_decode): Fine time/frequency
!   sync on the downsampled complex baseband signal.
!
! FT1 uses 3 groups of 4 Costas sync symbols:
!   G1: symbols 0-3, G2: symbols 47-50, G3: symbols 95-98
!   All groups use the same pattern per TX (RV-dependent)
!
! Reference: Spec 3c Section 5 (Coarse Synchronization)
! Template:  getcandidates4.f90

  include 'ft1_params.f90'

  real s(NH1,NHSYM)                        !Spectrogram
  real savg(NH1),savsm(NH1)
  real sbase(NH1)
  real x(NFFT1)
  real window(NFFT1)
  complex cx(0:NH1)
  real candidate(3,maxcand)                !freq, snr, rv_index
  real candidatet(3,maxcand)
  real dd(NMAX)
  real sync_rv(3)                          !Sync metric for each RV
  equivalence (x,cx)
  logical first
  data first/.true./
  save first,window

  ! Costas arrays for 3 redundancy versions (0-indexed tones)
  integer icos_rv0(0:3),icos_rv1(0:3),icos_rv2(0:3)
  data icos_rv0/0,2,3,1/                   !RV0 (initial transmission)
  data icos_rv1/1,3,2,0/                   !RV1 (first retransmission)
  data icos_rv2/3,0,2,1/                   !RV2 (second retransmission)

  ! Sync group starting positions (0-indexed symbol numbers)
  integer isync_pos(3)
  data isync_pos/0,47,95/

  if(first) then
    first=.false.
    pi=4.0*atan(1.)
    window=0.
    call nuttal_window(window,NFFT1)
  endif

! ================================================================
! Step 1: Compute spectrogram
! NFFT1=1024, NSTEP=107 (~quarter symbol at 28 Bd)
! df = 12000/1024 = 11.72 Hz, dt = 107/12000 = 8.92 ms
! ================================================================
  savg=0.
  df=12000.0/NFFT1
  fac=1.0/300.0
  do j=1,NHSYM
     ia=(j-1)*NSTEP + 1
     ib=ia+NFFT1-1
     if(ib.gt.NMAX) exit
     x=fac*dd(ia:ib)*window
     call four2a(x,NFFT1,1,-1,0)            !r2c FFT
     s(1:NH1,j)=abs(cx(1:NH1))**2
     savg=savg + s(1:NH1,j)                 !Accumulate average spectrum
  enddo
  savg=savg/NHSYM

  ! Smooth average spectrum (7-point running mean)
  ! FT1 h=1/2: signal BW ~78 Hz, 7 bins * 11.72 = 82 Hz
  ! (FT4 h=1 uses 15-point/176 Hz for its ~160 Hz BW)
  savsm=0.
  do i=4,NH1-3
    savsm(i)=sum(savg(i-3:i+3))/7.
  enddo

! ================================================================
! Step 2: Baseline estimation and normalization
! ================================================================
  nfa=nint(fa/df)
  if(nfa.lt.nint(200.0/df)) nfa=nint(200.0/df)
  nfb=nint(fb/df)
  if(nfb.gt.nint(4910.0/df)) nfb=nint(4910.0/df)

  call ft1_baseline(savg,nfa,nfb,sbase)
  if(any(sbase(nfa:nfb).le.0)) return
  savsm(nfa:nfb)=savsm(nfa:nfb)/sbase(nfa:nfb)

! ================================================================
! Step 3: Find spectral peaks as frequency candidates
! For each peak, correlate the spectrogram with Costas patterns
! to determine the best RV and its sync quality.
! ================================================================

  ! Tone spacing in frequency bins
  ! FT1: 4-CPM h=1/2 -> tone spacing = h * baud = 0.5 * 28 = 14 Hz
  nfos=nint(14.0/df)

  ! Time steps per symbol
  nssy=NSPS/NSTEP                            !429/107 = 4

  ! Number of time steps spanning the full frame
  ntsym=NN*nssy                              !99*4 = 396

  ! Frequency offset to align spectral peak with carrier
  ! FT1 4-CPM h=1/2: tones at 0, 14, 28, 42 Hz above carrier.
  ! At NFFT1=1024 (df=11.72 Hz), tones are unresolved (spacing=1.2 bins).
  ! Nuttall window (main lobe ~8 bins) merges all tones into a single
  ! broad peak. Empirically, the peak is ~10 Hz above carrier.
  ! Use -10 Hz and rely on sync1d fine search for precise correction.
  f_offset = -10.0

  ncand=0
  candidatet=0

  do i=nfa+1,nfb-1
     ! Look for spectral peaks in smoothed/normalized average spectrum
     if(savsm(i).ge.savsm(i-1) .and. savsm(i).ge.savsm(i+1) .and.  &
          savsm(i).ge.syncmin) then

        ! Parabolic interpolation for sub-bin frequency
        den=savsm(i-1)-2*savsm(i)+savsm(i+1)
        del=0.
        if(den.ne.0.0) del=0.5*(savsm(i-1)-savsm(i+1))/den
        fpeak=(i+del)*df+f_offset

        if(fpeak.lt.200.0 .or. fpeak.gt.4910.0) cycle
        speak=savsm(i) - 0.25*(savsm(i-1)-savsm(i+1))*del

        ! For this frequency peak, determine best RV by correlating
        ! the spectrogram with each Costas pattern across all 3 sync groups.
        ! Time search: use the center of the expected TX window.
        ! The fine sync will refine the timing later.
        sync_rv = 0.0
        irv_best = 0
        sbest_rv = 0.0

        ! Search a few time offsets around the expected center
        ! Expected TX start is near t=0.25s in the T/R window
        ! In spectrogram steps: j0_center ~ 0.25*12000/107 ~ 28
        j0_center = nint(0.25*12000.0/NSTEP)
        j0_min = max(1, j0_center - 2*nssy)
        j0_max = min(NHSYM - ntsym, j0_center + 2*nssy)

        do j0=j0_min,j0_max
           sync_rv = 0.0

           ! --- RV0: icos = [0,2,3,1] ---
           do ig=1,3
              igrp_start=isync_pos(ig)
              do k=0,3
                 it=j0 + (igrp_start+k)*nssy
                 if(it.lt.1 .or. it.gt.NHSYM) cycle
                 ifbin=i + icos_rv0(k)*nfos
                 if(ifbin.ge.1 .and. ifbin.le.NH1) then
                    sync_rv(1) = sync_rv(1) + s(ifbin,it)
                 endif
              enddo
           enddo

           ! --- RV1: icos = [1,3,2,0] ---
           do ig=1,3
              igrp_start=isync_pos(ig)
              do k=0,3
                 it=j0 + (igrp_start+k)*nssy
                 if(it.lt.1 .or. it.gt.NHSYM) cycle
                 ifbin=i + icos_rv1(k)*nfos
                 if(ifbin.ge.1 .and. ifbin.le.NH1) then
                    sync_rv(2) = sync_rv(2) + s(ifbin,it)
                 endif
              enddo
           enddo

           ! --- RV2: icos = [3,0,2,1] ---
           do ig=1,3
              igrp_start=isync_pos(ig)
              do k=0,3
                 it=j0 + (igrp_start+k)*nssy
                 if(it.lt.1 .or. it.gt.NHSYM) cycle
                 ifbin=i + icos_rv2(k)*nfos
                 if(ifbin.ge.1 .and. ifbin.le.NH1) then
                    sync_rv(3) = sync_rv(3) + s(ifbin,it)
                 endif
              enddo
           enddo

           ! Track best RV and best time offset
           do irv=1,3
              if(sync_rv(irv).gt.sbest_rv) then
                 sbest_rv = sync_rv(irv)
                 irv_best = irv - 1            !0-indexed RV
              endif
           enddo
        enddo  !j0

        ncand=ncand+1
        candidatet(1,ncand)=fpeak
        candidatet(2,ncand)=speak
        candidatet(3,ncand)=real(irv_best)
        if(ncand.eq.maxcand) exit
     endif
  enddo

! ================================================================
! Step 4: Prioritize candidates near QSO frequency
! ================================================================
  candidate=0.
  nq=count(abs(candidatet(1,1:ncand)-nfqso).le.20.0)
  n1=1
  n2=nq+1
  do i=1,ncand
     if(abs(candidatet(1,i)-nfqso).le.20.0) then
        candidate(1:3,n1)=candidatet(1:3,i)
        n1=n1+1
     else
        candidate(1:3,n2)=candidatet(1:3,i)
        n2=n2+1
     endif
  enddo

  return
end subroutine ft1_sync


subroutine ft1_baseline(s,nfa,nfb,sbase)

! Fit baseline to spectrum for normalization.
! Adapted from ft4_baseline -- identical algorithm, FT1 params.
!
! Input:  s(npts)      Linear scale power spectrum (modified to dB in-place)
! Output: sbase(npts)  Baseline estimate (linear scale)

  include 'ft1_params.f90'
  implicit real*8 (a-h,o-z)
  real*4 s(NH1)
  real*4 sbase(NH1)
  real*4 base
  real*8 x(1000),y(1000),a(5)
  data nseg/10/,npct/10/

  df=12000.0/NFFT1                          !11.72 Hz
  ia=max(nint(200.0d0/df),nfa)
  ib=min(NH1,nfb)

  ! Convert to dB scale
  do i=ia,ib
     if(s(i).gt.0.0) then
        s(i)=10.0*log10(s(i))
     else
        s(i)=-99.0
     endif
  enddo

  nterms=5
  nlen=(ib-ia+1)/nseg
  if(nlen.lt.1) nlen=1
  i0=(ib-ia+1)/2
  k=0
  do n=1,nseg
     ja=ia + (n-1)*nlen
     jb=ja+nlen-1
     if(jb.gt.ib) jb=ib
     call pctile(s(ja),jb-ja+1,npct,base)
     do i=ja,jb
        if(s(i).le.base) then
           if(k.lt.1000) k=k+1
           x(k)=i-i0
           y(k)=s(i)
        endif
     enddo
  enddo
  kz=k
  if(kz.lt.nterms) then
     sbase=1.0
     return
  endif
  a=0.
  call polyfit(x,y,y,kz,nterms,0,a,chisqr)
  do i=ia,ib
     t=i-i0
     sbase(i)=a(1)+t*(a(2)+t*(a(3)+t*(a(4)+t*(a(5))))) + 0.65
     sbase(i)=10**(sbase(i)/10.0)
  enddo

  return
end subroutine ft1_baseline
