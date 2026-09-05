# Design-partner pilot — draft, not launched

Owner: repository maintainer. This protocol prepares P9 in the [product plan](product-plan.md); it does not supply participant evidence. No invitations have been sent, participants enrolled, or outcomes measured by creating these documents.

## Decision the pilot should inform

Can a developer use Court Jester on a real Python or TypeScript change, understand its evidence, make a useful repair, and choose to use it again without excessive setup or interruption?

This is a small formative pilot, not a causal comparison with an unaided workflow. Do not describe observed repairs as benchmark lift or estimate time saved from a participant's impression. Keep the separate [private-beta release requirements](release-readiness-private-beta.md), including utility benchmarks and 5–10 users, intact.

## Authorization before launch

The maintainer must approve the participant list, invitation text, support owner, distribution commit/build, session budget, and a storage location with an agreed deletion date. Participants must have permission to run the tool on their chosen code. Confirm current-build gates and known limitations before selecting that build; an unpublished local commit is not a validated downloadable release.

No recruitment, paid services, incentive promises, public quotes, recording, telemetry collection, or code upload is authorized by this plan. Ask separately if any becomes necessary. The GitHub account/push issue must be resolved before offering the current unpublished build through GitHub.

Use the [participant guide](pilot/participant-guide.md) and a fresh private copy of the [session record](pilot/session-record.md). Do not commit filled records, contact details, reports, or private source code to this repository. A report can contain source, paths, arguments, and dependency details; it is not anonymous merely because names were removed.

## Proposed sequence

1. Recruit only after maintainer approval. Seek both Python and TypeScript projects and include setup failures in the record, rather than replacing them with successful sessions.
2. Before each session, agree on the task, execution profile, shared information, time cap, and deletion date. Record consent in the private session record. Observation/notes and raw artifact sharing are separate choices.
3. Observe setup without intervention first. Mark every intervention and its time; assisted success is not unassisted onboarding success. Stop or help when the participant asks—this is not a test of the participant.
4. Follow one real change through configuration, doctor, verify, evidence interpretation, and, if warranted, repair/replay/export. A clean result or inconclusive result is a valid session outcome. Do not seed a bug into the participant's code to manufacture a success.
5. Have the participant independently judge the finding against the intended behavior and their project checks. Record disagreements. A fail verdict, reproduced exception, or passing exported test alone does not establish a useful repair.
6. Ask permission for one follow-up after an agreed interval. Record whether they actually used the tool again, merely intend to, declined, or did not respond. No response is unknown, not churn or retention.
7. Review all sessions, including abandonments and setup failures, before deciding on another cohort. Keep any public summary aggregate and participant-approved where identifiable details or quotations are involved.

Suggested discussion prompts: What did you expect this command to tell you? Which part of the report changed your next action? What would make you ignore this finding? What did you need help with? When would you next choose to run it? Avoid prompting the participant to agree that the tool was useful.

## Measurement definitions

| Measure | Required record and denominator |
| --- | --- |
| Enrollment | Invited, opted in, declined, and unknown counts, kept distinct. No conversion percentage without an invitation denominator. |
| Setup completion | All consented attempts, split into unassisted, assisted, blocked, abandoned, and unknown. Record elapsed setup time and participant active effort separately. |
| Finding usefulness | Independently reviewed findings classified as actionable defect, expected behavior/false positive, uncertain contract, or infrastructure/tooling issue. Unreviewed findings remain unknown. |
| Repair conversion | Independently confirmed actionable defects with an attempted repair. Report attempted, accepted, rejected, and unknown repairs separately; preserve the total actionable-defect count so non-attempts are visible. |
| Repair acceptance | Participant confirms intended behavior, relevant project checks pass, original reproduces, and repaired replay has positive check evidence. Record missing checks individually; do not convert missing evidence into acceptance. |
| Regression adoption | Eligible export attempts, successful exports, and tests actually retained in the project. Export success is not adoption. Unsupported export kinds remain a separate count. |
| Friction | Setup blockers, interpretation errors, unnecessary repair attempts, abandoned commands, support interventions, and time spent resolving each. Do not count infrastructure abstention as a false-positive defect. |
| Repeat use | Participants who agreed to follow-up, reached participants, observed/reported repeat use, non-use, and unknown. State the observation interval. Intention is not usage. |

Keep each session's language, runtime profile, OS, exact binary digest/version, project task class, and support interventions. Do not pool incompatible builds without showing the breakdown. With a small cohort, report raw counts and individual ranges rather than treating unstable percentile estimates as population facts. Time and cost observations are descriptive; this pilot alone cannot establish causal savings.

## Stop and escalation rules

Stop execution on unexpected source changes, exposure of sensitive information, unexpected network behavior, or a request to stop. Preserve only the minimum information the participant permits; do not upload a reproducer automatically. Stop a session at its agreed time cap unless the participant explicitly chooses to extend it.

Pause further rollout if a reproduced tooling defect invalidates findings/replay, onboarding repeatedly requires undocumented intervention, or the support owner cannot handle issues. Record the cause and required verification before resuming. A single attractive bug catch does not override these failures.

## Exit review

Produce a dated internal review with the build identities, complete session denominators, setup outcomes, independently adjudicated repair examples, disagreement/abstention causes, repeat-use evidence, unresolved issues, and data-deletion status. Link authorized evidence from private storage rather than copying it into a public report.

The decision is one of: continue a bounded pilot, repair identified blockers and repeat, or stop. Broad availability still requires the independent release gates. P9 is not complete until actual consented sessions and follow-up outcomes support a review; this protocol and empty templates are preparation only.
