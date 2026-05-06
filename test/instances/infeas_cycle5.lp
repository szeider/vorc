\ MIP-infeasible, LP-feasible: 5-cycle vertex cover with tight budget.
\ Odd cycle needs ceil(5/2)=3 vertices, but budget allows at most 2.5.
\ LP relaxation: all x=0.5, sum=2.5, feasible.
\ MIP: minimum cover = 3, 2*3=6 > 5. Infeasible.
Minimize
  obj: x1 + x2 + x3 + x4 + x5
Subject To
  e12: x1 + x2 >= 1
  e23: x2 + x3 >= 1
  e34: x3 + x4 >= 1
  e45: x4 + x5 >= 1
  e51: x5 + x1 >= 1
  budget: 2 x1 + 2 x2 + 2 x3 + 2 x4 + 2 x5 <= 5
Bounds
Binary
  x1 x2 x3 x4 x5
End
