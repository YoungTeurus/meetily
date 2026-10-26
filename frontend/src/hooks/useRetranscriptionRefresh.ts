import { useEffect, useRef } from 'react';
import { listen } from '@tauri-apps/api/event';
import { retranscriptionService, type RetranscriptionResult } from '@/services/retranscriptionService';

/** Keep a saved meeting's text current even when its job dialog is closed. */
export function useRetranscriptionRefresh(meetingId: string, refresh?: () => Promise<void>) {
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    let active = true;
    const handled = new Set<string>();
    const complete = async (result: RetranscriptionResult) => {
      if (!active || result.meeting_id !== meetingId || handled.has(result.job_id)) return;
      handled.add(result.job_id);
      try { await refreshRef.current?.(); }
      catch (error) { handled.delete(result.job_id); console.error('Could not reload completed retranscription:', error); }
    };
    const listener = listen<RetranscriptionResult>('retranscription-complete', event => { void complete(event.payload); });
    // Subscribe first, then recover a completion delivered before this page mounted.
    void listener.then(async () => {
      const { job } = await retranscriptionService.getStatus(meetingId);
      if (active && job?.state === 'completed') await complete(job.result ?? { job_id: job.job_id, meeting_id: job.meeting_id });
    }).catch(error => { if (active) console.error('Could not subscribe to retranscription completion:', error); });
    return () => { active = false; void listener.then(unsubscribe => unsubscribe()).catch(() => {}); };
  }, [meetingId]);
}
