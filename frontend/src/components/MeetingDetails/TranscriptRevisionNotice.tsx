import { useEffect, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { retranscriptionService, type TranscriptionInfo } from '@/services/retranscriptionService';

export function TranscriptRevisionNotice({ meetingId, summaryStatus }: { meetingId: string; summaryStatus: string }) {
  const [info, setInfo] = useState<TranscriptionInfo | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    let version = 0;
    setInfo(null); setError('');
    const refresh = async () => {
      const request = ++version;
      try { const result = await retranscriptionService.getInfo(meetingId); if (active && version === request) { setInfo(result); setError(''); } }
      catch (cause) { if (active && version === request) setError(`Не удалось проверить, соответствует ли конспект текущему тексту: ${String(cause)}`); }
    };
    const listener = listen<{ meeting_id: string; transcript_revision: number; summary_stale: boolean }>('retranscription-complete', event => {
      if (active && event.payload.meeting_id === meetingId) { setInfo(event.payload); void refresh(); }
    });
    void listener.then(refresh).catch(cause => { if (active) setError(String(cause)); });
    return () => { active = false; void listener.then(unsubscribe => unsubscribe()).catch(() => {}); };
  }, [meetingId, summaryStatus]);
  if (error) return <p role="alert" className="border-b bg-amber-50 px-5 py-3 text-sm text-amber-900">{error}</p>;
  if (!info?.summary_stale) return null;
  return <p role="status" className="border-b bg-amber-50 px-5 py-3 text-sm text-amber-900">Транскрипт распознан заново. Сохранённый конспект относится к прежнему тексту. Обновите конспект отдельно, если нужно.</p>;
}
