'use client';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { toast } from 'sonner';
import { Button } from './ui/button';
export type DetectionSettings = { enabled: boolean; applications: string[]; debounce_ms: number; grace_ms: number; auto_stop: boolean; notification_mode: 'system' | 'in_app' };
export type CallPrompt = { kind: 'start' | 'stop'; session_id: string; application: string };
export type DetectionStatus = {
  settings: DetectionSettings;
  observations: { application: string; state: string; confidence: string; limitations: string[]; evidence: string[] }[];
  sessions: { session_id: string; application: string; phase: string; suppressed: boolean; recording_id?: string | null }[];
  prompts: CallPrompt[];
  platform: string;
};
const appNames: Record<string, string> = { zoom: 'Zoom', discord: 'Discord', teams: 'Microsoft Teams', browser: 'Браузер' };
export function CallPrompts({ compact = false }: { compact?: boolean }) {
  const [status, setStatus] = useState<DetectionStatus | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    const refresh = () => invoke<DetectionStatus>('get_detection_status').then(value => { if (active) setStatus(value); });
    // Subscribe before reading so a prompt created during startup is not lost.
    const listener = listen<DetectionStatus>('detection-changed', event => { if (active) setStatus(event.payload); });
    void listener.then(refresh).catch(cause => { if (active && compact) setError(String(cause)); });
    return () => { active = false; void listener.then(unlisten => unlisten()); };
  }, [compact]);
  const act = async (prompt: CallPrompt, action: 'start' | 'skip' | 'dismiss' | 'stop') => {
    if (pending) return;
    setPending(prompt.session_id); setError('');
    try {
      // The native service validates current session again before starting capture.
      await invoke('detection_action', { sessionId: prompt.session_id, action });
      const next = await invoke<DetectionStatus>('get_detection_status');
      setStatus(next);
      if (compact && next.prompts.length === 0) await invoke('close_call_action');
    } catch (cause) { setError(String(cause)); toast.error('Не удалось выполнить действие', { description: String(cause) }); }
    finally { setPending(null); }
  };
  const prompts = status?.settings.enabled ? status.prompts : [];
  if (!compact && !prompts.length) return null;
  return <aside aria-label="Предложения записи звонков" aria-live="polite" className={compact ? 'p-5 space-y-4' : 'fixed z-50 bottom-6 right-6 w-[min(420px,calc(100vw-2rem))] space-y-3'}>
    {error && <p role="alert" className="rounded bg-red-50 p-3 text-sm text-red-800">{error}</p>}
    {compact && !prompts.length && <div><h1 className="text-lg font-semibold">Нет текущего предложения</h1><p className="mt-2 text-sm text-gray-600">Звонок уже завершён или действие выполнено.</p><Button className="mt-4" variant="outline" onClick={() => void invoke('close_call_action')}>Закрыть</Button></div>}
    {prompts.map(prompt => <div key={`${prompt.session_id}:${prompt.kind}`} className="rounded-lg border bg-white p-5 shadow-lg">
      <div className="flex justify-between gap-3"><h2 className="font-semibold">{prompt.kind === 'start' ? 'Обнаружен звонок' : 'Звонок завершён'} · {appNames[prompt.application] ?? prompt.application}</h2><button aria-label="Закрыть предложение" disabled={!!pending} onClick={() => void act(prompt, 'dismiss')} className="text-gray-500 hover:text-gray-900">×</button></div>
      <p className="mt-2 text-sm text-gray-600">{prompt.kind === 'start' ? 'Будут записаны микрофон и системный звук всего компьютера, включая другие приложения.' : 'Завершить запись и сохранить транскрипт встречи?'}</p>
      <div className="mt-4 flex gap-2"><Button disabled={!!pending} onClick={() => void act(prompt, prompt.kind === 'start' ? 'start' : 'stop')}>{pending === prompt.session_id ? 'Подождите…' : prompt.kind === 'start' ? 'Начать запись' : 'Завершить и сохранить'}</Button><Button variant="outline" disabled={!!pending} onClick={() => void act(prompt, prompt.kind === 'start' ? 'skip' : 'dismiss')}>{prompt.kind === 'start' ? 'Пропустить' : 'Продолжить запись'}</Button></div>
    </div>)}
  </aside>;
}
