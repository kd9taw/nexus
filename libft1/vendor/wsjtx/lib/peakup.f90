subroutine peakup(ym,y0,yp,dx)

  b=(yp-ym)/2.0
  c=(yp+ym-2.0*y0)/2.0
  dx=-b/(2.0*c)

  return
end subroutine peakup
