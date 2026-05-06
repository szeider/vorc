\ Mixed-integer: max 2x + 3y + z, x binary, y integer, z continuous
\ c1: x + y + z <= 4, c2: 2y + z <= 5, c3: z <= 2.5
\ LP: x=1, y=2.5, z=0 or similar fractional y
\ Forces branching on y (integer) while z stays continuous
Maximize
  obj: 2 x + 3 y + z
Subject To
  c1: x + y + z <= 4
  c2: 2 y + z <= 5
  c3: z <= 2.5
General
  y
Binary
  x
End
