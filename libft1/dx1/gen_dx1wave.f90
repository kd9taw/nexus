! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! DX1 message encoder + M-FSK modulator.
!
!   dx1_encode_msg : 77-bit message text -> 174 coded bits (LDPC+CRC).
!   gen_dx1wave    : 174 coded bits -> 58 Gray-mapped 8-FSK symbols, prefixed
!                    by a linear-chirp sync preamble, rendered to real 12 kHz
!                    audio.  Continuous phase (limits splatter); detection is
!                    non-coherent so absolute phase need not be tracked.
! ---------------------------------------------------------------------------

subroutine dx1_encode_msg(msg, msgbits, codeword, msgsent, ok)
! Pack a free-form / standard message into 77 bits, then LDPC(174,91) encode.
!   msg      (in)  : message text (up to 37 chars)
!   msgbits  (out) : the 77 source bits (integer*1)
!   codeword (out) : 174 coded bits (integer*1)  [CRC-14 added inside]
!   msgsent  (out) : message as it will be received (round-trip of pack/unpack)
!   ok       (out) : .true. if the message packed/unpacked cleanly
  use packjt77
  implicit none
  character(len=*), intent(in)  :: msg
  integer*1,        intent(out) :: msgbits(77)
  integer*1,        intent(out) :: codeword(174)
  character(len=37),intent(out) :: msgsent
  logical,          intent(out) :: ok

  character*37 :: message
  character*77 :: c77
  integer :: i, i3, n3
  logical :: unpk77_success

  message = msg
  ! Strip leading blanks (mirrors genft1.f90).
  do i=1,37
     if(message(1:1).ne.' ') exit
     message=message(i+1:)
  enddo

  i3=-1
  n3=-1
  call pack77(message,i3,n3,c77)
  call unpack77(c77,0,msgsent,unpk77_success)

  ok = unpk77_success
  if(.not.ok) then
     msgbits=0
     codeword=0
     msgsent='*** bad message ***                  '
     return
  endif

  read(c77,'(77i1)',err=900) msgbits
  call encode174_91(msgbits,codeword)
  return

900 continue
  ok=.false.
  msgbits=0
  codeword=0
  msgsent='*** bad message ***                  '
  return
end subroutine dx1_encode_msg


subroutine gen_dx1wave(codeword, f0, fsample, wave, nwave)
! Generate the full DX1-S audio frame: chirp sync preamble + 58 8-FSK symbols.
!   codeword (in)  : 174 coded bits (integer*1, 0/1)
!   f0       (in)  : audio carrier = lower edge of the tone comb, Hz
!   fsample  (in)  : sample rate, Hz (12000)
!   wave     (out) : real audio, length nwave (= DX1_NFRAME)
!   nwave    (out) : number of samples written
  use dx1_params
  implicit none
  integer*1, intent(in)  :: codeword(DX1_NBITS)
  real,      intent(in)  :: f0, fsample
  real,      intent(out) :: wave(*)
  integer,   intent(out) :: nwave

  real(8) :: twopi, dt, phi, dphi, f
  real    :: env
  integer :: isym, ib, k, n, tone, bits(3)
  integer :: itone(DX1_NSYM)
  real(8) :: fchirp_lo, fchirp_hi, chirp_rate, t

  twopi = 8.d0*atan(1.d0)
  dt = 1.d0/dble(fsample)

  ! --- Map 174 coded bits -> 58 Gray 8-FSK tones (3 bits/sym, MSB first) ---
  do isym=1,DX1_NSYM
     ib = (isym-1)*DX1_BPS
     bits(1) = codeword(ib+1)
     bits(2) = codeword(ib+2)
     bits(3) = codeword(ib+3)
     call dx1_bits_to_tone(bits, tone)
     itone(isym) = tone        ! 0..7
  enddo

  phi = 0.d0
  n = 0

  ! --- Sync preamble: linear chirp across the occupied band (f0 .. f0+BW) ---
  fchirp_lo = dble(f0)
  fchirp_hi = dble(f0) + dble(DX1_BW)
  chirp_rate = (fchirp_hi - fchirp_lo) / (dble(DX1_NSPS_SYNC)*dt)  ! Hz/s
  do k=0,DX1_NSPS_SYNC-1
     t = dble(k)*dt
     f = fchirp_lo + chirp_rate*t
     dphi = twopi*f*dt
     phi = phi + dphi
     n = n + 1
     ! gentle raised-cosine ramp on first/last 64 samples
     env = 1.0
     if(k.lt.64)                  env = 0.5*(1.0-cos(3.14159265*real(k)/64.0))
     if(k.ge.DX1_NSPS_SYNC-64)    env = 0.5*(1.0-cos(3.14159265* &
                                       real(DX1_NSPS_SYNC-1-k)/64.0))
     wave(n) = env*real(sin(phi))
  enddo

  ! --- Data symbols: continuous-phase 8-FSK ---
  do isym=1,DX1_NSYM
     tone = itone(isym)
     f = dble(f0) + dble(tone)*dble(DX1_TONESP)
     dphi = twopi*f*dt
     do k=0,DX1_NSPS-1
        phi = phi + dphi
        n = n + 1
        wave(n) = real(sin(phi))
     enddo
  enddo

  ! gentle ramp at the very end of the data block to limit splatter
  do k=0,63
     wave(n-k) = wave(n-k) * (0.5*(1.0-cos(3.14159265*real(k)/64.0)))
  enddo

  nwave = n
  return
end subroutine gen_dx1wave
