# Court Jester: Building and Benchmarking an Agent-Facing Code Verifier

## What Court Jester is

Court Jester is a verification tool built for AI coding agents.

The idea is simple:

1. An agent writes or edits code.
2. Court Jester checks that code before the agent declares success.
3. If Court Jester finds a concrete failure, the agent gets another chance to repair the code using that feedback.

It is not a general-purpose CI replacement, and it is not trying to be a human code reviewer. It is a fast, structured verifier that fits inside an agent loop.

In this repo, Court Jester is exposed as a local CLI. It provides four main commands:

1. `analyze`
2. `lint`
3. `execute`
4. `verify`

At a high level, `verify` is the important one. It combines parsing, linting, sandboxed execution, and optional tests into a single verdict that an agent can act on.

## The problem we are trying to solve

AI coding agents are good at producing plausible code quickly. They are much less reliable at knowing when they are actually done.

That gap shows up in a few common ways:

1. The code passes obvious happy-path checks but fails edge cases.
2. The code looks reasonable but crashes at runtime.
3. A cross-file fix is locally plausible but globally inconsistent.
4. The agent stops too early because nothing in its loop forced a concrete disproof.

If you want agents to produce stronger code, you need more than generation quality. You need a mechanism that can:

1. catch failures early
2. turn those failures into structured feedback
3. feed that feedback back into the agent
4. do all of this cheaply enough to be used in practice

That is the role Court Jester is trying to fill.

## Our approach to building it

We did not start by trying to build a perfect static analyzer. We started with the actual workflow problem: what kind of feedback helps an agent repair bad code?

That led to a few design choices.

### 1. Build for the agent loop, not for human inspection

A lot of developer tools are optimized for a human reading a long report. Agents need something different:

1. a small number of structured outcomes
2. concrete failing repros
3. deterministic machine-readable responses
4. fast enough runtime that the tool can sit inside an iterative repair loop

That is why Court Jester produces stage-by-stage results and a simple overall pass/fail shape instead of only free-form text.

### 2. Combine several weak signals instead of relying on one perfect check

Court Jester does not assume any single technique is enough.

`verify` combines:

1. parse checks
2. lint checks
3. sandboxed execution
4. generated or provided tests

This matters because agent bugs are diverse. Some are syntax errors. Some are runtime crashes. Some are semantic misses that only appear under a specific input. A useful verifier has to cover multiple failure modes.

### 3. Sandbox execution aggressively

Because the tool is intended to run agent-written code, isolation matters.

The execution path is built around:

1. subprocess isolation
2. resource limits
3. timeout enforcement
4. temp-file management

This was not just a theoretical concern. During stress testing we found real subprocess-isolation bugs, including child processes inheriting the wrong stdio handles and interfering with benchmark execution. Fixing those issues was necessary before benchmark results could be trusted.

### 4. Prefer concrete counterexamples over vague advice

One of the strongest lessons from benchmarking was that agents respond much better to a specific failing repro than to a generic statement like “the code is wrong.”

So we shaped repair feedback around:

1. the failing stage
2. the file involved
3. the exact failing assertion or repro when available

We also strengthened the repair prompt itself so models are explicitly told:

1. treat failing repros as authoritative
2. change behavior on those repros
3. do not claim the code is already correct if the cited repro still fails

That change materially improved repair behavior for at least one weaker model in our benchmarks.

## Our approach to benchmarking

We benchmark Court Jester as a product, not just as a codebase.

That means we ask three different questions.

### 1. Is the service operationally reliable?

Before you can trust any model comparison, the verifier itself has to be stable.

So we built a stress harness around the CLI and exercised:

1. mixed tool traffic
2. concurrent clients
3. timeout-heavy payloads
4. memory-pressure payloads
5. long-running soak scenarios

This found real bugs:

1. subprocess stdin inheritance that could collapse verifier runs
2. temp/sibling path resolution issues in verification
3. lifecycle problems in the client harness

We fixed those before trusting benchmark outcomes.

### 2. Does Court Jester catch the kinds of failures agents actually make?

A verifier can be reliable and still not be useful.

So we benchmark against task fixtures designed to capture realistic agent failure modes:

1. hidden semantic misses
2. cross-file contract mistakes
3. import/path problems
4. runtime edge cases
5. public-pass hidden-fail situations

Each task includes:

1. a repo fixture
2. a prompt
3. public checks
4. hidden checks
5. Court Jester verify targets

This lets us distinguish between:

1. code that is obviously broken
2. code that looks fine but fails hidden semantics
3. code that Court Jester can disprove before hidden evaluation

### 3. Does the agent loop actually improve with Court Jester?

This is the real product question.

We compare at least two policies:

1. `baseline`
2. `repair-loop`

`baseline` means the model gets one shot.

`repair-loop` means the model writes code, Court Jester verifies it, and if Court Jester finds a concrete failure, the model gets another attempt with structured feedback.

The main metric is not “did verify fail?” The main metric is:

1. did the final task success rate improve?

In other words: does Court Jester help the agent ship stronger code than it would have shipped alone?

## How to read the benchmark results

We look at benchmark output in layers.

### First: raw task success

For each model and policy:

1. how many tasks succeeded?
2. how many failed?

This gives the basic product view.

### Second: failure category

We classify failures so we do not confuse different problems.

Examples:

1. `hidden_semantic_miss`
2. `public_check_failure`
3. `verify_caught_public_bug`
4. `provider_infra_busy`
5. `provider_error`

This matters because not every failure says the same thing.

A hidden semantic miss means the model produced code that looked plausible but was still wrong.

A provider infrastructure failure means the benchmark datapoint is noisy and should not be treated as evidence about code quality.

### Third: policy delta

The key comparison is:

1. baseline pass rate
2. repair-loop pass rate

If repair-loop wins, Court Jester is adding value.

If repair-loop is flat, Court Jester may still be operationally useful, but we have not yet shown product lift.

If repair-loop loses, then either:

1. the verifier feedback is bad
2. the prompt integration is bad
3. the verifier is catching the wrong things
4. the model cannot use the feedback effectively

## What we have learned so far

A few lessons are already clear.

### 1. Infrastructure quality matters more than you think

Early on, benchmark results were being polluted by harness and verifier bugs:

1. hidden evaluator path bugs
2. subprocess stdio interference
3. provider transport failures
4. CLI timeout artifacts

If we had not cleaned those up, we would have been attributing infrastructure noise to model quality.

### 2. Counterexample-driven repair feedback is materially better

When the verifier only said “this failed,” weaker models often rationalized that their code was fine.

When the verifier gave an explicit repro like:

1. `primary_plan_code({"plans": ["   ", " team "]}) == "TEAM"`

and the prompt forced the model to address that repro, repair performance improved.

### 3. Utility is model-dependent

The same verifier can help one model more than another.

In our current benchmarking, stronger models tend to need less help, but can still benefit on hidden semantic cases. Weaker or smaller models can benefit more, but only if the feedback is concrete enough and the repair loop is framed correctly.

### 4. “Noisy provider failure” and “bad code” must be separated

This sounds obvious, but it is easy to get wrong in practice.

If a hosted model returns `503 busy`, that is not evidence that Court Jester failed or that the model wrote bad code. It is infrastructure noise and should be labeled as such.

## Why this matters to an average engineer

The practical question is not “is this academically interesting?”

The practical question is:

1. if I put this in front of an AI coding agent, will I get fewer bad merges?

That is the bar.

Court Jester is valuable if it helps agents:

1. catch mistakes before they are declared done
2. recover from concrete failures with a repair loop
3. produce stronger final code than baseline generation alone

If it does that reliably enough and cheaply enough, it becomes a real product component rather than a demo.

## The standard we are holding it to

By the end of benchmarking, we want to be able to say one of two things clearly:

1. Court Jester improves final code quality in agent workflows
2. It does not, and here is exactly where the loop breaks down

That is the right standard.

We are not trying to prove that verification is good in theory. We are trying to prove or disprove that this verifier improves real agent outcomes on realistic tasks.

If it does, it is worth using.

If it does not, we should know why.
