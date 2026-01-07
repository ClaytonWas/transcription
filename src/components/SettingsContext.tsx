import React, { createContext, useContext, useEffect, useMemo, useState } from 'react';

export interface SettingsState {
  minChunksPerTopic: number;
  minPhraseWords: number;
  maxPhraseWords: number;
  contentDensity: number; // 0..1
  maxTopics: number;
  useTauriExport: boolean;
  enableLLMSummary: boolean;
  ollamaUrl: string;
  ollamaModel: string;
  llmMaxTokens: number;
  llmTemperature: number;
  recorderPreference: 'auto' | 'ffmpeg' | 'arecord';
  segmentSeconds: number;
}

interface SettingsCtx extends SettingsState {
  setMinChunksPerTopic: (v: number) => void;
  setMinPhraseWords: (v: number) => void;
  setMaxPhraseWords: (v: number) => void;
  setContentDensity: (v: number) => void;
  setMaxTopics: (v: number) => void;
  setUseTauriExport: (v: boolean) => void;
  setEnableLLMSummary: (v: boolean) => void;
  setOllamaUrl: (v: string) => void;
  setOllamaModel: (v: string) => void;
  setLlmMaxTokens: (v: number) => void;
  setLlmTemperature: (v: number) => void;
  setRecorderPreference: (v: 'auto' | 'ffmpeg' | 'arecord') => void;
  setSegmentSeconds: (v: number) => void;
}

const DEFAULTS: SettingsState = {
  minChunksPerTopic: 2,
  minPhraseWords: 2,
  maxPhraseWords: 6,
  contentDensity: 0.6,
  maxTopics: 15,
  useTauriExport: true,
  enableLLMSummary: false,
  ollamaUrl: 'http://localhost:11434',
  ollamaModel: 'phi3:mini',
  llmMaxTokens: 256,
  llmTemperature: 0.7,
  recorderPreference: 'auto',
  segmentSeconds: 5,
};

const KEY = 'lastgen.settings.v1';

const SettingsContext = createContext<SettingsCtx | null>(null);

export function SettingsProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<SettingsState>(() => {
    try {
      const raw = localStorage.getItem(KEY);
      if (raw) {
        const parsed = JSON.parse(raw);
        return { ...DEFAULTS, ...parsed } as SettingsState;
      }
    } catch {}
    return DEFAULTS;
  });

  useEffect(() => {
    try {
      localStorage.setItem(KEY, JSON.stringify(state));
    } catch {}
  }, [state]);

  const value: SettingsCtx = useMemo(() => ({
    ...state,
    setMinChunksPerTopic: (v: number) => setState(s => ({ ...s, minChunksPerTopic: Math.max(1, Math.round(v)) })),
    setMinPhraseWords: (v: number) => setState(s => ({ ...s, minPhraseWords: Math.max(1, Math.round(v)) })),
    setMaxPhraseWords: (v: number) => setState(s => ({ ...s, maxPhraseWords: Math.max(s.minPhraseWords, Math.round(v)) })),
    setContentDensity: (v: number) => setState(s => ({ ...s, contentDensity: Math.max(0.1, Math.min(0.95, v)) })),
    setMaxTopics: (v: number) => setState(s => ({ ...s, maxTopics: Math.max(5, Math.round(v)) })),
    setUseTauriExport: (v: boolean) => setState(s => ({ ...s, useTauriExport: v })),
    setEnableLLMSummary: (v: boolean) => setState(s => ({ ...s, enableLLMSummary: v })),
    setOllamaUrl: (v: string) => setState(s => ({ ...s, ollamaUrl: v })),
    setOllamaModel: (v: string) => setState(s => ({ ...s, ollamaModel: v })),
    setLlmMaxTokens: (v: number) => setState(s => ({ ...s, llmMaxTokens: Math.max(64, Math.round(v)) })),
    setLlmTemperature: (v: number) => setState(s => ({ ...s, llmTemperature: Math.max(0.0, Math.min(1.5, v)) })),
    setRecorderPreference: (v: 'auto' | 'ffmpeg' | 'arecord') => setState(s => ({ ...s, recorderPreference: v })),
    setSegmentSeconds: (v: number) => setState(s => ({ ...s, segmentSeconds: Math.max(5, Math.min(60, Math.round(v))) })),
  }), [state]);

  return (
    <SettingsContext.Provider value={value}>
      {children}
    </SettingsContext.Provider>
  );
}

export function useSettings() {
  const ctx = useContext(SettingsContext);
  if (!ctx) throw new Error('useSettings must be used within SettingsProvider');
  return ctx;
}
