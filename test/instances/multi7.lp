\ 7-variable multi-constraint ILP
\ IP opt = 18 (x1=1,x2=1,x3=1,x6=1), LP opt = 19.2 => branching required
\ Tests wider B&B trees with 7 variables and 3 constraints
Maximize
  obj: 3 x1 + 5 x2 + 4 x3 + 2 x4 + 7 x5 + 6 x6 + 3 x7
Subject To
  c1: 2 x1 + 3 x2 + x3 + 2 x4 + 4 x5 + 2 x6 + x7 <= 8
  c2: x1 + x2 + 2 x3 + 3 x4 + x5 + 3 x6 + 2 x7 <= 7
  c3: 3 x1 + x2 + x3 + x4 + 2 x5 + x6 + 3 x7 <= 6
Binary
  x1 x2 x3 x4 x5 x6 x7
End
