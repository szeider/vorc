\ 8-variable general integer program
\ Variables x1..x8 with bounds 0..5
\ Tests general integer branching (floor/ceil)
Minimize
  obj: 3 x1 + 7 x2 + 2 x3 + 5 x4 + 4 x5 + 6 x6 + 8 x7 + 3 x8
Subject To
  c1: 2 x1 + 3 x2 + x3 + 4 x4 + 2 x5 + x6 + 3 x7 + 2 x8 >= 15
  c2: x1 + 2 x2 + 3 x3 + x4 + 3 x5 + 2 x6 + x7 + 4 x8 >= 12
  c3: 3 x1 + x2 + 2 x3 + 2 x4 + x5 + 3 x6 + 2 x7 + x8 >= 13
Bounds
  0 <= x1 <= 5
  0 <= x2 <= 5
  0 <= x3 <= 5
  0 <= x4 <= 5
  0 <= x5 <= 5
  0 <= x6 <= 5
  0 <= x7 <= 5
  0 <= x8 <= 5
General
  x1 x2 x3 x4 x5 x6 x7 x8
End
