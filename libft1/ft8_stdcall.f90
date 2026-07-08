! Nexus: standalone copy of WSJT-X's `stdcall` callsign classifier.
!
! ft8apset (lib/ft8/ft8apset.f90) calls stdcall(), but in the WSJT-X tree the
! only definition lives inside lib/qra/q65/q65_set_list.f90 — compiling that
! would drag in the whole Q65 modem. stdcall is fully self-contained (no
! dependencies), so we vendor just this routine here (verbatim, GPLv3 — already
! the project license) to keep the FT8 build minimal.
!
! Determines whether `callsign` is a "standard" callsign (used to decide which
! a-priori (AP) symbol set ft8apset installs).

subroutine stdcall(callsign,std)

  character*12 callsign
  character*1 c
  logical is_digit,is_letter,std
!Statement functions:
  is_digit(c)=c.ge.'0' .and. c.le.'9'
  is_letter(c)=c.ge.'A' .and. c.le.'Z'

! Check for standard callsign
  iarea=-1
  n=len(trim(callsign))
  do i=n,2,-1
     if(is_digit(callsign(i:i))) exit
  enddo
  iarea=i                                   !Right-most digit (call area)
  npdig=0                                   !Digits before call area
  nplet=0                                   !Letters before call area
  do i=1,iarea-1
     if(is_digit(callsign(i:i))) npdig=npdig+1
     if(is_letter(callsign(i:i))) nplet=nplet+1
  enddo
  nslet=0                                   !Letters in suffix
  do i=iarea+1,n
     if(is_letter(callsign(i:i))) nslet=nslet+1
  enddo
  std=.true.
  if(iarea.lt.2 .or. iarea.gt.3 .or. nplet.eq.0 .or.       &
       npdig.ge.iarea-1 .or. nslet.gt.3) std=.false.

  return
end subroutine stdcall
