Upstream source families for this slice

- Express `res.location`
- Express `res.links`
- Express `res.vary`
- Express `res.sendStatus`

The public spec covers the obvious response-header helpers plus the key `sendStatus(204)` empty-body rule. Verifier and hidden checks push on `back` redirects and `vary` canonicalization.
