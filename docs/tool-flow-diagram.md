# Court Jester Tool And Flow Diagram

Date: 2026-04-10

```mermaid
flowchart TD
    A[Task manifest or direct CLI call] --> B{Entry path}

    B -->|Benchmark harness| C[bench/run_matrix.py / bench/runner.py]
    B -->|Direct command| D[src/main.rs]

    C --> E[Model provider edits temp workspace]
    E --> F[bench/cli_client.py shells out to court-jester]
    F --> D

    subgraph S1[Court Jester CLI]
        D --> G[resolve_code + parse_language]
        G --> H{Command}

        H --> I[analyze]
        H --> J[lint]
        H --> K[execute]
        H --> L[verify]

        I --> I1[src/tools/analyze.rs]
        I1 --> I2[tree-sitter parse]
        I2 --> I3[functions classes imports complexity]
        I3 --> I4[optional diff filter via src/tools/diff.rs]
        I4 --> I5[AnalysisResult JSON]

        J --> J1[src/tools/lint.rs]
        J1 --> J2[Python: ruff JSON]
        J1 --> J3[TypeScript: biome JSON via project local, sibling binary, or PATH]
        J2 --> J4[LintResult JSON]
        J3 --> J4

        K --> K1[src/tools/sandbox.rs]
        K1 --> K2[write temp or sibling file]
        K2 --> K3[detect .venv / node_modules / cwd]
        K3 --> K4[run python3 or tsx with timeout + memory guard]
        K4 --> K5[ExecutionResult JSON]

        L --> L1[src/tools/verify.rs]
        L1 --> L2[analyze stage]
        L2 --> L3[optional complexity gate]
        L3 --> L4[lint stage]
        L4 --> L5[synthesize fuzz harness]
        L5 --> L6[src/tools/synthesize.rs]
        L6 --> L7[property checks from signatures and types]
        L7 --> L8[execute synthesized harness in sandbox]
        L8 --> L9[optional explicit test stage]
        L9 --> L10[VerificationReport JSON]
    end

    L10 --> M{verify outcome}
    M -->|pass| N[public checks]
    M -->|fail| O[compact repro feedback]
    O --> P[repair attempt by provider]
    P --> F

    N --> Q[hidden evaluator in bench/evaluators]
    Q --> R[result.json + diff + artifacts]
    R --> T[bench/summarize_runs.py]
```

## Reading Notes

- `analyze`, `lint`, `execute`, and `verify` are CLI commands.
- `verify` is the main product loop: `analyze -> lint -> synthesize -> execute -> optional tests`.
- `diff` and `synthesize` are internal modules that support `analyze` and `verify`.
- The benchmark harness uses `verify` as either a gate or a repair-loop trigger before hidden evaluation.
