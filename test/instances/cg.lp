\ CG cut test: min x + y  s.t. 4x+y>=1, 4x-y<=2, x,y in {0,1}
\ IP opt = 1 (x=0, y=1), LP opt = 3/8 (fractional)
\ Should benefit from Gomory cuts
Minimize
  obj: x + y
Subject To
  c1: 4 x + y >= 1
  c2: 4 x - y <= 2
Binary
  x y
End
