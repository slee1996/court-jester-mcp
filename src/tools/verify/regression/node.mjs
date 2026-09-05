// CLI-backed regression; absence of the original failure alone is not success.
import assert from 'node:assert/strict';
import { readFileSync, realpathSync, statSync } from 'node:fs';
import { dirname, join, relative, isAbsolute, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

test('Court Jester recorded check', () => {
  const bundle = dirname(realpathSync(fileURLToPath(import.meta.url)));
  const manifest = JSON.parse(readFileSync(join(bundle, 'regression.json'), 'utf8'));
  assert.equal(manifest.artifact_schema_version, 1);
  assert.equal(manifest.artifact_type, 'court_jester_regression');
  let root = bundle;
  for (let i = 0; i < manifest.project_levels; i++) root = dirname(root);
  const source = realpathSync(join(root, manifest.source_file));
  const within = relative(root, source);
  assert.ok(statSync(source).isFile() && within.split(sep)[0] !== '..' && !isAbsolute(within), 'current regression source is unavailable');
  const result = spawnSync(process.env.COURT_JESTER_BINARY || 'court-jester', [
    'replay', '--report', join(bundle, 'report.json'), '--finding', manifest.finding_id,
    '--dependency-project-dir', root,
  ], { cwd: root, encoding: 'utf8' });
  assert.ifError(result.error);
  assert.equal(result.status, 1, String(result.stdout) + String(result.stderr));
  const replay = JSON.parse(result.stdout);
  assert.equal(replay.schema_version, 3);
  assert.equal(replay.finding_id, manifest.finding_id);
  assert.equal(replay.outcome, 'not_reproduced');
  assert.equal(replay.check_passed, true, 'recorded check did not pass: ' + result.stdout);
});
