\ 3-variable set cover: min x + y + z s.t. covering constraints
\ Tests deeper branching
Minimize
  obj: x + y + z
Subject To
  c1: x + y >= 1
  c2: y + z >= 1
  c3: x + z >= 1
Binary
  x y z
End
