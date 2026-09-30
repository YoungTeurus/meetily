'use client';

import { useCallback, useEffect, useSyncExternalStore } from 'react';
import { toast } from 'sonner';
import { meetingNotesService } from '@/services/meetingNotesService';

/** The draft belongs to a durable backend meeting ID, never the legacy UI UUID. */
export function MeetingNotes({ meetingId, recording = false }: { meetingId: string; recording?: boolean }) {
  const subscribe = useCallback((listener: () => void) => meetingNotesService.subscribe(meetingId, listener), [meetingId]);
  const snapshot = useCallback(() => meetingNotesService.snapshot(meetingId), [meetingId]);
  const notes = useSyncExternalStore(subscribe, snapshot, snapshot);
  useEffect(() => {
    void meetingNotesService.load(meetingId).catch(() => {});
    return () => {
      void meetingNotesService.flush(meetingId).catch(() => {
        toast.error('Meeting notes were not saved. Your draft is retained; reopen the meeting to retry.');
      });
    };
  }, [meetingId]);
  useEffect(() => {
    if (!recording) void meetingNotesService.flush(meetingId).catch(() => {});
  }, [meetingId, recording]);
  return <section className="mx-4 my-3 rounded-lg border bg-white p-3 shrink-0" aria-label="Meeting notes">
    <label htmlFor={`meeting-notes-${meetingId}`} className="font-medium">Meeting notes</label>
    <p className="text-sm text-gray-500">Add decisions, names, or points to include in the summary. Notes save automatically.</p>
    <textarea id={`meeting-notes-${meetingId}`} aria-label="Meeting notes" rows={3}
      className="mt-2 w-full rounded border p-2 text-sm" value={notes.notes}
      disabled={!notes.loaded} onChange={event => meetingNotesService.edit(meetingId, event.target.value)} />
    <div className="flex justify-between text-xs text-gray-500" aria-live="polite">
      <span>{notes.saving ? 'Saving…' : !notes.loaded ? 'Loading…' : notes.dirty ? 'Unsaved draft' : 'Saved'}</span>
      <span>{[...notes.notes].length.toLocaleString()} / 20,000</span>
    </div>
    {notes.error && <div role="alert" className="mt-2 text-sm text-red-700">
      <p>{notes.error}</p>
      {notes.conflict ? <div className="flex gap-3">
        <button type="button" className="underline" onClick={() => void meetingNotesService.resolve(meetingId, true).catch(() => {})}>Replace saved notes with my draft</button>
        <button type="button" className="underline" onClick={() => void meetingNotesService.resolve(meetingId, false).catch(() => {})}>Discard my draft and load saved notes</button>
      </div> : <button type="button" className="underline" onClick={() => void meetingNotesService.flush(meetingId).catch(() => {})}>Retry saving notes</button>}
    </div>}
  </section>;
}
