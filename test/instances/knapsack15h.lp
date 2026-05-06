\ 15-variable multi-dim knapsack (seed-generated)
\ IP opt = 82, LP opt = 85.9766, LP frac vars = 3, Gurobi nodes = 5
\ seed=151, cap_ratio=0.45
Maximize
  obj: 13 x1 + 8 x2 + 15 x3 + 15 x4 + 13 x5 + 12 x6 + 13 x7 + 5 x8 + 7 x9 + 6 x10
       + 7 x11 + 11 x12 + 3 x13 + 9 x14 + 9 x15
Subject To
  c1: 2 x1 + 8 x2 + 2 x3 + 7 x4 + 5 x5 + 3 x6 + 6 x7 + 2 x8 + 3 x9 + 3 x10
       + 5 x11 + 3 x12 + 3 x13 + 3 x14 + 6 x15 <= 27
  c2: 7 x1 + 1 x2 + 6 x3 + 8 x4 + 2 x5 + 5 x6 + 5 x7 + 3 x8 + 8 x9 + 4 x10
       + 3 x11 + 2 x12 + 6 x13 + 7 x14 + 2 x15 <= 31
  c3: 8 x1 + 7 x2 + 4 x3 + 7 x4 + 4 x5 + 2 x6 + 8 x7 + 1 x8 + 5 x9 + 8 x10
       + 4 x11 + 4 x12 + 4 x13 + 1 x14 + 7 x15 <= 33
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15
End
