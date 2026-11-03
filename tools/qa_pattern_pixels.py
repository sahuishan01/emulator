"""Independent screen-pixel formulas; no guest decoder or GPU readback as oracle."""
def expected_pixel(variant,px,py):
 if variant>=39:
  i=variant-39
  if i>=12:
   center=4<=px<12 and 2<=py<6
   if i==15 and not center:return (0,0,0,255)
   return (255,0,0,255) if center and (i==14 or i<14 and px>=8) else (0,255,0,255)
  rects=[(4,2,12,6),(0,0,16,8),(0,0,16,8),(2,1,14,7),(0,0,16,8),(0,0,16,8),(2,0,14,8),(.125,.125,15.875,7.875),(0,0,16,8),(2,0,14,8),(0,0,16,8),(4,0,12,8)]
  x,y=px+.5,py+.5;r=rects[i];inside=lambda q:q[0]<=x<q[2] and q[1]<=y<q[3]
  if not inside(r) or i in (5,11) and px<8:return (0,0,0,255)
  c=((r[0]+8)/2,(r[1]+4)/2,(r[2]+8)/2,(r[3]+4)/2)
  if inside(c) and (i!=10 or px+2*py<=14):r=c
  import math
  sx=max(0,min(3,math.floor(6*(x-r[0])/(r[2]-r[0])-1)))
  sy=max(0,min(3,math.floor(6*(y-r[1])/(r[3]-r[1])-1)))
  c=[(255,0,0,255),(255,255,0,255),(0,255,0,255),(0,255,255,255)][(sx+sy+(i==10))%4]
  return c # BGRA storage is interpreted by Vulkan during presentation.
 if variant>=23:
  i=variant-23;c=4<=px<12 and 2<=py<6
  region=px>=8 if i in (2,5) else 8<=px<14 and 1<=py<7 if i==3 else px==15 and py==7 if i==14 else True
  if i in (1,11) or not region:return (0,0,0,255)
  return (255,0,0,255) if c and (i in (6,10,13) or i==9 and px>=8) else (0,255,0,255)
 if variant>=12:
  i=variant-12;center=4<=px<12 and 2<=py<6
  return (0,0,0,255) if i==0 else (255,0,0,255) if center and (i in (3,6,7,8) or i>=9 and px>=8) else (0,255,0,255)
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
