"""python3 test_evaluate.py: checks the acceptance-rule numbers in evaluate.py (no model needed)."""
from evaluate import app_probabilities, recommend, rule

right = {"p": [0.9, 0.05, 0.05], "right": True, "nothing": False}
wrong = {"p": [0.5, 0.4, 0.1], "right": False, "nothing": False}
nothing = {"p": [0.34, 0.33, 0.33], "right": False, "nothing": True}
results = [right, wrong, nothing, None]   # None: a row laya could not use

# Sort everything: 2 of 2 files with a module sorted, 1 of 3 sorts right, nothing left alone.
r = rule(results, 0.0, 1.0)
assert (r["coverage"], r["precision"], r["rule_accuracy"], r["sorted"]) == (1.0, 1 / 3, 1 / 3, 3), r
# The app's rule today: only the sure one is sorted; the "nothing fits" file left alone counts as right.
r = rule(results, 0.6, 2.0)
assert (r["coverage"], r["precision"], r["rule_accuracy"]) == (0.5, 1.0, 2 / 3), r
assert rule(results, 0.95, 1.0)["precision"] is None

# The loosest pair at >= 95 % precision, ties broken towards smaller thresholds.
points = [rule(results, p, lead) for p in (0.0, 0.6, 0.8) for lead in (1.0, 2.0)]
best = recommend(points)
assert (best["min_probability"], best["min_lead"]) == (0.0, 2.0), best
assert recommend([rule([wrong], 0.0, 1.0)]) is None

# Temperatures as src/laya.rs picks them: the option-count bucket first, else the choice scalar.
cfg = {"temperature": [2.0, 1.0, 1.0], "temperature_by_options": {"choice:3-5": 0.5}}
assert abs(app_probabilities([1.0, 0.0], 2, cfg)[0] - 0.6225) < 1e-4       # T = 2
assert abs(app_probabilities([1.0, 0.0, 0.0], 3, cfg)[0] - 0.7870) < 1e-4  # T = 0.5
print("ok")
