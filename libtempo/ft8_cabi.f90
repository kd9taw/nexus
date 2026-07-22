! Nexus: C ABI wrappers for native FT8 decode/encode, built on the vendored
! WSJT-X GPL modem sources (lib/ft8). Exposes a clean iso_c_binding interface so
! the FT8 mode can be driven headlessly from C / Rust, mirroring tempofast_cabi.f90.
! No Qt, no GUI, and none of WSJT-X's nzhsym streaming / shmem machinery: this
! decodes one complete 15 s frame by calling the core decode primitives directly
! (ft8apset -> sync8 -> ft8b), exactly as the inner loop of ft8_decode::decode
! does (ft8_decode.f90 lines ~172-239). ft8b performs its own signal subtraction
! (lsubtract), so multi-pass weak-signal recovery works.
!
! The a7 CROSS-CYCLE AP path (WSJT-X iaptype=7) IS wired in, reusing the vendored
! ft8_a7 module verbatim: every direct decode of the authoritative pass is saved
! into ft8_a7's per-parity slot table (ft8_a7_save), and after the pass loop each
! remembered call pair from the PREVIOUS same-parity slot is replayed as ~206
! QSO-continuation hypotheses against the residual audio (ft8_a7d), recovering
! RR73/73/RRR/report/grid continuations a few dB below the direct threshold. The
! caller supplies the slot key `nutc` (slot UTC seconds-of-day) and `la7final`
! (1 = authoritative full-audio pass: save + replay; 0 = early partial pass:
! slot bookkeeping only). ft8_a7_reset() clears the table on band/QSO change.
!
! Underlying Fortran routines wrapped here:
!   genft8       (ft8/genft8.f90)      - message -> 79 channel tones
!   gen_ft8wave  (ft8/gen_ft8wave.f90) - tones -> real audio waveform
!   ft8apset     (ft8/ft8apset.f90)    - a-priori symbol setup from callsigns
!   sync8        (ft8/sync8.f90)       - Costas sync candidate search
!   ft8b         (ft8/ft8b.f90)        - per-candidate decode (+ internal subtract)
!
! Frame / array constants (from ft8/ft8_params.f90):
!   NN   = 79      total channel symbols
!   NSPS = 1920    samples per symbol @ 12 kHz
!   NMAX = 180000  raw audio samples (15.0 s @ 12 kHz)
!   NH1  = 1920    half-FFT length (sbase sizing)
!   NZ   = 151680  samples in the full 12.64 s waveform (NSPS*NN)

module ft8_cabi
  use iso_c_binding
  ! Vendored WSJT-X a7 cross-cycle AP table (process-global SAVE state). Renamed
  ! imports keep the short table names (f0, ndec, ...) from colliding with this
  ! module's dummy args and result variables. One-directional: ft8_a7 does not
  ! use ft8_cabi, so no module cycle.
  use ft8_a7, only: a7_dt0 => dt0, a7_f0 => f0, a7_msg0 => msg0, &
       a7_jseq => jseq, a7_ndec => ndec, ft8_a7_save, ft8_a7d
  ! Per-chain decoder context (tempo_ctx_size/reset/save/restore, below). Every
  ! import is renamed so nothing shadows a dummy argument or local in this file;
  ! these names appear ONLY in the ctx type and in ctx_xfer. See the block comment
  ! above `tempo_ctx_t` for why each symbol is here.
  use ft8_a7, only: cxv_a7_dt0 => dt0, cxv_a7_f0 => f0, cxv_a7_msg0 => msg0
  use ft8_downsample_state,       only: cxv_ft8_spec => cx
  use ft4_downsample_state,       only: cxv_ft4_spec => cx
  use tempofast_downsample_state, only: cxv_ft1_spec => cx
  use ir_harq_combine_mod,        only: harq_slot, cxv_slots => slots, &
       cxv_harq_init => harq_initialized
  use packjt77, only: cxv_calls10 => calls10, cxv_calls12 => calls12,   &
       cxv_calls22 => calls22, cxv_recent => recent_calls,              &
       cxv_mycall13 => mycall13, cxv_dxcall13 => dxcall13,              &
       cxv_ihash22 => ihash22, cxv_nzhash => nzhash
  implicit none

  ! Last slot key (nutc) seen by ft8_decode_frame; -1 = virgin/reset. Drives the
  ! once-per-slot k=1 -> k=0 shuffle of the a7 table (module scope => SAVE).
  integer :: nutc0_a7 = -1

  integer, parameter :: F8_NN      = 79
  integer, parameter :: F8_NSPS    = 1920
  integer, parameter :: F8_NMAX    = 180000          ! 15 * 12000
  integer, parameter :: F8_NH1     = 1920            ! NFFT1/2 = (2*1920)/2
  integer, parameter :: F8_NZ      = F8_NSPS * F8_NN ! 151680
  integer, parameter :: F8_MAXCAND = 600
  integer, parameter :: F8_MAXDEC  = 200   ! stock WSJT-X per-period cap (MAXDEC)

  ! Interop result struct. Layout MUST match ft8_decode_t in libtempo.h.
  !   off 0  float sync; 4 int snr; 8 float dt; 12 float freq;
  !   16 char message[38]; 54 (pad 2) int nap; 60 float qual; total 64.
  type, bind(C) :: ft8_decode_t
     real(c_float)          :: sync         ! sync metric
     integer(c_int)         :: snr          ! SNR estimate, dB (rounded)
     real(c_float)          :: dt           ! time offset, seconds (xdt-0.5)
     real(c_float)          :: freq         ! audio frequency, Hz
     character(kind=c_char) :: message(38)  ! NUL-terminated decoded text
     integer(c_int)         :: nap          ! AP type used (iaptype; 0 = none)
     real(c_float)          :: qual         ! decode quality metric [0,1]
  end type ft8_decode_t

  !-------------------------------------------------------------------------
  ! PER-CHAIN DECODER CONTEXT
  !
  ! The modem's decode state is process-global Fortran SAVE storage, so two
  ! radio chains decoding two bands in ONE process share every byte of it.
  ! That does not crash: chain A's a7 replay list / callsign hash table /
  ! IR-HARQ slot pool / cached wideband spectrum, consumed by chain B, yields a
  ! CRC-valid, syntactically perfect, WRONG decode - logged and uploaded, and
  ! indistinguishable afterwards from a real QSO. `tempo_ctx_t` is one chain's
  ! private copy of that state; the C entry points below swap it in and out
  ! around a decode() call.
  !
  ! WHICH SYMBOLS. libtempo/modem-state-manifest.toml is authoritative: its
  ! class-1 rows ARE this set. The components below are the class-1 symbols that
  ! are EXTERNALLY ADDRESSABLE - module variables (plus one COMMON block). The
  ! remaining class-1 rows are subroutine-local SAVE/DATA variables inside
  ! the vendored tree, which gfortran emits as FILE-LOCAL symbols (`nm` shows
  ! them lowercase: `mcq.34`, `apbits.5`, `hashmy10.13`, ...). No other
  ! compilation unit can name them, so they cannot be swapped without hoisting
  ! them into modules the way ft8/ft4/tempofast_downsample_state already were.
  !
  ! That leaves 43 of the manifest's 68 class-1 rows (26,598 B of 3,530,670)
  ! OUT of this context, and they are NOT all harmless: ft4_decode.f90's
  ! apbits/apmy_ru/aphis_fd/mycall0/hiscall0, tempofast_decode.f90's
  ! apbits/mycall0/hiscall0 and packjt77 unpack77's mycall13_0/dxcall13_0 +
  ! hashmy*/hashdx* are AP masks and hash memos latched on the live callsign
  ! pair, so two chains working DIFFERENT stations still share them. Hoisting
  ! those into modules is the next step, and until it happens two chains are
  ! safe on the tables that fabricate callsigns from hashes and on the a7
  ! replay, but not on the AP masks derived from hiscall.
  !
  ! LAYOUT. Bounds and character lengths are taken from the LIVE declarations
  ! (`size(...)` / `len(...)` of the use-associated symbol), never re-typed, so a
  ! vendor refresh that resizes a table resizes this context with it instead of
  ! silently truncating a copy.
  !
  ! FRESH CONTEXTS ARE NOT ZERO. Several of these symbols are DATA-initialized
  ! (.data, not .bss): ihash22 starts at -1 (0 means "hash slot 0 is OCCUPIED",
  ! by a blank callsign), every callsign table starts SPACE-filled (NUL-filled,
  ! `len(trim(...))` is 13 and hash10/hash12 return a callsign of NULs instead
  ! of `<...>`), and nutc0_a7 starts at -1 (0 is a valid slot key). Zero-filling
  ! a context and restoring it would corrupt the modem, which is why
  ! tempo_ctx_reset() exists and why every row of ctx_xfer's list carries its
  ! own load-time value.
  !
  ! Verified byte-for-byte against a virgin process (save the statics before
  ! anything runs, reset a second buffer, memcmp): 3,474,476 of 3,504,084 bytes
  ! are identical, and the two regions that differ are deliberate - a7_msg0 and
  ! /pfxcom/ addpfx are blank-filled here where the loader leaves .bss NULs.
  ! Blank is the correct empty for both (ft8_a7_reset below already assigns
  ! `a7_msg0 = ' '`, and getpfx1 tests `index(addpfx,' ')`), and neither is read
  ! before it is written in a fresh context.
  !
  ! The type deliberately declares NO default initialization, direct or inherited
  ! from a component's type. gfortran materializes a large derived type's default
  ! initializer as a STACK temporary at `allocate` - that overflowed a 2 MiB
  ! worker-thread stack for a 3.4 MiB context (measured: SIGSEGV at any stack
  ! below ~4 MiB; fine at 256 KiB once the initializers were gone) - and, if any
  ! byte of the template is non-zero, puts 3.4 MiB in .rodata that every shipped
  ! binary carries (measured with `size -A`: libtempo.so 6.30 MB -> 2.79 MB).
  ! CTX_INIT does the same job with neither cost.
  !-------------------------------------------------------------------------

  ! 8-byte words needed to hold ir_harq_combine_mod's whole slot pool, taken
  ! from the live declaration (element size x count, rounded up).
  integer, parameter :: CTX_HARQ_WORDS = &
       (size(cxv_slots) * (storage_size(cxv_slots) / 8) + 7) / 8

  type :: tempo_ctx_t
     ! --- FT8 -------------------------------------------------------------
     ! ft8_downsample_state cx/x: this chain's whole 192000-point wideband
     ! spectrum, refreshed only under the caller-owned `newdat`. Both call
     ! sites pass .false., so most calls CONSUME a spectrum they did not
     ! compute - shared, that downsamples chain B's audio at chain A's
     ! frequency. cx is the complex face of the EQUIVALENCE and is >= the real
     ! face `x`, so copying cx carries the whole block.
     complex           :: ft8_spec(size(cxv_ft8_spec))
     ! ft8_a7 cross-cycle AP table: the prior same-parity slot's decoded call
     ! pairs, replayed as ~206 QSO-continuation hypotheses. Shared, chain A's
     ! call pairs are replayed against chain B's audio (iaptype 7). These five
     ! move as ONE unit - the replay loop is bounded solely by ndec.
     real              :: a7_dt0(size(cxv_a7_dt0,1), 0:1, 0:1)
     real              :: a7_f0(size(cxv_a7_f0,1), 0:1, 0:1)
     character(len=len(cxv_a7_msg0)) :: a7_msg0(size(cxv_a7_msg0,1), 0:1, 0:1)
     integer           :: a7_ndec(0:1, 0:1)
     integer           :: a7_jseq
     ! ft8_cabi's own slot tracker. Splitting it from a7_jseq desynchronizes
     ! the even/odd rollover, so it lives in the same context. -1 = virgin.
     integer           :: cabi_nutc0_a7
     ! --- FT4 -------------------------------------------------------------
     complex           :: ft4_spec(size(cxv_ft4_spec))
     ! --- FT1 / TempoFast --------------------------------------------------
     complex           :: ft1_spec(size(cxv_ft1_spec))
     ! IR-HARQ soft-combining pool - the single largest object here. Shared,
     ! chain A's RV0 frame combines with chain B's RV1.
     !
     ! Held as OPAQUE 8-byte words rather than `type(harq_slot)` directly, for
     ! one reason: harq_slot carries component initializers (rv_count = -1,
     ! snr_est = -99.0, npts = HARQ_NDMAX), a derived-type component propagates
     ! them into the compiler-generated default-initializer template for the
     ! WHOLE of tempo_ctx_t, and a template with any non-zero byte lands in
     ! .rodata - 3.4 MiB of it, in every shipped binary, referenced by nothing at
     ! run time (measured with `size -A`; all-zero, it lands in .bss and costs
     ! no file bytes). Sized from the live symbol, and ctx_xfer maps it straight
     ! back to `type(harq_slot)` so the copy itself is still fully type-checked.
     integer(c_int64_t) :: harq_pool(CTX_HARQ_WORDS)
     logical           :: harq_init
     ! --- shared 77-bit / callsign machinery -------------------------------
     ! packjt77's hash tables: how a hashed <...> token resolves to a full
     ! callsign. THE cross-contamination vector - shared, chain B prints chain
     ! A's callsigns for hashes it never heard.
     character(len=len(cxv_calls10)) :: calls10(size(cxv_calls10))
     character(len=len(cxv_calls12)) :: calls12(size(cxv_calls12))
     character(len=len(cxv_calls22)) :: calls22(size(cxv_calls22))
     character(len=len(cxv_recent))  :: recent_calls(size(cxv_recent))
     character(len=len(cxv_mycall13)) :: mycall13
     character(len=len(cxv_dxcall13)) :: dxcall13
     integer           :: ihash22(size(cxv_ihash22))
     integer           :: nzhash
     ! COMMON /pfxcom/ - the add-on prefix used when packing/unpacking a
     ! compound call. Per-QSO; a wrong prefix is a well-formed WRONG callsign.
     ! Declared character*8 at packjt.f90:753 (re-declared in ctx_xfer).
     character(len=8)  :: addpfx
  end type tempo_ctx_t

  ! What ctx_xfer's single list is being walked FOR.
  integer, parameter :: CTX_SAVE    = 1   ! live statics -> context
  integer, parameter :: CTX_RESTORE = 2   ! context -> live statics
  integer, parameter :: CTX_INIT    = 3   ! load-time value -> context

  ! Mode-driven copy helpers, one per (type, rank) the context holds. They exist
  ! so ctx_xfer can state the symbol list ONCE: save, restore and init all walk
  ! the SAME list and cannot drift apart. The compiler resolves the generic by
  ! type+rank, so a component that drifts from its symbol fails to compile.
  interface ctx_move
     module procedure ctx_move_c1, ctx_move_r3, ctx_move_a3, ctx_move_i2, &
          ctx_move_i1, ctx_move_i0, ctx_move_l0, ctx_move_a1, ctx_move_a0, &
          ctx_move_h1
  end interface ctx_move

contains

  !-------------------------------------------------------------------------
  ! ft8_encode : message text -> 79 channel tones {0..7}
  !   msg       : NUL/space-terminated C string (<= 37 chars)
  !   msg_len   : valid chars in msg
  !   itone_out : output 79 tones
  !   nsym_out  : symbols written (79), or -1 on bad message
  !-------------------------------------------------------------------------
  subroutine ft8_encode(msg, msg_len, itone_out, nsym_out) bind(C, name="ft8_encode")
    character(kind=c_char), intent(in)  :: msg(*)
    integer(c_int), value,  intent(in)  :: msg_len
    integer(c_int),         intent(out) :: itone_out(F8_NN)
    integer(c_int),         intent(out) :: nsym_out

    character(len=37) :: msg37, msgsent37
    integer(kind=1)   :: msgbits(77)
    integer           :: itone(F8_NN)
    integer           :: i, n, i3, n3

    msg37 = ' '
    n = min(msg_len, 37)
    do i = 1, n
       if (msg(i) == c_null_char) exit
       msg37(i:i) = msg(i)
    end do

    call genft8(msg37, i3, n3, msgsent37, msgbits, itone)
    if (i3 < 0 .and. n3 < 0) then
       nsym_out = -1
       return
    end if
    itone_out(1:F8_NN) = itone(1:F8_NN)
    nsym_out = F8_NN
  end subroutine ft8_encode

  !-------------------------------------------------------------------------
  ! ft8_gen_wave : tones -> real audio waveform
  !   itone     : input tones (length nsym)
  !   nsym      : number of tones (79)
  !   fsample   : output sample rate (Hz), e.g. 12000.0
  !   f0        : audio carrier (Hz), e.g. 1500.0
  !   wave_out  : caller buffer (capacity nwave_out)
  !   nwave_out : in = capacity; out = samples produced (nsym*NSPS)
  !-------------------------------------------------------------------------
  subroutine ft8_gen_wave(itone, nsym, fsample, f0, wave_out, nwave_out) &
       bind(C, name="ft8_gen_wave")
    integer(c_int),        intent(in)    :: itone(*)
    integer(c_int), value, intent(in)    :: nsym
    real(c_float),  value, intent(in)    :: fsample, f0
    real(c_float),         intent(inout) :: wave_out(*)
    integer(c_int),        intent(inout) :: nwave_out

    integer :: nwave, itone_l(nsym)
    real    :: f0_l, fs_l, bt
    complex :: cwave(nsym*F8_NSPS)

    nwave = nsym * F8_NSPS
    if (nwave_out < nwave) then
       nwave_out = -1
       return
    end if
    itone_l(1:nsym) = itone(1:nsym)
    f0_l = f0
    fs_l = fsample
    bt   = 2.0   ! FT8 Gaussian BT
    call gen_ft8wave(itone_l, nsym, F8_NSPS, bt, fs_l, f0_l, cwave, &
                     wave_out, 0, nwave)
    nwave_out = nwave
  end subroutine ft8_gen_wave

  !-------------------------------------------------------------------------
  ! ft8_decode_frame : decode EVERY FT8 signal in a complete 15 s frame.
  !
  !   iwave         : F8_NMAX (180000) int16 audio samples @ 12 kHz
  !   nfa, nfb      : frequency search band edges (Hz)
  !   ndepth        : 1..3 (3 = full bp+osd, 3 passes; <=0 defaults to 3)
  !   mycall,hiscall: NUL/space-terminated callsigns for AP (may be empty)
  !   nqso_progress : QSO progress index (AP pass schedule)
  !   nfqso_in      : QSO/RX audio freq (Hz) the operator is working — WSJT-X's
  !                   nfqso. The deep a-priori passes (iaptype>=3, the MyCall+
  !                   DxCall masks) only fire within napwid of this, and sync8
  !                   prioritizes near it. Pass 0 / out-of-band ⇒ band center.
  !   nutc          : slot key = slot UTC seconds-of-day (0..86399; slot*15 for
  !                   FT8). Keys the a7 cross-cycle table: parity = mod(nutc/5,2)
  !                   and a nutc change rolls the per-parity table over. A nutc
  !                   BEHIND the last seen slot (a redecode of an older capture)
  !                   suppresses all a7 state changes so a replayed old slot
  !                   cannot scramble the live even/odd ping-pong.
  !   la7final      : 1 on the authoritative full-audio (boundary) pass — saves
  !                   direct decodes into the a7 table and runs the a7 replay.
  !                   0 on the early/partial pass — slot bookkeeping only.
  !   out           : caller array of ft8_decode_t (capacity max_out)
  !   max_out       : capacity of out
  !
  !   Returns the number of decodes written (>= 0), or -1 on error.
  !
  ! NOT thread-safe (the modem keeps process-global SAVE state + FFTW plans).
  !-------------------------------------------------------------------------
  function ft8_decode_frame(iwave, nfa, nfb, ndepth, mycall, hiscall, &
       nqso_progress, nfqso_in, nutc, la7final, out, max_out) result(ndec) &
       bind(C, name="ft8_decode_frame")
    integer(c_int16_t),     intent(in)  :: iwave(F8_NMAX)
    integer(c_int), value,  intent(in)  :: nfa, nfb, ndepth, nqso_progress, nfqso_in
    integer(c_int), value,  intent(in)  :: nutc, la7final, max_out
    character(kind=c_char), intent(in)  :: mycall(*)
    character(kind=c_char), intent(in)  :: hiscall(*)
    type(ft8_decode_t),     intent(out) :: out(*)
    integer(c_int)                      :: ndec

    real              :: dd(F8_NMAX)
    real              :: candidate(3, F8_MAXCAND)
    real              :: sbase(F8_NH1)
    integer           :: apsym(58), aph10(10)
    integer           :: itone(F8_NN)
    character(len=37) :: allmessages(F8_MAXDEC)
    character(len=37) :: msg37
    character(len=12) :: mycall12, hiscall12
    character(len=12) :: call_1, call_2
    character(len=4)  :: grid4
    integer :: ncontest, nfqso, nftx, ndepth_l, ndeep, npass, ipass
    integer :: maxc, ncand, icand, id, n, j, ib
    integer :: iaptype, nharderrors, nbadcrc, iappass
    integer :: nsnr, ndecodes, n2, napwid
    integer :: i, i1, i2, iz, ndelta
    real    :: sync, f1, xdt, xbase, dmin, xsnr, syncmin
    logical :: newdat, lsubtract, ldupe, lft8apon, lapcqonly, nagain
    logical :: is_new_slot, is_stale

    ndec = 0
    if (max_out <= 0) return

    dd(1:F8_NMAX) = real(iwave(1:F8_NMAX))
    call c_to_fstr12(mycall,  mycall12)
    call c_to_fstr12(hiscall, hiscall12)

    ndepth_l = ndepth
    if (ndepth_l <= 0) ndepth_l = 3
    ncontest  = 0
    ! Center deep AP + sync on the operator's QSO/RX freq when supplied; else the
    ! band midpoint (legacy behavior). nftx mirrors it (no separate TX freq here).
    if (nfqso_in >= nfa .and. nfqso_in <= nfb) then
       nfqso = nfqso_in
    else
       nfqso = (nfa + nfb) / 2
    end if
    nftx      = nfqso
    lft8apon  = .true.
    lapcqonly = .false.
    nagain    = .false.
    napwid    = 75

    call ft8apset(mycall12, hiscall12, ncontest, apsym, aph10)

    ! --- a7 cross-cycle slot bookkeeping (mirrors ft8_decode.f90 84-95) -----
    ! On a NEW slot, roll the a7 table for the PREVIOUS slot's parity (module
    ! a7_jseq, still the old value here) from current (k=1) to previous (k=0);
    ! the replay below then reads k=0 = the last same-parity slot's decodes.
    ! A nutc BEHIND nutc0_a7 is a redecode of an OLDER capture (the caller's
    ! F6 path can lag a zero-decode early pass): it must NOT shuffle, re-seed,
    ! or re-key the table — a spurious shuffle here scrambles the even/odd
    ! ping-pong for the following live slots. Deltas <= -43200 (over half a
    ! day back) are a UTC midnight wrap, i.e. genuinely new slots.
    is_stale = .false.
    if (nutc0_a7 < 0) then
       is_new_slot = .true.
    else
       ndelta = nutc - nutc0_a7
       is_new_slot = (ndelta > 0) .or. (ndelta <= -43200)
       is_stale = (ndelta < 0) .and. (ndelta > -43200)
    end if
    if (is_new_slot) then
       iz = a7_ndec(a7_jseq, 1)
       a7_dt0(1:iz, a7_jseq, 0)  = a7_dt0(1:iz, a7_jseq, 1)
       a7_f0(1:iz, a7_jseq, 0)   = a7_f0(1:iz, a7_jseq, 1)
       a7_msg0(1:iz, a7_jseq, 0) = a7_msg0(1:iz, a7_jseq, 1)
       a7_ndec(a7_jseq, 0) = iz
       a7_ndec(a7_jseq, 1) = 0
       a7_dt0(:, a7_jseq, 1) = 0.
       a7_f0(:, a7_jseq, 1)  = 0.
       nutc0_a7 = nutc
    end if
    ! Deterministic parity set (hardening over stock, which relies on
    ! ft8_a7_save to set jseq): equal to what ft8_a7_save would set whenever
    ! >= 1 decode exists, and keeps the ping-pong keyed on zero-decode slots.
    ! With nutc = slot*15 sec-of-day, mod(nutc/5,2) = slot parity.
    if (.not. is_stale) a7_jseq = mod(nutc/5, 2)

    ndecodes = 0
    allmessages = ' '
    n2 = 0

    ! Mirror ft8_decode::decode pass logic. ft8b subtracts decoded signals from
    ! dd internally when lsubtract is set, so later passes find weaker stations.
    npass = 3
    if (ndepth_l == 1) npass = 2
    do ipass = 1, npass
       newdat  = .true.
       syncmin = 1.3
       if (ndepth_l <= 2) syncmin = 1.6
       if (ipass == 1) then
          lsubtract = .true.
          ndeep = ndepth_l
          if (ndepth_l == 3) ndeep = 2
       else if (ipass == 2) then
          n2 = ndecodes
          if (ndecodes == 0) cycle
          lsubtract = .true.
          ndeep = ndepth_l
       else
          if ((ndecodes - n2) == 0) cycle
          lsubtract = .true.
          ndeep = ndepth_l
       end if

       maxc = F8_MAXCAND
       call sync8(dd, F8_NMAX, nfa, nfb, syncmin, nfqso, maxc, candidate, ncand, sbase)

       do icand = 1, ncand
          sync  = candidate(3, icand)
          f1    = candidate(1, icand)
          xdt   = candidate(2, icand)
          ib    = max(1, nint(f1 / 3.125))
          if (ib > F8_NH1) ib = F8_NH1
          xbase = 10.0 ** (0.1 * (sbase(ib) - 40.0))
          msg37 = ' '
          call ft8b(dd, newdat, nqso_progress, nfqso, nftx, ndeep, 50, lft8apon, &
               lapcqonly, napwid, lsubtract, nagain, ncontest, iaptype, mycall12, &
               hiscall12, f1, xdt, xbase, apsym, aph10, nharderrors, dmin, &
               nbadcrc, iappass, msg37, xsnr, itone)
          nsnr = nint(xsnr)
          xdt  = xdt - 0.5
          if (nbadcrc == 0) then
             ldupe = .false.
             do id = 1, ndecodes
                if (msg37 == allmessages(id)) ldupe = .true.
             end do
             if (.not. ldupe) then
                if (ndecodes >= F8_MAXDEC) cycle
                ndecodes = ndecodes + 1
                allmessages(ndecodes) = msg37
                if (ndecodes <= max_out) then
                   out(ndecodes)%sync = sync
                   out(ndecodes)%snr  = nsnr
                   out(ndecodes)%dt   = xdt
                   out(ndecodes)%freq = f1
                   out(ndecodes)%nap  = iaptype
                   out(ndecodes)%qual = 1.0 - (nharderrors + dmin) / 60.0
                   do j = 1, 38
                      out(ndecodes)%message(j) = c_null_char
                   end do
                   n = min(len_trim(msg37), 37)
                   do j = 1, n
                      out(ndecodes)%message(j) = msg37(j:j)
                   end do
                end if
                ! Seed the a7 table for the NEXT same-parity slot. Only on the
                ! authoritative pass: the early pass would double-save the same
                ! stations (the wrapper resets allmessages per call, unlike
                ! stock's carried ndec_early), and the boundary pass's full-
                ! audio decode set is a superset of the early one. xdt here is
                ! already xdt-0.5, matching ft8_decode.f90 line 232.
                if (la7final /= 0 .and. .not. is_stale) then
                   call ft8_a7_save(nutc, xdt, f1, msg37)
                end if
             end if
          end if
       end do
    end do

    ! --- a7 cross-cycle replay (mirrors ft8_decode.f90 245-278) -------------
    ! Authoritative pass only: replay each call pair remembered from the
    ! PREVIOUS same-parity slot as ~206 QSO-continuation hypotheses against
    ! the post-subtraction residual dd (ft8b's lsubtract already removed the
    ! direct decodes — intended, matches WSJT-X). Acceptance is ft8_a7d's
    ! soft-distance gate (dmin<=100 and dmin2/dmin>=1.3, hypotheses are
    ! constructed so CRC validity is implicit); worst case for a stale pair is
    ! wasted CPU, not a false decode. The dedup against allmessages is a small
    ! safe superset of stock (which relies on downstream GUI dedup) keeping
    ! this wrapper's existing no-duplicate-rows contract.
    if (la7final /= 0 .and. .not. is_stale .and. lft8apon .and. &
         a7_ndec(a7_jseq, 0) >= 1) then
       newdat = .true.
       do i = 1, a7_ndec(a7_jseq, 0)
          if (a7_f0(i, a7_jseq, 0) == -99.0) exit
          if (a7_f0(i, a7_jseq, 0) == -98.0) cycle   ! already worked this slot
          if (index(a7_msg0(i, a7_jseq, 0), '<') >= 1) cycle
          msg37 = a7_msg0(i, a7_jseq, 0)
          i1 = index(msg37, ' ')
          i2 = index(msg37(i1+1:), ' ') + i1
          call_1 = msg37(1:i1-1)
          call_2 = msg37(i1+1:i2-1)
          grid4 = msg37(i2+1:i2+4)
          if (grid4 == 'RR73' .or. index(grid4, '+') > 0 .or. &
               index(grid4, '-') > 0) grid4 = '    '
          xdt = a7_dt0(i, a7_jseq, 0)
          f1 = a7_f0(i, a7_jseq, 0)
          ib = max(1, nint(f1 / 3.125))
          if (ib > F8_NH1) ib = F8_NH1
          xbase = 10.0 ** (0.1 * (sbase(ib) - 40.0))
          msg37 = ' '
          call ft8_a7d(dd, newdat, call_1, call_2, grid4, xdt, f1, xbase, &
               nharderrors, dmin, msg37, xsnr)
          if (nharderrors >= 0) then
             ldupe = .false.
             do id = 1, ndecodes
                if (msg37 == allmessages(id)) ldupe = .true.
             end do
             if (.not. ldupe .and. ndecodes < F8_MAXDEC) then
                ndecodes = ndecodes + 1
                allmessages(ndecodes) = msg37
                if (ndecodes <= max_out) then
                   out(ndecodes)%sync = 0.0
                   out(ndecodes)%snr  = nint(xsnr)
                   out(ndecodes)%dt   = xdt      ! ft8_a7d returns t-0.5 already
                   out(ndecodes)%freq = f1
                   out(ndecodes)%nap  = 7
                   out(ndecodes)%qual = 1.0
                   do j = 1, 38
                      out(ndecodes)%message(j) = c_null_char
                   end do
                   n = min(len_trim(msg37), 37)
                   do j = 1, n
                      out(ndecodes)%message(j) = msg37(j:j)
                   end do
                end if
                call ft8_a7_save(nutc, xdt, f1, msg37)
             end if
          end if
       end do
    end if

    ! ndec == max_out here means the cap was hit and the weakest decodes were
    ! dropped: raise F8_MAXDEC and the Rust MAX_DECODES (crates/ft8) together.
    ndec = min(ndecodes, max_out)
  end function ft8_decode_frame

  !-------------------------------------------------------------------------
  ! ft8_a7_reset : clear the a7 cross-cycle decode table + slot tracker.
  !                Call on band change / QSO change so stale prior-cycle call
  !                pairs are not replayed as AP hypotheses against the new
  !                band's audio. Mirrors ft1_harq_reset.
  !-------------------------------------------------------------------------
  subroutine ft8_a7_reset() bind(C, name="ft8_a7_reset")
    a7_ndec = 0
    a7_jseq = 0
    a7_dt0  = 0.0
    a7_f0   = 0.0
    a7_msg0 = ' '
    nutc0_a7 = -1
  end subroutine ft8_a7_reset

  !=========================================================================
  ! PER-CHAIN DECODER CONTEXT - C entry points
  !
  !   tempo_ctx_size()     bytes one context needs. The caller allocates from
  !                        THIS answer, so a vendor refresh that resizes a
  !                        table cannot silently desync the buffer length.
  !   tempo_ctx_reset(p)   write the load-time image into a caller buffer.
  !                        A fresh context is NOT a zeroed one (see tempo_ctx_t).
  !   tempo_ctx_save(p)    copy the live statics OUT into the buffer.
  !   tempo_ctx_restore(p) copy the buffer IN over the live statics.
  !
  ! The buffer is opaque to the caller: sized by tempo_ctx_size(), aligned for
  ! 8-byte scalars, and only ever handed back to these four routines.
  !
  ! NOT thread-safe, by construction. restore -> decode -> save must happen
  ! under the SAME lock that serializes every other modem FFI call
  ! (tempo_fast_sys::MODEM_LOCK); a decode landing between restore and save
  ! is exactly the corruption this exists to prevent.
  !=========================================================================

  !-------------------------------------------------------------------------
  ! tempo_ctx_size : size of one per-chain context, in bytes.
  !-------------------------------------------------------------------------
  function tempo_ctx_size() result(nbytes) bind(C, name="tempo_ctx_size")
    integer(c_size_t) :: nbytes
    type(tempo_ctx_t), allocatable :: probe
    ! tempo_ctx_t has no default initialization (deliberately - see the type),
    ! so this is a plain malloc, not a 3.4 MiB initializer copy.
    allocate(probe)
    nbytes = int(storage_size(probe) / 8, c_size_t)
    deallocate(probe)
  end function tempo_ctx_size

  !-------------------------------------------------------------------------
  ! tempo_ctx_reset : write the modem's LOAD-TIME state into `ptr`.
  !
  ! Reads no modem state and writes none, so it needs no lock - only the
  ! caller's buffer is touched.
  !-------------------------------------------------------------------------
  subroutine tempo_ctx_reset(ptr) bind(C, name="tempo_ctx_reset")
    type(c_ptr), value, intent(in) :: ptr
    type(tempo_ctx_t), pointer     :: p
    if (.not. c_associated(ptr)) return
    call c_f_pointer(ptr, p)
    call ctx_xfer(p, CTX_INIT)
  end subroutine tempo_ctx_reset

  !-------------------------------------------------------------------------
  ! tempo_ctx_save : live statics -> context buffer.
  !-------------------------------------------------------------------------
  subroutine tempo_ctx_save(ptr) bind(C, name="tempo_ctx_save")
    type(c_ptr), value, intent(in) :: ptr
    type(tempo_ctx_t), pointer     :: p
    if (.not. c_associated(ptr)) return
    call c_f_pointer(ptr, p)
    call ctx_xfer(p, CTX_SAVE)
  end subroutine tempo_ctx_save

  !-------------------------------------------------------------------------
  ! tempo_ctx_restore : context buffer -> live statics.
  !-------------------------------------------------------------------------
  subroutine tempo_ctx_restore(ptr) bind(C, name="tempo_ctx_restore")
    type(c_ptr), value, intent(in) :: ptr
    type(tempo_ctx_t), pointer     :: p
    if (.not. c_associated(ptr)) return
    call c_f_pointer(ptr, p)
    call ctx_xfer(p, CTX_RESTORE)
  end subroutine tempo_ctx_restore

  !-------------------------------------------------------------------------
  ! ctx_xfer : THE ORDERED LIST.
  !
  ! Every symbol in the context appears here exactly ONCE, paired with its live
  ! symbol and its LOAD-TIME value; `mode` picks what the walk does. Save,
  ! restore and reset therefore cannot disagree about the member set, because
  ! there is only one member set to disagree with. To add a symbol: one
  ! component on tempo_ctx_t, one row here. Nowhere else.
  !
  ! The load-time values are the third argument. They are not decoration:
  ! ihash22 = -1 marks a hash slot EMPTY (0 means "slot 0 holds calls22(1)", so
  ! a zero-filled table resolves unknown hashes to a blank callsign), the
  ! callsign tables are blank-filled (NULs would print as a callsign of NULs),
  ! and nutc0_a7 = -1 means "no slot seen yet" (0 is a real slot key).
  !-------------------------------------------------------------------------
  subroutine ctx_xfer(p, mode)
    ! `target` so c_loc can take the address of the opaque harq_pool component.
    type(tempo_ctx_t), intent(inout), target :: p
    integer,           intent(in)            :: mode
    ! packjt.f90:753 getpfx1 - re-declared, not use-associated: it is a COMMON
    ! block, and the linker merges this declaration with that one.
    character(len=8) :: addpfx
    common /pfxcom/ addpfx
    ! The opaque harq_pool, viewed as what it actually holds, so the copy below
    ! is a type-checked harq_slot-to-harq_slot assignment. See tempo_ctx_t.
    type(harq_slot), pointer :: ctx_slots(:)

    call c_f_pointer(c_loc(p%harq_pool), ctx_slots, [size(cxv_slots)])

    !            context component     live symbol    mode   load-time value
    ! --- FT8 ---
    call ctx_move(p%ft8_spec,      cxv_ft8_spec,  mode, (0.0, 0.0))
    call ctx_move(p%a7_dt0,        a7_dt0,        mode, 0.0)
    call ctx_move(p%a7_f0,         a7_f0,         mode, 0.0)
    call ctx_move(p%a7_msg0,       a7_msg0,       mode, ' ')
    call ctx_move(p%a7_ndec,       a7_ndec,       mode, 0)
    call ctx_move(p%a7_jseq,       a7_jseq,       mode, 0)
    call ctx_move(p%cabi_nutc0_a7, nutc0_a7,      mode, -1)
    ! --- FT4 ---
    call ctx_move(p%ft4_spec,      cxv_ft4_spec,  mode, (0.0, 0.0))
    ! --- FT1 / TempoFast ---
    call ctx_move(p%ft1_spec,      cxv_ft1_spec,  mode, (0.0, 0.0))
    call ctx_move(ctx_slots,       cxv_slots,     mode)
    call ctx_move(p%harq_init,     cxv_harq_init, mode, .false.)
    ! --- shared 77-bit / callsign machinery ---
    call ctx_move(p%calls10,       cxv_calls10,   mode, ' ')
    call ctx_move(p%calls12,       cxv_calls12,   mode, ' ')
    call ctx_move(p%calls22,       cxv_calls22,   mode, ' ')
    call ctx_move(p%recent_calls,  cxv_recent,    mode, ' ')
    call ctx_move(p%mycall13,      cxv_mycall13,  mode, ' ')
    call ctx_move(p%dxcall13,      cxv_dxcall13,  mode, ' ')
    call ctx_move(p%ihash22,       cxv_ihash22,   mode, -1)
    call ctx_move(p%nzhash,        cxv_nzhash,    mode, 0)
    call ctx_move(p%addpfx,        addpfx,        mode, ' ')
  end subroutine ctx_xfer

  ! --- ctx_move specifics. `a` is always the context side, `b` the live symbol.
  !     One per (type, rank) the context holds; the generic resolves by type and
  !     rank, so a component that drifts from its symbol fails to compile.

  subroutine ctx_move_c1(a, b, mode, init)      ! complex, rank 1
    complex, intent(inout) :: a(:), b(:)
    integer, intent(in)    :: mode
    complex, intent(in)    :: init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_c1

  subroutine ctx_move_r3(a, b, mode, init)      ! real, rank 3
    real,    intent(inout) :: a(:,:,:), b(:,:,:)
    integer, intent(in)    :: mode
    real,    intent(in)    :: init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_r3

  subroutine ctx_move_a3(a, b, mode, init)      ! character, rank 3
    character(len=*), intent(inout) :: a(:,:,:), b(:,:,:)
    integer,          intent(in)    :: mode
    character(len=*), intent(in)    :: init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_a3

  subroutine ctx_move_i2(a, b, mode, init)      ! integer, rank 2
    integer, intent(inout) :: a(:,:), b(:,:)
    integer, intent(in)    :: mode, init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_i2

  subroutine ctx_move_i1(a, b, mode, init)      ! integer, rank 1
    integer, intent(inout) :: a(:), b(:)
    integer, intent(in)    :: mode, init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_i1

  subroutine ctx_move_i0(a, b, mode, init)      ! integer scalar
    integer, intent(inout) :: a, b
    integer, intent(in)    :: mode, init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_i0

  subroutine ctx_move_l0(a, b, mode, init)      ! logical scalar
    logical, intent(inout) :: a, b
    integer, intent(in)    :: mode
    logical, intent(in)    :: init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_l0

  subroutine ctx_move_a1(a, b, mode, init)      ! character, rank 1
    character(len=*), intent(inout) :: a(:), b(:)
    integer,          intent(in)    :: mode
    character(len=*), intent(in)    :: init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_a1

  subroutine ctx_move_a0(a, b, mode, init)      ! character scalar
    character(len=*), intent(inout) :: a, b
    integer,          intent(in)    :: mode
    character(len=*), intent(in)    :: init
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)    ; a = init
    end select
  end subroutine ctx_move_a0

  ! type(harq_slot), rank 1. No `init` argument: an empty slot is not one value
  ! but a whole record, and `empty` below IS that record - default-initialized
  ! exactly as ir_harq_combine_mod declares it (active=.false., rv_count=-1,
  ! snr_est=-99.0, npts=HARQ_NDMAX), with the sample buffers zeroed so a fresh
  ! context is byte-deterministic. Mirrors the vendored harq_init().
  subroutine ctx_move_h1(a, b, mode)
    type(harq_slot), intent(inout) :: a(:), b(:)
    integer,         intent(in)    :: mode
    type(harq_slot) :: empty
    integer :: i
    select case (mode)
    case (CTX_SAVE)    ; a = b
    case (CTX_RESTORE) ; b = a
    case (CTX_INIT)
       empty%cd_rv0 = (0.0, 0.0)
       empty%cd_rv1 = (0.0, 0.0)
       empty%cd_rv2 = (0.0, 0.0)
       do i = 1, size(a)
          a(i) = empty
       end do
    end select
  end subroutine ctx_move_h1

  !-------------------------------------------------------------------------
  ! c_to_fstr12 : marshal a NUL/space-terminated C string into character(12),
  !               space-padded (as the FT8 decoder expects callsigns).
  !-------------------------------------------------------------------------
  subroutine c_to_fstr12(cstr, fstr)
    character(kind=c_char), intent(in)  :: cstr(*)
    character(len=12),      intent(out) :: fstr
    integer :: i
    fstr = ' '
    do i = 1, 12
       if (cstr(i) == c_null_char) exit
       fstr(i:i) = cstr(i)
    end do
  end subroutine c_to_fstr12

end module ft8_cabi
