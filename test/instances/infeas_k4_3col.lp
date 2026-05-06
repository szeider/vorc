\ MIP-infeasible, LP-feasible: 3-coloring of K4 (chi=4). 12 binary vars.
Minimize
  obj: x0_0 + x0_1 + x0_2 + x1_0 + x1_1 + x1_2
      + x2_0 + x2_1 + x2_2 + x3_0 + x3_1 + x3_2
Subject To
  asgn_lo_0: x0_0 + x0_1 + x0_2 >= 1
  asgn_hi_0: x0_0 + x0_1 + x0_2 <= 1
  asgn_lo_1: x1_0 + x1_1 + x1_2 >= 1
  asgn_hi_1: x1_0 + x1_1 + x1_2 <= 1
  asgn_lo_2: x2_0 + x2_1 + x2_2 >= 1
  asgn_hi_2: x2_0 + x2_1 + x2_2 <= 1
  asgn_lo_3: x3_0 + x3_1 + x3_2 >= 1
  asgn_hi_3: x3_0 + x3_1 + x3_2 <= 1
  conf_0_0: x0_0 + x1_0 <= 1
  conf_0_1: x0_1 + x1_1 <= 1
  conf_0_2: x0_2 + x1_2 <= 1
  conf_1_0: x0_0 + x2_0 <= 1
  conf_1_1: x0_1 + x2_1 <= 1
  conf_1_2: x0_2 + x2_2 <= 1
  conf_2_0: x0_0 + x3_0 <= 1
  conf_2_1: x0_1 + x3_1 <= 1
  conf_2_2: x0_2 + x3_2 <= 1
  conf_3_0: x1_0 + x2_0 <= 1
  conf_3_1: x1_1 + x2_1 <= 1
  conf_3_2: x1_2 + x2_2 <= 1
  conf_4_0: x1_0 + x3_0 <= 1
  conf_4_1: x1_1 + x3_1 <= 1
  conf_4_2: x1_2 + x3_2 <= 1
  conf_5_0: x2_0 + x3_0 <= 1
  conf_5_1: x2_1 + x3_1 <= 1
  conf_5_2: x2_2 + x3_2 <= 1
Bounds
Binary
  x0_0 x0_1 x0_2 x1_0 x1_1 x1_2 x2_0 x2_1 x2_2 x3_0 x3_1 x3_2
End
