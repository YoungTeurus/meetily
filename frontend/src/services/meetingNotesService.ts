import { invoke } from '@tauri-apps/api/core';

export interface MeetingNotes { meeting_id: string; notes: string; revision: number }
export interface NotesSnapshot {
  notes: string; revision: number; loaded: boolean; dirty: boolean;
  saving: boolean; error: string | null; conflict: boolean;
}
interface Entry {
  snapshot: NotesSnapshot; listeners: Set<() => void>;
  loading?: Promise<void>; saving?: Promise<void>; timer?: ReturnType<typeof setTimeout>;
}
const entries = new Map<string, Entry>();
const key = (id: string) => `meetily.notes.draft.${id}`;
function draft(id: string): { notes: string; revision: number } | null {
  try {
    const value = JSON.parse(localStorage.getItem(key(id)) || 'null');
    return value && typeof value.notes === 'string' && Number.isSafeInteger(value.revision) ? value : null;
  } catch { return null; }
}
function entry(id: string): Entry {
  let value = entries.get(id);
  if (!value) {
    const saved = draft(id);
    value = { snapshot: { notes: saved?.notes ?? '', revision: saved?.revision ?? 0, loaded: false,
      dirty: !!saved, saving: false, error: null, conflict: false }, listeners: new Set() };
    entries.set(id, value);
  }
  return value;
}
function update(value: Entry, change: Partial<NotesSnapshot>) {
  value.snapshot = { ...value.snapshot, ...change };
  value.listeners.forEach(listener => listener());
}
function persist(id: string, value: Entry) {
  try {
    if (value.snapshot.dirty) localStorage.setItem(key(id), JSON.stringify({ notes: value.snapshot.notes, revision: value.snapshot.revision }));
    else localStorage.removeItem(key(id));
  } catch {
    update(value, { error: 'Draft recovery storage is unavailable. Keep this window open until notes are saved.' });
  }
}
const message = (error: unknown) => error instanceof Error ? error.message :
  typeof error === 'object' && error && 'message' in error ? String(error.message) : String(error);

export const meetingNotesService = {
  snapshot: (id: string) => entry(id).snapshot,
  subscribe(id: string, listener: () => void) {
    const value = entry(id); value.listeners.add(listener);
    return () => { value.listeners.delete(listener); };
  },
  async load(id: string): Promise<void> {
    const value = entry(id);
    if (value.snapshot.loaded) return;
    if (value.loading) return value.loading;
    value.loading = (async () => {
      try {
        const saved = await invoke<MeetingNotes>('get_meeting_notes', { meetingId: id });
        if (saved.meeting_id !== id) throw new Error('Unexpected meeting notes response');
        const conflict = value.snapshot.dirty && value.snapshot.revision !== saved.revision;
        update(value, value.snapshot.dirty
          ? { loaded: true, conflict, error: conflict ? 'Notes changed in another window. Your draft is retained. Choose which version to keep.' : null }
          : { loaded: true, notes: saved.notes, revision: saved.revision, error: null });
      } catch (error) {
        update(value, { error: message(error) }); throw new Error(message(error));
      } finally { value.loading = undefined; }
    })();
    return value.loading;
  },
  edit(id: string, notes: string) {
    const value = entry(id);
    if (!value.snapshot.loaded) return;
    if ([...notes].length > 20000 || notes.includes('\0')) {
      update(value, { error: 'Notes must contain at most 20,000 characters and no NUL characters.' }); return;
    }
    update(value, { notes, dirty: true, error: value.snapshot.conflict ? value.snapshot.error : null });
    persist(id, value);
    clearTimeout(value.timer);
    value.timer = setTimeout(() => { void meetingNotesService.flush(id).catch(() => {}); }, 600);
  },
  async flush(id: string): Promise<void> {
    // A summary with no local draft needs no read: its backend loads authoritative notes.
    if (!entries.has(id) && !draft(id)) return;
    const value = entry(id);
    clearTimeout(value.timer);
    await meetingNotesService.load(id);
    if (value.saving) { await value.saving; return meetingNotesService.flush(id); }
    if (value.snapshot.conflict) throw new Error('Resolve the notes conflict before generating a summary. Your draft is retained.');
    if (!value.snapshot.dirty) return;
    update(value, { saving: true, error: null });
    value.saving = (async () => {
      try {
        while (value.snapshot.dirty) {
          const notes = value.snapshot.notes;
          const saved = await invoke<MeetingNotes>('save_meeting_notes', {
            meetingId: id, notes, expectedRevision: value.snapshot.revision,
          });
          update(value, { revision: saved.revision, dirty: value.snapshot.notes !== notes });
          persist(id, value);
        }
      } catch (error) {
        const conflict = typeof error === 'object' && error !== null && 'code' in error && error.code === 'notes_conflict';
        update(value, { error: conflict ? 'Notes changed in another window. Your draft is retained. Choose which version to keep.' : `Notes were not saved: ${message(error)}. Your draft is retained.`, conflict });
        throw new Error(value.snapshot.error || 'Notes could not be saved');
      } finally { update(value, { saving: false }); value.saving = undefined; }
    })();
    return value.saving;
  },
  async resolve(id: string, keepDraft: boolean) {
    const value = entry(id);
    try {
      const saved = await invoke<MeetingNotes>('get_meeting_notes', { meetingId: id });
      update(value, { revision: saved.revision, notes: keepDraft ? value.snapshot.notes : saved.notes,
        dirty: keepDraft, loaded: true, conflict: false, error: null });
      persist(id, value);
      if (keepDraft) await meetingNotesService.flush(id);
    } catch (error) { update(value, { error: message(error) }); throw new Error(message(error)); }
  },
};
