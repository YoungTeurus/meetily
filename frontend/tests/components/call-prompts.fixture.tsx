import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import type { DetectionStatus } from '../../src/components/CallPrompts';
const originalCore = { ...await import('@tauri-apps/api/core') };
const originalEvent = { ...await import('@tauri-apps/api/event') };
const originalSonner = { ...await import('sonner') };
let status: DetectionStatus;
let rejectAction = false;
let notify: (event: { payload: DetectionStatus }) => void;
const invoke = mock(async (command: string, args?: Record<string, unknown>) => {
  if (command === 'get_detection_status') return status;
  if (command === 'detection_action') {
    if (rejectAction) throw new Error('Сессия уже завершена');
    status = { ...status, prompts: [] };
    return { session_id: args?.sessionId };
  }
  if (command === 'close_call_action') return;
  throw new Error(command);
});
mock.module('@tauri-apps/api/core', () => ({ ...originalCore, invoke }));
mock.module('@tauri-apps/api/event', () => ({ ...originalEvent, listen: async (_: string, callback: typeof notify) => { notify = callback; return () => {}; } }));
mock.module('sonner', () => ({ ...originalSonner, toast: { error: () => {} } }));
const { CallPrompts } = await import('../../src/components/CallPrompts');
let renderer: ReactTestRenderer | undefined;
beforeEach(() => {
  invoke.mockClear(); rejectAction = false;
  status = { settings: { enabled: true, applications: ['zoom'], debounce_ms: 3000, grace_ms: 20000, auto_stop: false, notification_mode: 'system' }, observations: [], sessions: [], prompts: [{ kind: 'start', session_id: 'session-1', application: 'zoom' }], platform: 'windows' };
});
afterEach(() => { if (renderer) act(() => renderer!.unmount()); renderer = undefined; });
afterAll(() => { mock.module('@tauri-apps/api/core', () => originalCore); mock.module('@tauri-apps/api/event', () => originalEvent); mock.module('sonner', () => originalSonner); });
const mount = async () => { await act(async () => { renderer = create(<CallPrompts compact />); }); };
const click = async (text: string) => { const button = renderer!.root.findAllByType('button').find(button => button.children.includes(text)); expect(button).toBeDefined(); await act(async () => { await button!.props.onClick(); }); };
const actions = () => invoke.mock.calls.filter(call => call[0] === 'detection_action');
describe('Call notification actions', () => {
  test('detecting a call and rendering compact actions never starts recording', async () => {
    await mount(); expect(actions()).toHaveLength(0);
    await act(async () => { notify({ payload: status }); });
    expect(actions()).toHaveLength(0);
    expect(JSON.stringify(renderer!.toJSON())).toContain('звук всего компьютера');
  });
  test('start uses the displayed session only after a click', async () => {
    await mount(); await click('Начать запись');
    expect(actions()).toEqual([['detection_action', { sessionId: 'session-1', action: 'start' }]]);
    expect(invoke.mock.calls.some(call => call[0] === 'close_call_action')).toBe(true);
  });
  test('skip suppresses the displayed session without a recording command', async () => {
    await mount(); await click('Пропустить');
    expect(actions()).toEqual([['detection_action', { sessionId: 'session-1', action: 'skip' }]]);
  });
  test('expired session errors stay visible and do not close the action window', async () => {
    rejectAction = true; await mount(); await click('Начать запись');
    expect(JSON.stringify(renderer!.toJSON())).toContain('Сессия уже завершена');
    expect(invoke.mock.calls.some(call => call[0] === 'close_call_action')).toBe(false);
  });
});
