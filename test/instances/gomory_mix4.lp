\ Mixed-sense 4-var instance designed for Gomory cuts
\ min 3x1 + 5x2 + 2x3 + 4x4
\ s.t.  x1 + 2x2 + x3 + x4 >= 2   (G)
\       2x1 + x2 + 2x3 + x4 <= 3   (L)
\       x1 + x2 + x3 + 2x4 >= 2   (G)
\ LP relaxation should be fractional with mixed senses enabling CG cuts
Minimize
  obj: 3 x1 + 5 x2 + 2 x3 + 4 x4
Subject To
  c1: x1 + 2 x2 + x3 + x4 >= 2
  c2: 2 x1 + x2 + 2 x3 + x4 <= 3
  c3: x1 + x2 + x3 + 2 x4 >= 2
Binary
  x1 x2 x3 x4
End
