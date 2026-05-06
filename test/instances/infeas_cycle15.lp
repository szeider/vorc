\ MIP-infeasible, LP-feasible: 15-cycle vertex cover with tight budget.
\ Odd cycle needs ceil(15/2)=8 vertices, but budget allows at most 7.5.
\ LP relaxation: all x=0.5, sum=7.5, feasible.
\ MIP: minimum cover = 8, 2*8=16 > 15. Infeasible.
Minimize
  obj: x1 + x2 + x3 + x4 + x5 + x6 + x7 + x8 + x9 + x10
     + x11 + x12 + x13 + x14 + x15
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
  e15: x15 + x1 >= 1
  budget: 2 x1 + 2 x2 + 2 x3 + 2 x4 + 2 x5 + 2 x6 + 2 x7 + 2 x8
        + 2 x9 + 2 x10 + 2 x11 + 2 x12 + 2 x13 + 2 x14 + 2 x15 <= 15
Bounds
Binary
  x1 x2 x3 x4 x5 x6 x7 x8 x9 x10 x11 x12 x13 x14 x15
End
