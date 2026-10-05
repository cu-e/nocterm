// Exercise the real release-please TOML updater against the configured selector.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {GenericToml} = require('release-please/build/src/updaters/generic-toml');
const {Version} = require('release-please/build/src/version');
const YAML = require('yaml');

const root = path.resolve(__dirname, '..');
const read = file => fs.readFileSync(path.join(root, file), 'utf8');
const config = JSON.parse(read('release-please-config.json')).packages['.'];
const updater = file => new GenericToml(
  config['extra-files'].find(entry => entry.path === file).jsonpath,
  Version.parse('9.8.7'),
);
const lock = read('Cargo.lock');
const manifest = read('Cargo.toml');
const version = manifest.match(/^version = "([^"]+)"/m)[1];
assert.equal(lock.match(/\[\[package\]\]\nname = "nocterm"\nversion = "([^"]+)"/)[1], version);
assert.equal(read('version.txt').trim(), version);
const updatedLock = updater('Cargo.lock').updateContent(lock);
assert.equal(updatedLock, lock.replace(
  `name = "nocterm"\nversion = "${version}"`, 'name = "nocterm"\nversion = "9.8.7"',
), 'only the released package may change in the lockfile');
assert.equal(updater('Cargo.toml').updateContent(manifest), manifest.replace(
  `version = "${version}"`, 'version = "9.8.7"',
));
const fixture = '[[package]]\nname = "nocterm-ai"\nversion = "0.0.0"\n\n'
  + '[[package]]\nname = "nocterm"\nversion = "1.0.0"\n\n'
  + '[[package]]\nname = "dependency"\nversion = "1.0.0"\n';
assert.equal(updater('Cargo.lock').updateContent(fixture), fixture.replace(
  'name = "nocterm"\nversion = "1.0.0"', 'name = "nocterm"\nversion = "9.8.7"',
));
assert.equal(config['release-type'], 'simple');

const workflow = name => YAML.parse(read(`.github/workflows/${name}.yml`));
const release = workflow('release');
const ci = workflow('ci');
const pkg = workflow('package');
const prepared = '${{ needs.release-please.outputs.prepared_sha }}';
assert.equal(release.concurrency['cancel-in-progress'], false);
assert.equal(release.jobs['release-ci'].with.ref, prepared);
assert.equal(release.jobs['release-package-validation'].with.ref, prepared);
assert.equal(release.jobs['release-package-validation'].with.tag, undefined);
assert.equal(release.jobs.packages.with.tag, '${{ needs.release-please.outputs.tag_name }}');
assert.equal(pkg.jobs.upload.if, "inputs.tag != ''");
assert.equal(pkg.jobs.upload.steps[0].with.pattern, '*-${{ inputs.tag }}');
for (const name of ['linux', 'windows']) {
  const artifact = pkg.jobs[name].steps.find(step => step.uses?.startsWith('actions/upload-artifact@'));
  assert.match(artifact.with.name, /inputs\.ref \|\| inputs\.tag \|\| github\.sha/);
}
for (const job of Object.values(pkg.jobs)) {
  for (const step of job.steps ?? []) {
    if (step.uses?.startsWith('actions/checkout@')) {
      assert.equal(step.with.ref, '${{ inputs.ref || inputs.tag || github.sha }}');
    }
  }
}
for (const job of Object.values(ci.jobs)) {
  for (const step of job.steps ?? []) {
    if (step.uses?.startsWith('actions/checkout@')) {
      assert.equal(step.with.ref, '${{ inputs.ref || github.ref }}');
    }
  }
}
assert.deepEqual(release.jobs['release-validation'].needs,
  ['release-please', 'release-ci', 'release-package-validation']);
assert.match(release.jobs['release-validation'].if, /^always\(\)/);
// Execute the actual status script for every reusable-workflow result pair.
const script = release.jobs['release-validation'].steps[0].with.script;
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
(async () => {
  const pending = [];
  const pendingScript = release.jobs['release-please'].steps.find(
    step => step.name === 'Mark release validation pending',
  ).with.script;
  await new AsyncFunction('github', 'context', 'process', pendingScript)(
    {rest: {repos: {createCommitStatus: async status => pending.push(status)}}},
    {repo: {owner: 'test', repo: 'release'}, serverUrl: 'https://github.com', runId: 1},
    {env: {PREPARED_SHA: 'prepared-commit'}},
  );
  assert.deepEqual(pending.map(status => [status.context, status.state, status.sha]), [
    ['Release validation', 'pending', 'prepared-commit'],
    ['Conventional Commits', 'pending', 'prepared-commit'],
  ]);
  for (const ciResult of ['success', 'failure', 'cancelled', 'skipped']) {
    for (const packageResult of ['success', 'failure', 'cancelled', 'skipped']) {
      const statuses = [];
      const failures = [];
      const github = {rest: {repos: {createCommitStatus: async status => statuses.push(status)}}};
      const context = {repo: {owner: 'test', repo: 'release'}, serverUrl: 'https://github.com', runId: 1};
      await new AsyncFunction('github', 'context', 'core', 'process', script)(
        github, context, {setFailed: error => failures.push(error)},
        {env: {PREPARED_SHA: 'prepared-commit', CI_RESULT: ciResult, PACKAGE_RESULT: packageResult}},
      );
      const passed = ciResult === 'success' && packageResult === 'success';
      assert.equal(statuses.length, 2);
      assert.equal(statuses[0].context, 'Conventional Commits');
      assert.equal(statuses[0].sha, 'prepared-commit');
      assert.equal(statuses[0].state, ciResult === 'success' ? 'success' : 'failure');
      assert.equal(statuses[1].context, 'Release validation');
      assert.equal(statuses[1].sha, 'prepared-commit');
      assert.equal(statuses[1].state, passed ? 'success' : 'failure');
      assert.equal(failures.length, passed ? 0 : 1);
    }
  }
  console.log('Release updater and validation regressions passed.');
})().catch(error => { console.error(error); process.exitCode = 1; });
