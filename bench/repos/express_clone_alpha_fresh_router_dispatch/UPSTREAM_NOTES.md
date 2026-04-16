Upstream source families for this slice

- Express router mounting and dispatch behavior
- `Route(...).all(...)` dispatch behavior

This fixture is intentionally narrower than the full monolith. The public spec covers mounted child-app dispatch plus standalone route dispatch, while verifier and hidden checks exercise deeper mounted-path semantics.
