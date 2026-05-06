\ Mixed-integer: min 3x1 + 2x2 + x3 + 4y1 + y2 + 2y3
\ x1,x2,x3 binary; y1,y2,y3 continuous >= 0
\ Production planning: integer setup, continuous production
Minimize
  obj: 3 x1 + 2 x2 + x3 + 4 y1 + y2 + 2 y3
Subject To
  demand1: y1 + y2 >= 3
  demand2: y2 + y3 >= 2
  link1: y1 - 5 x1 <= 0
  link2: y2 - 5 x2 <= 0
  link3: y3 - 5 x3 <= 0
  budget: x1 + x2 + x3 <= 2
Binary
  x1 x2 x3
End
