# Court Jester pilot participant guide

Draft for maintainer review. This is not an invitation already sent or a consent record. The maintainer must fill in the build, support contact, session time cap, storage location, and deletion date before use.

## What we want to learn

Court Jester looks for counterexamples in changed Python and TypeScript code. We want to learn whether its setup, reports, and repair workflow help with a real change you choose. A pass is scoped evidence, not permission to ship. A fail can need interpretation, and an inconclusive result means evidence is missing. Your existing review and project tests remain necessary.

Participation is optional. You may skip a step, decline to share an artifact, or stop at any time. There is no requirement to find a bug, make a repair, keep the tool, or provide positive feedback. Do not use code you are not authorized to execute or share.

## Agree before starting

- Approved build and its checksum: **to be supplied**.
- Maintainer/support contact and agreed time cap: **to be supplied**.
- Are observation and written notes permitted? **participant choice**.
- Which, if any, redacted reports or examples may be shared? **separate participant choice**.
- Private storage location, access, deletion date, and withdrawal contact: **to be agreed**.
- Is one follow-up permitted, and when? **participant choice**.

No recording, automatic uploads, or publication is part of this protocol. Raw reports may contain code, filesystem paths, inputs, and dependency information. Review any artifact before choosing to share it. The maintainer must obtain separate permission for public quotations or examples.

## Session workflow

Use a disposable branch or worktree for a change you understand. Review the working-tree status before and after the session. Select a source and an existing test that actually imports it. The commands below use placeholders; substitute your own approved paths and language.

```sh
court-jester --version
court-jester doctor --file <source> --language <python-or-typescript> --show-config
court-jester doctor --file <source> --language <python-or-typescript> --test-file <test>
```

These doctor commands do not import your target or run its tests. Before the next commands, choose the execution profile. The default `local-trusted` executes project code on your machine; it is not a security boundary. `isolated` uses the existing Docker runner with no network and read-only project mounts. It needs installed images and compatible dependencies and never silently falls back to local execution. Add `--runtime-profile isolated` consistently if selected. See [configuration and readiness](../repository-config.md#opt-in-entrypoint-readiness).

Only after agreeing to execute the selected code:

```sh
court-jester doctor --file <source> --language <python-or-typescript> --test-file <test> --probe-entrypoint
court-jester verify --file <source> --language <python-or-typescript> --test-file <test> --summary repair-json
```

For TypeScript, select the appropriate documented test runner; do not replace the project's test framework simply to obtain a passing probe. If configuration is wrong or an import is blocked, stop to inspect it. Do not automatically raise budgets, install packages, or rewrite code in response to inconclusive output.

If there is a plausible defect, save the report privately and inspect its input and oracle. Reproduce it, decide what the intended behavior should be, repair the code if appropriate, and rerun both replay and relevant project tests. A non-reproduced finding alone is not proof of a good repair: replay must also show `check_passed: true` for the recorded check. Unsupported replay/export is a limitation to record, not a reason to force success.

```sh
court-jester replay --report <saved-report.json> --finding <finding-id> --dependency-project-dir <project-root>
```

Only if you want to retain a supported regression, add `--export-regression <new-directory>`. Review the generated files before adding them to your project. Inferred expectations require an explicit acceptance decision; do not accept them just to get past the tool. Never share the saved report automatically.

## Finish

Tell us what helped, what was confusing, what interrupted your work, and what you would change. A decision not to use Court Jester again is useful feedback. Confirm which artifacts, if any, may be retained and until when. Your session is not complete until you have had the chance to stop sharing and ask questions.
