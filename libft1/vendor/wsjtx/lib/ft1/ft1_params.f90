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
! FT1
! 4-CPM h=1/2 BT=0.3, LDPC(174,91) code, three 4x4 Costas arrays for sync
! Turbo equalization: iterative BCJR + LDPC BP decoding

parameter (KK=91)                     !Information bits (77 + CRC14)
parameter (ND=87)                     !Data symbols (174 coded bits / 2 bits per symbol)
parameter (NS=12)                     !Sync symbols (3 groups of 4, Costas 4x4)
parameter (NN=NS+ND)                  !Total channel symbols (99)
parameter (NSPS_NUM=3000,NSPS_DEN=7)  !Samples per symbol = 3000/7 = 428.571...
parameter (NSPS=429)                  !Nominal samples per symbol (rounded up)
parameter (NZ=42429)                  !Total TX samples (99 * 3000/7, rounded)
parameter (NMAX=4*12000)              !Samples in iwave (4.0s * 12000 Hz = 48000)
parameter (NFFT1=1024, NH1=NFFT1/2)  !Length of FFTs for spectrogram
parameter (NSTEP=107)                 !Spectrogram step size (~quarter symbol)
parameter (NHSYM=(NMAX-NFFT1)/NSTEP) !Number of spectral columns
parameter (NDOWN=54)                  !Downsample factor (12000/54 = 222.2 Hz, ~8 samp/sym)
parameter (NSS=8)                     !Downsampled samples/symbol: ceil(NSPS/NDOWN)=ceil(7.94)
parameter (MAXCAND=200)               !Maximum sync candidates
