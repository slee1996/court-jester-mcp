assert plan_price("starter") == 0
assert plan_price(" pro ") == 1900
assert plan_price("TEAM") == 4900
