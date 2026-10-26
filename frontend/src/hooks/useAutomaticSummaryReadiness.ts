import { useCallback, useEffect, useRef, useState, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';

type Scope = { meetingId: string; enabled: boolean; attempt: number };
type Readiness = { scope: Scope; ready: boolean; message: string; error: boolean };
type Job = { meeting_id: string; state: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled' };

/** Only gates a newly recorded meeting's requested auto-summary; manual generation remains available. */
export function useAutomaticSummaryReadiness(meetingId: string, enabled: boolean, refresh?: () => Promise<void>) {
  const [attempt, setAttempt] = useState(0);
  const scope = useMemo(() => ({ meetingId, enabled, attempt }), [meetingId, enabled, attempt]);
  const [state, setState] = useState<Readiness>({ scope, ready: false, message: '', error: false });
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setState({ scope, ready: false, message: 'Проверяем фоновое распознавание перед созданием Summary…', error: false });
    const check = async () => {
      try {
        const result = await invoke<{ jobs: Job[] }>('get_automatic_retranscription_status', { meetingId });
        if (!active) return;
        const job = result.jobs.find(value => value.meeting_id === meetingId);
        if (job?.state === 'queued' || job?.state === 'running') {
          setState({ scope, ready: false, message: 'Summary будет создан после фонового распознавания. Ручная генерация использует текущий транскрипт.', error: false });
          timer = setTimeout(() => { void check(); }, 2000);
          return;
        }
        if (job && !['completed', 'failed', 'cancelled'].includes(job.state)) throw new Error('Неизвестное состояние фонового задания');
        if (job?.state === 'completed') await refreshRef.current?.();
        if (!active) return;
        setState({ scope, ready: true, error: false, message:
          job?.state === 'failed' || job?.state === 'cancelled'
            ? 'Фоновое распознавание не завершилось. Summary использует сохранённый исходный транскрипт.' : '' });
      } catch (cause) {
        if (active) setState({ scope, ready: false, error: true,
          message: `Не удалось подготовить автоматический Summary: ${cause instanceof Error ? cause.message : String(cause)}. Можно повторить проверку или запустить Summary вручную.` });
        // No automatic retry loop on a persistent backend/read error.
      }
    };
    void check();
    return () => { active = false; clearTimeout(timer); };
  }, [scope, meetingId, enabled]);
  const retry = useCallback(() => { setState({ scope, ready: false, message: '', error: false }); setAttempt(value => value + 1); }, [scope]);
  // Guard the render before a new meeting's effect has run.
  return { ready: enabled && state.scope === scope && state.ready,
    message: enabled && state.scope === scope ? state.message : '',
    error: enabled && state.scope === scope && state.error, retry };
}
