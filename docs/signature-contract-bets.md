# Signature Contract Bets

This note tracks the next five product bets for Court Jester's signature/context-first verification path. These runs intentionally do not pass `--test-file`; benchmark manifests are used only after the run to score whether CJ caught a known-bug fixture or passed a known-good fixture.

## Bet 1: Semver Contract Pack

**Claim.** Function signatures and semver-shaped names/types are enough to generate hard checks for prerelease ordering, caret ranges, build metadata normalization, and max-stable selection.

**Result.** Strong. The slice produced 4 true positives and 2 true negatives with no false positives or misses.

**Ledger.** `bench/results/autoresearch/signature-contracts/bet-semver/run-1777083468369702000/ledger.json`

## Bet 2: Query/String Serialization Pack

**Claim.** Mapping-to-string signatures plus query/stringify names are enough to find nullish leaks, blank value handling bugs, and canonicalization failures.

**Result.** Strong but incomplete. The slice produced 5 true positives and 2 true negatives, with 1 miss on `ts-qs-stringify-deep-collections`.

**Next implementation move.** Expand the serializer pack from nullish/blank/canonical pairs into nested collection shape checks: arrays, objects, bracket notation, repeated keys, and deterministic ordering.

**Ledger.** `bench/results/autoresearch/signature-contracts/bet-query-serialization/run-1777083475399553000/ledger.json`

## Bet 3: Structured Fallbacks

**Claim.** Object-shaped inputs plus return-string selector names are enough to catch missing fallback behavior without tests.

**Result.** Promising but currently under-scored. All 8 selected fallback tasks failed under signature-only CJ via typed-input crashes, but their manifests do not declare `expected_verify_outcome`, so the loop records them as unscored.

**Next implementation move.** Promote these task manifests into scored signature-recall fixtures and split typed crashes from semantic blank/fallback property failures.

**Ledger.** `bench/results/autoresearch/signature-contracts/bet-structured-fallbacks/run-1777083482050773000/ledger.json`

## Bet 4: Collection Transform Contracts

**Claim.** Array/object transform signatures are enough to infer slice-like bounds, no mutation, length monotonicity, and key/index preservation.

**Result.** Mixed. The slice produced 2 true positives and 2 true negatives, but missed the array slice bug in both the mutation and original task.

**Next implementation move.** Add slice-specific contract generation for array/string signatures: start/end edge cases, negative indices if supported by the local API, stable order, and length not exceeding the selected window.

**Ledger.** `bench/results/autoresearch/signature-contracts/bet-collection-transforms/run-1777083491373106000/ledger.json`

## Bet 5: Express/Framework Contract Pack

**Claim.** Framework-shaped modules can be verified from public APIs and known framework semantics without tests.

**Result.** Gap. CJ passed the known-good Express fixture and avoided false positives, but mostly did not infer framework semantics. The only scored bug in this slice was missed.

**Next implementation move.** Build an Express-specific pack around route matching, param extraction, `req.query`, `res.location`, `res.sendStatus`, body parsing, and wrapper behavior. This is likely the highest-upside new product surface, but it requires domain contracts rather than generic signature inference.

**Ledger.** `bench/results/autoresearch/signature-contracts/bet-express-framework/run-1777083498338043000/ledger.json`

## Ranking

1. Semver pack: already working; expand cautiously.
2. Query/string serialization pack: working with a clear nested-collection miss.
3. Collection transforms: mixed but tractable with slice-specific contracts.
4. Structured fallbacks: needs benchmark scoring cleanup before product conclusions.
5. Express/framework pack: biggest gap and biggest upside, but requires new domain modeling.
