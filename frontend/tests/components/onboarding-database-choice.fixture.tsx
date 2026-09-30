import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
const originalCore = { ...await import('@tauri-apps/api/core') };
const originalOs = { ...await import('@tauri-apps/plugin-os') };
const originalOnboarding = { ...await import('../../src/contexts/OnboardingContext') };
let picked: string | null = null;
const goToStep = mock((_step: number) => {});
const setDatabaseExists = mock((_value: boolean) => {});
const invoke = mock(async (command: string, _args?: Record<string, unknown>) => {
  if (command === 'check_first_launch') return true;
  if (command === 'select_legacy_database_path') return picked;
  if (command === 'initialize_fresh_database' || command === 'import_and_initialize_database') return;
  throw new Error(command);
});
mock.module('@tauri-apps/api/core', () => ({ ...originalCore, invoke }));
mock.module('@tauri-apps/plugin-os', () => ({ ...originalOs, platform: () => 'windows' }));
mock.module('../../src/contexts/OnboardingContext', () => ({ ...originalOnboarding, useOnboarding: () => ({ databaseExists: false, setDatabaseExists, goToStep }) }));
const { SetupOverviewStep } = await import('../../src/components/onboarding/steps/SetupOverviewStep');
let renderer: ReactTestRenderer | undefined;
beforeEach(() => { invoke.mockClear(); goToStep.mockClear(); setDatabaseExists.mockClear(); picked = null; });
afterEach(() => { if (renderer) act(() => renderer!.unmount()); renderer = undefined; });
afterAll(() => { mock.module('@tauri-apps/api/core', () => originalCore); mock.module('@tauri-apps/plugin-os', () => originalOs); mock.module('../../src/contexts/OnboardingContext', () => originalOnboarding); });
const mount = async () => { await act(async () => { renderer = create(<SetupOverviewStep />); }); };
const click = async (text: string) => { const button = renderer!.root.findAllByType('button').find(button => button.children.includes(text)); expect(button).toBeDefined(); await act(async () => { await button!.props.onClick(); }); };
const writes = () => invoke.mock.calls.filter(call => call[0] !== 'check_first_launch');
describe('Explicit first-launch database choice', () => {
  test('mount only checks first launch and never scans/imports/initializes another database', async () => {
    await mount(); expect(writes()).toEqual([]); expect(goToStep).not.toHaveBeenCalled();
  });
  test('fresh initialization happens only after its button', async () => {
    await mount(); await click('Создать новое хранилище');
    expect(writes()).toEqual([['initialize_fresh_database']]); expect(goToStep).toHaveBeenCalledWith(3);
  });
  test('canceled import does not initialize a fresh database or advance', async () => {
    await mount(); await click('Импортировать базу Meetily');
    expect(writes()).toEqual([['select_legacy_database_path']]); expect(goToStep).not.toHaveBeenCalled();
  });
  test('import uses only the explicitly selected source', async () => {
    picked = '/selected/meetily.sqlite'; await mount(); await click('Импортировать базу Meetily');
    expect(writes()).toEqual([['select_legacy_database_path'], ['import_and_initialize_database', { legacyDbPath: '/selected/meetily.sqlite' }]]);
    expect(setDatabaseExists).toHaveBeenCalledWith(true); expect(goToStep).toHaveBeenCalledWith(3);
  });
});
