\ Knapsack: max 3x + 4y s.t. 2x + 3y <= 4
\ LP opt = 5.5 (x=0.5,y=1), IP opt = 4 (x=0,y=1)
\ Requires branching: floor(5.5) = 5 > 4
Maximize
  obj: 3 x + 4 y
Subject To
  c1: 2 x + 3 y <= 4
Binary
  x y
End
