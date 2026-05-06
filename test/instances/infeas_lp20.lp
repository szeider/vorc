\ LP-infeasible: 20 binary variables, contradictory capacity/demand.
\ Capacity: at most 3 from each group of 10.
\ Demand: at least 4 from each group.
\ Infeasible even as LP (cap <= 3 < 4 <= dem).
Minimize
  obj: x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10
     + x11 + x12 + x13 + x14 + x15 + x16 + x17 + x18 + x19 + x20
Subject To
  cap1: x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10 <= 3
  cap2: x11 + x12 + x13 + x14 + x15 + x16 + x17 + x18 + x19 + x20 <= 3
  dem1: x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10 >= 4
  dem2: x11 + x12 + x13 + x14 + x15 + x16 + x17 + x18 + x19 + x20 >= 4
Bounds
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10
  x11 x12 x13 x14 x15 x16 x17 x18 x19 x20
End
