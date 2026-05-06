\ 4-variable covering with tight LP relaxation
\ LP opt = IP opt = 2 (no branching needed, root prune)
\ Edge case: tests root-only certificate generation
Minimize
  obj: x1 + x2 + x3 + x4
Subject To
  c1: x1 + x2 >= 1
  c2: x3 + x4 >= 1
  c3: x1 + x3 >= 1
Binary
  x1 x2 x3 x4
End
