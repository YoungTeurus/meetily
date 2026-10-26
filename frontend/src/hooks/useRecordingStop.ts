import { meetingNotesService } from '@/services/meetingNotesService';
import { useEffect, useCallback, useRef } from 'react';
import { useRouter } from 'next/navigation';
import { invoke } from '@tauri-apps/api/core';
import { toast } from 'sonner';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { useSidebar } from '@/components/Sidebar/SidebarProvider';
import { useRecordingState, RecordingStatus } from '@/contexts/RecordingStateContext';
import { storageService } from '@/services/storageService';
import {
  applyPinnedSummaryLanguageToMeeting,
  detectAndCacheSummaryLanguage,
} from '@/lib/summary-language-preferences';

type SummaryStatus = 'idle' | 'processing' | 'summarizing' | 'regenerating' | 'completed' | 'error';
interface UseRecordingStopReturn {
  handleRecordingStop: (callApi: boolean, finalizedMeetingId?: string) => Promise<void>;
  isStopping: boolean;
  isProcessingTranscript: boolean;
  isSavingTranscript: boolean;
  summaryStatus: SummaryStatus;
  setIsStopping: (value: boolean) => void;
}

/** Backend stop drains and commits the meeting. React only refreshes and navigates. */
export function useRecordingStop(
  setIsRecording: (value: boolean) => void,
  setIsRecordingDisabled: (value: boolean) => void
): UseRecordingStopReturn {
  const { status, setStatus, isStopping, isProcessing, isSaving } = useRecordingState();
  const { flushBuffer, clearTranscripts, markMeetingAsSaved } = useTranscripts();
  const { refetchMeetings, setCurrentMeeting, setIsMeetingActive } = useSidebar();
  const router = useRouter();
  const stopInProgressRef = useRef(false);
  const handledMeetingRef = useRef<string | null>(null);

  const handleRecordingStop = useCallback(async (successful: boolean, finalizedMeetingId?: string) => {
    if (stopInProgressRef.current && !finalizedMeetingId) return;
    stopInProgressRef.current = true;
    let ownsLifecycle = false;
    try {
      if (!successful) throw new Error('Recording could not be finalized. The backend retains its saved transcript checkpoints.');
      let meetingId = finalizedMeetingId;
      if (!meetingId) {
        const result = await invoke<{ recording: { meeting_id: string; state: string; error?: string } | null }>('get_recording_session');
        const session = result.recording;
        if (!session || session.state !== 'finalized') {
          throw new Error(session?.error || 'Recording is still being processed.');
        }
        meetingId = session.meeting_id;
      }
      if (handledMeetingRef.current === meetingId) return;
      handledMeetingRef.current = meetingId;
      // A notes failure must not turn a successfully finalized recording into an error.
      try { await meetingNotesService.flush(meetingId); } catch {
        toast.error('Meeting notes were not saved. Your draft is retained; retry on the meeting page.');
      }
      const meeting = await storageService.getMeeting(meetingId);
      try {
        if (!(await applyPinnedSummaryLanguageToMeeting(meetingId))) {
          await detectAndCacheSummaryLanguage(meetingId, meeting.transcripts.map((t: { text: string }) => t.text));
        }
      } catch (error) {
        console.warn('Could not set summary language preference:', error);
      }
      await refetchMeetings();
      // A completion event can arrive after a new recording has started.
      // Refresh its saved meeting, then leave the new session's UI and recovery data intact.
      const latest = await invoke<{ recording: { meeting_id: string; state: string } | null }>('get_recording_session');
      const current = latest.recording;
      if (current && current.meeting_id !== meetingId && ['starting', 'recording', 'paused', 'stopping', 'processing'].includes(current.state)) {
        toast.success('Recording saved successfully!');
        return;
      }
      ownsLifecycle = true;
      setIsRecording(false);
      setIsRecordingDisabled(true);
      flushBuffer();
      await markMeetingAsSaved();
      setCurrentMeeting({ id: meetingId, title: meeting.title });
      setIsMeetingActive(false);
      setStatus(RecordingStatus.COMPLETED);
      toast.success('Recording saved successfully!');
      router.push(`/meeting-details?id=${meetingId}&source=recording`);
      clearTranscripts();
      setStatus(RecordingStatus.IDLE);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (!finalizedMeetingId) setStatus(RecordingStatus.ERROR, message);
      toast.error('Could not finish recording', { description: message });
    } finally {
      if (ownsLifecycle) setIsRecordingDisabled(false);
      stopInProgressRef.current = false;
    }
  }, [setIsRecording, setIsRecordingDisabled, setStatus, flushBuffer, clearTranscripts,
    markMeetingAsSaved, refetchMeetings, setCurrentMeeting, setIsMeetingActive, router]);

  const handlerRef = useRef(handleRecordingStop);
  useEffect(() => { handlerRef.current = handleRecordingStop; });
  useEffect(() => {
    (window as any).handleRecordingStop = (success = true) => handlerRef.current(success);
    return () => { delete (window as any).handleRecordingStop; };
  }, []);
  return {
    handleRecordingStop, isStopping, isProcessingTranscript: isProcessing,
    isSavingTranscript: isSaving,
    summaryStatus: status === RecordingStatus.PROCESSING_TRANSCRIPTS ? 'processing' : 'idle',
    setIsStopping: value => setStatus(value ? RecordingStatus.STOPPING : RecordingStatus.IDLE),
  };
}
