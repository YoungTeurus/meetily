'use client';
import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { ModelConfig } from '@/services/configService';
import { Button } from './ui/button';

interface Diagnostic {
  available: boolean;
  binaryPath: string | null;
  version: string | null;
  authenticated: boolean | null;
  supportsIsolation: boolean;
  message: string;
}

export function CodexCliSettings({ modelConfig, setModelConfig }: {
  modelConfig: ModelConfig;
  setModelConfig: (config: ModelConfig | ((previous: ModelConfig) => ModelConfig)) => void;
}) {
  const [checking, setChecking] = useState(false);
  const [diagnostic, setDiagnostic] = useState<Diagnostic | null>(null);
  const [error, setError] = useState('');
  const request = useRef(0);
  const check = async () => {
    const id = ++request.current;
    setChecking(true); setDiagnostic(null); setError('');
    try {
      const result = await invoke<Diagnostic>('api_check_codex_cli', { binaryPath: modelConfig.codexBinaryPath?.trim() || null });
      if (id === request.current) setDiagnostic(result);
    } catch (cause) { if (id === request.current) setError(String(cause)); }
    finally { if (id === request.current) setChecking(false); }
  };
  return <section className="space-y-3 border-t pt-4">
    <p className="text-sm text-gray-600">Generate summary использует установленный Codex CLI и вашу существующую авторизацию. Транскрипт и заметки отправляются через Codex. Выполните <code>codex login</code> в терминале, если ещё не вошли.</p>
    <label className="block text-sm">Модель Codex (необязательно)
      <input aria-label="Модель Codex" className="mt-1 block w-full rounded border p-2" value={modelConfig.model} placeholder="Модель по умолчанию в Codex" onChange={event => setModelConfig(previous => ({ ...previous, model: event.target.value }))} />
    </label>
    <label className="block text-sm">Путь к Codex CLI (необязательно)
      <input aria-label="Путь к Codex CLI" className="mt-1 block w-full rounded border p-2" value={modelConfig.codexBinaryPath ?? ''} placeholder="Автоматическое обнаружение" onChange={event => { ++request.current; setChecking(false); setDiagnostic(null); setError(''); setModelConfig(previous => ({ ...previous, codexBinaryPath: event.target.value })); }} />
    </label>
    <Button variant="outline" disabled={checking} onClick={() => void check()}>{checking ? 'Проверяем Codex…' : 'Проверить Codex CLI'}</Button>
    {diagnostic && <div role="status" className="text-sm space-y-1">
      <p>{diagnostic.message}</p>
      {diagnostic.binaryPath && <p className="break-all">{diagnostic.binaryPath}</p>}
      {diagnostic.version && <p>{diagnostic.version}</p>}
      <p>{diagnostic.authenticated === true ? 'Авторизация найдена' : diagnostic.authenticated === false ? 'Требуется codex login' : 'Авторизацию не удалось проверить'}</p>
      {diagnostic.available && !diagnostic.supportsIsolation && <p>Обновите Codex CLI: эта версия не поддерживает необходимый режим запуска.</p>}
    </div>}
    {error && <p role="alert" className="text-sm text-red-700">{error}</p>}
  </section>;
}
