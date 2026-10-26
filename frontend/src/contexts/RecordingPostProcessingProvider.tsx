'use client';
import React, { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useRecordingStop } from '@/hooks/useRecordingStop';

export interface MeetingFinalizedEvent { recording_id: string; meeting_id: string; data?: { folder_path?: string } }
const noOp = () => {};
/** Native finalization is authoritative regardless of GUI, tray, CLI or detector source. */
export function RecordingPostProcessingProvider({ children }: { children: React.ReactNode }) {
  const { handleRecordingStop } = useRecordingStop(noOp, noOp);
  const handler = useRef(handleRecordingStop);
  handler.current = handleRecordingStop;
  useEffect(() => {
    let active = true;
    const handled = new Set<string>();
    const listener = listen<MeetingFinalizedEvent>('meeting:finalized', event => {
      const { recording_id, meeting_id } = event.payload;
      if (!active || !recording_id || !meeting_id || handled.has(recording_id)) return;
      handled.add(recording_id);
      // Pass the completed meeting explicitly, even when a newer session already exists.
      void handler.current(true, meeting_id);
    });
    return () => { active = false; void listener.then(unlisten => unlisten()); };
  }, []);
  return <>{children}</>;
}
