'use client';

import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { isPermissionGranted, requestPermission } from '@tauri-apps/plugin-notification';
import { Switch } from './ui/switch';
import { Button } from './ui/button';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import type { DetectionStatus, DetectionSettings } from './CallPrompts';

type IntegrationSettings = { enabled: boolean; allow_control: boolean; credential_file?: string; port?: number };
const applications = [['zoom', 'Zoom'], ['discord', 'Discord'], ['teams', 'Microsoft Teams'], ['browser', 'Звонки в браузере']] as const;
const stateNames: Record<string, string> = { confirmed_call: 'Обнаружен звонок', no_call: 'Звонка нет', unknown: 'Недостаточно признаков', unsupported: 'Недоступно на этой системе' };

export function CallIntegrationSettings() {
  const [status, setStatus] = useState<DetectionStatus | null>(null);
  const [integration, setIntegration] = useState<IntegrationSettings | null>(null);
  const [login, setLogin] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const { transcriptModelConfig, selectedLanguage, selectedDevices } = useConfig();

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      const results = await Promise.allSettled([
        invoke<DetectionStatus>('get_detection_status'),
        invoke<IntegrationSettings>('get_integration_settings'),
        invoke<boolean>('get_login_start'),
      ]);
      if (!active) return;
      const [detection, integrations, autostart] = results;
      if (detection.status === 'fulfilled') setStatus(detection.value);
      if (integrations.status === 'fulfilled') setIntegration(integrations.value);
      if (autostart.status === 'fulfilled') setLogin(autostart.value);
      const failure = results.find(result => result.status === 'rejected');
      if (failure?.status === 'rejected') setError(String(failure.reason));
    };
    void refresh();
    const listener = listen<DetectionStatus>('detection-changed', event => { if (active) setStatus(event.payload); });
    return () => { active = false; void listener.then(unlisten => unlisten()); };
  }, []);

  const save = async (operation: () => Promise<unknown>) => {
    setBusy(true); setError('');
    try { await operation(); } catch (cause) { setError(String(cause)); toast.error('Не удалось сохранить настройку'); }
    finally { setBusy(false); }
  };
  const updateDetection = (patch: Partial<DetectionSettings>) => save(async () => {
    if (!status) return;
    if ((patch.enabled === true && status.settings.notification_mode === 'system') || patch.notification_mode === 'system') {
      if (!(await isPermissionGranted()) && await requestPermission() !== 'granted') {
        throw new Error('Разрешите уведомления в настройках ОС или выберите уведомления в приложении.');
      }
    }
    await invoke('set_detection_settings', { settings: { ...status.settings, ...patch } });
    setStatus(await invoke<DetectionStatus>('get_detection_status'));
  });
  const updateIntegration = (patch: Partial<IntegrationSettings>) => save(async () => {
    if (!integration) return;
    const settings = { ...integration, ...patch };
    await invoke('set_integration_settings', { enabled: settings.enabled, allowControl: settings.allow_control });
    setIntegration(await invoke<IntegrationSettings>('get_integration_settings'));
  });

  return <div className="space-y-6">
    {error && <p role="alert" className="rounded border border-red-200 bg-red-50 p-3 text-sm text-red-800">{error}</p>}
    <section className="rounded-lg border bg-white p-6 shadow-sm space-y-4">
      <h3 className="text-lg font-semibold">Обнаружение звонков</h3>
      <p className="text-sm text-gray-600">Meetily предложит запись. Она начнётся только после нажатия «Начать запись». Наблюдение работает, пока приложение запущено, в том числе в области уведомлений.</p>
      <div className="flex items-center justify-between gap-4"><label htmlFor="call-detection">Предлагать запись звонков</label><Switch id="call-detection" disabled={busy || !status} checked={status?.settings.enabled ?? false} onCheckedChange={enabled => void updateDetection({ enabled })} /></div>
      {status?.platform === 'macos' && <div className="rounded border border-blue-200 bg-blue-50 p-3 text-sm text-blue-900">
        <p>Чтобы отличать звонок от открытого приложения, Meetily читает названия кнопок звонка в Zoom и Discord. Разрешите «Универсальный доступ» в настройках macOS. Без разрешения звонок может не определяться.</p>
        <Button variant="outline" className="mt-2" disabled={busy} onClick={() => void save(() => invoke('open_detection_permissions'))}>Открыть настройки доступа</Button>
      </div>}
      {status?.platform === 'windows' && <p className="text-sm text-gray-600">Для подтверждения звонка Meetily читает названия кнопок Zoom и Discord. Приложения должны работать от того же пользователя и с теми же правами доступа.</p>}
      <div className="space-y-3">
        {applications.map(([id, title]) => {
          const observations = status?.observations.filter(observation => observation.application === id) ?? [];
          const supported = id === 'zoom' || id === 'discord';
          return <div key={id} className="rounded border p-3 text-sm">
            <label className="flex items-center gap-2"><input type="checkbox" disabled={busy || !status || !supported} checked={status?.settings.applications.includes(id) ?? false} onChange={event => void updateDetection({ applications: event.target.checked ? [...(status?.settings.applications ?? []), id] : (status?.settings.applications ?? []).filter(app => app !== id) })} />{title}</label>
            {observations.length ? observations.map((observation, index) => <div key={index} className="mt-2 text-gray-600"><p>{stateNames[observation.state] ?? observation.state}</p>{observation.limitations.map((limitation, i) => <p key={i}>{limitation}</p>)}</div>) : <p className="mt-1 text-gray-500">Возможности появятся после включения наблюдения. Наличие открытого приложения ещё не означает звонок.</p>}
            {!supported && <p className="mt-1 text-gray-500">Адаптер пока недоступен. Наблюдение для этого приложения не включается.</p>}
            {id === 'browser' && <p className="mt-1 text-gray-500">Приблизительное обнаружение. Активный микрофон сам по себе не подтверждает звонок.</p>}
          </div>;
        })}
      </div>
      <label className="block text-sm">Где показывать предложение<select className="mt-1 block w-full rounded border p-2" disabled={busy || !status} value={status?.settings.notification_mode ?? 'system'} onChange={event => void updateDetection({ notification_mode: event.target.value as 'system' | 'in_app' })}><option value="system">Системное уведомление и окно действий</option><option value="in_app">В окне Meetily</option></select></label>
      {status?.platform === 'macos' && <p className="text-sm text-gray-600">На macOS вместе с системным уведомлением откроется небольшое окно с действиями.</p>}
      <div className="flex items-center justify-between gap-4"><div><label htmlFor="call-auto-stop">Завершать запись после выхода из звонка</label><p className="text-xs text-gray-500">Только для записи, начатой из предложения этого звонка.</p></div><Switch id="call-auto-stop" disabled={busy || !status} checked={status?.settings.auto_stop ?? false} onCheckedChange={auto_stop => void updateDetection({ auto_stop })} /></div>
      <div className="grid grid-cols-2 gap-3 text-sm">{(['debounce_ms', 'grace_ms'] as const).map(key => <label key={key}>{key === 'debounce_ms' ? 'Подтверждение звонка, секунд' : 'Ожидание при потере признаков, секунд'}<input type="number" min={key === 'debounce_ms' ? 0.5 : 5} max={key === 'debounce_ms' ? 60 : 300} step={0.5} className="mt-1 block w-full rounded border p-2" disabled={busy || !status} value={(status?.settings[key] ?? (key === 'debounce_ms' ? 3000 : 20000)) / 1000} onChange={event => { const seconds = Number(event.target.value); if (Number.isFinite(seconds) && seconds >= (key === 'debounce_ms' ? 0.5 : 5) && seconds <= (key === 'debounce_ms' ? 60 : 300)) void updateDetection({ [key]: seconds * 1000 }); }} /></label>)}</div>
      <div className="flex items-center justify-between gap-4"><div><label htmlFor="call-login-start">Запускать Meetily при входе в систему</label><p className="text-xs text-gray-500">Запускается наблюдение. Запись автоматически не начинается.</p></div><Switch id="call-login-start" checked={login} disabled={busy} onCheckedChange={enabled => void save(async () => { await invoke('set_login_start', { enabled }); setLogin(enabled); })} /></div>
    </section>
    <section className="rounded-lg border bg-white p-6 shadow-sm space-y-4">
      <h3 className="text-lg font-semibold">Локальные CLI и MCP</h3><p className="text-sm text-gray-600">Доступ для Codex и meetilyctl на этом компьютере. Для транскриптов не нужен API-ключ OpenAI или движок конспектов.</p>
      <div className="flex items-center justify-between"><label htmlFor="call-integrations">Разрешить чтение встреч</label><Switch id="call-integrations" checked={integration?.enabled ?? false} disabled={busy || !integration} onCheckedChange={enabled => void updateIntegration({ enabled })} /></div>
      <div className="flex items-center justify-between"><label htmlFor="call-integration-control">Разрешить управление записью</label><Switch id="call-integration-control" checked={integration?.allow_control ?? false} disabled={busy || !integration?.enabled} onCheckedChange={allow_control => void updateIntegration({ allow_control })} /></div>
      {integration?.credential_file && <p className="text-xs text-gray-600 break-all">Файл доступа: {integration.credential_file}</p>}
      <Button variant="outline" disabled={busy || !integration} onClick={() => void save(async () => { await invoke('rotate_integration_keys'); setIntegration(await invoke<IntegrationSettings>('get_integration_settings')); toast.success('Старые ключи доступа отозваны'); })}>Отозвать и обновить ключи</Button>
    </section>
    <section className="rounded-lg border bg-white p-6 shadow-sm space-y-2 text-sm">
      <h3 className="text-lg font-semibold">Что будет записано</h3><p>Микрофон и системный звук всего компьютера, включая другие приложения. Звук не ограничен Zoom или Discord.</p>
      <p>Микрофон: {selectedDevices.micDevice ?? 'По умолчанию'}. Системный звук: {selectedDevices.systemDevice ?? 'По умолчанию'}.</p>
      <p>Распознавание: {transcriptModelConfig.provider}, {transcriptModelConfig.model}. Язык: {transcriptModelConfig.provider === 'parakeet' ? 'автоопределение (Parakeet)' : selectedLanguage}.</p>
      <p className="text-gray-600">Для русского с фиксированным языком выберите Whisper и «Russian (ru)» в настройках распознавания. Устройства, модель и разрешения проверяются перед началом записи.</p>
    </section>
  </div>;
}
