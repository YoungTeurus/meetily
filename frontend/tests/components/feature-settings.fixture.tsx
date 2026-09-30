import { afterEach, beforeEach, expect, mock, test } from 'bun:test';
import React, { useState } from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
import type { ModelConfig } from '../../src/services/configService';
const core = { ...await import('@tauri-apps/api/core') };
let settings: any, jobs: any[], failure: boolean;
const invoke = mock(async (command: string, args?: any): Promise<any> => {
  if (command === 'get_automatic_retranscription_settings') return { ...settings };
  if (command === 'set_automatic_retranscription_settings') { if (failure) throw new Error('Save failed'); settings = args.settings; return { ...settings }; }
  if (command === 'get_automatic_retranscription_status') return { jobs };
  if (command === 'cancel_automatic_retranscription') { jobs = jobs.map(job => ({ ...job, state: job.job_id === args.jobId ? 'cancelled' : job.state })); return; }
  if (command === 'whisper_get_available_models') return [{ name: 'large-v3', status: 'Available', size_mb: 2900 }, { name: 'missing', status: 'Missing', size_mb: 500 }];
  if (command === 'parakeet_get_available_models') return [{ name: 'parakeet-test', status: 'Available', size_mb: 670 }];
  if (command === 'api_get_auto_generate_setting') return false;
  if (command === 'api_check_codex_cli') return { available: true, binaryPath: '/bin/codex', version: 'codex 1.0', authenticated: true, supportsIsolation: true, message: 'Ready' };
  throw new Error(command);
});
mock.module('@tauri-apps/api/core', () => ({ ...core, invoke }));
const { AutomaticRetranscriptionSettings } = await import('../../src/components/AutomaticRetranscriptionSettings');
const { CodexCliSettings } = await import('../../src/components/CodexCliSettings');
let config: ModelConfig;
function Codex() { const [value, setValue] = useState<ModelConfig>({ provider: 'codex-cli', model: '', whisperModel: 'base' }); config = value; return <CodexCliSettings modelConfig={value} setModelConfig={setValue} />; }
let renderer: ReactTestRenderer | undefined;
beforeEach(() => { settings = { enabled: false, provider: 'whisper', model: '', language: 'auto' }; jobs = []; failure = false; invoke.mockClear(); });
afterEach(() => { if (renderer) act(() => renderer!.unmount()); renderer = undefined; });
async function mount(view = <AutomaticRetranscriptionSettings />) { await act(async () => { renderer = create(view); }); }
const text = () => JSON.stringify(renderer!.toJSON());
const button = (label: string) => renderer!.root.findAllByType('button').find(node => node.children.includes(label))!;
async function click(label: string) { await act(async () => { await button(label).props.onClick(); }); }
async function change(label: string, value: string | boolean) { await act(async () => { renderer!.root.findByProps({ 'aria-label': label }).props.onChange({ target: typeof value === 'boolean' ? { checked: value } : { value } }); }); }

test('automatic mode defaults off and requires an installed model before enabling', async () => {
  await mount(); expect(renderer!.root.findByProps({ 'aria-label': 'Повторять распознавание после каждого звонка' }).props.checked).toBe(false);
  expect(renderer!.root.findByProps({ 'aria-label': 'Модель автоматического распознавания' }).findAllByType('option').map(option => option.props.value)).toEqual(['', 'whisper:large-v3', 'parakeet:parakeet-test']);
  await change('Повторять распознавание после каждого звонка', true); expect(button('Сохранить автоматическое распознавание').props.disabled).toBe(true);
  expect(invoke.mock.calls.some(([command]) => command === 'set_automatic_retranscription_settings')).toBe(false);
});
test('saves independent Whisper model and Russian language, failure never claims success', async () => {
  await mount(); await change('Повторять распознавание после каждого звонка', true); await change('Модель автоматического распознавания', 'whisper:large-v3'); await change('Язык автоматического распознавания', 'ru'); await click('Сохранить автоматическое распознавание');
  expect(settings).toEqual({ enabled: true, provider: 'whisper', model: 'large-v3', language: 'ru' }); expect(text()).toContain('Настройка сохранена');
  expect(invoke.mock.calls.some(([command]) => command === 'api_save_transcript_config')).toBe(false);
  failure = true; await change('Язык автоматического распознавания', 'en'); await click('Сохранить автоматическое распознавание'); expect(text()).toContain('Save failed'); expect(text()).not.toContain('Настройка сохранена');
});
test('Parakeet forces automatic language and explicitly does not use hints', async () => {
  await mount(); await change('Язык автоматического распознавания', 'ru'); await change('Модель автоматического распознавания', 'parakeet:parakeet-test'); await change('Повторять распознавание после каждого звонка', true); await click('Сохранить автоматическое распознавание');
  expect(settings.language).toBe('auto'); expect(renderer!.root.findByProps({ 'aria-label': 'Язык автоматического распознавания' }).props.disabled).toBe(true); expect(text()).toContain('не использует подсказки');
});
test('missing saved model can still be disabled', async () => {
  settings = { enabled: true, provider: 'whisper', model: 'removed', language: 'ru' }; await mount(); expect(button('Сохранить автоматическое распознавание').props.disabled).toBe(true);
  await change('Повторять распознавание после каждого звонка', false); expect(button('Сохранить автоматическое распознавание').props.disabled).toBe(false); await click('Сохранить автоматическое распознавание'); expect(settings.enabled).toBe(false);
});
test('queue status recovers on mount and cancellation targets the actual job', async () => {
  jobs = [{ job_id: 'job-2', meeting_id: 'meeting/2', state: 'queued', config: { model: 'large-v3' } }, { job_id: 'job-1', meeting_id: 'meeting/1', state: 'completed', config: { model: 'base' }, result: { warnings: ['Database saved; file mirror unavailable'] } }]; await mount(); expect(text()).toContain('В очереди');
  await click('Отменить фоновое распознавание'); expect(invoke.mock.calls).toContainEqual(['cancel_automatic_retranscription', { jobId: 'job-2' }]); expect(text()).toContain('Отменено'); expect(text()).toContain('Database saved; file mirror unavailable');
});
test('Codex allows default model, existing login and explicit executable path without API key', async () => {
  await mount(<Codex />); expect(config.model).toBe(''); await change('Путь к Codex CLI', '/Applications/Codex CLI/codex'); await click('Проверить Codex CLI');
  expect(invoke.mock.calls).toContainEqual(['api_check_codex_cli', { binaryPath: '/Applications/Codex CLI/codex' }]); expect(text()).toContain('Авторизация найдена'); expect(invoke.mock.calls.some(([command]) => command === 'api_get_api_key')).toBe(false);
  await change('Модель Codex', 'gpt-5.4'); expect(config.model).toBe('gpt-5.4');
});

// Exercise the actual shared model editor's save boundary, including explicit
// empty-string path semantics (null means preserve for older callers).
mock.module('../../src/contexts/ConfigContext', () => ({ useConfig: () => ({}) }));
mock.module('../../src/components/Sidebar/SidebarProvider', () => ({ useSidebar: () => ({ serverAddress: null }) }));
mock.module('../../src/contexts/OllamaDownloadContext', () => ({ useOllamaDownload: () => ({ isDownloading: () => false, getProgress: () => null, downloadingModels: new Map() }) }));
mock.module('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
const Box = ({ children, ...props }: any) => <div {...props}>{children}</div>;
mock.module('../../src/components/ui/select', () => ({ Select: Box, SelectContent: Box, SelectItem: Box, SelectTrigger: Box, SelectValue: Box }));
const { ModelSettingsModal } = await import('../../src/components/ModelSettingsModal');
const savedConfig = mock((_config: ModelConfig) => {});
function Editor() { const [value, setValue] = useState<ModelConfig>({ provider: 'codex-cli', model: '', whisperModel: 'base', codexBinaryPath: '/old/codex' }); return <ModelSettingsModal modelConfig={value} setModelConfig={setValue} onSave={savedConfig} skipInitialFetch />; }
test('shared model editor saves an explicitly cleared executable and default model', async () => {
  const storage = new Map<string, string>();
  const oldStorage = globalThis.localStorage;
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: { getItem: (key: string) => storage.get(key) ?? null, setItem: (key: string, value: string) => storage.set(key, value) } });
  try { await mount(<Editor />); await change('Путь к Codex CLI', ''); await click('Save'); expect(savedConfig).toHaveBeenLastCalledWith(expect.objectContaining({ provider: 'codex-cli', model: '', codexBinaryPath: '', apiKey: null })); }
  finally { Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: oldStorage }); }
});
