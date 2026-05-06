\ 6-variable ILP designed to trigger Farkas certificates
\ Conflict constraints (c3,c4) + coverage (c5) make x5=0 branch LP-infeasible
\ IP opt = 17, LP opt = 15 => branching required, Farkas on infeasible branches
Minimize
  obj: 2 x1 + 3 x2 + 3 x3 + 2 x4 + 8 x5 + 5 x6
Subject To
  c1: x1 + x2 >= 1
  c2: x3 + x4 >= 1
  c3: x1 + x3 <= 1
  c4: x2 + x4 <= 1
  c5: x1 + x2 + x3 + x4 + x5 >= 3
  c6: 2 x5 + 5 x6 >= 5
Binary
  x1 x2 x3 x4 x5 x6
End
