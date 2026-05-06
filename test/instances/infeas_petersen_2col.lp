\ MIP-infeasible, LP-feasible: 2-coloring of Petersen graph (chi=3). 20 binary vars, 15 edges.
Minimize
  obj: x0_0 + x0_1 + x1_0 + x1_1 + x2_0 + x2_1
      + x3_0 + x3_1 + x4_0 + x4_1 + x5_0 + x5_1
      + x6_0 + x6_1 + x7_0 + x7_1 + x8_0 + x8_1
      + x9_0 + x9_1
Subject To
  asgn_lo_0: x0_0 + x0_1 >= 1
  asgn_hi_0: x0_0 + x0_1 <= 1
  asgn_lo_1: x1_0 + x1_1 >= 1
  asgn_hi_1: x1_0 + x1_1 <= 1
  asgn_lo_2: x2_0 + x2_1 >= 1
  asgn_hi_2: x2_0 + x2_1 <= 1
  asgn_lo_3: x3_0 + x3_1 >= 1
  asgn_hi_3: x3_0 + x3_1 <= 1
  asgn_lo_4: x4_0 + x4_1 >= 1
  asgn_hi_4: x4_0 + x4_1 <= 1
  asgn_lo_5: x5_0 + x5_1 >= 1
  asgn_hi_5: x5_0 + x5_1 <= 1
  asgn_lo_6: x6_0 + x6_1 >= 1
  asgn_hi_6: x6_0 + x6_1 <= 1
  asgn_lo_7: x7_0 + x7_1 >= 1
  asgn_hi_7: x7_0 + x7_1 <= 1
  asgn_lo_8: x8_0 + x8_1 >= 1
  asgn_hi_8: x8_0 + x8_1 <= 1
  asgn_lo_9: x9_0 + x9_1 >= 1
  asgn_hi_9: x9_0 + x9_1 <= 1
  conf_0_0: x0_0 + x1_0 <= 1
  conf_0_1: x0_1 + x1_1 <= 1
  conf_1_0: x1_0 + x2_0 <= 1
  conf_1_1: x1_1 + x2_1 <= 1
  conf_2_0: x2_0 + x3_0 <= 1
  conf_2_1: x2_1 + x3_1 <= 1
  conf_3_0: x3_0 + x4_0 <= 1
  conf_3_1: x3_1 + x4_1 <= 1
  conf_4_0: x4_0 + x0_0 <= 1
  conf_4_1: x4_1 + x0_1 <= 1
  conf_5_0: x5_0 + x7_0 <= 1
  conf_5_1: x5_1 + x7_1 <= 1
  conf_6_0: x7_0 + x9_0 <= 1
  conf_6_1: x7_1 + x9_1 <= 1
  conf_7_0: x9_0 + x6_0 <= 1
  conf_7_1: x9_1 + x6_1 <= 1
  conf_8_0: x6_0 + x8_0 <= 1
  conf_8_1: x6_1 + x8_1 <= 1
  conf_9_0: x8_0 + x5_0 <= 1
  conf_9_1: x8_1 + x5_1 <= 1
  conf_10_0: x0_0 + x5_0 <= 1
  conf_10_1: x0_1 + x5_1 <= 1
  conf_11_0: x1_0 + x6_0 <= 1
  conf_11_1: x1_1 + x6_1 <= 1
  conf_12_0: x2_0 + x7_0 <= 1
  conf_12_1: x2_1 + x7_1 <= 1
  conf_13_0: x3_0 + x8_0 <= 1
  conf_13_1: x3_1 + x8_1 <= 1
  conf_14_0: x4_0 + x9_0 <= 1
  conf_14_1: x4_1 + x9_1 <= 1
Bounds
Binary
  x0_0 x0_1 x1_0 x1_1 x2_0 x2_1 x3_0 x3_1 x4_0 x4_1 x5_0 x5_1 x6_0 x6_1
  x7_0 x7_1 x8_0 x8_1 x9_0 x9_1
End
