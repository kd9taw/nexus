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
subroutine gen_ft1wave(itone,nsym,nsps_num,nsps_den,fsample,f0, &
     wave,nwave)

! Generate FT1 4-CPM waveform with h=1/2, Gaussian BT=0.3, L=3.
!
! This differs fundamentally from gen_ft4wave/gen_ft8wave:
!   - Modulation index h = 1/2 (not 1.0)
!   - Non-integer samples per symbol: nsps_num/nsps_den = 3000/7 at 12 kHz
!   - At 48 kHz output: effective nsps = 4*3000/7 = 12000/7 = 1714.286...
!   - Symbol start positions computed via rounding for exact timing
!   - 24-sample raised-cosine envelope taper (at 12 kHz rate, scaled to fsample)
!
! Input:
!   itone(nsym)   - quaternary channel symbols {0,1,2,3}, length 99
!   nsym          - number of symbols (99)
!   nsps_num      - numerator of samples-per-symbol ratio (3000)
!   nsps_den      - denominator of samples-per-symbol ratio (7)
!   fsample       - output sample rate in Hz (48000.0 for WSJT-X)
!   f0            - audio carrier frequency in Hz
!
! Output:
!   wave(nwave)   - real-valued audio waveform
!   nwave         - number of output samples

  implicit none
  integer nsym, nsps_num, nsps_den, nwave
  real wave(nwave)
  integer itone(nsym)
  real fsample, f0
  real gfsk_pulse                        !External function

! Local variables
  integer, parameter :: MAXPULSE=6000    !Max pulse length (L*nsps at 48kHz)
  integer, parameter :: MAXDPHI=250000   !Max dphi array length
  real pulse(MAXPULSE)
  real dphi(0:MAXDPHI-1)
  real twopi, dt, hmod, bt, dphi_peak, dphi_carrier
  real phi, nsps_real, tt
  integer npulse, ntotal, nramp_out
  integer i, j, k, ie, ncount
  integer n_start_j
  real upsample_ratio

  logical first
  save pulse, first, twopi, dt, hmod, npulse, nsps_real, &
       dphi_peak, nramp_out
  data first/.true./

! Compute upsampling ratio relative to 12 kHz native rate
  upsample_ratio = fsample / 12000.0

  if(first) then
     twopi = 8.0*atan(1.0)
     dt = 1.0/fsample
     hmod = 0.5                          !Modulation index h = 1/2
     bt = 0.3                            !Gaussian BT parameter

! Effective samples per symbol at output sample rate
! At 12 kHz: nsps = nsps_num/nsps_den = 3000/7 = 428.571...
! At 48 kHz: nsps = 4 * 3000/7 = 12000/7 = 1714.286...
     nsps_real = upsample_ratio * real(nsps_num) / real(nsps_den)

! Pulse length: L=3 symbol periods at the output sample rate
     npulse = nint(3.0 * nsps_real)      !e.g., 5143 at 48kHz

! Precompute the Gaussian frequency pulse shape
! The pulse spans L=3 symbol periods, centered at the symbol midpoint
! gfsk_pulse(BT, t) is the instantaneous frequency pulse shape
! WSJT-X's gfsk_pulse integrates to 1.0 over the real line.
! Standard CPM requires integral = 0.5 over the pulse support.
! We normalize after computation so sum(pulse) = nsps_real/2.
     do i = 1, npulse
        tt = (real(i) - 1.5*nsps_real) / nsps_real
        pulse(i) = gfsk_pulse(bt, tt)
     enddo

! Normalize pulse for standard CPM: sum(pulse) = nsps_real / 2
! This ensures phase change per symbol = pi*h*a = pi/2*a for h=1/2
     dphi_peak = 0.0
     do i = 1, npulse
        dphi_peak = dphi_peak + pulse(i)
     enddo
     if(dphi_peak .gt. 0.0) then
        dphi_peak = 0.5 * nsps_real / dphi_peak
        do i = 1, npulse
           pulse(i) = pulse(i) * dphi_peak
        enddo
     endif

! Peak frequency deviation per sample
! dphi_peak * sum(pulse) * symbol_value = 2*pi*h*0.5 * symbol_value
     dphi_peak = twopi * hmod / nsps_real

! Envelope taper: 24 samples at 12 kHz = 2 ms, scaled to output rate
     nramp_out = nint(24.0 * upsample_ratio)  !96 at 48kHz

     first = .false.
  endif

! Total output samples: nsym * nsps, using exact rational arithmetic
! At 48kHz: nint(99 * 12000 / 7) = nint(169714.286) = 169714
  ntotal = nint(real(nsym) * nsps_real)
  if(ntotal.gt.nwave) ntotal = nwave

! Build the frequency deviation array
! For each symbol j, add the scaled pulse contribution starting at
! the symbol's exact start sample
  dphi(0:ntotal+npulse-1) = 0.0

  do j = 1, nsym
! Exact start sample for symbol j (0-indexed symbol, 0-indexed sample)
     n_start_j = nint(real(j-1) * nsps_real)
     ie = min(n_start_j + npulse - 1, ntotal + npulse - 2)
     ncount = ie - n_start_j + 1
     if(ncount.gt.0) then
        do k = 1, ncount
           dphi(n_start_j + k - 1) = dphi(n_start_j + k - 1) + &
                dphi_peak * pulse(k) * itone(j)
        enddo
     endif
  enddo

! Add carrier frequency to the phase increment array
  dphi_carrier = twopi * f0 * dt
  do i = 0, ntotal - 1
     dphi(i) = dphi(i) + dphi_carrier
  enddo

! Accumulate phase and generate cosine audio waveform
  phi = 0.0
  wave = 0.0
  do i = 0, ntotal - 1
     phi = phi + dphi(i)
     phi = mod(phi, twopi)
     wave(i+1) = cos(phi)
  enddo

! Apply raised-cosine envelope taper
! Rise ramp (first nramp_out samples)
  do i = 0, nramp_out - 1
     wave(i+1) = wave(i+1) * &
          0.5*(1.0 - cos(twopi*real(i)/(2.0*real(nramp_out))))
  enddo

! Fall ramp (last nramp_out samples)
  do i = 0, nramp_out - 1
     k = ntotal - nramp_out + i + 1
     if(k.ge.1 .and. k.le.nwave) then
        wave(k) = wave(k) * &
             0.5*(1.0 + cos(twopi*real(i)/(2.0*real(nramp_out))))
     endif
  enddo

! Zero out any remaining samples beyond the transmission
  if(ntotal+1.le.nwave) then
     wave(ntotal+1:nwave) = 0.0
  endif

  return
end subroutine gen_ft1wave
