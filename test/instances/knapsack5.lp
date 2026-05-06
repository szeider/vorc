\ 5-variable knapsack with LP-IP gap: max 5x1+8x2+3x3+7x4+2x5
\ s.t. 4x1+5x2+3x3+6x4+2x5 <= 10
\ IP opt = 13 (x1=1,x2=1), LP opt ≈ 14.17, floor = 14 > 13 => branching required
Maximize
  obj: 5 x1 + 8 x2 + 3 x3 + 7 x4 + 2 x5
Subject To
  c1: 4 x1 + 5 x2 + 3 x3 + 6 x4 + 2 x5 <= 10
Binary
  x1 x2 x3 x4 x5
End
