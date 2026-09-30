import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
const originalCore = { ...await import('@tauri-apps/api/core') };
const originalEvents = { ...await import('@tauri-apps/api/event') };
const originalSonner = { ...await import('sonner') };
let registrationDelay: Promise<void> | null = null;
const handlers = new Map<string, (event: { payload: unknown }) => void>();
let session: () => Promise<unknown> = async () => ({ recording: null });
mock.module('@tauri-apps/api/core', () => ({ ...originalCore, invoke: async (command: string) => command === 'get_recording_session' ? session() : ({ is_recording: false, is_paused: false, is_active: false, recording_duration: null, active_duration: null }) }));
mock.module('@tauri-apps/api/event', () => ({ ...originalEvents, listen: async (event: string, callback: (event: { payload: unknown }) => void) => { if (registrationDelay) await registrationDelay; handlers.set(event, callback); return () => { if (handlers.get(event) === callback) handlers.delete(event); }; } }));
mock.module('sonner', () => ({ ...originalSonner, toast: { error: () => {}, info: () => {} } }));
const { RecordingStateProvider, useRecordingState, RecordingStatus } = await import('../../src/contexts/RecordingStateContext');
let state: ReturnType<typeof useRecordingState>;
function Consumer() { state = useRecordingState(); return <div />; }
let renderer: ReactTestRenderer | undefined;
beforeEach(() => { handlers.clear(); registrationDelay = null; session = async () => ({ recording: null }); });
afterEach(() => { if (renderer) act(() => renderer!.unmount()); renderer = undefined; });
afterAll(() => { mock.module('@tauri-apps/api/core', () => originalCore); mock.module('@tauri-apps/api/event', () => originalEvents); mock.module('sonner', () => originalSonner); });
const mount = async () => { await act(async () => { renderer = create(<RecordingStateProvider><Consumer /></RecordingStateProvider>); }); };
const event = async (name: string, payload: unknown = {}) => { await act(async () => { handlers.get(name)!({ payload }); }); };
describe('Global native completion state', () => {
  test('external stop leaves STOPPING once native meeting is durable', async () => {
    await mount(); await event('recording-started'); await event('recording:started', { recording_id: 'recording-1', meeting_id: 'meeting-1' });
    expect(state.meetingId).toBe('meeting-1');
    await event('recording-stopped', { message: 'Capture stopped' }); expect(state.status).toBe(RecordingStatus.STOPPING);
    await event('meeting:finalized', { recording_id: 'recording-1', meeting_id: 'meeting-1' });
    expect(state.status).toBe(RecordingStatus.IDLE); expect(state.isRecording).toBe(false); expect(state.meetingId).toBe('meeting-1');
    await event('recording-stopped', { message: 'Recording saved', recording_id: 'recording-1', state: 'finalized' });
    expect(state.status).toBe(RecordingStatus.IDLE);
  });
  test('a delayed old completion cannot stop a newer recording', async () => {
    await mount(); await event('recording-started'); await event('recording:started', { recording_id: 'recording-new' });
    await event('meeting:finalized', { recording_id: 'recording-old', meeting_id: 'meeting-old' });
    await event('recording-stopped', { message: 'Recording saved', recording_id: 'recording-old', state: 'finalized' });
    expect(state.isRecording).toBe(true); expect(state.status).toBe(RecordingStatus.RECORDING);
  });
});

 test('durable meeting identity hydrates when the recording page reopens', async () => {
  session = async () => ({ recording: { recording_id: 'native-recording', meeting_id: 'durable-meeting' } });
  await mount(); expect(state.meetingId).toBe('durable-meeting');
});
 test('stale hydration cannot replace the identity from a newer recording event', async () => {
  let resolve!: (value: unknown) => void;
  session = () => new Promise(done => { resolve = done; });
  await mount(); await event('recording:started', { recording_id: 'new-native', meeting_id: 'new-meeting' });
  await act(async () => resolve({ recording: { recording_id: 'old-native', meeting_id: 'old-meeting' } }));
  expect(state.meetingId).toBe('new-meeting');
});

test('identity listener registration completing after unmount is released', async () => {
  let finish!: () => void;
  registrationDelay = new Promise<void>(resolve => { finish = resolve; });
  await mount();
  await act(async () => renderer!.unmount()); renderer = undefined;
  await act(async () => finish());
  expect(handlers.size).toBe(0);
});
