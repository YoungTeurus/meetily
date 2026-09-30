import { useEffect, useMemo, useRef, useState } from 'react';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useRouter } from 'next/navigation';
import { toast } from 'sonner';
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '../ui/dialog';
import { Button } from '../ui/button';
import { useConfig } from '@/contexts/ConfigContext';
import { LANGUAGES } from '@/constants/languages';
import { useTranscriptionModels } from '@/hooks/useTranscriptionModels';
import { retranscriptionService, type RetranscriptionProgress, type RetranscriptionResult, type RetranscriptionError, type WhisperVocabulary, type RetranscriptionJob } from '@/services/retranscriptionService';

interface RetranscribeDialogProps {
  open: boolean; onOpenChange: (open: boolean) => void; meetingId: string;
  meetingFolderPath: string | null; onComplete?: () => void | Promise<void>;
}
const stages: Record<string, string> = { queued: 'Подготавливаем распознавание', vad: 'Определяем участки речи', complete: 'Готово', loading: 'Открываем запись', decoding: 'Читаем аудио', preprocessing: 'Подготавливаем аудио', transcribing: 'Распознаём речь', saving: 'Сохраняем новый текст', completed: 'Готово', cancelled: 'Отменено' };

export function RetranscribeDialog({ open, onOpenChange, meetingId, onComplete }: RetranscribeDialogProps) {
  const { selectedLanguage, transcriptModelConfig } = useConfig();
  const router = useRouter();
  const { availableModels, selectedModelKey, setSelectedModelKey, loadingModels, fetchModels, resetSelection } = useTranscriptionModels(transcriptModelConfig);
  const [language, setLanguage] = useState(selectedLanguage || 'auto');
  const [terms, setTerms] = useState('');
  const [vocabulary, setVocabulary] = useState<WhisperVocabulary | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [ready, setReady] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [progress, setProgress] = useState<RetranscriptionProgress | null>(null);
  const [error, setError] = useState('');
  const [cancelled, setCancelled] = useState(false);
  const [runConfig, setRunConfig] = useState<Pick<RetranscriptionJob, 'provider' | 'model' | 'language' | 'vocabulary_terms' | 'effective_vocabulary'> | null>(null);
  const jobId = useRef<string | null>(null);
  const completedJobs = useRef(new Set<string>());
  const consumeJob = useRef<(job: RetranscriptionJob, closeCompleted?: boolean) => Promise<void>>(async () => {});
  const callbacks = useRef({ onComplete, onOpenChange });
  callbacks.current = { onComplete, onOpenChange };
  const selection = useMemo(() => availableModels.find(model => `${model.provider}:${model.name}` === selectedModelKey), [availableModels, selectedModelKey]);
  const isParakeet = selection?.provider === 'parakeet';
  const fetchModelsRef = useRef(fetchModels); fetchModelsRef.current = fetchModels;

  useEffect(() => {
    if (!open) return;
    let active = true;
    const unsubscribers: UnlistenFn[] = [];
    jobId.current = null; resetSelection(); setReady(false); setProcessing(false); setCancelling(false);
    setProgress(null); setError(''); setTerms(''); setVocabulary(null); setRunConfig(null); setConfirmed(false); setCancelled(false); setLanguage(selectedLanguage || 'auto');
    const complete = async (result: RetranscriptionResult, closeDialog = true) => {
      if (!active || result.meeting_id !== meetingId || result.job_id !== jobId.current || completedJobs.current.has(result.job_id)) return;
      completedJobs.current.add(result.job_id); setProcessing(false); setCancelling(false);
      try {
        await callbacks.current.onComplete?.();
        if (!active) return;
        if (result.warnings?.length) toast.warning('Новый текст сохранён в Meetily, но файлы копии требуют внимания', { description: result.warnings.join('\n'), duration: 8000 });
        else toast.success('Транскрипт обновлён', { description: result.summary_stale ? 'Сохранённый конспект относится к прежнему тексту. Его можно обновить отдельно.' : undefined });
        if (closeDialog) callbacks.current.onOpenChange(false);
      } catch (cause) { completedJobs.current.delete(result.job_id); if (active) setError(`Новый текст сохранён, но не удалось обновить страницу: ${String(cause)}`); }
    };
    const fail = (result: RetranscriptionError) => {
      if (!active || result.meeting_id !== meetingId || result.job_id !== jobId.current) return;
      setProcessing(false); setCancelling(false); setCancelled(result.cancelled);
      setError(result.cancelled ? '' : result.error);
    };
    consumeJob.current = async (job, closeCompleted = true) => {
      if (!active || job.meeting_id !== meetingId || job.job_id !== jobId.current) return;
      setProgress(job); setRunConfig(job);
      if (job.state === 'running') setProcessing(true);
      else if (job.state === 'completed') await complete(job.result ?? { job_id: job.job_id, meeting_id: job.meeting_id }, closeCompleted);
      else fail({ job_id: job.job_id, meeting_id: job.meeting_id, error: job.error || job.message, cancelled: job.state === 'cancelled' });
    };
    const setup = async () => {
      try {
        for (const [name, callback] of [
          ['retranscription-progress', (event: { payload: RetranscriptionProgress }) => { if (active && event.payload.meeting_id === meetingId && event.payload.job_id === jobId.current) setProgress(event.payload); }],
          ['retranscription-complete', (event: { payload: RetranscriptionResult }) => { void complete(event.payload); }],
          ['retranscription-error', (event: { payload: RetranscriptionError }) => fail(event.payload)],
        ] as const) {
          const unsubscribe = await listen(name, callback as (event: { payload: unknown }) => void);
          if (!active) { unsubscribe(); return; }
          unsubscribers.push(unsubscribe);
        }
        const [saved, status] = await Promise.all([retranscriptionService.getVocabulary(), retranscriptionService.getStatus(meetingId), fetchModelsRef.current()]);
        if (!active) return;
        setVocabulary(saved);
        if (status.job) {
          jobId.current = status.job.job_id;
          // Recover saved text and warnings, while leaving the reopened form available
          // for another deliberate pass instead of immediately closing it.
          await consumeJob.current(status.job, false);
        }
        setReady(true);
      } catch (cause) { if (active) setError(String(cause)); }
    };
    void setup();
    return () => { active = false; unsubscribers.forEach(unsubscribe => unsubscribe()); };
    // Each open/meeting change restores a backend job, without resetting on global config updates.
  }, [open, meetingId, resetSelection]);

  useEffect(() => {
    if (isParakeet) setLanguage('auto');
  }, [isParakeet]);

  // A reopened dialog can observe a job even if its completion event was delivered before it mounted.
  useEffect(() => {
    if (!open || !processing) return;
    let active = true;
    const poll = async () => {
      try {
        const { job } = await retranscriptionService.getStatus(meetingId);
        if (!active || !job || job.job_id !== jobId.current) return;
        await consumeJob.current(job);
      } catch (cause) { if (active) setError(String(cause)); }
    };
    const timer = setInterval(() => void poll(), 1000);
    return () => { active = false; clearInterval(timer); };
  }, [open, processing, meetingId]);

  const start = async () => {
    if (!ready || !selection || !confirmed || processing) return;
    const id = crypto.randomUUID(); jobId.current = id;
    setRunConfig({ provider: selection.provider, model: selection.name, language: isParakeet || language === 'auto' ? null : language, vocabulary_terms: isParakeet ? null : terms.trim() || null });
    setProcessing(true); setCancelling(false); setCancelled(false); setProgress(null); setError('');
    try {
      await retranscriptionService.start({ meetingId, jobId: id, language: isParakeet || language === 'auto' ? null : language, model: selection.name, provider: selection.provider, vocabularyTerms: isParakeet ? null : terms.trim() || null, confirmReplace: true });
    } catch (cause) { setProcessing(false); setError(String(cause)); }
  };
  const cancel = async () => {
    if (!jobId.current || cancelling) return;
    setCancelling(true); setError('');
    try { await retranscriptionService.cancel(jobId.current); }
    catch (cause) { setCancelling(false); setError(String(cause)); }
  };

  return <Dialog open={open} onOpenChange={next => { if (!processing) onOpenChange(next); }}>
    <DialogContent className="sm:max-w-[540px] max-h-[90vh] overflow-y-auto" onEscapeKeyDown={event => { if (processing) event.preventDefault(); }} onInteractOutside={event => { if (processing) event.preventDefault(); }}>
      <DialogHeader><DialogTitle>{processing ? 'Распознаём запись заново' : 'Распознать встречу заново'}</DialogTitle><DialogDescription>Выберите установленную модель, язык и подсказки. Старая запись аудио сохраняется.</DialogDescription></DialogHeader>
      <div className="space-y-4 text-sm">
        <p className="rounded bg-blue-50 p-3 text-blue-900">Старый транскрипт сохраняется при отмене или ошибке. После успешного распознавания он заменится новым текстом. Существующий конспект останется и будет помечен как относящийся к прежнему тексту.</p>
        {!processing && <>
          <label className="block">Модель распознавания<select aria-label="Модель распознавания" disabled={!ready || loadingModels} value={selectedModelKey} onChange={event => setSelectedModelKey(event.target.value)} className="mt-1 block w-full rounded border p-2">{!availableModels.length && <option value="">Нет установленных моделей</option>}{availableModels.map(model => <option key={`${model.provider}:${model.name}`} value={`${model.provider}:${model.name}`}>{model.displayName} ({Math.round(model.size_mb)} MB)</option>)}</select></label>
          {!loadingModels && !availableModels.length && <Button variant="outline" onClick={() => { onOpenChange(false); router.push('/settings'); }}>Установить модель в настройках распознавания</Button>}
          <label className="block">Язык<select aria-label="Язык распознавания" disabled={isParakeet || !ready} value={isParakeet ? 'auto' : language} onChange={event => setLanguage(event.target.value)} className="mt-1 block w-full rounded border p-2">{(isParakeet ? LANGUAGES.filter(item => item.code === 'auto') : LANGUAGES).map(item => <option key={item.code} value={item.code}>{item.name}</option>)}</select></label>
          {isParakeet ? <p className="text-gray-600">Parakeet использует автоопределение языка и не поддерживает подсказки. Для русского с языком ru и подсказками выберите Whisper.</p> : <div className="space-y-3 rounded border p-3">
            <p className="font-medium">Подсказки для Whisper</p><p className="text-xs text-gray-600">Имена и специальные термины. Это подсказки, а не команды. Общие подсказки из настроек добавляются автоматически. Подсказки этого запуска имеют приоритет. Длинный список может использоваться не полностью из-за ограничения модели.</p>
            <div><p className="text-xs font-medium">Сохранённые общие подсказки</p><p className="mt-1 max-h-24 overflow-auto whitespace-pre-wrap rounded bg-gray-50 p-2">{vocabulary ? vocabulary.global || 'Общих подсказок пока нет' : 'Загружаем подсказки…'}</p></div>
            <label className="block">Дополнительные подсказки только для этого запуска<textarea aria-label="Подсказки для этого запуска" disabled={!ready} value={terms} rows={3} onChange={event => setTerms(Array.from(event.target.value).slice(0, vocabulary?.max_chars ?? 1000).join(''))} className="mt-1 block w-full rounded border p-2" placeholder="Имена участников, названия проектов, термины" /></label><p className="text-xs text-gray-500">{Array.from(terms).length}/{vocabulary?.max_chars ?? 1000}. Эти подсказки не меняют настройки будущих встреч.</p>
          </div>}
          <label className="flex items-start gap-2"><input type="checkbox" aria-label="Подтвердить замену транскрипта" checked={confirmed} onChange={event => setConfirmed(event.target.checked)} />Заменить старый транскрипт только после успешного распознавания</label>
        </>}
        {processing && runConfig?.model && <div className="rounded border p-3 space-y-2 text-xs text-gray-600"><p>Модель: {runConfig.provider} · {runConfig.model}. Язык: {runConfig.language ?? 'автоопределение'}.</p>{runConfig.provider !== 'parakeet' && <><p>Подсказки только для этого запуска: {runConfig.vocabulary_terms || 'не добавлены'}.</p>{runConfig.effective_vocabulary && <p className="whitespace-pre-wrap">Общий список подсказок: {runConfig.effective_vocabulary}</p>}</>}</div>}
        {processing && <div role="status" className="space-y-2"><p>{cancelling ? 'Отменяем… Старый текст сохранится.' : stages[progress?.stage ?? 'loading'] ?? progress?.message ?? 'Подготавливаем распознавание…'}</p><progress className="w-full" max={100} value={Math.min(100, Math.max(0, progress?.progress_percentage ?? 0))} /><p>{Math.round(progress?.progress_percentage ?? 0)}%</p></div>}
        {cancelled && <p role="status" className="text-gray-700">Распознавание отменено. Старый транскрипт сохранён.</p>}
        {error && <p role="alert" className="rounded bg-red-50 p-3 text-red-800">{error}</p>}
      </div>
      <DialogFooter>{processing ? <><Button variant="ghost" onClick={() => onOpenChange(false)}>Скрыть окно</Button><Button variant="outline" disabled={cancelling} onClick={() => void cancel()}>{cancelling ? 'Отменяем…' : 'Отменить распознавание'}</Button></> : <><Button variant="outline" onClick={() => onOpenChange(false)}>Закрыть</Button><Button disabled={!ready || loadingModels || !selection || !confirmed} onClick={() => void start()}>Распознать заново</Button></>}</DialogFooter>
    </DialogContent>
  </Dialog>;
}
