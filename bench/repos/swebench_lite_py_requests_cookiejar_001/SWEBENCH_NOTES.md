# SWE-bench Lite Pilot Notes

This fixture is a first external-style pilot task for the Court Jester benchmark harness.

It is intentionally shaped like a small vendored Python repository:

- package code under `mini_requests/`
- repo-local tests under `tests/`
- a task-local gold patch under `gold/`
- a small setup step suitable for workspace caching

This is a `requests`-style cookie-header behavior task, not an official upstream SWE-bench instance yet.

The point of this fixture is to validate:

- repo-local setup commands
- setup cache reuse
- task-level gold patch replay
- visible vs hidden judge commands on a repo-shaped Python task
