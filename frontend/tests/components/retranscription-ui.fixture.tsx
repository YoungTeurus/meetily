import { afterEach, beforeEach, describe, expect, mock, test } from 'bun:test';
import React from 'react';
import { act, create, type ReactTestRenderer } from 'react-test-renderer';
const core = { ...await import('@tauri-apps/api/core') };
const events = { ...await import('@tauri-apps/api/event') };
const originalConfig = { ...await import('../../src/contexts/ConfigContext') };
const handlers = new Map<string, (event: { payload: any }) => void>();
let models: { name: string; size_mb: number; status: string }[];
let globalTerms: string | null;
let vocabularyFailure = false;
let stale = true;
let nativeJob: any = null;
const closed = mock((_open: boolean) => {});
const complete = mock(async () => {});
const warning = mock((_title: string, _options?: unknown) => {});
const invoke = mock(async (command: string, args?: Record<string, unknown>): Promise<any> => {
  if (command === 'whisper_get_available_models' || command === 'parakeet_get_available_models') return models.filter(model => (command.startsWith('whisper') ? !model.name.startsWith('parakeet') : model.name.startsWith('parakeet')));
  if (command === 'get_whisper_vocabulary') { if (vocabularyFailure) throw new Error('Settings unavailable'); return { global: globalTerms, max_chars: 1000 }; }
  if (command === 'save_global_whisper_vocabulary') { if (vocabularyFailure) throw new Error('Save failed'); globalTerms = args!.vocabulary as string | null; return { global: globalTerms, max_chars: 1000 }; }
  if (command === 'get_retranscription_status_command') return { job: nativeJob };
  if (command === 'start_retranscription_command') return { job_id: args!.jobId, meeting_id: args!.meetingId, message: 'Started' };
  if (command === 'cancel_retranscription_command') return;
  if (command === 'get_meeting_transcription_info') return { transcript_revision: 2, summary_stale: stale };
  throw new Error(command);
});
mock.module('@tauri-apps/api/core', () => ({ ...core, invoke }));
mock.module('@tauri-apps/api/event', () => ({ ...events, listen: async (name: string, handler: (event: { payload: any }) => void) => { handlers.set(name, handler); return () => { if (handlers.get(name) === handler) handlers.delete(name); }; } }));
mock.module('../../src/contexts/ConfigContext', () => ({ ...originalConfig, useConfig: () => ({ selectedLanguage: 'ru', transcriptModelConfig: { provider: 'localWhisper', model: 'base' } }) }));
mock.module('next/navigation', () => ({ useRouter: () => ({ push: () => {} }) }));
mock.module('sonner', () => ({ toast: { success: () => {}, error: () => {}, warning } }));
const Box = ({ children, ...props }: any) => <div {...props}>{children}</div>;
mock.module('../../src/components/ui/dialog', () => ({ Dialog: Box, DialogContent: Box, DialogDescription: Box, DialogFooter: Box, DialogHeader: Box, DialogTitle: Box }));
const { RetranscribeDialog } = await import('../../src/components/MeetingDetails/RetranscribeDialog');
const { WhisperVocabularySettings } = await import('../../src/components/WhisperVocabularySettings');
const { TranscriptRevisionNotice } = await import('../../src/components/MeetingDetails/TranscriptRevisionNotice');
const { useRetranscriptionRefresh } = await import('../../src/hooks/useRetranscriptionRefresh');
function PageRefresh() { useRetranscriptionRefresh('meeting-1', complete); return <div />; }
let renderer: ReactTestRenderer | undefined;
beforeEach(() => { models = [{ name: 'base', size_mb: 145, status: 'Available' }, { name: 'missing', size_mb: 1500, status: 'Missing' }, { name: 'parakeet-test', size_mb: 670, status: 'Available' }]; globalTerms = 'HRMS, Гермес'; vocabularyFailure = false; stale = true; nativeJob = null; handlers.clear(); invoke.mockClear(); closed.mockClear(); complete.mockClear(); warning.mockClear(); });
afterEach(() => { if (renderer) act(() => renderer!.unmount()); renderer = undefined; });
const mount = async (element = <RetranscribeDialog open onOpenChange={closed} meetingId="meeting-1" meetingFolderPath="/recording" onComplete={complete} />) => { await act(async () => { renderer = create(element); }); };
const button = (text: string) => renderer!.root.findAllByType('button').find(node => node.children.includes(text))!;
const change = async (label: string, value: string | boolean) => { const node = renderer!.root.findByProps({ 'aria-label': label }); await act(async () => { node.props.onChange({ target: typeof value === 'boolean' ? { checked: value } : { value } }); }); };
const click = async (text: string) => { await act(async () => { await button(text).props.onClick(); }); };
const started = () => invoke.mock.calls.find(([command]) => command === 'start_retranscription_command')![1]!;
const emit = async (name: string, payload: any) => { await act(async () => { handlers.get(name)!({ payload }); }); };

describe('Safe retranscription and vocabulary UI', () => {
  test('uses installed models only and requires explicit replacement confirmation', async () => {
    await mount(); const select = renderer!.root.findByProps({ 'aria-label': 'Модель распознавания' });
    expect(select.findAllByType('option').map(node => node.props.value)).toEqual(['whisper:base', 'parakeet:parakeet-test']);
    expect(button('Распознать заново').props.disabled).toBe(true);
    await change('Подтвердить замену транскрипта', true); expect(button('Распознать заново').props.disabled).toBe(false);
    expect(invoke.mock.calls.some(([command]) => command === 'start_retranscription_command')).toBe(false);
  });
  test('missing installed models never falls back to starting an unspecified backend model', async () => {
    models = []; await mount(); await change('Подтвердить замену транскрипта', true);
    expect(button('Распознать заново').props.disabled).toBe(true);
    expect(button('Установить модель в настройках распознавания')).toBeDefined();
    expect(invoke.mock.calls.some(([command]) => command === 'start_retranscription_command')).toBe(false);
  });
  test('reopening attaches to native job progress and its saved hints without starting another operation', async () => {
    nativeJob = { job_id: 'native-job', meeting_id: 'meeting-1', state: 'running', stage: 'transcribing', progress_percentage: 41, message: 'Transcribing', provider: 'whisper', model: 'large-v3', language: 'ru', vocabulary_terms: 'Hermes', effective_vocabulary: 'Hermes, HRMS' };
    await mount(); expect(renderer!.root.findByType('progress').props.value).toBe(41);
    expect(JSON.stringify(renderer!.toJSON())).toContain('Hermes, HRMS');
    expect(invoke.mock.calls.some(([command]) => command === 'start_retranscription_command')).toBe(false);
    await click('Отменить распознавание'); expect(invoke.mock.calls).toContainEqual(['cancel_retranscription_command', { jobId: 'native-job' }]);
  });
  test('shows saved global terms and sends run-only additions with exact engine model and Russian language', async () => {
    await mount(); expect(JSON.stringify(renderer!.toJSON())).toContain('HRMS, Гермес');
    await change('Подсказки для этого запуска', 'Hermes, Яндекс Маркет'); await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
    expect(started()).toMatchObject({ meetingId: 'meeting-1', provider: 'whisper', model: 'base', language: 'ru', vocabularyTerms: 'Hermes, Яндекс Маркет', confirmReplace: true });
    expect(started().jobId).toMatch(/^[0-9a-f-]{36}$/);
    expect(invoke.mock.calls.some(([command]) => command === 'save_global_whisper_vocabulary')).toBe(false);
  });
  test('Parakeet explicitly uses auto language and sends no unsupported hints', async () => {
    await mount(); await change('Подсказки для этого запуска', 'Whisper-only'); await change('Модель распознавания', 'parakeet:parakeet-test');
    expect(renderer!.root.findByProps({ 'aria-label': 'Язык распознавания' }).props.disabled).toBe(true);
    expect(renderer!.root.findAllByProps({ 'aria-label': 'Подсказки для этого запуска' })).toHaveLength(0);
    await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
    expect(started()).toMatchObject({ provider: 'parakeet', language: null, vocabularyTerms: null });
  });
  test('ignores stale jobs, cancels the displayed operation and waits for actual cancelled outcome', async () => {
    await mount(); await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
    await emit('retranscription-progress', { meeting_id: 'meeting-1', job_id: 'older-job', stage: 'transcribing', progress_percentage: 97, message: 'Old' });
    expect(renderer!.root.findByType('progress').props.value).toBe(0);
    await click('Отменить распознавание'); expect(invoke.mock.calls).toContainEqual(['cancel_retranscription_command', { jobId: started().jobId }]);
    expect(closed).not.toHaveBeenCalled(); expect(complete).not.toHaveBeenCalled(); expect(button('Отменяем…').props.disabled).toBe(true);
    await emit('retranscription-error', { meeting_id: 'meeting-1', job_id: started().jobId, error: 'Cancelled', cancelled: true });
    expect(JSON.stringify(renderer!.toJSON())).toContain('Старый транскрипт сохранён'); expect(complete).not.toHaveBeenCalled();
  });
  test('matching successful completion refreshes the meeting once and closes after refresh', async () => {
    await mount(); await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
    const payload = { meeting_id: 'meeting-1', job_id: started().jobId, transcript_revision: 2, summary_stale: true };
    await emit('retranscription-complete', { ...payload, job_id: 'older-job' }); expect(complete).not.toHaveBeenCalled();
    await emit('retranscription-complete', payload); await emit('retranscription-complete', payload);
    expect(complete).toHaveBeenCalledTimes(1); expect(closed).toHaveBeenCalledWith(false);
  });
  test('file mirror warnings still refresh successful database text and explain the remaining issue', async () => {
    await mount(); await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
    await emit('retranscription-complete', { meeting_id: 'meeting-1', job_id: started().jobId, warnings: ['metadata.json could not be updated'], summary_stale: true });
    expect(complete).toHaveBeenCalledTimes(1); expect(closed).toHaveBeenCalledWith(false);
    expect(warning).toHaveBeenCalledWith('Новый текст сохранён в Meetily, но файлы копии требуют внимания', { description: 'metadata.json could not be updated', duration: 8000 });
  });
  test('hidden job completion is recovered once on reopen with retained file warnings', async () => {
    await mount(); await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
    const id = started().jobId;
    await click('Скрыть окно'); expect(closed).toHaveBeenCalledWith(false);
    await act(async () => { renderer!.update(<RetranscribeDialog open={false} onOpenChange={closed} meetingId="meeting-1" meetingFolderPath="/recording" onComplete={complete} />); });
    nativeJob = { job_id: id, meeting_id: 'meeting-1', state: 'completed', stage: 'complete', progress_percentage: 100, message: 'Done', result: { job_id: id, meeting_id: 'meeting-1', summary_stale: true, warnings: ['metadata mirror failed'] } };
    await act(async () => { renderer!.update(<RetranscribeDialog open onOpenChange={closed} meetingId="meeting-1" meetingFolderPath="/recording" onComplete={complete} />); });
    expect(complete).toHaveBeenCalledTimes(1); expect(warning).toHaveBeenCalledTimes(1);
    expect(warning.mock.calls[0]![1]).toMatchObject({ description: 'metadata mirror failed' });
    expect(invoke.mock.calls.filter(([command]) => command === 'start_retranscription_command')).toHaveLength(1);
    expect(closed).toHaveBeenCalledTimes(1); // Only Hide closed it; terminal recovery leaves the new-run form open.
    expect(button('Распознать заново')).toBeDefined();
  });
  test('completion recovered by polling shares warning-aware finalization', async () => {
    const originalInterval = globalThis.setInterval;
    const originalClear = globalThis.clearInterval;
    let poll: (() => Promise<void> | void) | undefined;
    globalThis.setInterval = ((callback: () => Promise<void> | void) => { poll = callback; return 1; }) as any;
    globalThis.clearInterval = (() => {}) as any;
    try {
      await mount(); await change('Подтвердить замену транскрипта', true); await click('Распознать заново');
      const id = started().jobId;
      nativeJob = { job_id: id, meeting_id: 'meeting-1', state: 'completed', stage: 'complete', progress_percentage: 100, message: 'Done', result: { job_id: id, meeting_id: 'meeting-1', summary_stale: true, warnings: ['transcripts mirror failed'] } };
      await act(async () => { await poll!(); });
      expect(complete).toHaveBeenCalledTimes(1); expect(warning.mock.calls[0]![1]).toMatchObject({ description: 'transcripts mirror failed' });
      expect(closed).toHaveBeenCalledWith(false);
    } finally { globalThis.setInterval = originalInterval; globalThis.clearInterval = originalClear; }
  });
  test.each(['failed', 'cancelled'])('reopen recovers terminal %s without starting or refreshing replaced text', async state => {
    nativeJob = { job_id: 'finished-job', meeting_id: 'meeting-1', state, stage: state, progress_percentage: 25, message: 'Stopped', error: 'Decoder failed' };
    await mount(); expect(complete).not.toHaveBeenCalled();
    expect(JSON.stringify(renderer!.toJSON())).toContain(state === 'failed' ? 'Decoder failed' : 'Старый транскрипт сохранён');
    expect(invoke.mock.calls.some(([command]) => command === 'start_retranscription_command')).toBe(false);
  });
  test('page refreshes completed text without an open job dialog and ignores another meeting', async () => {
    await mount(<PageRefresh />);
    await emit('retranscription-complete', { job_id: 'job-other', meeting_id: 'meeting-other' }); expect(complete).not.toHaveBeenCalled();
    const result = { job_id: 'job-1', meeting_id: 'meeting-1' };
    await emit('retranscription-complete', result); await emit('retranscription-complete', result);
    expect(complete).toHaveBeenCalledTimes(1);
  });
  test('page mount refreshes terminal completion delivered before its listener existed', async () => {
    nativeJob = { job_id: 'job-1', meeting_id: 'meeting-1', state: 'completed', stage: 'complete', progress_percentage: 100, message: 'Done', result: { job_id: 'job-1', meeting_id: 'meeting-1' } };
    await mount(<PageRefresh />); expect(complete).toHaveBeenCalledTimes(1);
    await emit('retranscription-complete', nativeJob.result); expect(complete).toHaveBeenCalledTimes(1);
  });
  test('global hints persist only after explicit save and count Unicode code points', async () => {
    await mount(<WhisperVocabularySettings />); expect(invoke.mock.calls.some(([command]) => command === 'save_global_whisper_vocabulary')).toBe(false);
    await change('Общие подсказки', '😀'.repeat(1000)); expect(renderer!.root.findByProps({ 'aria-label': 'Общие подсказки' }).props.value).toBe('😀'.repeat(1000));
    await click('Сохранить подсказки'); expect(globalTerms).toBe('😀'.repeat(1000)); expect(JSON.stringify(renderer!.toJSON())).toContain('Подсказки сохранены');
  });
  test('failed vocabulary loads cannot overwrite unknown saved global terms', async () => {
    vocabularyFailure = true; await mount(<WhisperVocabularySettings />);
    expect(button('Сохранить подсказки').props.disabled).toBe(true); expect(renderer!.root.findByProps({ role: 'alert' }).children.join('')).toContain('Settings unavailable');
    vocabularyFailure = false; await click('Загрузить подсказки ещё раз'); expect(renderer!.root.findByProps({ 'aria-label': 'Общие подсказки' }).props.value).toBe('HRMS, Гермес');
  });
  test('stale summary notice persists after a reload and clears when backend confirms a new summary', async () => {
    await mount(<TranscriptRevisionNotice meetingId="meeting-1" summaryStatus="idle" />); expect(JSON.stringify(renderer!.toJSON())).toContain('Сохранённый конспект относится к прежним данным');
    stale = false; await act(async () => { renderer!.update(<TranscriptRevisionNotice meetingId="meeting-1" summaryStatus="completed" />); }); expect(renderer!.toJSON()).toBeNull();
    stale = true;
    await emit('meeting-notes-updated', { meeting_id: 'other-meeting', revision: 1 }); expect(renderer!.toJSON()).toBeNull();
    await emit('meeting-notes-updated', { meeting_id: 'meeting-1', revision: 1 });
    expect(JSON.stringify(renderer!.toJSON())).toContain('Транскрипт или заметки изменились');
  });
});
