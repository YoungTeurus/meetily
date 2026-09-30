'use client';
import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { LANGUAGES } from '@/constants/languages';
import { useTranscriptionModels } from '@/hooks/useTranscriptionModels';
import { Button } from './ui/button';

interface Settings { enabled: boolean; provider: 'whisper' | 'parakeet'; model: string; language: string }
interface Job { job_id: string; meeting_id: string; state: 'queued' | 'running' | 'completed' | 'failed' | 'cancelled'; error?: string | null; config: Settings; result?: { warnings?: string[] } | null }
const stateLabels: Record<Job['state'], string> = { queued: 'В очереди', running: 'Распознаётся', completed: 'Готово', failed: 'Ошибка', cancelled: 'Отменено' };

export function AutomaticRetranscriptionSettings() {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState('');
  const [statusError, setStatusError] = useState('');
  const [reload, setReload] = useState(0);
  const [cancelling, setCancelling] = useState<string | null>(null);
  const { availableModels, loadingModels, fetchModels } = useTranscriptionModels(undefined);
  useEffect(() => { void fetchModels(); }, [fetchModels, reload]);
  useEffect(() => {
    let active = true;
    setLoading(true); setSettings(null); setSaved(false); setError('');
    void invoke<Settings>('get_automatic_retranscription_settings').then(value => { if (active) setSettings(value); }).catch(cause => { if (active) setError(String(cause)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [reload]);
  useEffect(() => {
    let active = true;
    let pending = false;
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try { const result = await invoke<{ jobs: Job[] }>('get_automatic_retranscription_status', { meetingId: null }); if (active) { setJobs(result.jobs); setStatusError(''); } }
      catch (cause) { if (active) setStatusError(String(cause)); }
      finally { pending = false; }
    };
    void refresh(); const interval = setInterval(() => void refresh(), 2000);
    return () => { active = false; clearInterval(interval); };
  }, [reload]);
  const change = (update: Partial<Settings>) => { setSettings(previous => previous ? { ...previous, ...update } : previous); setSaved(false); };
  const validModel = !!settings && availableModels.some(model => model.provider === settings.provider && model.name === settings.model);
  const save = async () => {
    if (!settings || (settings.enabled && !validModel)) return;
    setSaving(true); setError(''); setSaved(false);
    try { const result = await invoke<Settings>('set_automatic_retranscription_settings', { settings: { ...settings, language: settings.provider === 'parakeet' ? 'auto' : settings.language } }); setSettings(result); setSaved(true); }
    catch (cause) { setError(String(cause)); }
    finally { setSaving(false); }
  };
  const cancel = async (jobId: string) => {
    setCancelling(jobId); setStatusError('');
    try { await invoke('cancel_automatic_retranscription', { jobId }); const result = await invoke<{ jobs: Job[] }>('get_automatic_retranscription_status', { meetingId: null }); setJobs(result.jobs); }
    catch (cause) { setStatusError(String(cause)); }
    finally { setCancelling(null); }
  };
  return <section className="rounded-lg border bg-white p-4 space-y-3">
    <h3 className="font-semibold">Автоматическое повторное распознавание</h3>
    <p className="text-sm text-gray-600">После каждой новой завершённой записи приложение повторно распознаёт сохранённое аудио в фоне. Можно закрыть окно встречи. Импортированные записи автоматически не обрабатываются.</p>
    <fieldset disabled={loading || saving || !settings} className="space-y-3">
      <label className="flex items-center gap-2 text-sm"><input type="checkbox" aria-label="Повторять распознавание после каждого звонка" checked={settings?.enabled ?? false} onChange={event => change({ enabled: event.target.checked })} />После каждого завершённого звонка</label>
      <label className="block text-sm">Модель для повторного распознавания
        <select aria-label="Модель автоматического распознавания" className="mt-1 block w-full rounded border p-2" disabled={loadingModels || !settings} value={settings?.model ? `${settings.provider}:${settings.model}` : ''} onChange={event => { const model = availableModels.find(item => `${item.provider}:${item.name}` === event.target.value); if (model) change({ provider: model.provider, model: model.name, language: model.provider === 'parakeet' ? 'auto' : settings?.language ?? 'auto' }); }}>
          <option value="">Выберите установленную модель</option>
          {settings?.model && !validModel && <option value={`${settings.provider}:${settings.model}`}>{settings.model} — не установлена</option>}
          {availableModels.map(model => <option key={`${model.provider}:${model.name}`} value={`${model.provider}:${model.name}`}>{model.displayName}</option>)}
        </select>
      </label>
      <label className="block text-sm">Язык
        <select aria-label="Язык автоматического распознавания" className="mt-1 block w-full rounded border p-2" value={settings?.provider === 'parakeet' ? 'auto' : settings?.language ?? 'auto'} disabled={settings?.provider === 'parakeet'} onChange={event => change({ language: event.target.value })}>{LANGUAGES.filter(language => language.code !== 'auto-translate').map(language => <option key={language.code} value={language.code}>{language.name}</option>)}</select>
      </label>
      <p className="text-sm text-gray-600">{settings?.provider === 'parakeet' ? 'Parakeet определяет язык автоматически и не использует подсказки.' : 'Whisper использует общие подсказки, сохранённые к моменту завершения звонка.'}</p>
      <p className="text-sm text-gray-600">Успешный результат заменяет транскрипт; прежний Summary помечается устаревшим. При ошибке или отмене исходный транскрипт сохраняется. Выключение отменяет очередь; текущую обработку можно отменить ниже.</p>
      {settings?.enabled && !validModel && !loadingModels && <p role="alert" className="text-sm text-amber-700">Для включения установите и выберите модель.</p>}
      <Button disabled={!settings || loading || saving || (settings.enabled && (!validModel || loadingModels))} onClick={() => void save()}>{saving ? 'Сохраняем…' : 'Сохранить автоматическое распознавание'}</Button>
    </fieldset>
    {saved && <p role="status" className="text-sm text-green-700">Настройка сохранена</p>}
    {error && <p role="alert" className="text-sm text-red-700">{error}</p>}
    <Button variant="outline" disabled={loading || saving} onClick={() => setReload(value => value + 1)}>Обновить настройки и модели</Button>
    {jobs.length > 0 && <div className="space-y-2"><h4 className="text-sm font-medium">Последние фоновые задания</h4>{jobs.map(job => <div key={job.job_id} className="rounded border p-2 text-sm"><p><a href={`/meeting-details?id=${encodeURIComponent(job.meeting_id)}`} className="underline">Открыть встречу</a> · {stateLabels[job.state]} · {job.config.model}</p>{job.error && <p className="text-red-700">{job.error}</p>}{job.result?.warnings?.map((warning, index) => <p key={index} className="text-amber-700">{warning}</p>)}{(job.state === 'queued' || job.state === 'running') && <Button variant="outline" size="sm" disabled={cancelling !== null} onClick={() => void cancel(job.job_id)}>{cancelling === job.job_id ? 'Отменяем…' : 'Отменить фоновое распознавание'}</Button>}</div>)}</div>}
    {statusError && <p role="alert" className="text-sm text-red-700">Не удалось обновить очередь: {statusError}</p>}
  </section>;
}
