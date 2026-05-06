\ Tiny 0-1 ILP: min x + y s.t. x + 2y >= 2, 2x + y >= 2
\ LP opt = 4/3 (fractional), IP opt = 2 (x=1,y=1)
\ Should prune at root after rounding: ceil(4/3) = 2 >= z*
Minimize
  obj: x + y
Subject To
  c1: x + 2 y >= 2
  c2: 2 x + y >= 2
Binary
  x y
End
