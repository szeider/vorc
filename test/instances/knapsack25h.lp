\ 25-variable multi-dim knapsack (seed-generated)
\ IP opt = 137, LP opt = 140.7588, LP frac vars = 4, Gurobi nodes = 30
\ seed=12, cap_ratio=0.5
Maximize
  obj: 9 x1 + 6 x2 + 12 x3 + 10 x4 + 12 x5 + 7 x6 + 4 x7 + 8 x8 + 2 x9 + 7 x10
       + 9 x11 + 6 x12 + 12 x13 + 14 x14 + 9 x15 + 13 x16 + 15 x17 + 11 x18 + 5 x19 + 10 x20
       + 2 x21 + 12 x22 + 11 x23 + 4 x24 + 9 x25
Subject To
  c1: 6 x1 + 3 x2 + 6 x3 + 4 x4 + 1 x5 + 4 x6 + 2 x7 + 6 x8 + 7 x9 + 2 x10
       + 1 x11 + 1 x12 + 4 x13 + 2 x14 + 7 x15 + 8 x16 + 2 x17 + 7 x18 + 3 x19 + 6 x20
       + 3 x21 + 1 x22 + 3 x23 + 2 x24 + 7 x25 <= 49
  c2: 7 x1 + 8 x2 + 8 x3 + 7 x4 + 1 x5 + 2 x6 + 4 x7 + 5 x8 + 6 x9 + 6 x10
       + 7 x11 + 5 x12 + 2 x13 + 5 x14 + 4 x15 + 6 x16 + 6 x17 + 6 x18 + 3 x19 + 1 x20
       + 7 x21 + 7 x22 + 1 x23 + 1 x24 + 4 x25 <= 59
  c3: 7 x1 + 1 x2 + 7 x3 + 4 x4 + 2 x5 + 4 x6 + 3 x7 + 2 x8 + 5 x9 + 1 x10
       + 7 x11 + 5 x12 + 8 x13 + 6 x14 + 1 x15 + 8 x16 + 6 x17 + 4 x18 + 6 x19 + 5 x20
       + 8 x21 + 8 x22 + 8 x23 + 4 x24 + 3 x25 <= 61
  c4: 8 x1 + 6 x2 + 3 x3 + 4 x4 + 4 x5 + 1 x6 + 5 x7 + 6 x8 + 3 x9 + 4 x10
       + 1 x11 + 1 x12 + 4 x13 + 2 x14 + 6 x15 + 3 x16 + 1 x17 + 6 x18 + 5 x19 + 2 x20
       + 4 x21 + 8 x22 + 8 x23 + 6 x24 + 5 x25 <= 53
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15 x16 x17 x18 x19 x20
  x21 x22 x23 x24 x25
End
