\ Mixed-integer: min x + y + z, x integer, y binary, z continuous
\ c1: x + 2z >= 3, c2: y + z >= 1.5
\ LP: x=1, y=0, z=1 → obj=2. IP must have x integer, y binary.
\ Optimal: x=1, y=1, z=0.5 → obj=2.5 (or x=1, y=0, z=1.5 → obj=2.5)
\ Actually let's make it simpler: unique optimal for easy verification.
\ min x + y + 2z s.t. x + z >= 2, y + z >= 1, x in Z, y in {0,1}, z >= 0
\ LP opt: x=2, y=1, z=0 → obj=3. That's already integer.
\ Need fractional LP relaxation. Try:
\ min x + 3y + 2z s.t. 2x + z >= 3, y + z >= 2, x in {0,1,...}, y in {0,1}, z >= 0
\ LP: z=3-2x, z=2-y → need z >= max(3-2x, 2-y).
\ x free integer, y binary, z continuous.
\ IP opt: x=1, y=1, z=1 → obj = 1+3+2 = 6
Minimize
  obj: x + 3 y + 2 z
Subject To
  c1: 2 x + z >= 3
  c2: y + z >= 2
General
  x
Binary
  y
End
