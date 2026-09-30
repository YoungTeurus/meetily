import { afterAll, afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
const originalEvents = { ...await import('@tauri-apps/api/event') };
const originalStop = { ...await import('../../src/hooks/useRecordingStop') };
const handlers = new Map<string, (event: { payload: unknown }) => void>();
const complete = mock(async (_successful: boolean, _meetingId?: string) => {});
const unlisten = mock(() => {});
mock.module('@tauri-apps/api/event', () => ({ ...originalEvents, listen: async (event: string, callback: (event: { payload: unknown }) => void) => { handlers.set(event, callback); return unlisten; } }));
mock.module('../../src/hooks/useRecordingStop', () => ({ useRecordingStop: () => ({ handleRecordingStop: complete }) }));
const { RecordingPostProcessingProvider } = await import('../../src/contexts/RecordingPostProcessingProvider');
let renderer: ReactTestRenderer | undefined;
beforeEach(() => { handlers.clear(); complete.mockClear(); unlisten.mockClear(); });
afterEach(() => { if (renderer) act(() => renderer!.unmount()); renderer = undefined; });
afterAll(() => { mock.module('@tauri-apps/api/event', () => originalEvents); mock.module('../../src/hooks/useRecordingStop', () => originalStop); });
const mount = async () => { await act(async () => { renderer = create(<RecordingPostProcessingProvider><div /></RecordingPostProcessingProvider>); }); };
const finalized = (recording_id: string, meeting_id: string) => handlers.get('meeting.finalized')!({ payload: { recording_id, meeting_id } });
describe('Finalization from CLI, MCP and detector', () => {
  test('refreshes explicit finalized meeting without a GUI stop or latest-session lookup', async () => {
    await mount(); finalized('recording-old', 'meeting-old');
    expect(complete).toHaveBeenCalledWith(true, 'meeting-old');
    finalized('recording-new', 'meeting-new');
    expect(complete).toHaveBeenLastCalledWith(true, 'meeting-new');
  });
  test('duplicate completion events do not process a meeting twice', async () => {
    await mount(); finalized('recording-1', 'meeting-1'); finalized('recording-1', 'meeting-1');
    expect(complete).toHaveBeenCalledTimes(1);
  });
  test('late callbacks after unmount do not process or navigate', async () => {
    await mount(); act(() => renderer!.unmount()); renderer = undefined;
    finalized('recording-1', 'meeting-1'); expect(complete).not.toHaveBeenCalled();
  });
});
