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
module ft1_decode

   type :: ft1_decoder
      procedure(ft1_decode_callback), pointer :: callback
      integer :: cur_rv = -1         ! RV (0/1/2) of the decode currently reported,
                                     ! read by the callback (the abstract interface
                                     ! is unchanged, so other implementers are unaffected)
      integer :: frame_time_ms = 0   ! wall-clock ms of this frame, for cross-frame
                                     ! IR-HARQ slot keying/expiry (set before decode())
   contains
      procedure :: decode
   end type ft1_decoder

   abstract interface
      subroutine ft1_decode_callback (this,sync,snr,dt,freq,decoded,nap,qual)
         import ft1_decoder
         implicit none
         class(ft1_decoder), intent(inout) :: this
         real, intent(in) :: sync
         integer, intent(in) :: snr
         real, intent(in) :: dt
         real, intent(in) :: freq
         character(len=37), intent(in) :: decoded
         integer, intent(in) :: nap
         real, intent(in) :: qual
      end subroutine ft1_decode_callback
   end interface

contains

   subroutine decode(this,callback,iwave,nQSOProgress,nfqso,    &
      nfa,nfb,ndepth,lapcqonly,ncontest,mycall,hiscall)

! Full FT1 decoder pipeline:
!   1. Convert int16 audio to real, compute spectrogram + Costas sync
!   2. For each candidate: downconvert, fine sync, turbo decode
!   3. OSD fallback, AP decoding (6 types)
!   4. Signal subtraction for multi-decode (3 passes)
!
! Reference: Spec 3e (Receiver Algorithm)
! Template:  ft4_decode.f90

      use timer_module, only: timer
      use packjt77
      use ir_harq_combine_mod
      use cpm_trellis_mod
      use matched_filter_bank_mod
      include 'ft1/ft1_params.f90'

      class(ft1_decoder), intent(inout) :: this
      procedure(ft1_decode_callback) :: callback
      logical, intent(in) :: lapcqonly

      parameter (NDMAX=NMAX/NDOWN)            !Max downsampled samples (48000/54=888)

      character message*37,msgsent*37
      character c77*77
      character*37 decodes(100)
      character*17 cdatetime0
      character*12 mycall,hiscall
      character*12 mycall0,hiscall0
      character*6 hhmmss

      complex cd2(0:NDMAX-1)                  !Complex downsampled waveform
      complex cb(0:NDMAX-1)                   !Working copy after freq correction
      complex cd_harq(0:NDMAX-1)              !RV-aware-synced baseband for IR-HARQ buffering
      complex ctwk_dum(4*NSS)                   !Dummy for sync1d interface
      complex z_coh_g1,z_coh_g2,z_coh_g3      !Coherent sync group correlations
      complex z_coh_try,corr_coh               !Working vars for coherent freq est
      integer s_coh                             !State for coherent correlation
! Coherent sync-group frequency estimation
      real :: dphi_coh_13, dphi_coh_12         !Phase differences G3-G1, G2-G1
      real :: dphi_coh_pred                    !Predicted G2 phase difference
      real :: df_fine_coh                      !Fine freq from coherent method
      real :: err_coh_min, err_coh_try         !Phase consistency errors
      integer :: ik3_coh, im_coh, ik2_coh     !Disambiguation search indices

      real a(5)
      real dd(NMAX)
      real llr_out(174)
      real candidate(3,MAXCAND)               !(freq, snr, rv_index)
      real savg(NH1),sbase(NH1)

      integer apbits(2*ND)
      integer apmy_ru(28),aphis_fd(28)
      integer*2 iwave(NMAX)                   !Raw received data
      integer*1 message77(77),rvec(77),apmask(174),cw(174)
      integer*1 message91(91)
      integer i4tone(NN)
      integer nappasses(0:5)
      integer naptypes(0:5,4)
      integer mcq(29),mcqru(29),mcqfd(29),mcqtest(29),mcqww(29)
      integer mrrr(19),m73(19),mrr73(19)

      logical nohiscall,unpk77_success
      logical first, dobigfft
      logical dosubtract,doosd
      logical harq_ok
      integer*1 harq_msg77(77)
      integer harq_nerr, itime_ms
      integer harq_islot, harq_sibest, harq_srvc, irv_det
      real harq_sfreq

! Frequency search variables
      integer, parameter :: NFREQ=49        !Number of coarse freq trials (±12 Hz at 0.5 Hz)
      real sync_freq(NFREQ)                 !Sync metric at each coarse frequency
      integer ibest_freq(NFREQ)             !Best timing at each coarse frequency
      real df_try                           !Trial frequency offset
      real df_coarse_best                   !Best coarse frequency offset
      real t_samp_est                       !Sample time for twist

! Frequency search variables
      integer :: ncheck_out                  !ncheck from turbo decode
      real :: nsps_dn                        !Downsampled samples per symbol
      integer :: nss_ds                      !Samples per symbol (integer)
      integer, parameter :: NFSWEEP=101      !Fine freq search candidates (+-2.5 Hz)
      real :: df_fine                        !Fine frequency from Viterbi search
      real :: df_total                       !Total frequency correction
      real :: df_c                           !Candidate frequency offset
      real :: df_fine_est                    !Frequency estimate from sync phase fit
      logical, save :: freq_est_ref_ready=.false.  !Track if ref generation clobbered FFT
      real :: xq1, xq2, xq3, dxq           !Quadratic interpolation temps
      integer :: idf_s                       !Frequency sweep loop index
      integer :: ipass_vit, nf_vit          !Two-pass Viterbi loop control
      real :: df_center, df_step            !Pass-specific center freq and step
      complex :: all_corr(NSTATES, 0:3, NN) !MF correlations all sym/state/input
      real :: alpha_vit(NSTATES)             !Viterbi forward variables
      real :: alpha_vit_new(NSTATES)         !Updated Viterbi forward variables
      real :: vit_metric(NFSWEEP)            !Max Viterbi metric per frequency
      real :: vit_best_metric               !Best metric found so far (inline tracking)
      real :: vit_best_prev, vit_best_next  !Neighbors of best for quadratic interp
      real :: vm_prev, vm_curr              !Previous and current metric in sweep
      integer :: idf_best                    !Index of best frequency in sweep
      real :: vit_cos, vit_sin              !Rotation cos/sin per symbol
      real :: bm_vit                         !Viterbi branch metric
      real :: theta_rot                      !Phase rotation angle
      integer :: s_vit, u_vit, n_vit        !Viterbi loop indices
      integer :: s_next_v                    !Next state in Viterbi
      integer :: idx_vit, k_vit, n_samp     !Correlation computation indices
      complex :: corr_vit                    !Correlation accumulator
! Costas arrays for sync verification (0-indexed tones)
      integer icos_rv(0:3,0:2)
      data icos_rv/0,2,3,1, 1,3,2,0, 3,0,2,1/

! Sync map for Viterbi frequency search (sync forcing)
      integer :: sync_sym_map(NN)            !Known symbol at sync positions, -1 for data
      integer :: j_sync, k_sync, s_g1       !G1 phase estimation loop indices
      integer :: idx_g1                      !Sample index for G1 correlation
      complex :: corr_g1, phase_sum_g1       !G1 phase estimation
      real    :: phi_g1                      !G1 phase angle

! Multi-probe frequency search variables
      integer, parameter :: NPROBE=11        !Number of freq probes (±0.25 Hz at 0.05 Hz)
      integer :: iprobe, ibest_probe         !Probe loop index, best probe
      complex :: cb_probe(0:NDMAX-1)         !Probe baseband signal
      integer :: ncheck_probe_val            !ncheck from probe decode
      integer :: ncheck_best_probe           !Best ncheck seen
      integer*1 :: msg91_probe(91)           !Probe message buffer
      integer :: ntype_probe, nharderror_probe
      real :: dmin_probe, df_probe_offset
      real :: llr_probe(174)

! Top-K Phase 1 candidate selection for Phase 2 Viterbi
      integer, parameter :: NTOP=3           !Number of Phase 1 peaks to try
      integer :: itop_idx(3)                 !Indices of top Phase 1 peaks
      real    :: df_top(3)                   !Coarse freq for each top peak
      integer :: ibest_top(3)                !Best timing for each top peak
      real    :: vit_metric_top(3)           !Best Viterbi metric for each peak
      real    :: df_final_top(3)             !Final freq (P1+P2) for each peak
      integer :: itop, jtop, ntop_found      !Loop indices
      real    :: sync_copy(49)               !Copy for peak finding
      real    :: vit_overall_best            !Best Viterbi metric across all peaks

      data first/.true./
      data     mcq/0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0/
      data   mcqru/0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,1,1,1,1,0,0,1,1,0,0/
      data   mcqfd/0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,0,0,1,0,0,1,0,0,0,1,0/
      data mcqtest/0,0,0,0,0,0,0,0,0,1,1,0,0,0,0,1,0,1,0,1,1,1,1,1,1,0,0,1,0/
      data   mcqww/0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1,1,0,0,1,1,0,1,1,1,1,0/
      data    mrrr/0,1,1,1,1,1,1,0,1,0,0,1,0,0,1,0,0,0,1/
      data     m73/0,1,1,1,1,1,1,0,1,0,0,1,0,1,0,0,0,0,1/
      data   mrr73/0,1,1,1,1,1,1,0,0,1,1,1,0,1,0,1,0,0,1/

! FT1 does NOT use XOR scrambling (rvec=0), unlike FT4
      data rvec/77*0/

      save fs,dt_samp,tt,txt,twopi,h,first,apbits,nappasses,naptypes, &
         mycall0,hiscall0

      this%callback => callback
      hhmmss=cdatetime0(8:13)
      dxcall13=hiscall
      mycall13=mycall

      smax1=0.
      nd1=0

      if(first) then
         fs=12000.0/NDOWN                    !Sample rate after downsampling
         dt_samp=1.0/fs                      !Sample interval (s)
         tt=real(NSPS)/12000.0               !Symbol period (s)
         txt=real(NZ)/12000.0                !TX duration (s)
         twopi=8.0*atan(1.0)
         h=0.5                               !CPM modulation index

! FT1 has no rvec scrambling, but keep the AP bit infrastructure
! consistent with FT4 for code reuse. With rvec=0, mod(x+0,2)=x.
         mcq=2*mod(mcq+rvec(1:29),2)-1
         mcqru=2*mod(mcqru+rvec(1:29),2)-1
         mcqfd=2*mod(mcqfd+rvec(1:29),2)-1
         mcqtest=2*mod(mcqtest+rvec(1:29),2)-1
         mcqww=2*mod(mcqww+rvec(1:29),2)-1
         mrrr=2*mod(mrrr+rvec(59:77),2)-1
         m73=2*mod(m73+rvec(59:77),2)-1
         mrr73=2*mod(mrr73+rvec(59:77),2)-1

! AP decode passes per QSO progress state
         nappasses(0)=2
         nappasses(1)=2
         nappasses(2)=2
         nappasses(3)=2
         nappasses(4)=2
         nappasses(5)=3

! AP type assignments per (QSO state, decode pass)
! iaptype:
!   1  CQ     ???    ???           (29 ap bits)
!   2  MyCall ???    ???           (29 ap bits)
!   3  MyCall DxCall ???           (58 ap bits)
!   4  MyCall DxCall RRR           (77 ap bits)
!   5  MyCall DxCall 73            (77 ap bits)
!   6  MyCall DxCall RR73          (77 ap bits)
         naptypes(0,1:4)=(/1,2,0,0/)
         naptypes(1,1:4)=(/2,3,0,0/)
         naptypes(2,1:4)=(/2,3,0,0/)
         naptypes(3,1:4)=(/3,6,0,0/)
         naptypes(4,1:4)=(/3,6,0,0/)
         naptypes(5,1:4)=(/3,1,2,0/)

         mycall0=''
         hiscall0=''
         first=.false.
      endif

! ================================================================
! Prepare AP bits from mycall/hiscall
! ================================================================
      l1=index(mycall,char(0))
      if(l1.ne.0) mycall(l1:)=" "
      l1=index(hiscall,char(0))
      if(l1.ne.0) hiscall(l1:)=" "

      if(mycall.ne.mycall0 .or. hiscall.ne.hiscall0) then
         apbits=0
         apbits(1)=99
         apbits(30)=99
         apmy_ru=0
         aphis_fd=0

         if(len(trim(mycall)) .lt. 3) go to 10

         nohiscall=.false.
         hiscall0=hiscall
         if(len(trim(hiscall0)).lt.3) then
            hiscall0=mycall
            nohiscall=.true.
         endif
         message=trim(mycall)//' '//trim(hiscall0)//' RR73'
         i3=-1
         n3=-1
         call pack77(message,i3,n3,c77)
         call unpack77(c77,1,msgsent,unpk77_success)
         if(i3.ne.1 .or. (message.ne.msgsent) .or. .not.unpk77_success) go to 10
         read(c77,'(77i1)') message77
         apmy_ru=2*mod(message77(1:28)+rvec(2:29),2)-1
         aphis_fd=2*mod(message77(30:57)+rvec(29:56),2)-1
         message77=mod(message77+rvec,2)
         call encode174_91(message77,cw)
         apbits=2*cw-1
         if(nohiscall) apbits(30)=99

10       continue
         mycall0=mycall
         hiscall0=hiscall
      endif

! ================================================================
! Initialize CPM trellis and matched filter bank (for sync1d_ft1)
! ================================================================
      call init_cpm_trellis()
      call init_matched_filters(NSS)

! ================================================================
! Main decode loop
! ================================================================
      ndecodes=0
      decodes=' '
      fa=nfa
      fb=nfb
      dd=iwave                               !Convert int16 -> real

! ndepth=3: 3 subtraction passes, turbo+osd
! ndepth=2: 3 subtraction passes, turbo only (no osd)
! ndepth=1: 1 pass, no subtraction, no osd
      max_iterations=40
      syncmin=2.0
      dosubtract=.true.
      doosd=.true.
      nsp=3
      if(ndepth.eq.2) then
         doosd=.false.
      endif
      if(ndepth.eq.1) then
         nsp=1
         dosubtract=.false.
         doosd=.false.
      endif

! ================================================================
! Subtraction passes
! ================================================================
      do isp=1,nsp
         if(isp.eq.2) then
            if(ndecodes.eq.0) exit
            nd1=ndecodes
         elseif(isp.eq.3) then
            nd2=ndecodes-nd1
            if(nd2.eq.0) exit
         endif

! ================================================================
! Step 1: Sync detection -- find candidates
! ================================================================
         candidate=0.0
         ncand=0
         call timer('ft1sync ',0)
         call ft1_sync(dd,fa,fb,syncmin,nfqso,MAXCAND,savg,     &
            candidate,ncand,sbase)
         call timer('ft1sync ',1)

         dobigfft=.true.

! ================================================================
! Step 2: Process each candidate
! ================================================================
         do icand=1,ncand
            f0=candidate(1,icand)
            snr0=candidate(2,icand)-1.0
            irv=nint(candidate(3,icand))     !RV index hint (0,1,2) from sync
            this%cur_rv=0                    !Reported RV = #RVs combined; standalone/AP
                                             !report 0, HARQ combine overrides to 1/2 below.
                                             !(Independent of the sync RV hint, which is
                                             !an unvalidated spectrogram discriminator.)
! ================================================================
! Step 2a: Downsample to ~8 samples/symbol
! ================================================================
            call timer('ft1down ',0)
            call ft1_downsample(dd,dobigfft,f0,cd2)
            call timer('ft1down ',1)
            if(dobigfft) dobigfft=.false.

! Normalize power
            sum2=sum(cd2*conjg(cd2))/(real(NMAX)/real(NDOWN))
            if(sum2.gt.0.0) cd2=cd2/sqrt(sum2)


               f1=f0

! ================================================================
! Step 2c: SNR estimate for turbo decoder
! ================================================================
               if(snr0.gt.0.0) then
                  xsnr=10*log10(snr0)-14.8
               else
                  xsnr=-21.0
               endif
               snr_est=max(-21.0,xsnr)

! ================================================================
! Step 2d: Coarse sync sweep + Viterbi frequency search
! Phase 1: Coarse sweep at 0.5 Hz steps over +-12 Hz using sync1d
!   (magnitude-based). Gets frequency to +-0.25 Hz and timing.
! Phase 2: Viterbi forward metric sweep over +-2.5 Hz at 0.05 Hz
!   steps. Full CPM trellis with phase-rotated branch metrics
!   avoids pi/2 phase ambiguity. Quadratic interpolation.
! Phase 3: Single turbo call at the refined frequency.
! ================================================================

               nss_ds = NSS
               nsps_dn = real(NSPS_NUM)/(real(NSPS_DEN)*real(NDOWN))

! Phase 1: Coarse sync metric at 1 Hz steps
               call timer('sync1d  ',0)
               sync_freq = 0.0
               ibest_freq = 0
               do ifreq=1,NFREQ
                  df_try = real(ifreq - 25) * 0.5  !-12 to +12 Hz at 0.5 Hz steps
                  do i=0,NDMAX-1
                     t_samp_est = real(i) * dt_samp
                     cb(i) = cd2(i) * &
                          cmplx(cos(-twopi*df_try*t_samp_est), &
                                sin(-twopi*df_try*t_samp_est))
                  enddo
                  smax_all=-99.
                  ibest_all=0
                  do istart=0,200,4
                     call sync1d_ft1(cb,istart,ctwk_dum,0,  &
                          icos_rv(0:3,0),sync)
                     if(sync.gt.smax_all) then
                        smax_all=sync
                        ibest_all=istart
                     endif
                  enddo
                  sync_freq(ifreq) = smax_all

                  ! Fine timing around coarse best
                  ibest=-1
                  smax=-99.
                  do istart=max(0,ibest_all-5),ibest_all+5
                     call sync1d_ft1(cb,istart,ctwk_dum,0,  &
                          icos_rv(0:3,0),sync)
                     if(sync.gt.smax) then
                        smax=sync
                        ibest=istart
                     endif
                  enddo
                  ibest_freq(ifreq) = ibest
                  sync_freq(ifreq) = smax
               enddo

! Find top-NTOP Phase 1 peaks (separated by >=2 grid points = 1 Hz)
               sync_copy = sync_freq
               ntop_found = 0
               do itop = 1, NTOP
                  ifreq = maxloc(sync_copy, 1)
                  if(sync_copy(ifreq) .le. -90.0) exit
                  ntop_found = ntop_found + 1
                  itop_idx(ntop_found) = ifreq
                  ! Zero out this peak and neighbors to find next distinct peak
                  do jtop = max(1,ifreq-2), min(NFREQ,ifreq+2)
                     sync_copy(jtop) = -99.0
                  enddo
               enddo

! Run Phase 2 Viterbi on each top-K Phase 1 candidate
               vit_overall_best = -1.0e30
               do itop = 1, ntop_found
                  ifreq = itop_idx(itop)
                  ! Parabolic interpolation for sub-step frequency accuracy
                  if(ifreq.gt.1 .and. ifreq.lt.NFREQ) then
                     xq1 = sync_freq(ifreq-1)
                     xq2 = sync_freq(ifreq)
                     xq3 = sync_freq(ifreq+1)
                     dxq = xq1 - 2.0*xq2 + xq3
                     if(dxq.ne.0.0) then
                        df_fine = 0.5*(xq1 - xq3)/dxq
                     else
                        df_fine = 0.0
                     endif
                  else
                     df_fine = 0.0
                  endif
                  df_top(itop) = (real(ifreq - 25) + df_fine) * 0.5
                  ibest_top(itop) = ibest_freq(ifreq)

! Phase 2: Viterbi frequency search using all 99 symbols
! Matches turbo_decode's proven VitSweep pattern:
!   1. Apply Phase 1's frequency correction to cd2 → cb
!   2. Estimate G1 carrier phase ONCE on corrected signal
!   3. Apply phase correction to cb
!   4. Compute MF correlations on phase+freq corrected cb
!   5. Viterbi sweep with frequency-ONLY rotation (no per-trial phase)

! Set up sync symbol map (-1 = data, >=0 = known Costas tone)
               sync_sym_map = -1
               sync_sym_map(1:4)   = icos_rv(0:3, 0)
               sync_sym_map(48:51) = icos_rv(0:3, 0)
               sync_sym_map(96:99) = icos_rv(0:3, 0)

! Apply Phase 1's frequency correction to cd2 → cb
               df_coarse_best = df_top(itop)
               ibest = ibest_top(itop)
               do i=0,NDMAX-1
                  t_samp_est = real(i) * dt_samp
                  cb(i) = cd2(i) * cmplx( &
                       cos(-twopi*df_coarse_best*t_samp_est), &
                       sin(-twopi*df_coarse_best*t_samp_est))
               enddo

! Estimate G1 carrier phase ONCE on the corrected signal
               phase_sum_g1 = cmplx(0.0, 0.0)
               s_g1 = 1
               do j_sync = 1, 4
                  idx_vit = nint(real(j_sync - 1) * nsps_dn) + ibest
                  corr_vit = cmplx(0.0, 0.0)
                  do k_vit = 1, nss_ds
                     n_samp = idx_vit + k_vit - 1
                     if(n_samp.ge.0 .and. n_samp.le.NDMAX-1) then
                        corr_vit = corr_vit + cb(n_samp) * &
                             conjg(mf_bank(k_vit, s_g1, &
                             sync_sym_map(j_sync)))
                     endif
                  enddo
                  phase_sum_g1 = phase_sum_g1 + corr_vit
                  s_g1 = next_state(s_g1, sync_sym_map(j_sync))
               enddo
               phi_g1 = atan2(aimag(phase_sum_g1), real(phase_sum_g1))

! Apply phase correction to cb
               do i=0,NDMAX-1
                  cb(i) = cb(i) * cmplx(cos(-phi_g1), sin(-phi_g1))
               enddo

! Compute complex MF correlations on phase+freq corrected signal
               do n_vit = 1, NN
                  idx_vit = nint(real(n_vit - 1) * nsps_dn) + ibest
                  do s_vit = 1, NSTATES
                     do u_vit = 0, 3
                        corr_vit = cmplx(0.0, 0.0)
                        do k_vit = 1, nss_ds
                           n_samp = idx_vit + k_vit - 1
                           if(n_samp.ge.0 .and. n_samp.le.NDMAX-1) then
                              corr_vit = corr_vit + cb(n_samp) * &
                                   conjg(mf_bank(k_vit, s_vit, u_vit))
                           endif
                        enddo
                        all_corr(s_vit, u_vit, n_vit) = corr_vit
                     enddo
                  enddo
               enddo

! Coarse Viterbi sweep: ±3 Hz at 0.1 Hz (61 points)
! Frequency-only rotation — no per-trial phase estimation.
               vit_best_metric = -1.0e30
               idf_best = 31
               do idf_s = 1, 61
                  df_try = real(idf_s - 31) * 0.1

                  alpha_vit = -1.0e30
                  alpha_vit(1) = 0.0        ! Known start state
                  do n_vit = 1, NN
                     theta_rot = twopi*df_try*real(n_vit-1)/28.0
                     vit_cos = cos(theta_rot)
                     vit_sin = sin(theta_rot)
                     alpha_vit_new = -1.0e30
                     if(sync_sym_map(n_vit) .ge. 0) then
                        u_vit = sync_sym_map(n_vit)
                        do s_vit = 1, NSTATES
                           if(alpha_vit(s_vit) .le. -1.0e20) cycle
                           s_next_v = next_state(s_vit, u_vit)
                           bm_vit = real(all_corr(s_vit,u_vit,n_vit)) &
                                *vit_cos + aimag(all_corr(s_vit,u_vit, &
                                n_vit))*vit_sin
                           if(alpha_vit(s_vit)+bm_vit .gt. &
                                alpha_vit_new(s_next_v)) then
                              alpha_vit_new(s_next_v) = &
                                   alpha_vit(s_vit)+bm_vit
                           endif
                        enddo
                     else
                        do s_vit = 1, NSTATES
                           if(alpha_vit(s_vit) .le. -1.0e20) cycle
                           do u_vit = 0, 3
                              s_next_v = next_state(s_vit, u_vit)
                              bm_vit = real(all_corr(s_vit,u_vit,n_vit)) &
                                   *vit_cos + aimag(all_corr(s_vit,u_vit, &
                                   n_vit))*vit_sin
                              if(alpha_vit(s_vit)+bm_vit .gt. &
                                   alpha_vit_new(s_next_v)) then
                                 alpha_vit_new(s_next_v) = &
                                      alpha_vit(s_vit)+bm_vit
                              endif
                           enddo
                        enddo
                     endif
                     alpha_vit = alpha_vit_new
                  enddo

                  bm_vit = maxval(alpha_vit)
                  if(bm_vit .gt. vit_best_metric) then
                     vit_best_metric = bm_vit
                     idf_best = idf_s
                  endif
               enddo
               df_fine = real(idf_best - 31) * 0.1

! Fine Viterbi sweep: ±0.2 Hz at 0.01 Hz (41 points)
               df_center = df_fine
               vit_best_metric = -1.0e30
               idf_best = 21
               do idf_s = 1, 41
                  df_try = df_center + real(idf_s - 21) * 0.01

                  alpha_vit = -1.0e30
                  alpha_vit(1) = 0.0
                  do n_vit = 1, NN
                     theta_rot = twopi*df_try*real(n_vit-1)/28.0
                     vit_cos = cos(theta_rot)
                     vit_sin = sin(theta_rot)
                     alpha_vit_new = -1.0e30
                     if(sync_sym_map(n_vit) .ge. 0) then
                        u_vit = sync_sym_map(n_vit)
                        do s_vit = 1, NSTATES
                           if(alpha_vit(s_vit) .le. -1.0e20) cycle
                           s_next_v = next_state(s_vit, u_vit)
                           bm_vit = real(all_corr(s_vit,u_vit,n_vit)) &
                                *vit_cos + aimag(all_corr(s_vit,u_vit, &
                                n_vit))*vit_sin
                           if(alpha_vit(s_vit)+bm_vit .gt. &
                                alpha_vit_new(s_next_v)) then
                              alpha_vit_new(s_next_v) = &
                                   alpha_vit(s_vit)+bm_vit
                           endif
                        enddo
                     else
                        do s_vit = 1, NSTATES
                           if(alpha_vit(s_vit) .le. -1.0e20) cycle
                           do u_vit = 0, 3
                              s_next_v = next_state(s_vit, u_vit)
                              bm_vit = real(all_corr(s_vit,u_vit,n_vit)) &
                                   *vit_cos + aimag(all_corr(s_vit,u_vit, &
                                   n_vit))*vit_sin
                              if(alpha_vit(s_vit)+bm_vit .gt. &
                                   alpha_vit_new(s_next_v)) then
                                 alpha_vit_new(s_next_v) = &
                                      alpha_vit(s_vit)+bm_vit
                              endif
                           enddo
                        enddo
                     endif
                     alpha_vit = alpha_vit_new
                  enddo

                  bm_vit = maxval(alpha_vit)
                  vit_metric(idf_s) = bm_vit
                  if(bm_vit .gt. vit_best_metric) then
                     vit_best_metric = bm_vit
                     idf_best = idf_s
                  endif
               enddo

! Parabolic interpolation on fine sweep for sub-step accuracy
               if(idf_best.gt.1 .and. idf_best.lt.41) then
                  xq1 = vit_metric(idf_best-1)
                  xq2 = vit_metric(idf_best)
                  xq3 = vit_metric(idf_best+1)
                  dxq = xq1 - 2.0*xq2 + xq3
                  if(abs(dxq).gt.1.0e-10) then
                     df_fine = 0.5*(xq1 - xq3)/dxq
                  else
                     df_fine = 0.0
                  endif
               else
                  df_fine = 0.0
               endif
! Store Phase 1 + Phase 2 Viterbi result for this candidate
               df_final_top(itop) = df_coarse_best + df_center + &
                    (real(idf_best - 21) + df_fine) * 0.01
               vit_metric_top(itop) = vit_best_metric
               if(vit_best_metric .gt. vit_overall_best) then
                  vit_overall_best = vit_best_metric
               endif
               enddo  ! itop (top-K Phase 1 candidates)

! Select Phase 1 candidate with best Phase 2 Viterbi metric
               do itop = 1, ntop_found
                  if(vit_metric_top(itop) .ge. vit_overall_best) then
                     df_coarse_best = df_final_top(itop)
                     ibest = ibest_top(itop)
                     exit
                  endif
               enddo

               call timer('sync1d  ',1)

! ================================================================
! Step 2d.1: Full turbo decode at coarse sync frequency.
! Single call: internal Viterbi sweep provides ±4 Hz frequency
! refinement, so coarse sync ±0.25 Hz accuracy is sufficient.
! ================================================================
               call timer('turbodec',0)

               df_total = df_coarse_best
               f1 = f0 + df_total
               do i=0,NDMAX-1
                  t_samp_est = real(i) * dt_samp
                  cb(i) = cd2(i) * cmplx( &
                       cos(-twopi*df_total*t_samp_est), &
                       sin(-twopi*df_total*t_samp_est))
               enddo

               apmask=0
               iaptype=0
               message91=0
               ntype=-1
               nharderror=-1
               call turbo_decode_ft1(cb,NDMAX,0.0,real(ibest), &
                    snr_est,llr_out,message91,ntype,nharderror, &
                    dmin,0,ncheck_out)

               call timer('turbodec',1)

! ================================================================
! Step 2e: If turbo succeeded, process the decode
! ================================================================
               if(nharderror.ge.0) then
                  message77=message91(1:77)
                  message77=mod(message77+rvec,2)    !Remove scrambling (noop for FT1)
                  write(c77,'(77i1)') message77(1:77)
                  call unpack77(c77,1,message,unpk77_success)
                  if(.not.unpk77_success) cycle

! Signal subtraction
                  if(dosubtract) then
                     call get_ft1_tones_from_77bits(message77,i4tone)
                     xdt=real(ibest)/fs
                     call timer('subtract',0)
                     call subtractft1(dd,i4tone,f1,xdt)
                     call timer('subtract',1)
                  endif

! Duplicate check
                  idupe=0
                  do i=1,ndecodes
                     if(decodes(i).eq.message) idupe=1
                  enddo
                  if(idupe.eq.1) cycle

                  ndecodes=ndecodes+1
                  decodes(ndecodes)=message

                  nsnr=nint(snr_est)
                  xdt=ibest/fs - 0.5
                  qual=1.0-(nharderror+dmin)/60.0
                  call this%callback(smax,nsnr,xdt,f1,message,      &
                       iaptype,qual)
                  cycle
               endif

! ================================================================
! Step 2f: AP decoding passes (if standard decode failed)
! ================================================================
               napwid=80
               npasses=nappasses(nQSOProgress)
               if(lapcqonly) npasses=min(npasses,1)
               if(ndepth.eq.1) npasses=0
               if(ncontest.ge.6) npasses=0

               do ipass=1,npasses
                  iaptype=naptypes(nQSOProgress,ipass)
                  if(lapcqonly) iaptype=1

! Bail-out conditions for AP
                  if(ncontest.le.5 .and. iaptype.ge.3 .and.          &
                     (abs(f1-nfqso).gt.napwid)) cycle
                  if(iaptype.ge.2 .and. apbits(1).gt.1) cycle
                  if(iaptype.ge.3 .and. apbits(30).gt.1) cycle

! Build AP mask and inject known LLRs
                  apmask=0
                  apmag=30.0    !Strong AP LLR magnitude

                  if(iaptype.eq.1) then
! CQ: fix first 29 bits
                     apmask(1:29)=1
                     llr_out(1:29)=apmag*mcq(1:29)
                     if(ncontest.eq.1) llr_out(1:29)=apmag*mcqtest(1:29)
                     if(ncontest.eq.2) llr_out(1:29)=apmag*mcqtest(1:29)
                     if(ncontest.eq.3) llr_out(1:29)=apmag*mcqfd(1:29)
                     if(ncontest.eq.4) llr_out(1:29)=apmag*mcqru(1:29)
                     if(ncontest.eq.5) llr_out(1:29)=apmag*mcqww(1:29)
                  endif

                  if(iaptype.eq.2) then
! MyCall: fix first 29 bits
                     apmask(1:29)=1
                     if(ncontest.eq.0.or.ncontest.eq.1.or.ncontest.eq.5) then
                        llr_out(1:29)=apmag*apbits(1:29)
                     else if(ncontest.eq.2) then
                        apmask(1:28)=1
                        llr_out(1:28)=apmag*apbits(1:28)
                     else if(ncontest.eq.3) then
                        apmask(1:28)=1
                        llr_out(1:28)=apmag*apbits(1:28)
                     else if(ncontest.eq.4) then
                        apmask(2:29)=1
                        llr_out(2:29)=apmag*apmy_ru(1:28)
                     endif
                  endif

                  if(iaptype.eq.3) then
! MyCall+DxCall: fix first 58 bits
                     apmask(1:58)=1
                     if(ncontest.eq.0.or.ncontest.eq.1.or.            &
                        ncontest.eq.2.or.ncontest.eq.5) then
                        llr_out(1:58)=apmag*apbits(1:58)
                     else if(ncontest.eq.3) then
                        apmask(1:56)=1
                        llr_out(1:28)=apmag*apbits(1:28)
                        llr_out(29:56)=apmag*aphis_fd(1:28)
                     else if(ncontest.eq.4) then
                        apmask(2:57)=1
                        llr_out(2:29)=apmag*apmy_ru(1:28)
                        llr_out(30:57)=apmag*apbits(30:57)
                     endif
                  endif

                  if(iaptype.eq.4 .or. iaptype.eq.5 .or. iaptype.eq.6) then
! Full message: fix 77 bits
                     if(ncontest.le.5) then
                        apmask(1:77)=1
                        if(iaptype.eq.6) llr_out(1:77)=apmag*apbits(1:77)
                     endif
                  endif

! Re-run turbo decode with AP LLRs (no VitSweep — freq already estimated)
                  call timer('turbodec',0)
                  call turbo_decode_ft1(cb,NDMAX,0.0,real(ibest), &
                       snr_est,llr_out,message91,ntype,nharderror, &
                       dmin,-1,ncheck_out)
                  call timer('turbodec',1)

                  if(nharderror.ge.0) then
                     message77=message91(1:77)
                     message77=mod(message77+rvec,2)
                     write(c77,'(77i1)') message77(1:77)
                     call unpack77(c77,1,message,unpk77_success)
                     if(.not.unpk77_success) cycle

                     if(dosubtract) then
                        call get_ft1_tones_from_77bits(message77,i4tone)
                        xdt=real(ibest)/fs
                        call timer('subtract',0)
                        call subtractft1(dd,i4tone,f1,xdt)
                        call timer('subtract',1)
                     endif

                     idupe=0
                     do i=1,ndecodes
                        if(decodes(i).eq.message) idupe=1
                     enddo
                     if(idupe.eq.1) exit

                     ndecodes=ndecodes+1
                     decodes(ndecodes)=message
                     nsnr=nint(snr_est)
                     xdt=ibest/fs - 0.5
                     qual=1.0-(nharderror+dmin)/60.0
                     call this%callback(smax,nsnr,xdt,f1,message,   &
                          iaptype,qual)
                     exit
                  endif
               enddo                          !AP passes

               if(nharderror.ge.0) cycle

! ================================================================
! Step 2g: IR-HARQ soft combining (if standard+AP decode failed)
!
! RV0 failed: store turbo-extracted LLRs for future combining
! RV1 arrived: combine with stored RV0 LLRs, decode LDPC(261,91)
! RV2 arrived: combine with stored RV0+RV1, decode LDPC(348,91)
! ================================================================
               if(nharderror.lt.0) then
                  itime_ms=this%frame_time_ms     !Wall-clock ms for cross-frame HARQ keying

! IR-HARQ with reliable RV-aware combining. The ft1_sync spectrogram RV tag is
! NOT trusted. RV0 is decoded standalone (above); a failed frame is either a
! fresh RV0 (no prior slot at this freq) or a retransmission. For a retransmission
! we re-reference the frame to the stored RV0 slot's reliable freq + timing
! (RV0/RV1/RV2 of a QSO arrive at the same slot alignment), classify the RV
! coherently (ft1_rv_detect), and joint-turbo-combine on a valid RV progression.
                  call harq_lookup(f1,itime_ms,harq_islot,harq_sfreq, &
                       harq_sibest,harq_srvc)

                  if(harq_islot.le.0) then
! No prior slot here: treat as a fresh RV0. Align the RV0-synced baseband (cb)
! to symbol 0 by its own ibest and store it, with (f1,ibest) as the anchor.
                     cd_harq=(0.0,0.0)
                     do i=0,NDMAX-1
                        if(i+ibest.le.NDMAX-1) cd_harq(i)=cb(i+ibest)
                     enddo
                     call harq_store_rv0(f1,cd_harq,NDMAX,snr_est,ibest,itime_ms)

                  else
! Retransmission candidate: re-reference at the slot's reliable freq + timing,
! then classify the RV coherently.
                     call ft1_downsample(dd,.true.,harq_sfreq,cd2)
                     sum2=sum(real(cd2*conjg(cd2)))/real(NDMAX)
                     if(sum2.gt.0.0) cd2=cd2/sqrt(sum2)
                     cd_harq=(0.0,0.0)
                     do i=0,NDMAX-1
                        if(i+harq_sibest.ge.0 .and. i+harq_sibest.le.NDMAX-1) &
                             cd_harq(i)=cd2(i+harq_sibest)
                     enddo
                     call ft1_rv_detect(cd_harq,NDMAX,0,irv_det)

                     harq_ok=.false.
                     if(irv_det.eq.0) then
! Looks like a fresh RV0 (partner restarted) -> refresh the slot.
                        call harq_store_rv0(harq_sfreq,cd_harq,NDMAX,snr_est, &
                             harq_sibest,itime_ms)
                     else if(irv_det.eq.1 .and. harq_srvc.eq.0) then
                        call harq_combine_rv1(harq_sfreq,cd_harq,NDMAX,snr_est, &
                             itime_ms,harq_msg77,harq_nerr,harq_ok)
                        if(harq_ok) this%cur_rv=1
                     else if(irv_det.eq.2 .and. harq_srvc.ge.1) then
                        call harq_combine_rv2(harq_sfreq,cd_harq,NDMAX,snr_est, &
                             itime_ms,harq_msg77,harq_nerr,harq_ok)
                        if(harq_ok) this%cur_rv=2
                     endif
! (other irv_det/rv_count combinations are progression mismatches -> ignored)

                     if(harq_ok) then
                        message77=harq_msg77
                        nharderror=harq_nerr
                        iaptype=0
                        write(c77,'(77i1)') message77(1:77)
                        call unpack77(c77,1,message,unpk77_success)
                        if(unpk77_success) then
                           idupe=0
                           do i=1,ndecodes
                              if(decodes(i).eq.message) idupe=1
                           enddo
                           if(idupe.eq.0) then
                              ndecodes=ndecodes+1
                              decodes(ndecodes)=message
                              nsnr=nint(snr_est)
                              xdt=real(harq_sibest)/fs - 0.5
                              qual=1.0-(nharderror+dmin)/60.0
                              call this%callback(smax,nsnr,xdt,harq_sfreq,message, &
                                   iaptype,qual)
                           endif
                           cycle
                        endif
                     endif
                  endif
               endif

         enddo                                !Candidate list

! Expire stale IR-HARQ buffers (older than EXPIRY_MS vs this frame's wall clock)
         call harq_expire(this%frame_time_ms)

      enddo                                   !Subtraction passes

      return
   end subroutine decode

end module ft1_decode


subroutine sync1d_ft1(cd0,i0,ctwk,itwk,icos,sync)

! Compute sync power for a complex, downsampled FT1 signal.
! Correlates against 3 groups of 4 Costas symbols using CPM
! matched filter references from matched_filter_bank_mod.
!
! Group 1: starting state is known (state 1 = frame start).
! Groups 2-3: search all 16 correlative states at theta=0 and
!   take the best magnitude. This is necessary because with h=1/2
!   CPM the matched filter waveforms have significant tone overlap,
!   making state-dependent correlation essential for discrimination.
!
! FT1 sync positions (0-indexed symbols):
!   G1: symbols 0-3, G2: symbols 47-50, G3: symbols 95-98

  use cpm_trellis_mod
  use matched_filter_bank_mod

  include 'ft1/ft1_params.f90'
  parameter(NP=NMAX/NDOWN,NSS_DS=NSS)          !NP=888, NSS_DS=8
  complex cd0(0:NP-1)
  complex csync(4*NSS_DS)                       !Reference sync waveform
  complex csync2(4*NSS_DS)                      !Tweaked sync waveform
  complex ctwk(4*NSS_DS)
  complex z1,z_try
  integer icos(0:3)                             !Costas array
  integer s_state, s_try
  real scale_amp, nsps_down
  real sync2, sync3, best_mag, mag_try

  p(z1)=(real(z1)**2 + aimag(z1)**2)**0.5       !Statement function for amplitude

  scale_amp=sqrt(real(NSS_DS))
  nsync_len=4*NSS_DS                            !32 samples per sync group

! Sync group positions using fractional samples/symbol
  nsps_down=real(NSPS_NUM)/(real(NSPS_DEN)*real(NDOWN))  !7.9365
  i1=i0                                         !G1: symbol 0
  i2=i0+nint(47.0*nsps_down)                    !G2: symbol 47
  i3=i0+nint(95.0*nsps_down)                    !G3: symbol 95

! ---- Group 1: known starting state (state 1) ----
  s_state=1
  k=1
  do i=0,3
     do j=1,NSS_DS
        csync(k)=scale_amp*mf_bank(j,s_state,icos(i))
        k=k+1
     enddo
     s_state=next_state(s_state,icos(i))
  enddo

  if(itwk.eq.1) then
     do k=1,4*NSS_DS
        csync2(k)=ctwk(k)*csync(k)
     enddo
  else
     csync2=csync
  endif

  z1=0.
  if(i1.ge.0 .and. i1+nsync_len-1.le.NP-1) then
     z1=sum(cd0(i1:i1+nsync_len-1)*conjg(csync2(1:nsync_len)))
  elseif(i1.lt.0 .and. i1+nsync_len-1.ge.0) then
     noff=-i1
     npts=nsync_len-noff
     if(npts.gt.4) then
        z1=sum(cd0(0:npts-1)*conjg(csync2(noff+1:nsync_len)))
     endif
  endif

! ---- Groups 2-3: search 16 correlative states at theta=0 ----
  sync2=0.
  if(i2.ge.0 .and. i2+nsync_len-1.le.NP-1) then
     best_mag=0.
     do s_try=1,16
        s_state=s_try
        k=1
        do i=0,3
           do j=1,NSS_DS
              csync(k)=scale_amp*mf_bank(j,s_state,icos(i))
              k=k+1
           enddo
           s_state=next_state(s_state,icos(i))
        enddo
        if(itwk.eq.1) then
           do k=1,4*NSS_DS
              csync2(k)=ctwk(k)*csync(k)
           enddo
        else
           csync2=csync
        endif
        z_try=sum(cd0(i2:i2+nsync_len-1)*conjg(csync2(1:nsync_len)))
        mag_try=p(z_try)
        if(mag_try.gt.best_mag) best_mag=mag_try
     enddo
     sync2=best_mag/nsync_len
  endif

  sync3=0.
  if(i3.ge.0 .and. i3+nsync_len-1.le.NP-1) then
     best_mag=0.
     do s_try=1,16
        s_state=s_try
        k=1
        do i=0,3
           do j=1,NSS_DS
              csync(k)=scale_amp*mf_bank(j,s_state,icos(i))
              k=k+1
           enddo
           s_state=next_state(s_state,icos(i))
        enddo
        if(itwk.eq.1) then
           do k=1,4*NSS_DS
              csync2(k)=ctwk(k)*csync(k)
           enddo
        else
           csync2=csync
        endif
        z_try=sum(cd0(i3:i3+nsync_len-1)*conjg(csync2(1:nsync_len)))
        mag_try=p(z_try)
        if(mag_try.gt.best_mag) best_mag=mag_try
     enddo
     sync3=best_mag/nsync_len
  elseif(i3.ge.0 .and. i3.le.NP-1 .and. i3+nsync_len-1.gt.NP-1) then
     npts=NP-i3
     if(npts.gt.4) then
        best_mag=0.
        do s_try=1,16
           s_state=s_try
           k=1
           do i=0,3
              do j=1,NSS_DS
                 csync(k)=scale_amp*mf_bank(j,s_state,icos(i))
                 k=k+1
              enddo
              s_state=next_state(s_state,icos(i))
           enddo
           if(itwk.eq.1) then
              do k=1,4*NSS_DS
                 csync2(k)=ctwk(k)*csync(k)
              enddo
           else
              csync2=csync
           endif
           z_try=sum(cd0(i3:NP-1)*conjg(csync2(1:npts)))
           mag_try=p(z_try)
           if(mag_try.gt.best_mag) best_mag=mag_try
        enddo
        sync3=best_mag/nsync_len
     endif
  endif

  fac=1.0/(nsync_len)
  sync = p(z1*fac) + sync2 + sync3

  return
end subroutine sync1d_ft1


subroutine subtractft1(dd,itone,f0,dt)

! Subtract an FT1 signal from the audio data for multi-decode.
!
! Algorithm:
!   1. Generate real reference CPM waveform at (f0, dt)
!   2. Estimate signal amplitude via least-squares fit
!   3. Subtract amplitude-scaled reference from dd
!
! gen_ft1wave produces a real-valued waveform. The amplitude is
! estimated as: a = sum(dd*ref) / sum(ref*ref) over the frame.
! This is the optimal LS estimate assuming the reference waveform
! shape is correct (which it is, since we decoded the message).
!
! Reference: subtractft4.f90 (adapted for real-valued gen_ft1wave)

  include 'ft1/ft1_params.f90'

  parameter (NFRAME=NN*NSPS+2*NSPS)         !Frame + guard samples

  real*4 dd(NMAX)
  real*4 waveref(NFRAME)                     !Real reference waveform
  real*4 amp_est                             !Amplitude estimate
  integer itone(NN)

  nstart=nint(dt*12000.0)+1-NSPS

! Generate real-valued CPM waveform at (f0, dt)
! gen_ft1wave interface: (itone,nsym,nsps_num,nsps_den,fsample,f0,wave,nwave)
  call gen_ft1wave(itone,NN,NSPS_NUM,NSPS_DEN,12000.0,f0,        &
       waveref,NFRAME)

! Estimate amplitude: a = sum(dd*ref) / sum(ref*ref)
  ref_pow=0.0
  cross_pow=0.0
  do i=1,NFRAME
     j=nstart+i-1
     if(j.ge.1 .and. j.le.NMAX) then
        ref_pow = ref_pow + waveref(i)**2
        cross_pow = cross_pow + dd(j)*waveref(i)
     endif
  enddo

  if(ref_pow.gt.0.0) then
     amp_est = cross_pow / ref_pow
  else
     return
  endif

! Subtract the amplitude-scaled reference waveform
  do i=1,NFRAME
     j=nstart+i-1
     if(j.ge.1 .and. j.le.NMAX) then
        dd(j) = dd(j) - amp_est*waveref(i)
     endif
  enddo

  return
end subroutine subtractft1


subroutine ft1_freq_est_acorr(cd0, npts, i0, df_est)
!
! Estimate frequency offset via self-referenced sync correlation.
!
! Generates a reference signal through the exact same signal chain
! (gen_ft1wave + ft1_downsample) to eliminate systematic phase model
! errors from Euler integration, FFT-based downsampling, and the
! startup ramp.
!
! Uses all 4 G1 symbols (known state) plus symbols 2-3 of G2/G3
! (where the correlative state has converged, leaving only an unknown
! θ offset that is constant per group).
!
! Searches over (θ_G2, θ_G3, n1_wrap, n2_wrap) with analytical
! 2-parameter linear fit (intercept + slope) to 8 phase measurements.
!
  include 'ft1/ft1_params.f90'
  parameter(NP=NMAX/NDOWN, NSS_DS=NSS)
  parameter(NMEAS=8)

  integer npts, i0
  complex cd0(0:npts-1)
  real df_est

  ! Costas array for RV0
  integer icos(0:3)
  data icos/0,2,3,1/

  ! Sync group starting symbol indices (0-indexed)
  integer isync_pos(3)
  data isync_pos/0,47,95/

  ! Cached self-reference (generated once through same signal chain)
  complex, save :: ref_cd(0:NP-1)
  logical, save :: ref_init = .false.

  ! Measurement arrays
  complex :: corr(NMEAS)
  real    :: phi(NMEAS), t_meas(NMEAS)
  real    :: phi_adj(NMEAS)

  ! Working variables
  real    :: nsps_down, pi, twopi, dt_ds
  real    :: sum_t, sum_t2, denom_fit
  real    :: sum_phi, sum_tphi, resid, best_resid
  real    :: a_fit, b_fit, phase_offset, sum2
  integer :: ic, ig, m, ks, n_dd, idx_dd
  integer :: th2, th3, n1, n2
  integer, parameter :: NWRAP_MAX = 3

  ! For reference generation (only used on first call)
  integer :: itone_ref(NN)
  real    :: wave_ref(NMAX), dd_ref(NMAX)
  logical :: newdata_ref

  pi = 4.0*atan(1.0)
  twopi = 2.0*pi
  nsps_down = 3000.0 / (7.0 * real(NDOWN))
  dt_ds = real(NDOWN) / 12000.0

! ================================================================
! Step 1: Generate self-reference (once, cached)
! ================================================================
  if(.not. ref_init) then
     ! Build tone array: sync symbols at correct positions, data = 0
     itone_ref = 0
     do ig = 1, 3
        do m = 0, 3
           itone_ref(isync_pos(ig) + m + 1) = icos(m)
        enddo
     enddo

     ! Generate waveform through exact same signal chain
     wave_ref = 0.0
     call gen_ft1wave(itone_ref, NN, NSPS_NUM, NSPS_DEN, &
          12000.0, 1500.0, wave_ref, NMAX)
     dd_ref = 0.0
     dd_ref(1:NMAX) = wave_ref(1:NMAX)
     newdata_ref = .true.
     call ft1_downsample(dd_ref, newdata_ref, 1500.0, ref_cd)

     ! Normalize to unit power (same as signal normalization)
     sum2 = sum(real(ref_cd * conjg(ref_cd))) / real(NP)
     if(sum2 .gt. 0.0) ref_cd = ref_cd / sqrt(sum2)

     ref_init = .true.
  endif

! ================================================================
! Step 2: Correlate actual signal with reference at sync positions
! ================================================================
! G1: all 4 symbols (reference state matches exactly)
! G2/G3: symbols 2-3 only (correlative state converged, only θ unknown)
  ic = 0
  do m = 0, 3
     ic = ic + 1
     idx_dd = nint(real(isync_pos(1) + m) * nsps_down) + i0
     corr(ic) = cmplx(0.0, 0.0)
     do ks = 0, NSS_DS-1
        n_dd = idx_dd + ks
        if(n_dd .ge. 0 .and. n_dd .le. NP-1) then
           corr(ic) = corr(ic) + cd0(n_dd) * conjg(ref_cd(n_dd))
        endif
     enddo
     t_meas(ic) = real(idx_dd + NSS_DS/2) * dt_ds
  enddo
  do ig = 2, 3
     do m = 2, 3
        ic = ic + 1
        idx_dd = nint(real(isync_pos(ig) + m) * nsps_down) + i0
        corr(ic) = cmplx(0.0, 0.0)
        do ks = 0, NSS_DS-1
           n_dd = idx_dd + ks
           if(n_dd .ge. 0 .and. n_dd .le. NP-1) then
              corr(ic) = corr(ic) + cd0(n_dd) * conjg(ref_cd(n_dd))
           endif
        enddo
        t_meas(ic) = real(idx_dd + NSS_DS/2) * dt_ds
     enddo
  enddo

! ================================================================
! Step 3: Extract phases and unwrap within groups
! ================================================================
  do ic = 1, NMEAS
     phi(ic) = atan2(aimag(corr(ic)), real(corr(ic)))
  enddo

  ! Unwrap G1 (points 1-4)
  do ic = 2, 4
     do while(phi(ic) - phi(ic-1) .gt. pi)
        phi(ic) = phi(ic) - twopi
     enddo
     do while(phi(ic) - phi(ic-1) .lt. -pi)
        phi(ic) = phi(ic) + twopi
     enddo
  enddo
  ! Unwrap G2 pair (points 5-6)
  do while(phi(6) - phi(5) .gt. pi)
     phi(6) = phi(6) - twopi
  enddo
  do while(phi(6) - phi(5) .lt. -pi)
     phi(6) = phi(6) + twopi
  enddo
  ! Unwrap G3 pair (points 7-8)
  do while(phi(8) - phi(7) .gt. pi)
     phi(8) = phi(8) - twopi
  enddo
  do while(phi(8) - phi(7) .lt. -pi)
     phi(8) = phi(8) + twopi
  enddo

! ================================================================
! Step 4: Search (θ_G2, θ_G3, n1_wrap, n2_wrap), fit slope
! ================================================================
! Model: phi(i) = a + b*t(i)  (after applying group-specific θ + wrap)
! G1 (points 1-4): no correction needed (reference matches)
! G2 (points 5-6): add -θ₂*π/2 + n1*2π
! G3 (points 7-8): add -θ₃*π/2 + (n1+n2)*2π

  sum_t = 0.0
  sum_t2 = 0.0
  do ic = 1, NMEAS
     sum_t = sum_t + t_meas(ic)
     sum_t2 = sum_t2 + t_meas(ic)**2
  enddo
  denom_fit = sum_t2 - sum_t*sum_t/real(NMEAS)

  best_resid = 1.0e30
  df_est = 0.0

  do th2 = 0, 3
     do th3 = 0, 3
        do n1 = -NWRAP_MAX, NWRAP_MAX
           do n2 = -NWRAP_MAX, NWRAP_MAX
              ! Apply corrections
              phi_adj(1:4) = phi(1:4)
              phase_offset = -real(th2)*pi/2.0 + real(n1)*twopi
              phi_adj(5) = phi(5) + phase_offset
              phi_adj(6) = phi(6) + phase_offset
              phase_offset = -real(th3)*pi/2.0 + real(n1+n2)*twopi
              phi_adj(7) = phi(7) + phase_offset
              phi_adj(8) = phi(8) + phase_offset

              ! Analytical 2-parameter linear fit: phi = a + b*t
              sum_phi = 0.0
              sum_tphi = 0.0
              do ic = 1, NMEAS
                 sum_phi = sum_phi + phi_adj(ic)
                 sum_tphi = sum_tphi + t_meas(ic)*phi_adj(ic)
              enddo

              b_fit = (sum_tphi - sum_t*sum_phi/real(NMEAS)) / denom_fit
              a_fit = (sum_phi - b_fit*sum_t) / real(NMEAS)

              ! Compute residual sum of squares
              resid = 0.0
              do ic = 1, NMEAS
                 resid = resid + (phi_adj(ic) - a_fit - b_fit*t_meas(ic))**2
              enddo

              if(resid .lt. best_resid) then
                 best_resid = resid
                 df_est = b_fit / twopi
              endif
           enddo  ! n2
        enddo  ! n1
     enddo  ! th3
  enddo  ! th2

  return
end subroutine ft1_freq_est_acorr
