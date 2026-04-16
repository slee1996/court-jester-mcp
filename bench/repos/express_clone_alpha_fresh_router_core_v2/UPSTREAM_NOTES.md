Upstream source families for this slice

- Express router mounting and dispatch behavior
- `Route(...).all(...)` dispatch behavior
- Route param callback behavior

This fixture is intentionally narrower than the full monolith. The public spec covers the core visible routing contract plus simple `app.param(...)` callbacks, while verifier and hidden checks exercise deeper mount and multi-param semantics.
