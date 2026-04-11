# Court Jester Stress Harness

This directory is for stress and soak testing of the `court-jester` CLI under repeated agent traffic.

It is intentionally separate from the task benchmark in `bench/run_matrix.py`.

Use the task benchmark when you want to answer:

- does Court Jester improve agent coding outcomes?
- does a repair loop improve hidden-check pass rate?

Use the stress harness when you want to answer:

- does the CLI stay healthy under sustained agent traffic?
- how do latency and failures behave under concurrency?
- do timeouts, memory pressure, and temp-file cleanup behave correctly?

## Scenarios

Scenarios live in `bench/stress/scenarios/`.

Each scenario declares:

- `mode`: `per_worker_cli`
- `concurrency`
- `requests_per_worker`
- `request_timeout_seconds`
- `request_mix`
- `payloads`

The current implementation supports `per_worker_cli`, where each worker shells out to `court-jester` directly for every request.

## Run

```bash
python -m bench.stress.run_stress --scenario low_pressure_mixed
python -m bench.stress.run_stress --scenario mixed_verify
python -m bench.stress.run_stress --scenario timeout_pressure
python -m bench.stress.run_stress --scenario memory_pressure
python -m bench.stress.run_stress --scenario soak
```

## Regression checklist

Run this set after changes to:

- `src/tools/sandbox.rs`
- `src/tools/verify.rs`
- `src/tools/lint.rs`
- CLI process lifecycle or wrapper code

Recommended order:

1. `python -m bench.stress.run_stress --scenario low_pressure_mixed`
2. `python -m bench.stress.run_stress --scenario mixed_verify`
3. `python -m bench.stress.run_stress --scenario timeout_pressure`
4. `python -m bench.stress.run_stress --scenario memory_pressure`
5. `python -m bench.stress.run_stress --scenario soak`

Pass criteria:

1. `low_pressure_mixed`: `success_rate == 1.0`
2. `mixed_verify`: `success_rate == 1.0`
3. `timeout_pressure`: no wrapper crashes, timeout responses stay structured
4. `memory_pressure`: no wrapper crashes, memory failures stay structured
5. `soak`: no wrapper crashes, stable latency over 200 requests

Things to inspect when a scenario fails:

1. `summary.json`
2. `requests.ndjson`
3. `error_counts`
4. `process_exit_counts`
5. `stderr_tail` on the first failing request

## Output

Each run writes:

- `summary.json`
- `requests.ndjson`

under `bench/results/stress/<scenario>/<timestamp>/`.

`requests.ndjson` includes per-request:

- error kind
- error message
- process pid
- process return code
- stderr tail captured at failure time

## Scenario intent

- `low_pressure_mixed`: single-worker smoke test for request sequencing and CLI stability
- `mixed_verify`: concurrent verify-heavy workload that approximates agent traffic
- `timeout_pressure`: execute-only timeout enforcement under repeated adversarial code
- `memory_pressure`: execute-only memory pressure under repeated adversarial code
- `soak`: steady-state verify-heavy workload (2 workers, 100 requests each) for long-running stability
