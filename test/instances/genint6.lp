\ 6-variable general integer, designed with LP-IP gap
\ LP-relaxation has fractional values, forcing branching
Maximize
  obj: 7 x1 + 11 x2 + 5 x3 + 9 x4 + 3 x5 + 6 x6
Subject To
  c1: 3 x1 + 4 x2 + 2 x3 + 5 x4 + x5 + 3 x6 <= 18
  c2: 2 x1 + x2 + 3 x3 + 2 x4 + 4 x5 + 2 x6 <= 15
  c3: x1 + 3 x2 + x3 + x4 + 2 x5 + 4 x6 <= 14
Bounds
  0 <= x1 <= 3
  0 <= x2 <= 3
  0 <= x3 <= 3
  0 <= x4 <= 3
  0 <= x5 <= 3
  0 <= x6 <= 3
General
  x1 x2 x3 x4 x5 x6
End
