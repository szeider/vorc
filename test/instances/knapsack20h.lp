\ 20-variable multi-dim knapsack (seed-generated)
\ IP opt = 114, LP opt = 119.9508, LP frac vars = 5, Gurobi nodes = 23
\ seed=13, cap_ratio=0.48
Maximize
  obj: 6 x1 + 6 x2 + 12 x3 + 12 x4 + 14 x5 + 15 x6 + 4 x7 + 12 x8 + 5 x9 + 12 x10
       + 4 x11 + 15 x12 + 5 x13 + 12 x14 + 13 x15 + 4 x16 + 4 x17 + 3 x18 + 10 x19 + 15 x20
Subject To
  c1: 4 x1 + 5 x2 + 1 x3 + 7 x4 + 3 x5 + 1 x6 + 5 x7 + 3 x8 + 2 x9 + 5 x10
       + 8 x11 + 7 x12 + 3 x13 + 5 x14 + 6 x15 + 4 x16 + 8 x17 + 7 x18 + 6 x19 + 7 x20 <= 46
  c2: 6 x1 + 2 x2 + 6 x3 + 5 x4 + 8 x5 + 3 x6 + 8 x7 + 8 x8 + 3 x9 + 5 x10
       + 4 x11 + 3 x12 + 6 x13 + 5 x14 + 6 x15 + 8 x16 + 5 x17 + 5 x18 + 7 x19 + 3 x20 <= 50
  c3: 8 x1 + 4 x2 + 4 x3 + 4 x4 + 6 x5 + 3 x6 + 2 x7 + 7 x8 + 8 x9 + 7 x10
       + 1 x11 + 7 x12 + 1 x13 + 4 x14 + 3 x15 + 8 x16 + 8 x17 + 5 x18 + 3 x19 + 7 x20 <= 48
  c4: 5 x1 + 6 x2 + 4 x3 + 6 x4 + 3 x5 + 4 x6 + 8 x7 + 4 x8 + 1 x9 + 1 x10
       + 6 x11 + 6 x12 + 3 x13 + 5 x14 + 4 x15 + 3 x16 + 2 x17 + 4 x18 + 3 x19 + 2 x20 <= 38
  c5: 7 x1 + 1 x2 + 8 x3 + 6 x4 + 2 x5 + 1 x6 + 3 x7 + 5 x8 + 2 x9 + 7 x10
       + 5 x11 + 5 x12 + 2 x13 + 8 x14 + 6 x15 + 1 x16 + 1 x17 + 6 x18 + 6 x19 + 3 x20 <= 40
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15 x16 x17 x18 x19 x20
End
