\ 4-variable general integer program
\ Tests general integer branching
Maximize
  obj: 5 x1 + 8 x2 + 3 x3 + 7 x4
Subject To
  c1: 2 x1 + 3 x2 + x3 + 4 x4 <= 12
  c2: x1 + 2 x2 + 3 x3 + x4 <= 10
Bounds
  0 <= x1 <= 4
  0 <= x2 <= 3
  0 <= x3 <= 3
  0 <= x4 <= 3
General
  x1 x2 x3 x4
End
