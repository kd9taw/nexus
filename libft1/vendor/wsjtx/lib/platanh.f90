subroutine platanh(x,y)
  isign=+1
  z=x
  if( x.lt.0 ) then
    isign=-1
    z=abs(x)
  endif
  if( z.le. 0.664 ) then
    y=x/0.83
    return
  elseif( z.le. 0.9217 ) then
    y=isign*(z-0.4064)/0.322
    return
  elseif( z.le. 0.9951 ) then
    y=isign*(z-0.8378)/0.0524
    return
  elseif( z.le. 0.9998 ) then
    y=isign*(z-0.9914)/0.0012
    return
  else
    y=isign*7.0
    return
  endif
end subroutine platanh
