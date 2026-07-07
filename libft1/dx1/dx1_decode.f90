! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! DX1 top-level decoder.
!
! Pipeline: chirp sync -> non-coherent M-FSK detect -> bit LLRs -> LDPC BP
! decode (174,91) -> CRC check -> unpack77 -> text.
!
!   dd       (in)  : raw 12 kHz audio, length ndd
!   f0       (in)  : nominal audio carrier (lower comb edge), Hz
!   fsample  (in)  : sample rate, Hz
!   idt_lo/hi(in)  : sample-offset search window for the sync chirp
!   msgout   (out) : decoded message text (37 chars)
!   msgbits  (out) : decoded 77 source bits
!   nharderr (out) : bpdecode hard-error count (<0 => decode/CRC failure)
!   snr_est  (out) : crude per-symbol SNR estimate (dB) from tone energies
!   sync_metric(out): sync correlation peak (quality)
! ---------------------------------------------------------------------------
subroutine dx1_decode(dd, ndd, f0, fsample, idt_lo, idt_hi, &
                      msgout, msgbits, nharderr, snr_est, sync_metric)
  use dx1_params
  use packjt77
  implicit none
  integer, intent(in)  :: ndd
  real,    intent(in)  :: dd(ndd)
  real,    intent(in)  :: f0, fsample
  integer, intent(in)  :: idt_lo, idt_hi
  character(len=37), intent(out) :: msgout
  integer*1, intent(out) :: msgbits(77)
  integer,   intent(out) :: nharderr
  real,      intent(out) :: snr_est
  real,      intent(out) :: sync_metric

  real    :: energy(DX1_M, DX1_NSYM)
  real    :: noisevar
  real    :: llr(DX1_NBITS)
  integer*1 :: apmask(DX1_NBITS)
  integer*1 :: cw(DX1_NBITS)
  integer*1 :: msg77(77)
  integer   :: iter, ncheck, istart, isym
  real      :: df
  real      :: f0eff
  character*77 :: c77
  character*37 :: msg37
  logical   :: unpk_ok
  real(8)   :: sigsum, totsum
  integer   :: i

  ! --- Coarse + fine sync ---
  call dx1_sync(dd, ndd, f0, fsample, idt_lo, idt_hi, istart, df, sync_metric)
  f0eff = f0 + df

  ! --- Non-coherent detection at the sync-resolved time/freq ---
  call dx1_detect(dd, ndd, istart, f0eff, fsample, energy, noisevar)

  ! --- Crude SNR estimate: peak tone energy vs noise floor ---
  sigsum = 0.d0
  totsum = 0.d0
  do isym=1,DX1_NSYM
     sigsum = sigsum + dble(maxval(energy(:,isym)))
  enddo
  if(noisevar.gt.0.0) then
     snr_est = 10.0*log10(max(real(sigsum/dble(DX1_NSYM))/noisevar,1.0e-6))
  else
     snr_est = -99.0
  endif

  ! --- Soft LLRs ---
  call dx1_llr(energy, noisevar, llr)

  ! --- LDPC(174,91) belief-propagation decode ---
  apmask = 0
  call bpdecode174_91(llr, apmask, 50, msg77, cw, nharderr, iter, ncheck)

  msgbits = msg77
  if(nharderr.ge.0) then
     write(c77,'(77i1)') msg77
     call unpack77(c77, 1, msg37, unpk_ok)
     if(unpk_ok) then
        msgout = msg37
     else
        msgout = '*** unpack failed ***                '
     endif
  else
     msgout = ''
  endif

  return
end subroutine dx1_decode
