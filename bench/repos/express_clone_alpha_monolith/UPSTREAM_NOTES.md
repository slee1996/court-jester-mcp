This fixture is an Express-clone alpha built to benchmark Court Jester on upstream-derived framework behavior.

Primary upstream sources:

- `expressjs/express/test/Router.js`
- `expressjs/express/test/app.use.js`
- `expressjs/express/test/app.param.js`
- `expressjs/express/test/app.routes.error.js`
- `expressjs/express/test/req.baseUrl.js`
- `expressjs/express/test/req.get.js`
- `expressjs/express/test/req.protocol.js`
- `expressjs/express/test/req.secure.js`
- `expressjs/express/test/req.xhr.js`
- `expressjs/express/test/req.query.js`
- `expressjs/express/test/express.json.js`
- `expressjs/express/test/express.raw.js`
- `expressjs/express/test/express.text.js`
- `expressjs/express/test/express.urlencoded.js`
- `expressjs/express/test/res.format.js`
- `expressjs/express/test/res.links.js`
- `expressjs/express/test/res.location.js`
- `expressjs/express/test/res.redirect.js`
- `expressjs/express/test/res.sendStatus.js`
- `expressjs/express/test/res.vary.js`

This is not a verbatim copy of Express. It is a compact compatibility fixture intended for:

- public-test repair loops
- Court Jester verify-guided repair loops
- known-good control runs on a framework-shaped repo
