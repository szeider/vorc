\ 12-variable general integer program with tighter gap
\ Variables with bounds 0..4, coefficients designed to force branching
Maximize
  obj: 11 x1 + 7 x2 + 13 x3 + 5 x4 + 9 x5 + 3 x6 + 8 x7 + 14 x8 + 6 x9 + 10 x10 + 4 x11 + 12 x12
Subject To
  c1: 3 x1 + 5 x2 + 2 x3 + 4 x4 + x5 + 6 x6 + 3 x7 + 2 x8 + x9 + 4 x10 + 5 x11 + 3 x12 <= 22
  c2: 2 x1 + 3 x2 + 4 x3 + x4 + 5 x5 + 2 x6 + x7 + 3 x8 + 4 x9 + 2 x10 + 3 x11 + x12 <= 18
  c3: x1 + 2 x2 + x3 + 3 x4 + 2 x5 + x6 + 4 x7 + x8 + 3 x9 + 5 x10 + 2 x11 + 4 x12 <= 20
  c4: 4 x1 + x2 + 3 x3 + 2 x4 + x5 + 3 x6 + 2 x7 + 4 x8 + x9 + x10 + 4 x11 + 2 x12 <= 19
Bounds
  0 <= x1 <= 4
  0 <= x2 <= 4
  0 <= x3 <= 4
  0 <= x4 <= 4
  0 <= x5 <= 4
  0 <= x6 <= 4
  0 <= x7 <= 4
  0 <= x8 <= 4
  0 <= x9 <= 4
  0 <= x10 <= 4
  0 <= x11 <= 4
  0 <= x12 <= 4
General
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12
End
