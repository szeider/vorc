\ MIP-infeasible, LP-feasible: 21-cycle vertex cover with tight budget.
\ Odd cycle needs ceil(21/2)=11 vertices, but budget allows at most 10.5.
\ LP relaxation: all x=0.5, sum=10.5, feasible.
\ MIP: minimum cover = 11, 2*11=22 > 21. Infeasible.
Minimize
  obj: x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10
     + x11 + x12 + x13 + x14 + x15 + x16 + x17 + x18 + x19 + x20
     + x21
Subject To
  e01: x1 + x2 >= 1
  e02: x2 + x3 >= 1
  e03: x3 + x4 >= 1
  e04: x4 + x5 >= 1
  e05: x5 + x6 >= 1
  e06: x6 + x7 >= 1
  e07: x7 + x8 >= 1
  e08: x8 + x9 >= 1
  e09: x9 + x10 >= 1
  e10: x10 + x11 >= 1
  e11: x11 + x12 >= 1
  e12: x12 + x13 >= 1
  e13: x13 + x14 >= 1
  e14: x14 + x15 >= 1
  e15: x15 + x16 >= 1
  e16: x16 + x17 >= 1
  e17: x17 + x18 >= 1
  e18: x18 + x19 >= 1
  e19: x19 + x20 >= 1
  e20: x20 + x21 >= 1
  e21: x21 + x1 >= 1
  budget: 2 x1 + 2 x2 + 2 x3 + 2 x4 + 2 x5 + 2 x6 + 2 x7 + 2 x8
        + 2 x9 + 2 x10 + 2 x11 + 2 x12 + 2 x13 + 2 x14 + 2 x15
        + 2 x16 + 2 x17 + 2 x18 + 2 x19 + 2 x20 + 2 x21 <= 21
Bounds
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15
  x16 x17 x18 x19 x20 x21
End
