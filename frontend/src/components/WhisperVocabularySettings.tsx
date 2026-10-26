'use client';
import { useEffect, useState } from 'react';
import { Button } from './ui/button';
import { retranscriptionService } from '@/services/retranscriptionService';
export function WhisperVocabularySettings() {
  const [value, setValue] = useState('');
  const [limit, setLimit] = useState(1000);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [reload, setReload] = useState(0);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    setLoading(true); setLoaded(false); setError('');
    void retranscriptionService.getVocabulary().then(config => { if (active) { setValue(config.global ?? ''); setLimit(config.max_chars); setLoaded(true); } }).catch(cause => { if (active) setError(String(cause)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [reload]);
  const save = async () => {
    setSaving(true); setError(''); setSaved(false);
    try { const config = await retranscriptionService.saveVocabulary(value); setValue(config.global ?? ''); setSaved(true); }
    catch (cause) { setError(String(cause)); }
    finally { setSaving(false); }
  };
  return <section className="rounded-lg border bg-white p-4 space-y-3">
    <h3 className="font-semibold">Подсказки для Whisper</h3>
    <p className="text-sm text-gray-600">Имена, названия компаний, сокращения и специальные термины помогают распознавать речь. Это подсказки, а не команды. Они применяются к будущим запускам Whisper; Parakeet их не поддерживает.</p>
    <label className="block text-sm">Общие подсказки<textarea aria-label="Общие подсказки" rows={4} disabled={loading || saving || !loaded} className="mt-1 block w-full rounded border p-2" value={value} onChange={event => { setValue(Array.from(event.target.value).slice(0, limit).join('')); setSaved(false); }} placeholder="Meetily, Андрей, Kubernetes" /></label>
    <div className="flex items-center justify-between gap-3"><span className="text-xs text-gray-500">{Array.from(value).length}/{limit}. Разделяйте термины запятыми или строками.</span><Button disabled={loading || saving || !loaded} onClick={() => void save()}>{saving ? 'Сохраняем…' : 'Сохранить подсказки'}</Button></div>
    {saved && <p role="status" className="text-sm text-green-700">Подсказки сохранены</p>}
    {error && <div><p role="alert" className="text-sm text-red-700">{error}</p>{!loaded && <Button variant="outline" className="mt-2" onClick={() => setReload(previous => previous + 1)}>Загрузить подсказки ещё раз</Button>}</div>}
  </section>;
}
