import { invoke } from '@tauri-apps/api/core';
export interface WhisperVocabulary { global: string | null; max_chars: number }
export interface TranscriptionInfo { transcript_revision: number; summary_stale: boolean }
export interface RetranscriptionJob { job_id: string; meeting_id: string; state: 'running' | 'completed' | 'failed' | 'cancelled'; stage: string; progress_percentage: number; message: string; error?: string; provider?: string; model?: string; language?: string | null; vocabulary_terms?: string | null; effective_vocabulary?: string | null; result?: RetranscriptionResult | null }
export interface RetranscriptionProgress { job_id: string; meeting_id: string; stage: string; progress_percentage: number; message: string }
export interface RetranscriptionResult { job_id: string; meeting_id: string; segments_count?: number; duration_seconds?: number; language?: string | null; transcript_revision?: number; summary_stale?: boolean; warnings?: string[] }
export interface RetranscriptionError { job_id: string; meeting_id: string; error: string; cancelled: boolean }
export const retranscriptionService = {
  getVocabulary: () => invoke<WhisperVocabulary>('get_whisper_vocabulary'),
  saveVocabulary: (vocabulary: string) => invoke<WhisperVocabulary>('save_global_whisper_vocabulary', { vocabulary: vocabulary.trim() || null }),
  getInfo: (meetingId: string) => invoke<TranscriptionInfo>('get_meeting_transcription_info', { meetingId }),
  getStatus: (meetingId: string) => invoke<{ job: RetranscriptionJob | null }>('get_retranscription_status_command', { meetingId }),
  start: (options: { meetingId: string; jobId: string; language: string | null; model: string; provider: 'whisper' | 'parakeet'; vocabularyTerms: string | null; confirmReplace: true }) => invoke<{ meeting_id: string; job_id: string; message: string }>('start_retranscription_command', options),
  cancel: (jobId: string) => invoke('cancel_retranscription_command', { jobId }),
};
