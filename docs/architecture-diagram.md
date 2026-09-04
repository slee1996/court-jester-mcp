# Court Jester Architecture Diagram

Date: 2026-07-10

Court Jester is a CLI verifier for Python and TypeScript. The Rust binary owns analysis, domain planning, harness generation, runtime execution, typed reports, repair views, and replay. The Python benchmark harness is a separate consumer that measures agent-loop utility and precision.

For the current file-level boundaries, see [Implementation map](code-map.md). The binary delegates to `src/cli/`; verifier decisions, report rendering, replay, and subprocess supervision have dedicated internal modules.

```mermaid
flowchart TD
    User[User or agent loop] --> CLI[court-jester CLI]
    Bench[Benchmark harness\nbench.run_matrix / runner] --> CLI
    CLI --> Args[Validated flags\nverdict, gates, profile, limits]
    Args --> Commands{Subcommand}
    Commands --> VerifyCmd[verify]
    Commands --> CiCmd[ci]
    Commands --> ExecuteCmd[execute]
    Commands --> DoctorCmd[doctor]
    Commands --> ReplayCmd[replay]
    Commands --> AnalyzeCmd[analyze]
    Commands --> LintCmd[lint]

    subgraph Analysis[Analysis and planning]
        Parse[Tree-sitter parse]
        Extract[functions, classes, imports, aliases, exports]
        Diff[diff scoping and call graph]
        Domains[repository-derived domain IR]
        Plan[verification plan\nsurfaces, inputs, contracts, callers]
        Parse --> Extract --> Diff --> Domains --> Plan
    end
    VerifyCmd --> Parse
    AnalyzeCmd --> Parse

    subgraph Pipeline[Verify pipeline]
        ParseStage[parse]
        ComplexityStage[complexity gate]
        LintStage[lint advisory]
        CoverageStage[coverage evidence]
        ExecuteStage[execute findings]
        TestStage[authoritative test]
        Verdict[typed verdict + strength]
        ParseStage --> ComplexityStage --> LintStage --> CoverageStage --> ExecuteStage --> TestStage --> Verdict
    end
    Parse --> ParseStage
    Plan --> CoverageStage
    Plan --> ExecuteStage
    TestStage --> Verdict

    subgraph Harness[Synthesis and evidence]
        Generate[Python/TypeScript harness]
        Oracle[typed oracle\nprovenance + confidence]
        Shrink[bounded minimization]
        Repro[structured repro + sentinel snippet]
        Generate --> Oracle --> Shrink --> Repro
    end
    Plan --> Generate
    ExecuteStage --> Generate
    Repro --> Verdict

    subgraph Runtime[Runtime profiles]
        Local[local-trusted\nhost subprocess]
        Docker[isolated\nDocker, no network, read-only mounts]
        Limits[timeout, memory, pids, file size]
        Local --> Limits
        Docker --> Limits
    end
    ExecuteCmd --> Runtime
    ExecuteStage --> Runtime
    TestStage --> Runtime
    Runtime --> Execution[ExecutionResult]
    Execution --> Verdict

    subgraph Differential[Optional differential verification]
        Base[complete read-only baseline tree]
        Candidate[candidate tree]
        Snapshot[normalized behavior snapshots]
        Advisory[advisory regression unless authoritative oracle]
        Base --> Snapshot
        Candidate --> Snapshot
        Snapshot --> Advisory
    end
    VerifyCmd --> Differential
    Advisory --> Verdict

    Verdict --> Report[report schema v3\nverdict, strength, stages, coverage, findings]
    Report --> Repair[repair-json summary]
    Report --> Replay[replay stored finding]
    DoctorCmd --> Readiness[doctor schema-v3 readiness report]

    subgraph Benchmark[Artifact-v1 benchmark evidence]
        Matrix[matrix.json / run.json / result.json]
        Summary[abstention-aware summary + paired stats]
        Bundle[portable evidence bundle\nmanifest + redaction]
        Shadow[opt-in shadow JSONL]
        Matrix --> Summary --> Bundle
        Matrix --> Shadow
    end
    Bench --> Matrix
    Report --> Matrix
```

## Boundary notes

- **CLI boundary:** `court-jester` is the product surface. It has no transport server or editor protocol; callers invoke subcommands and consume JSON or human output.
- **Verification contract:** reports use schema v3, typed `pass|fail|inconclusive` verdicts, typed evidence strength, stage statuses, typed findings/oracles, and coverage summaries. Consumers must use these fields rather than reconstructing a verdict from legacy booleans.
- **Coverage:** default `changed-exports` requires changed exported/invocable surfaces (or all exported/invocable surfaces without a diff). Factory/caller reach is distinct from behavioral checking. Tests-only coverage is proven by instrumentation events from the same authoritative test process.
- **Confidence:** source directives, fixtures, and authoritative tests can gate. Name/context inference is low-confidence and advisory by default. Differential changes are advisory without an authoritative oracle.
- **Runtime:** `local-trusted` preserves host execution semantics but is not a security boundary. `isolated` uses Docker resource and filesystem policy; Docker/image/daemon failures are inconclusive and never fall back to local execution.
- **Replay:** findings carry self-contained snippets and structured expectations. Persisted reports add a replay command; differential replay validates embedded source and dependency contracts.
- **Benchmark boundary:** the Python harness copies fixtures, runs providers/policies and public/hidden checks, and writes artifact-v1 matrix/run/result/summary/manifest files requiring verifier schema 3. Evidence bundles are relative, checksummed, and redaction-aware; missing/mixed versions abstain, gates use the shared summary, and local shadow JSONL never changes task success.

The GitHub remote currently retains its historical `court-jester-mcp` URL because the repository rename endpoint returned 404. This is an operational remote contingency, not the product or crate identity; historical benchmark notes may mention MCP isolation only when describing disabled third-party connector state.
