! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.
!
! libft1 is free software: you can redistribute it and/or modify it under
! the terms of the GNU General Public License as published by the Free
! Software Foundation, either version 3 of the License, or (at your option)
! any later version.
!
! Distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY;
! without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
! PARTICULAR PURPOSE.  See the GNU General Public License for more details.
!
! ---------------------------------------------------------------------------
! DX1-S parameter module.
!
! DX1-S is a non-coherent M-FSK + soft-LDPC mode:
!   - M = 8 orthogonal tones, 3 bits/symbol, Gray coded.
!   - baud = 6.25 Hz, tone spacing = baud = 6.25 Hz.
!   - Occupied data BW = M * baud = 8 * 6.25 = 50 Hz.
!   - fsample = 12000 Hz -> NSPS = fsample/baud = 1920 samples/symbol.
!     A 1920-pt FFT gives exactly 6.25 Hz/bin, so each of the 8 tones lands
!     on its own bin: clean orthogonal non-coherent detection.
!   - Message path: 77-bit message -> LDPC(174,91) codeword (CRC-14 inside)
!     -> 174 coded bits -> 58 data symbols (174/3) -> M-FSK audio.
!   - Sync: a linear chirp preamble swept across the 50 Hz occupied band,
!     correlated at the RX over a coarse time/freq grid to recover dt + df.
! ---------------------------------------------------------------------------
module dx1_params
  implicit none

  ! --- Modulation ---
  integer, parameter :: DX1_M      = 8          ! number of FSK tones
  integer, parameter :: DX1_BPS    = 3          ! bits per symbol (log2 M)
  integer, parameter :: DX1_NSYM   = 58         ! data symbols (174/3)
  integer, parameter :: DX1_NBITS  = 174        ! coded bits (LDPC 174,91)
  integer, parameter :: DX1_K77    = 77         ! source message bits

  ! --- Sampling / timing ---
  real,    parameter :: DX1_FS     = 12000.0    ! sample rate, Hz
  real,    parameter :: DX1_BAUD   = 6.25       ! symbol rate, Hz
  integer, parameter :: DX1_NSPS   = 1920       ! samples per symbol (FS/BAUD)
  real,    parameter :: DX1_TONESP = 6.25       ! tone spacing, Hz (= baud)
  real,    parameter :: DX1_BW     = 50.0       ! occupied BW = M*baud, Hz

  ! --- Sync chirp preamble ---
  !  A linear FM sweep covering the occupied band, lasting NSYM_SYNC symbol
  !  periods.  Sample count = DX1_NSYNC * DX1_NSPS.
  integer, parameter :: DX1_NSYNC  = 4          ! sync symbols (~0.64 s)
  integer, parameter :: DX1_NSPS_SYNC = DX1_NSYNC * DX1_NSPS   ! 7680

  ! --- Full on-air frame layout (samples) ---
  !  [ chirp sync | 58 data symbols ]
  integer, parameter :: DX1_NDATA_SAMP = DX1_NSYM * DX1_NSPS         ! 111360
  integer, parameter :: DX1_NFRAME     = DX1_NSPS_SYNC + DX1_NDATA_SAMP ! 119040

  ! --- Receive buffer length.  15 s T/R slot at 12 kHz = 180000 samples.
  !  We size the test buffer to comfortably hold the frame plus a time
  !  offset margin.
  integer, parameter :: DX1_NMAX   = 184320     ! 15.36 s * 12 kHz

  ! --- Default audio carrier (Hz) = lower edge of the M-FSK comb.
  real,    parameter :: DX1_F0     = 1500.0

  ! --- Gray code map: symbol value (0..7) -> 3 bits (MSB first).
  !  graymap(s+1) holds the 3-bit Gray codeword for symbol index s.
  !  Standard binary-reflected Gray code: g = b xor (b>>1).
  !  Bit layout for a symbol: bits stored MSB..LSB in graybits(1:3,s+1).
  integer, parameter :: GRAYMAP(0:7) = (/ 0,1,3,2,6,7,5,4 /)
  ! Inverse map: gray codeword (0..7) -> symbol index (0..7).
  integer, parameter :: IGRAYMAP(0:7) = (/ 0,1,3,2,7,6,4,5 /)

contains

  ! Return the 3 bits (MSB first) of the Gray codeword for tone index s (0..7).
  subroutine dx1_tone_to_bits(s, b)
    integer, intent(in)  :: s          ! tone index 0..7
    integer, intent(out) :: b(3)       ! MSB..LSB
    integer :: g
    g = GRAYMAP(s)
    b(1) = iand(ishft(g,-2),1)
    b(2) = iand(ishft(g,-1),1)
    b(3) = iand(g,1)
  end subroutine dx1_tone_to_bits

  ! Inverse: 3 bits (MSB first) -> tone index 0..7.
  subroutine dx1_bits_to_tone(b, s)
    integer, intent(in)  :: b(3)       ! MSB..LSB
    integer, intent(out) :: s          ! tone index 0..7
    integer :: g
    g = ishft(b(1),2) + ishft(b(2),1) + b(3)
    s = IGRAYMAP(g)
  end subroutine dx1_bits_to_tone

end module dx1_params
