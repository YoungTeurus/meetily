import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { OnboardingContainer } from '../OnboardingContainer';
import { useOnboarding } from '@/contexts/OnboardingContext';

export function SetupOverviewStep() {
  const { goToStep, databaseExists, setDatabaseExists } = useOnboarding();
  const [isMac, setIsMac] = useState(false);
  const [checking, setChecking] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let active = true;
    void import('@tauri-apps/plugin-os').then(({ platform }) => { if (active) setIsMac(platform() === 'macos'); }).catch(() => {});
    void invoke<boolean>('check_first_launch').then(firstLaunch => { if (active) setDatabaseExists(!firstLaunch); }).catch(cause => { if (active) setError(String(cause)); }).finally(() => { if (active) setChecking(false); });
    return () => { active = false; };
  }, [setDatabaseExists]);

  const chooseDatabase = async (mode: 'fresh' | 'import') => {
    if (busy || checking) return;
    setBusy(true); setError('');
    try {
      if (mode === 'import') {
        const selected = await invoke<string | null>('select_legacy_database_path');
        if (!selected) return;
        await invoke('import_and_initialize_database', { legacyDbPath: selected });
      } else {
        await invoke('initialize_fresh_database');
      }
      setDatabaseExists(true);
      goToStep(3);
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); }
  };
  return <OnboardingContainer title="Настройка Meetily" description="Для записи и транскрипта нужен только движок распознавания. Движок конспектов и API-ключи необязательны." step={2} totalSteps={isMac ? 4 : 3}>
    <div className="mx-auto w-full max-w-lg space-y-6">
      <div className="rounded-lg border bg-white p-5 text-sm space-y-3">
        <h2 className="font-semibold">Локальное хранилище встреч</h2>
        {checking ? <p>Проверяем хранилище…</p> : databaseExists ? <p>Хранилище этой установки готово. Существующие встречи сохраняются.</p> : <><p>Создайте новое хранилище или явно выберите файл базы из прежней установки Meetily.</p><p className="text-gray-600">Импорт создаёт копию. Исходный файл не изменяется. Эта установка использует отдельную папку данных.</p><div className="flex flex-wrap gap-3"><Button disabled={busy} onClick={() => void chooseDatabase('fresh')}>{busy ? 'Подождите…' : 'Создать новое хранилище'}</Button><Button variant="outline" disabled={busy} onClick={() => void chooseDatabase('import')}>Импортировать базу Meetily</Button></div></>}
      </div>
      {error && <p role="alert" className="rounded border border-red-200 bg-red-50 p-3 text-sm text-red-800">{error}</p>}
      <p className="text-sm text-gray-600">Затем скачаем движок распознавания. Для русского с фиксированным языком после настройки можно выбрать Whisper и язык ru; Parakeet использует автоопределение.</p>
      {databaseExists && <Button disabled={busy || checking} onClick={() => goToStep(3)}>Продолжить</Button>}
    </div>
  </OnboardingContainer>;
}
