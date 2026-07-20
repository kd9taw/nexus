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
subroutine ft1_downsample(dd,newdata,f0,c)

! Bandpass filter, downconvert, and downsample a raw audio signal
! for matched-filter processing of FT1 candidates.
!
! Algorithm:
!   1. FFT the full input signal (NMAX samples)
!   2. Bandpass filter around f0 (Tukey window, ~120 Hz bandwidth)
!   3. Frequency-shift to baseband (move passband to DC)
!   4. Inverse FFT of reduced-length section
!   5. Output: complex baseband signal downsampled by NDOWN=54
!      -> effective rate: 12000/54 = 222.2 Hz = ~8 samples/symbol
!
! Reference: ft4_downsample.f90
!
! Interface:
!   dd(NMAX)          Raw audio samples (48000 samples at 12kHz)
!   newdata           Logical: true on first call (compute full FFT)
!   f0                Center frequency of candidate signal (Hz)
!   c(0:NDMAX-1)      Complex downsampled output

  include 'ft1_params.f90'
  parameter (NDMAX=NMAX/NDOWN)              !48000/54 = 888

  real dd(NMAX)
  complex c(0:NDMAX-1)
  complex c1(0:NDMAX-1)
  complex cx(0:NMAX/2)
  real x(NMAX), window(0:NDMAX-1)
  equivalence (x,cx)
  logical first, newdata
  data first/.true./
  save first,window,x

  df=12000.0/NMAX                           !0.25 Hz/bin
  baud=12000.0/real(NSPS)                   !~28 Bd

  if(first) then
     pi=4.0*atan(1.0)

     ! Design bandpass window in frequency domain
     ! Flat passband of ~4*baud = 112 Hz (covers 4 CPM tones)
     ! Raised-cosine transition of ~0.5*baud = 14 Hz on each side
     bw_transition = 0.5*baud
     bw_flat = 4*baud
     iwt = nint(bw_transition / df)
     iwf = nint(bw_flat / df)

     ! Build the window: rising edge, flat, falling edge, zeros
     window = 0.0
     if(iwt.gt.0) then
        do i=0,iwt-1
           window(i) = 0.5*(1.0+cos(pi*real(iwt-1-i)/real(iwt)))
        enddo
     endif
     do i=iwt,iwt+iwf-1
        if(i.le.NDMAX-1) window(i) = 1.0
     enddo
     if(iwt.gt.0) then
        do i=0,iwt-1
           j = iwt+iwf+i
           if(j.le.NDMAX-1) window(j) = 0.5*(1.0+cos(pi*real(i)/real(iwt)))
        enddo
     endif
     do i=2*iwt+iwf,NDMAX-1
        window(i) = 0.0
     enddo

     ! Shift window so flat passband is centered on CPM spectral center.
     ! Signal spectral center = mean tone = 0.75*baud = 21 Hz for h=1/2.
     ! Filter flat center before shift = (iwt + iwf/2)*df = 70 Hz.
     ! Required shift = 70 - 21 = 49 Hz = 1.75*baud/df bins.
     iws = nint(1.75 * baud / df)
     window = cshift(window, iws)

     first=.false.
  endif

  if(newdata) then
     x=dd
     call four2a(cx,NMAX,1,-1,0)            !r2c FFT to freq domain
  endif

  ! Extract frequency bins centered on f0 and shift to baseband
  i0=nint(f0/df)
  c1=0.
  if(i0.ge.0 .and. i0.le.NMAX/2) c1(0)=cx(i0)
  do i=1,NDMAX/2
     if(i0+i.ge.0 .and. i0+i.le.NMAX/2) c1(i)=cx(i0+i)
     if(i0-i.ge.0 .and. i0-i.le.NMAX/2) c1(NDMAX-i)=cx(i0-i)
  enddo

  ! Apply bandpass window and normalize
  c1=c1*window/NDMAX

  ! Inverse FFT back to time domain (complex-to-complex)
  call four2a(c1,NDMAX,1,1,1)
  c=c1(0:NDMAX-1)

  return
end subroutine ft1_downsample
