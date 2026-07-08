! DX1 - Non-coherent weak-signal HF digital mode (DX1-S baseline)
! Copyright (C) 2026 KD9TAW
!
! This file is part of libft1 / Tempo.  GPLv3 (see dx1_params.f90 header).
!
! ---------------------------------------------------------------------------
! DX1 FULL-PASSBAND decoder: decode EVERY signal in the audio passband in one
! slot, like FT1's acquisition (vs dx1_decode, which decodes one known carrier).
!
! Why three stages (the single-carrier dx1_sync costs ~0.9 s over a full slot
! buffer; a naive ~213-carrier sweep would be ~190 s -- far over the 15 s slot):
!
!   Stage A  Coarse carrier scan.  dx1_sync only captures +/-6.25 Hz around its
!            nominal f0, so we step a candidate carrier across [f_lo,f_hi] on a
!            12.5 Hz grid (each candidate's +/-6.25 Hz window tiles the band with
!            no gap).  For each candidate we run a CHEAP chirp correlation:
!            pre-folded per-frequency-bin replicas (no inner-loop trig), a wide
!            TSTEP, and an O(1) windowed signal-power via a prefix sum.  The
!            normalised metric |corr|^2/sigpow is comparable across carriers, so
!            this yields a metric-vs-frequency curve.
!
!   Stage B  Peak-pick.  Median-based noise floor -> threshold; keep local
!            maxima above it; greedily de-duplicate within a min-separation (one
!            emitter's 50 Hz chirp lights several adjacent cells).
!
!   Stage C  Full decode per survivor.  Narrow re-sync (full-resolution dx1_sync
!            around the coarse time), dx1_detect -> dx1_llr -> bpdecode174_91.
!            The CRC-14 inside bpdecode174_91 rejects false peaks, so only
!            genuine codewords (nharderr >= 0) are emitted.  Carrier reported is
!            f0_candidate + df from the refined sync.
!
! Existing dx1_sync / dx1_detect / dx1_llr / dx1_decode are reused verbatim and
! left UNCHANGED -- the single-carrier path is unaffected.
!
!   dd        (in)  : raw 12 kHz audio, length ndd
!   f_lo,f_hi (in)  : carrier (lower-comb-edge) scan range, Hz
!   fsample   (in)  : sample rate, Hz
!   idt_lo/hi (in)  : sample-offset search window for the sync chirp start
!   maxdec    (in)  : capacity of the output arrays (also caps decodes/slot)
!   msgs      (out) : decoded message text, one per decode (1:ndec)
!   freqs     (out) : resolved carrier (Hz) per decode
!   snrs      (out) : crude SNR estimate (dB) per decode
!   syncs     (out) : sync correlation metric per decode
!   ndec      (out) : number of decodes found (0..maxdec)
! ---------------------------------------------------------------------------
subroutine dx1_decode_all(dd, ndd, f_lo, f_hi, fsample, idt_lo, idt_hi, &
                          maxdec, msgs, freqs, snrs, syncs, ndec)
  use dx1_params
  use packjt77
  implicit none
  integer, intent(in)  :: ndd
  real,    intent(in)  :: dd(ndd)
  real,    intent(in)  :: f_lo, f_hi, fsample
  integer, intent(in)  :: idt_lo, idt_hi
  integer, intent(in)  :: maxdec
  character(len=37), intent(out) :: msgs(maxdec)
  real,    intent(out) :: freqs(maxdec)
  real,    intent(out) :: snrs(maxdec)
  real,    intent(out) :: syncs(maxdec)
  integer, intent(out) :: ndec

  ! --- Scan geometry ---
  integer, parameter :: NSY    = DX1_NSPS_SYNC      ! chirp length (7680)
  real,    parameter :: GRID_HZ = 12.5              ! carrier grid (2 * +/-6.25)
  integer, parameter :: TSTEP_A = 192               ! coarse time step (< chirp
                                                    !   autocorr width ~240)
  real,    parameter :: FSTEP  = 1.5625             ! freq sub-bin (matches dx1_sync)
  integer, parameter :: NFOFF  = 9                  ! +/- 4 bins -> +/- 6.25 Hz
  real,    parameter :: NOISE_FACTOR = 4.0          ! threshold = NF * median(metric)
  real,    parameter :: MINSEP_HZ    = 25.0         ! de-dup separation (peaks & msgs)
  integer, parameter :: MAX_SURV     = 64           ! cap survivors into Stage C
  integer, parameter :: BP_MAXITER   = 50
  ! Linear-chirp time<->frequency ambiguity: a delayed chirp looks frequency-
  ! shifted, so Stage A's joint (it,jf) peak sits anywhere along an ambiguity
  ! ridge.  A +/-6.25 Hz jf range (=(NFOFF-1)/2*FSTEP) maps to a ~+/-960-sample
  ! time offset (chirp_rate = DX1_BW/(NSY/FS) = 78.1 Hz/s -> 6.25/(78.1/12000)
  ! ~= 960 samples).  So Stage C must re-search a window WIDE enough to recover
  ! the true (it, df~=0) peak, not just +/-TSTEP_A.
  integer, parameter :: STAGE_C_MARGIN = 1280       ! ~960 ambiguity + coarse step

  ! --- Stage A buffers ---
  complex, allocatable :: replf(:,:)                ! pre-folded replicas (NSY,NFOFF)
  real,    allocatable :: cmetric(:)                ! per-candidate peak metric
  integer, allocatable :: cbestit(:)                ! per-candidate coarse sync start
  real,    allocatable :: cf0(:)                    ! per-candidate carrier
  integer :: ncand

  ! --- Stage B buffers ---
  integer, allocatable :: peakidx(:)                ! candidate indices of local maxima
  integer :: npeak
  real    :: median, threshold

  ! --- Stage C scratch (reused per survivor) ---
  real      :: energy(DX1_M, DX1_NSYM)
  real      :: noisevar
  real      :: llr(DX1_NBITS)
  integer*1 :: apmask(DX1_NBITS), cw(DX1_NBITS), msg77(77)
  integer   :: iter, ncheck, istart, nh, isym
  real      :: df, f0eff, sync_l, snr_l
  real(8)   :: sigsum
  character(len=77) :: c77
  character(len=37) :: msg37
  logical   :: unpk_ok, dup

  integer :: ilo, ihi, i, j, isurv
  real    :: f0c

  ndec = 0
  if (maxdec <= 0) return

  ! Frame must fully fit: sync start it satisfies it+NFRAME-1 <= ndd.
  ilo = max(idt_lo, 1)
  ihi = min(idt_hi, ndd - DX1_NFRAME + 1)
  if (ihi < ilo) return
  if (f_hi < f_lo) return

  ncand = int((f_hi - f_lo) / GRID_HZ) + 1
  if (ncand < 1) return

  allocate(replf(NSY, NFOFF))
  allocate(cmetric(ncand), cbestit(ncand), cf0(ncand))
  allocate(peakidx(ncand))

  ! ===================== Stage A: coarse carrier scan =====================
  do i = 1, ncand
     f0c = f_lo + real(i-1)*GRID_HZ
     cf0(i) = f0c
     call build_replicas(f0c)
     call scan_carrier(cmetric(i), cbestit(i))
  enddo

  ! ===================== Stage B: peak-pick =====================
  median = median_of(cmetric, ncand)
  threshold = NOISE_FACTOR * median
  if (threshold <= 0.0) threshold = tiny(1.0)

  npeak = 0
  do i = 1, ncand
     if (cmetric(i) <= threshold) cycle
     ! local maximum (plateau-safe): strictly greater than the right neighbour,
     ! >= the left neighbour.
     if (i > 1) then
        if (cmetric(i) < cmetric(i-1)) cycle
     endif
     if (i < ncand) then
        if (cmetric(i) <= cmetric(i+1)) cycle
     endif
     npeak = npeak + 1
     peakidx(npeak) = i
  enddo

  ! Sort peaks by metric descending (simple selection sort; npeak is small).
  do i = 1, npeak-1
     do j = i+1, npeak
        if (cmetric(peakidx(j)) > cmetric(peakidx(i))) then
           call iswap(peakidx(i), peakidx(j))
        endif
     enddo
  enddo

  ! ===================== Stage C: decode survivors =====================
  do isurv = 1, npeak
     if (ndec >= maxdec) exit
     if (isurv > MAX_SURV) exit       ! bound Stage C attempts (metric-sorted, so
                                      !   real signals -- high metric -- come first)
     i = peakidx(isurv)
     f0c = cf0(i)

     ! Greedy min-separation against already-accepted DECODES (drop the broad
     ! chirp response's neighbouring cells; CRC handles the rest).
     dup = .false.
     do j = 1, ndec
        if (abs(freqs(j) - f0c) < MINSEP_HZ) then
           dup = .true.
           exit
        endif
     enddo
     if (dup) cycle

     ! Full-resolution re-sync around the coarse time, widened to cover the
     ! chirp time/frequency ambiguity (see STAGE_C_MARGIN).
     call dx1_sync(dd, ndd, f0c, fsample, &
                   max(cbestit(i)-STAGE_C_MARGIN, ilo), &
                   min(cbestit(i)+STAGE_C_MARGIN, ihi), istart, df, sync_l)
     f0eff = f0c + df

     call dx1_detect(dd, ndd, istart, f0eff, fsample, energy, noisevar)

     ! Crude per-symbol SNR (matches dx1_decode).
     sigsum = 0.d0
     do isym = 1, DX1_NSYM
        sigsum = sigsum + dble(maxval(energy(:,isym)))
     enddo
     if (noisevar > 0.0) then
        snr_l = 10.0*log10(max(real(sigsum/dble(DX1_NSYM))/noisevar, 1.0e-6))
     else
        snr_l = -99.0
     endif

     call dx1_llr(energy, noisevar, llr)
     apmask = 0
     call bpdecode174_91(llr, apmask, BP_MAXITER, msg77, cw, nh, iter, ncheck)
     if (nh < 0) cycle

     write(c77,'(77i1)') msg77
     call unpack77(c77, 1, msg37, unpk_ok)
     if (.not. unpk_ok) cycle
     if (len_trim(msg37) == 0) cycle

     ! De-dup identical text at a nearby carrier (same emitter via two cells).
     dup = .false.
     do j = 1, ndec
        if (trim(msgs(j)) == trim(msg37) .and. &
            abs(freqs(j) - f0eff) < MINSEP_HZ) then
           dup = .true.
           exit
        endif
     enddo
     if (dup) cycle

     ndec = ndec + 1
     msgs(ndec)  = msg37
     freqs(ndec) = f0eff
     snrs(ndec)  = snr_l
     syncs(ndec) = sync_l
  enddo

  deallocate(replf, cmetric, cbestit, cf0, peakidx)
  return

contains

  ! Build the NFOFF pre-folded chirp replicas at carrier f0: each column is the
  ! conjugate chirp times the conjugate of one frequency-offset phasor, so the
  ! Stage-A inner product is a pure complex MAC (no trig in the hot loop).
  ! Mathematically identical to dx1_sync's per-(it,jf) correland.
  subroutine build_replicas(f0)
    real, intent(in) :: f0
    real(8) :: twopi, dt, t, phch, fbase, chirp_rate, foff, arg
    integer :: k, jfl
    twopi = 8.d0*atan(1.d0)
    dt = 1.d0/dble(fsample)
    fbase = dble(f0)
    chirp_rate = dble(DX1_BW)/(dble(NSY)*dt)
    do jfl = 1, NFOFF
       foff = dble(jfl - (NFOFF+1)/2)*dble(FSTEP)    ! -4..+4 * FSTEP
       phch = 0.d0
       do k = 1, NSY
          t = dble(k-1)*dt
          phch = phch + twopi*(fbase + chirp_rate*t)*dt   ! chirp phase
          arg = phch + twopi*foff*t                        ! + freq-offset phase
          replf(k, jfl) = cmplx(real(cos(arg)), -real(sin(arg)))
       enddo
    enddo
  end subroutine build_replicas

  ! Coarse time x (pre-folded) freq correlation at the current replf carrier.
  ! Returns the peak |correlation|^2 and the coarse sync-start sample.
  !
  ! Ranks by the RAW matched-filter output |acc|^2 -- NOT |acc|^2/sigpow.  The
  ! chirp correlation is frequency-selective, so an out-of-band neighbour (a
  ! signal at another carrier overlapping in time) is orthogonal to this
  ! carrier's replica and contributes ~0 to acc.  Normalising by total windowed
  ! power, as the single-carrier dx1_sync does, would inflate sigpow with the
  ! neighbour's energy and SUPPRESS this carrier's metric -- which dropped a
  ! perfectly-decodable signal below the peak-pick threshold in multi-signal
  ! scenes.  The median-adaptive threshold in Stage B supplies the noise
  ! reference (white noise => uniform |acc|^2 noise floor across the band).
  subroutine scan_carrier(metric_out, bestit_out)
    real,    intent(out) :: metric_out
    integer, intent(out) :: bestit_out
    integer :: it, k, jfl
    real(8) :: accre, accim
    real    :: mag
    metric_out = -1.0
    bestit_out = ilo
    do it = ilo, ihi, TSTEP_A
       do jfl = 1, NFOFF
          accre = 0.d0
          accim = 0.d0
          do k = 1, NSY
             accre = accre + dble(dd(it+k-1))*dble(real(replf(k,jfl)))
             accim = accim + dble(dd(it+k-1))*dble(aimag(replf(k,jfl)))
          enddo
          mag = real(accre*accre + accim*accim)
          if (mag > metric_out) then
             metric_out = mag
             bestit_out = it
          endif
       enddo
    enddo
  end subroutine scan_carrier

  subroutine iswap(a, b)
    integer, intent(inout) :: a, b
    integer :: t
    t = a; a = b; b = t
  end subroutine iswap

  ! Median of x(1:n) via a sorted copy (n is small: ~hundreds of candidates).
  real function median_of(x, n)
    integer, intent(in) :: n
    real,    intent(in) :: x(n)
    real, allocatable :: c(:)
    integer :: a, b
    real    :: t
    allocate(c(n))
    c = x(1:n)
    do a = 1, n-1                 ! insertion sort
       t = c(a+1)
       b = a
       do while (b >= 1)
          if (c(b) <= t) exit
          c(b+1) = c(b)
          b = b - 1
       enddo
       c(b+1) = t
    enddo
    if (mod(n,2) == 0) then
       median_of = 0.5*(c(n/2) + c(n/2+1))
    else
       median_of = c((n+1)/2)
    endif
    deallocate(c)
  end function median_of

end subroutine dx1_decode_all
