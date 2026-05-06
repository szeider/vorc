\ 15-variable set cover (min)
\ 8 sets must cover 6 elements, with costs
\ Tests minimization with >= constraints
Minimize
  obj: 3 x1 + 5 x2 + 2 x3 + 7 x4 + 4 x5 + 6 x6 + 3 x7 + 8 x8 + 2 x9 + 5 x10
       + 4 x11 + 3 x12 + 6 x13 + 2 x14 + 7 x15
Subject To
  e1: x1 + x3 + x5 + x7 + x10 + x12 + x14 >= 1
  e2: x2 + x4 + x6 + x8 + x11 + x13 >= 1
  e3: x1 + x2 + x9 + x10 + x15 >= 1
  e4: x3 + x4 + x5 + x11 + x12 >= 1
  e5: x6 + x7 + x8 + x13 + x14 + x15 >= 1
  e6: x1 + x4 + x9 + x11 + x15 >= 1
  e7: x2 + x5 + x10 + x12 + x14 >= 1
  e8: x3 + x6 + x7 + x13 + x15 >= 1
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15
End
