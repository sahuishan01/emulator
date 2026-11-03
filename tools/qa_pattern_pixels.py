"""Independent screen-pixel formulas; no guest decoder or GPU readback as oracle."""
def expected_pixel(variant,px,py):
 center=4<=px<12 and 2<=py<6 and (variant!=5 or px+2*py<=14)
 if variant<6:
  sx,sy=((6*px-29)//8,(6*py-13)//4) if center else ((6*px-13)//16,(6*py-5)//8)
  return [(255,0,0,255),(255,255,0,255),(0,255,0,255),(0,255,255,255)][(min(3,max(0,sx))+min(3,max(0,sy))+(variant in(2,5)))%4]
 i=variant-6;w,h=[(4,4),(17,33),(7,5),(7,9),(5,7),(13,9)][i]
 sx,sy=(((6*px-29)*w)//32,((6*py-13)*h)//16) if center else (((6*px-13)*w)//64,((6*py-5)*h)//32)
 x=min(w-1,max(0,sx));y=min(h-1,max(0,sy))
 if i<2:c=[(255,0,0),(255,255,0),(0,255,0),(0,255,255)][(x+(3 if i==1 else 1)*y)%4]
 elif i==2:c=[(255,0,0),(0,255,0),(170,85,0),(85,170,0)][(x+y)%4]
 elif i==3:c=[(0,255,0),(255,0,0),(128,128,0),(0,0,0)][(x+2*y)%4]
 elif i==4:c=[(0,255,0),(255,0,0),(85,170,0),(170,85,0)][(x+y)%4]
 else:c=[(0,0,255),(255,255,255),(85,85,255),(170,170,255)][(2*x+y)%4]
 # Presentation is opaque; source alpha is separately checked in native readback.
 return (*c,255)
