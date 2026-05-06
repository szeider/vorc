\ MIP-infeasible, LP-feasible: bin packing, 5 items size 3, 3 bins capacity 5.
\ LP: fractional assignment feasible (total 15 = 15).
\ MIP: each bin holds at most floor(5/3)=1 item, so 3 bins hold 3 items < 5. Infeasible.
\ Variables: y_{item}_{bin} binary.
Minimize
  obj: y0_0 + y0_1 + y0_2 + y1_0 + y1_1 + y1_2
     + y2_0 + y2_1 + y2_2 + y3_0 + y3_1 + y3_2
     + y4_0 + y4_1 + y4_2
Subject To
  asgn0_lo: y0_0 + y0_1 + y0_2 >= 1
  asgn0_hi: y0_0 + y0_1 + y0_2 <= 1
  asgn1_lo: y1_0 + y1_1 + y1_2 >= 1
  asgn1_hi: y1_0 + y1_1 + y1_2 <= 1
  asgn2_lo: y2_0 + y2_1 + y2_2 >= 1
  asgn2_hi: y2_0 + y2_1 + y2_2 <= 1
  asgn3_lo: y3_0 + y3_1 + y3_2 >= 1
  asgn3_hi: y3_0 + y3_1 + y3_2 <= 1
  asgn4_lo: y4_0 + y4_1 + y4_2 >= 1
  asgn4_hi: y4_0 + y4_1 + y4_2 <= 1
  cap0: 3 y0_0 + 3 y1_0 + 3 y2_0 + 3 y3_0 + 3 y4_0 <= 5
  cap1: 3 y0_1 + 3 y1_1 + 3 y2_1 + 3 y3_1 + 3 y4_1 <= 5
  cap2: 3 y0_2 + 3 y1_2 + 3 y2_2 + 3 y3_2 + 3 y4_2 <= 5
Bounds
Binary
  y0_0 y0_1 y0_2 y1_0 y1_1 y1_2 y2_0 y2_1 y2_2
  y3_0 y3_1 y3_2 y4_0 y4_1 y4_2
End
