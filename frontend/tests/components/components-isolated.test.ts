import { expect, test } from 'bun:test';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Bun module mocks mutate a process-wide cache. Existing hook suites replace
// entire context modules and leave partial exports behind. Run each mocked UI
// fixture in a fresh Bun process so it tests the real component and cannot inherit
// or leak another suite's mocks. Every fixture still runs all behavioral tests.
const fixtures = [
  ['call-prompts', 4],
  ['recording-finalized', 3],
  ['recording-state-finalized', 2],
  ['onboarding-database-choice', 4],
] as const;

for (const [name, assertions] of fixtures) {
  test(`${name}: all ${assertions} behavioral cases in an isolated module cache`, () => {
    const path = fileURLToPath(new URL(`./${name}.fixture.tsx`, import.meta.url));
    const result = spawnSync(process.execPath, ['test', path], {
      cwd: fileURLToPath(new URL('../../', import.meta.url)),
      encoding: 'utf8', timeout: 15_000,
    });
    const output = `${result.stdout ?? ''}${result.stderr ?? ''}`;
    // Keep the individual case results visible in the standard full-suite log.
    process.stderr.write(output);
    expect(result.error, output).toBeUndefined();
    expect(result.status, output).toBe(0);
    expect(output, 'The child must execute every behavioral case').toContain(`${assertions} pass`);
    expect(output).toContain('0 fail');
  }, 20_000);
}
