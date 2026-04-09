# Court Jester Product Loop

This is the intended end-state product behavior for Court Jester as an in-loop breaker for code agents.

It is not a polite final checker. It is a hostile verifier that tries to break AI-generated code quickly, returns an actionable repro, and drives one or more repair turns before finalization.

## Core Loop

```text
agent receives coding task
        |
        v
agent writes or edits code
        |
        v
Court Jester verify runs immediately
        |
        +------------------------------+
        |                              |
   verify passes                  verify fails
        |                              |
        v                              v
repo/public checks                Court Jester returns:
        |                          - failing file
        |                          - minimal repro
        |                          - violated property
        |                          - observed bad output or crash
        |                              |
        |                              v
        |                        agent repairs code
        |                              |
        |                              v
        +----------------------<-------+
                   repeat until:
                   - verify passes
                   - attempt budget is exhausted
        |
        v
hidden evaluator / final acceptance
        |
        v
ship / commit / return result
```

## Compact Form

```text
agent patch
  -> Court Jester verify
     -> pass -> continue
     -> fail -> repro feedback -> agent repair -> verify -> ...
```

## Product Principles

- Court Jester should optimize for failure discovery, not style scoring.
- The main unit of value is a concrete repro that causes a good repair.
- The main loop is `verify -> repair -> reverify`.
- `required-final` style hard gating is a control, not the product center.
- The primary success metric is final task success after recovery.

## Failure Feedback Shape

Court Jester should return the minimum useful payload for repair:

- failing file or function
- one or a few concrete repro inputs
- observed output, crash, or violated property
- short evidence from the failing stage

It should avoid dumping the entire hidden suite back into the model.

## Intended Measurement

The headline benchmark question is:

> Did Court Jester increase final task success by breaking bad code early enough for the agent to repair it?

Supporting measurements:

- repair-loop success rate
- repair conversion rate after verify failure
- false-positive rate
- failure provenance: verify, public, hidden
- task success delta versus baseline

## Near-Term Evolution

Today the hidden evaluator is local and partially obscured.

The intended future shape is:

```text
agent -> Court Jester verify -> repair loop -> external hidden evaluator -> final result
```

That preserves the same product loop while moving final correctness measurement behind a real isolation boundary.
