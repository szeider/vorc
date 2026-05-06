\ MIP-infeasible, LP-feasible: 4-coloring of K5 (chi=5). 20 binary vars.
Minimize
  obj: x0_0 + x0_1 + x0_2 + x0_3 + x1_0 + x1_1
      + x1_2 + x1_3 + x2_0 + x2_1 + x2_2 + x2_3
      + x3_0 + x3_1 + x3_2 + x3_3 + x4_0 + x4_1
      + x4_2 + x4_3
Subject To
  asgn_lo_0: x0_0 + x0_1 + x0_2 + x0_3 >= 1
  asgn_hi_0: x0_0 + x0_1 + x0_2 + x0_3 <= 1
  asgn_lo_1: x1_0 + x1_1 + x1_2 + x1_3 >= 1
  asgn_hi_1: x1_0 + x1_1 + x1_2 + x1_3 <= 1
  asgn_lo_2: x2_0 + x2_1 + x2_2 + x2_3 >= 1
  asgn_hi_2: x2_0 + x2_1 + x2_2 + x2_3 <= 1
  asgn_lo_3: x3_0 + x3_1 + x3_2 + x3_3 >= 1
  asgn_hi_3: x3_0 + x3_1 + x3_2 + x3_3 <= 1
  asgn_lo_4: x4_0 + x4_1 + x4_2 + x4_3 >= 1
  asgn_hi_4: x4_0 + x4_1 + x4_2 + x4_3 <= 1
  conf_0_0: x0_0 + x1_0 <= 1
  conf_0_1: x0_1 + x1_1 <= 1
  conf_0_2: x0_2 + x1_2 <= 1
  conf_0_3: x0_3 + x1_3 <= 1
  conf_1_0: x0_0 + x2_0 <= 1
  conf_1_1: x0_1 + x2_1 <= 1
  conf_1_2: x0_2 + x2_2 <= 1
  conf_1_3: x0_3 + x2_3 <= 1
  conf_2_0: x0_0 + x3_0 <= 1
  conf_2_1: x0_1 + x3_1 <= 1
  conf_2_2: x0_2 + x3_2 <= 1
  conf_2_3: x0_3 + x3_3 <= 1
  conf_3_0: x0_0 + x4_0 <= 1
  conf_3_1: x0_1 + x4_1 <= 1
  conf_3_2: x0_2 + x4_2 <= 1
  conf_3_3: x0_3 + x4_3 <= 1
  conf_4_0: x1_0 + x2_0 <= 1
  conf_4_1: x1_1 + x2_1 <= 1
  conf_4_2: x1_2 + x2_2 <= 1
  conf_4_3: x1_3 + x2_3 <= 1
  conf_5_0: x1_0 + x3_0 <= 1
  conf_5_1: x1_1 + x3_1 <= 1
  conf_5_2: x1_2 + x3_2 <= 1
  conf_5_3: x1_3 + x3_3 <= 1
  conf_6_0: x1_0 + x4_0 <= 1
  conf_6_1: x1_1 + x4_1 <= 1
  conf_6_2: x1_2 + x4_2 <= 1
  conf_6_3: x1_3 + x4_3 <= 1
  conf_7_0: x2_0 + x3_0 <= 1
  conf_7_1: x2_1 + x3_1 <= 1
  conf_7_2: x2_2 + x3_2 <= 1
  conf_7_3: x2_3 + x3_3 <= 1
  conf_8_0: x2_0 + x4_0 <= 1
  conf_8_1: x2_1 + x4_1 <= 1
  conf_8_2: x2_2 + x4_2 <= 1
  conf_8_3: x2_3 + x4_3 <= 1
  conf_9_0: x3_0 + x4_0 <= 1
  conf_9_1: x3_1 + x4_1 <= 1
  conf_9_2: x3_2 + x4_2 <= 1
  conf_9_3: x3_3 + x4_3 <= 1
Bounds
Binary
  x0_0 x0_1 x0_2 x0_3 x1_0 x1_1 x1_2 x1_3 x2_0 x2_1 x2_2 x2_3 x3_0 x3_1
  x3_2 x3_3 x4_0 x4_1 x4_2 x4_3
End
