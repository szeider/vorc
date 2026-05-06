\ 10-variable multi-dimensional knapsack
\ 4 knapsack constraints with non-unit coefficients
\ IP opt = 38, LP opt ≈ 39.92, floor = 39 > 38 => branching required
\ Tests numerical stability with larger coefficient matrix
Maximize
  obj: 7 x1 + 11 x2 + 5 x3 + 9 x4 + 3 x5 + 13 x6 + 6 x7 + 8 x8 + 4 x9 + 10 x10
Subject To
  k1: 3 x1 + 5 x2 + 2 x3 + 4 x4 + x5 + 6 x6 + 3 x7 + 2 x8 + x9 + 4 x10 <= 15
  k2: 2 x1 + 3 x2 + 4 x3 + x4 + 5 x5 + 2 x6 + x7 + 3 x8 + 4 x9 + 2 x10 <= 12
  k3: x1 + 2 x2 + x3 + 3 x4 + 2 x5 + x6 + 4 x7 + x8 + 3 x9 + 5 x10 <= 13
  k4: 4 x1 + x2 + 3 x3 + 2 x4 + x5 + 3 x6 + 2 x7 + 4 x8 + x9 + x10 <= 11
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10
End
