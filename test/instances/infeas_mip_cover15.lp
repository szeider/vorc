\ MIP-infeasible, LP-feasible: 15 binary variables.
\ Cover all 5 elements, but no combination of allowed sets can cover all.
\ LP relaxation is feasible (fractional solutions exist).
\ Sets: {1,2}, {2,3}, {3,4}, {4,5}, {1,5}
\ Constraint: pick at most 2 sets. But 2 sets can cover at most 4 elements.
Minimize
  obj: x1 + x2 + x3 + x4 + x5
Subject To
  cover1: x1 + x5 >= 1
  cover2: x1 + x2 >= 1
  cover3: x2 + x3 >= 1
  cover4: x3 + x4 >= 1
  cover5: x4 + x5 >= 1
  budget: x1 + x2 + x3 + x4 + x5 <= 2
Bounds
Binary
  x1 x2 x3 x4 x5
End
