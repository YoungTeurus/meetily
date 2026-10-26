import { afterEach, expect, mock, test } from 'bun:test';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
const saved = new Map<string, { meeting_id: string; notes: string; revision: number }>();
const storage = new Map<string, string>();
Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
  getItem: (key: string) => storage.get(key) ?? null,
  setItem: (key: string, value: string) => storage.set(key, value), removeItem: (key: string) => storage.delete(key),
} });
let customGet: ((id: string) => Promise<unknown>) | null = null;
let customSave: ((id: string, notes: string, revision: number) => Promise<unknown>) | null = null;
const invoke = mock(async (command: string, args: any) => {
  const id = args.meetingId;
  if (command === 'get_meeting_notes') return customGet ? customGet(id) : saved.get(id) ?? { meeting_id: id, notes: '', revision: 0 };
  if (customSave) return customSave(id, args.notes, args.expectedRevision);
  const previous = saved.get(id);
  if ((previous?.revision ?? 0) !== args.expectedRevision) throw { code: 'notes_conflict', message: 'conflict' };
  const result = { meeting_id: id, notes: args.notes, revision: args.expectedRevision + 1 }; saved.set(id, result); return result;
});
const notify = mock(() => {});
mock.module('@tauri-apps/api/core', () => ({ invoke }));
mock.module('sonner', () => ({ toast: { error: notify } }));
const { meetingNotesService: service } = await import('../../src/services/meetingNotesService');
const { MeetingNotes } = await import('../../src/components/MeetingNotes');
let renderer: ReactTestRenderer | undefined;
afterEach(async () => { if (renderer) await act(async () => renderer!.unmount()); renderer = undefined; customGet = null; customSave = null; });
async function mount(id: string, recording = true) { await act(async () => { renderer = create(<MeetingNotes meetingId={id} recording={recording} />); }); }
async function type(notes: string) { await act(async () => { renderer!.root.findByType('textarea').props.onChange({ target: { value: notes } }); }); }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }

test('typing persists a draft immediately and debounce saves the latest text', async () => {
  await mount('typing'); await type('first'); await type('latest 😀');
  expect(JSON.parse(storage.get('meetily.notes.draft.typing')!).notes).toBe('latest 😀');
  expect(saved.has('typing')).toBe(false);
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 650)); });
  expect(saved.get('typing')?.notes).toBe('latest 😀'); expect(storage.has('meetily.notes.draft.typing')).toBe(false);
});
test('late loading another meeting cannot replace the current editor', async () => {
  const old = deferred<unknown>(); customGet = async id => id === 'old-load' ? old.promise : { meeting_id: id, notes: 'new meeting', revision: 0 };
  await mount('old-load');
  await act(async () => renderer!.update(<MeetingNotes meetingId="new-load" recording />));
  await type('new draft');
  await act(async () => old.resolve({ meeting_id: 'old-load', notes: 'old text', revision: 0 }));
  expect(renderer!.root.findByType('textarea').props.value).toBe('new draft');
});
test('unmount failure retains the draft and reports a visible notification', async () => {
  await mount('unmount-fail'); await type('do not lose me');
  customSave = async () => { throw new Error('disk full'); };
  await act(async () => renderer!.unmount()); renderer = undefined;
  expect(service.snapshot('unmount-fail').dirty).toBe(true);
  expect(JSON.parse(storage.get('meetily.notes.draft.unmount-fail')!).notes).toBe('do not lose me');
  expect(notify).toHaveBeenCalled();
});
test('stop flushes notes without waiting for the debounce timer', async () => {
  await mount('stop'); await type('decision');
  await act(async () => renderer!.update(<MeetingNotes meetingId="stop" recording={false} />));
  expect(saved.get('stop')?.notes).toBe('decision');
});
test('an edit during save is serialized after the first save with the new revision', async () => {
  await service.load('inflight'); service.edit('inflight', 'first');
  const first = deferred<unknown>(); let count = 0; const revisions: number[] = [];
  customSave = async (id, notes, revision) => { revisions.push(revision); if (++count === 1) return first.promise; return { meeting_id: id, notes, revision: revision + 1 }; };
  const flush = service.flush('inflight'); await Promise.resolve(); await Promise.resolve();
  service.edit('inflight', 'second'); first.resolve({ meeting_id: 'inflight', notes: 'first', revision: 1 }); await flush;
  expect(revisions).toEqual([0, 1]); expect(service.snapshot('inflight').dirty).toBe(false); expect(service.snapshot('inflight').notes).toBe('second');
});
test('conflict blocks summary flush and requires an explicit choice before replacement', async () => {
  await mount('conflict'); await type('my draft');
  saved.set('conflict', { meeting_id: 'conflict', notes: 'other window', revision: 3 });
  await act(async () => { await expect(service.flush('conflict')).rejects.toBeDefined(); });
  expect(service.snapshot('conflict').notes).toBe('my draft'); expect(service.snapshot('conflict').conflict).toBe(true);
  await expect(service.flush('conflict')).rejects.toThrow('Resolve the notes conflict');
  await act(async () => service.resolve('conflict', true));
  expect(saved.get('conflict')).toEqual({ meeting_id: 'conflict', notes: 'my draft', revision: 4 });
});
test('a restored draft checks its revision and does not overwrite newer saved notes', async () => {
  storage.set('meetily.notes.draft.restored', JSON.stringify({ notes: 'recovered draft', revision: 1 }));
  saved.set('restored', { meeting_id: 'restored', notes: 'new server notes', revision: 2 });
  await mount('restored');
  expect(service.snapshot('restored').conflict).toBe(true);
  expect(renderer!.root.findByType('textarea').props.value).toBe('recovered draft');
  await act(async () => service.resolve('restored', false)); expect(service.snapshot('restored').notes).toBe('new server notes');
});
test('Unicode characters count as code points and invalid text does not replace a draft', async () => {
  await service.load('limit'); service.edit('limit', '😀'.repeat(20000)); expect(service.snapshot('limit').notes.length).toBe(40000);
  service.edit('limit', 'x'.repeat(20001)); expect(service.snapshot('limit').notes.length).toBe(40000);
  service.edit('limit', 'NUL\0'); expect(service.snapshot('limit').notes.length).toBe(40000);
  await service.flush('limit');
});
