import { afterEach, beforeEach, expect, mock, test } from 'bun:test';
import { useEffect } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
let query: (id: string) => Promise<unknown>;
const invoke = mock(async (_command: string, args: { meetingId: string }) => query(args.meetingId));
mock.module('@tauri-apps/api/core', () => ({ invoke }));
const { useAutomaticSummaryReadiness } = await import('../../src/hooks/useAutomaticSummaryReadiness');
let state: ReturnType<typeof useAutomaticSummaryReadiness>;
let refresh: () => Promise<void>;
let started: string[];
function View({ id = 'meeting-a', enabled = true }: { id?: string; enabled?: boolean }) {
  state = useAutomaticSummaryReadiness(id, enabled, refresh);
  useEffect(() => { if (state.ready) started.push(id); }, [state.ready, id]);
  return <div>{state.message}</div>;
}
let renderer: ReactTestRenderer | undefined;
const timers = new Map<number, () => void>(); let timerId = 0;
const originalSetTimeout = globalThis.setTimeout; const originalClearTimeout = globalThis.clearTimeout;
beforeEach(() => {
  timers.clear(); started = []; invoke.mockClear(); query = async () => ({ jobs: [] }); refresh = async () => {};
  globalThis.setTimeout = ((callback: () => void) => { const id = ++timerId; timers.set(id, callback); return id; }) as typeof setTimeout;
  globalThis.clearTimeout = ((id: number) => { timers.delete(id); }) as typeof clearTimeout;
});
afterEach(async () => { if (renderer) await act(async () => renderer!.unmount()); renderer = undefined; globalThis.setTimeout = originalSetTimeout; globalThis.clearTimeout = originalClearTimeout; });
async function show(id = 'meeting-a', enabled = true) { await act(async () => { const view = <View id={id} enabled={enabled} />; if (renderer) renderer.update(view); else renderer = create(view); }); }
async function tick() { await act(async () => { const pending = [...timers.values()]; timers.clear(); pending.forEach(callback => callback()); }); }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
const jobs = (state: string, meeting_id = 'meeting-a') => ({ jobs: [{ meeting_id, state }] });

test('initial pending status query blocks automatic generation', async () => {
  const pending = deferred<unknown>(); query = () => pending.promise;
  await show(); expect(started).toEqual([]); expect(state.ready).toBe(false);
  await act(async () => pending.resolve({ jobs: [] })); expect(started).toEqual(['meeting-a']);
});
test('queued and preempted running jobs wait, then completed refresh finishes before summary starts', async () => {
  let job = 'queued'; query = async () => jobs(job); const refreshed = deferred<void>(); refresh = () => refreshed.promise;
  await show(); expect(started).toEqual([]);
  job = 'running'; await tick(); expect(started).toEqual([]);
  job = 'queued'; await tick(); expect(started).toEqual([]);
  job = 'completed'; await tick(); expect(started).toEqual([]);
  await act(async () => refreshed.resolve()); expect(started).toEqual(['meeting-a']); expect(timers.size).toBe(0);
});
test('failed and cancelled jobs allow the retained original transcript with an explanatory status', async () => {
  query = async id => jobs(id === 'failed' ? 'failed' : 'cancelled', id);
  await show('failed'); expect(state.ready).toBe(true); expect(state.message).toContain('исходный транскрипт');
  await show('cancelled'); expect(state.ready).toBe(true); expect(started).toEqual(['failed', 'cancelled']);
});
test('status request error blocks automatic start without an error retry loop', async () => {
  query = async () => { throw new Error('database unavailable'); };
  await show(); expect(state.error).toBe(true); expect(state.ready).toBe(false); expect(timers.size).toBe(0);
  await tick(); expect(invoke).toHaveBeenCalledTimes(1); expect(started).toEqual([]);
  query = async () => ({ jobs: [] }); await act(async () => state.retry()); expect(started).toEqual(['meeting-a']);
});
test('refresh failure does not generate on potentially stale visible transcripts', async () => {
  query = async () => jobs('completed'); refresh = async () => { throw new Error('refresh failed'); };
  await show(); expect(state.ready).toBe(false); expect(state.error).toBe(true); expect(started).toEqual([]); expect(timers.size).toBe(0);
});
test('late old meeting result cannot release a newer meeting wait', async () => {
  const old = deferred<unknown>(); query = async id => id === 'old' ? old.promise : jobs('running', id);
  await show('old'); await show('new'); await act(async () => old.resolve({ jobs: [] }));
  expect(state.ready).toBe(false); expect(started).toEqual([]);
});
test('no history replay when automatic intent is absent or explicitly consumed by a manual action', async () => {
  query = async () => jobs('running');
  await show('meeting-a', false); expect(invoke).not.toHaveBeenCalled();
  await show(); expect(timers.size).toBe(1);
  await show('meeting-a', false); expect(timers.size).toBe(0);
  query = async () => jobs('completed'); await tick(); expect(started).toEqual([]); expect(state.ready).toBe(false);
});
