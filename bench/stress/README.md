# Court Jester Stress Harness

This directory is for service-level stress and soak testing of the MCP server.

It is intentionally separate from the task benchmark in `bench/run_matrix.py`.

Use the task benchmark when you want to answer:

- does Court Jester improve agent coding outcomes?
- does a repair loop improve hidden-check pass rate?

Use the stress harness when you want to answer:

- does the MCP server stay healthy under sustained agent traffic?
- how do latency and failures behave under concurrency?
- do timeouts, memory pressure, and temp-file cleanup behave correctly?

## Scenarios

Scenarios live in `bench/stress/scenarios/`.

Each scenario declares:

- `mode`: `per_agent_server` or `shared_server`
- `concurrency`
- `requests_per_worker`
- `request_timeout_seconds`
- `request_mix`
- `payloads`

The first implementation supports `per_agent_server`. `shared_server` is reserved for a follow-up pass with a multiplexed client.

## Run

```bash
python -m bench.stress.run_stress --scenario mixed_verify
python -m bench.stress.run_stress --scenario low_pressure_mixed
python -m bench.stress.run_stress --scenario timeout_pressure
python -m bench.stress.run_stress --scenario memory_pressure
```

## Regression checklist

Run this set after changes to:

- `src/tools/sandbox.rs`
- `src/tools/verify.rs`
- `src/tools/lint.rs`
- MCP stdio transport or process lifecycle code

Recommended order:

1. `python -m bench.stress.run_stress --scenario low_pressure_mixed`
2. `python -m bench.stress.run_stress --scenario mixed_verify`
3. `python -m bench.stress.run_stress --scenario timeout_pressure`
4. `python -m bench.stress.run_stress --scenario memory_pressure`

Pass criteria:

1. `low_pressure_mixed`: `success_rate == 1.0`
2. `mixed_verify`: `success_rate == 1.0`
3. `timeout_pressure`: no `connection_closed`, no process exits, timeout responses stay structured
4. `memory_pressure`: no `connection_closed`, no process exits, memory failures stay structured

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

- `low_pressure_mixed`: single-worker smoke test for request sequencing and session stability
- `mixed_verify`: concurrent verify-heavy workload that approximates agent traffic
- `timeout_pressure`: execute-only timeout enforcement under repeated adversarial code
- `memory_pressure`: execute-only memory pressure under repeated adversarial code
