import React, { useState, useCallback, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { save } from '@tauri-apps/plugin-dialog';
import { writeTextFile } from '@tauri-apps/plugin-fs';
import { useSettings } from './SettingsContext';

interface LiveChunk {
  chunk: number;
  text: string;
  timestamp: string;
  path?: string;
  size?: number;
}

interface DetectedTopic {
  keyword: string;
  confidence: number;
}

// Minimal filter - just filler words and very short words
const FILLER_WORDS = new Set(['um', 'uh', 'ah', 'oh', 'er', 'like', 'yeah', 'okay', 'ok', 'so', 'well', 'and', 'the', 'a', 'an', 'i', 'you', 'it', 'is', 'to', 'of', 'in', 'that', 'for', 'on', 'with', 'as', 'be', 'was', 'are', 'this', 'but', 'or', 'at', 'by', 'we', 'they', 'have', 'from']);

export function LiveTranscriberV2() {
  const {
    recorderPreference,
    segmentSeconds,
    minChunksPerTopic,
    contentDensity,
    maxTopics,
    enableLLMSummary,
    ollamaUrl,
    ollamaModel,
    llmMaxTokens,
    llmTemperature,
  } = useSettings();

  const [isRecording, setIsRecording] = useState(false);
  const [chunks, setChunks] = useState<LiveChunk[]>([]);
  const [topics, setTopics] = useState<DetectedTopic[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [startTime, setStartTime] = useState<number | null>(null);
  const [elapsed, setElapsed] = useState<string>('00:00');
  const [summary, setSummary] = useState<string>('');  
  const [isSummarizing, setIsSummarizing] = useState<boolean>(false);
  const [pendingChunk, setPendingChunk] = useState<number | null>(null);  const extractTopics = useCallback((allChunks: LiveChunk[]) => {
    if (allChunks.length < minChunksPerTopic) {
      setTopics([]);
      return;
    }
    const fullText = allChunks.map(c => c.text).join(' ');
    // Clean text: remove punctuation except apostrophes within words
    const cleanText = fullText.toLowerCase().replace(/[^a-z0-9'\s-]/g, ' ').replace(/\s+/g, ' ').trim();
    const words = cleanText.split(' ').filter(w => w.length > 2 && !FILLER_WORDS.has(w));
    
    // Simple word frequency approach - most mentioned words
    const wordFreq = new Map<string, number>();
    words.forEach(w => {
      const cleaned = w.replace(/^['-]+|['-]+$/g, '');
      if (cleaned.length > 2 && !FILLER_WORDS.has(cleaned) && !/^\d+$/.test(cleaned)) {
        wordFreq.set(cleaned, (wordFreq.get(cleaned) || 0) + 1);
      }
    });

    // Also extract bigrams (two-word phrases) that appear multiple times
    const bigrams = new Map<string, number>();
    for (let i = 0; i < words.length - 1; i++) {
      const w1 = words[i].replace(/^['-]+|['-]+$/g, '');
      const w2 = words[i + 1].replace(/^['-]+|['-]+$/g, '');
      if (w1.length > 2 && w2.length > 2 && !FILLER_WORDS.has(w1) && !FILLER_WORDS.has(w2)) {
        const bigram = w1 + ' ' + w2;
        bigrams.set(bigram, (bigrams.get(bigram) || 0) + 1);
      }
    }

    // Combine: use bigrams that appear 2+ times, plus top single words
    const candidates: { phrase: string; score: number }[] = [];
    
    // Add bigrams with count >= 2
    bigrams.forEach((count, bigram) => {
      if (count >= 2) {
        candidates.push({ phrase: bigram, score: count * 2 }); // weight bigrams higher
      }
    });
    
    // Add single words with count >= 2
    wordFreq.forEach((count, word) => {
      if (count >= 2) {
        // Don't add if word is already part of a bigram we're including
        const isInBigram = candidates.some(c => c.phrase.includes(word));
        if (!isInBigram) {
          candidates.push({ phrase: word, score: count });
        }
      }
    });

    // Sort by score and take top N
    candidates.sort((a, b) => b.score - a.score);
    const topCandidates = candidates.slice(0, maxTopics);
    
    const maxScore = topCandidates[0]?.score || 1;
    const detectedTopics = topCandidates.map(({ phrase, score }) => ({
      keyword: phrase,
      confidence: Math.min(0.99, (score / maxScore) * contentDensity),
    }));

    setTopics(detectedTopics);
  }, [minChunksPerTopic, contentDensity, maxTopics]);

  const summarizeTranscript = useCallback(async (textChunks: LiveChunk[]) => {
    const text = textChunks.map(c => c.text).join(' ').trim();
    if (!text) { setSummary(''); return; }
    
    // If no Ollama URL/model configured, fall back to extractive summary
    if (!ollamaUrl || !ollamaModel) {
      const sentences = text.split(/[.!?]+\s+/).filter(s => s.trim().length > 0);
      const words = text.toLowerCase().split(/\s+/).filter(w => w.length > 2 && !FILLER_WORDS.has(w));
      const freq = new Map<string, number>();
      words.forEach(w => freq.set(w, (freq.get(w) || 0) + 1));
      const scored = sentences.map(s => {
        const sw = s.toLowerCase().split(/\s+/);
        const score = sw.reduce((sum, w) => sum + (freq.get(w) || 0), 0) / Math.max(sw.length, 1);
        return { s: s.trim(), score };
      });
      scored.sort((a, b) => b.score - a.score);
      const take = Math.min(5, Math.max(2, Math.floor(scored.length * 0.2)));
      setSummary(scored.slice(0, take).map(x => x.s).join('. ') + (take > 0 ? '.' : ''));
      return;
    }

    // Call Ollama API
    try {
      setIsSummarizing(true);
      setSummary('Connecting to Ollama...');
      
      const response = await fetch(`${ollamaUrl}/api/generate`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          model: ollamaModel,
          prompt: `Summarize this transcript concisely in 2-4 sentences. Focus on the main points and key information.\n\nTranscript:\n${text}\n\nSummary:`,
          stream: false,
          options: {
            num_predict: llmMaxTokens,
            temperature: llmTemperature,
          }
        }),
      });

      if (!response.ok) {
        const errText = await response.text();
        throw new Error(`Ollama error: ${response.status} - ${errText}`);
      }

      const data = await response.json();
      setSummary(data.response?.trim() || 'No summary generated');
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes('Failed to fetch') || msg.includes('NetworkError')) {
        setSummary(`⚠️ Cannot connect to Ollama at ${ollamaUrl}. Is Ollama running?`);
      } else {
        setSummary(`⚠️ ${msg}`);
      }
    } finally {
      setIsSummarizing(false);
    }
  }, [ollamaUrl, ollamaModel, llmMaxTokens, llmTemperature]);

  useEffect(() => {
    let timer: number | undefined;
    if (isRecording && startTime) {
      timer = window.setInterval(() => {
        const secs = Math.floor((Date.now() - startTime) / 1000);
        const m = String(Math.floor(secs / 60)).padStart(2, '0');
        const s = String(secs % 60).padStart(2, '0');
        setElapsed(m + ':' + s);
        // Update pending chunk indicator based on segment timing
        const expectedChunk = Math.floor(secs / segmentSeconds);
        if (expectedChunk > chunks.length) {
          setPendingChunk(expectedChunk);
        }
      }, 1000);
    }
    return () => { if (timer) window.clearInterval(timer); };
  }, [isRecording, startTime, segmentSeconds, chunks.length]);

  useEffect(() => {
    const unlisten = listen<LiveChunk>('live-transcript-chunk', (event) => {
      const chunk = event.payload;
      setChunks((prev) => {
        const updated = [...prev, { ...chunk, timestamp: elapsed }];
        extractTopics(updated);
        return updated;
      });
      setPendingChunk(null);
      setError(null);
    });

    const unlistenError = listen<string>('live-recording-error', (event) => {
      setError(event.payload);
      setPendingChunk(null);
    });

    return () => {
      unlisten.then((fn) => fn());
      unlistenError.then((fn) => fn());
    };
  }, [elapsed, extractTopics]);

  const startRecording = useCallback(async () => {
    try {
      setError(null);
      setChunks([]);
      setTopics([]);
      setSummary('');
      setElapsed('00:00');
      setPendingChunk(0);
      await invoke<string>('start_live_recording', {
        preferred_recorder: recorderPreference,
        segment_seconds: segmentSeconds,
      });
      setIsRecording(true);
      setStartTime(Date.now());
    } catch (e) {
      setError('Failed to start: ' + e);
      setPendingChunk(null);
    }
  }, [recorderPreference, segmentSeconds]);

  const stopRecording = useCallback(async () => {
    try {
      await invoke<string>('stop_live_recording');
      setIsRecording(false);
      setStartTime(null);
      setPendingChunk(null);
      if (chunks.length > 0 && enableLLMSummary) {
        await summarizeTranscript(chunks);
      }
    } catch (e) {
      setError('Failed to stop: ' + e);
      setIsRecording(false);
    }
  }, [chunks, enableLLMSummary, summarizeTranscript]);

  const exportJSON = useCallback(async () => {
    try {
      const data = {
        metadata: {
          duration: chunks.length * segmentSeconds,
          chunks_count: chunks.length,
          created: new Date().toISOString(),
        },
        chunks: chunks.map(c => ({ index: c.chunk, time: c.timestamp, text: c.text })),
        full_text: chunks.map(c => c.text).join(' '),
        topics: topics.map(t => t.keyword),
        summary,
      };
      const json = JSON.stringify(data, null, 2);
      const path = await save({
        defaultPath: 'transcript-' + Date.now() + '.json',
        filters: [{ name: 'JSON', extensions: ['json'] }]
      });
      if (path) await writeTextFile(path, json);
    } catch (e) {
      setError('Export failed: ' + e);
    }
  }, [chunks, segmentSeconds, topics, summary]);

  const copyText = useCallback(() => {
    navigator.clipboard.writeText(chunks.map(c => c.text).join('\n\n'));
  }, [chunks]);

  const clearAll = useCallback(() => {
    setChunks([]);
    setTopics([]);
    setSummary('');
  }, []);

  return (
    <div style={styles.container}>
      <div style={styles.header}>
        <div style={styles.headerLeft}>
          <h1 style={styles.title}>Live Transcription</h1>
          <div style={styles.statusInfo}>
            <span>Encoder: <strong>{recorderPreference}</strong></span>
            <span>Segment: <strong>{segmentSeconds}s</strong></span>
          </div>
        </div>
        <div style={styles.headerRight}>
          <button
            onClick={isRecording ? stopRecording : startRecording}
            style={{
              ...styles.recordButton,
              backgroundColor: isRecording ? '#dc2626' : '#16a34a',
            }}
          >
            {isRecording ? 'Stop' : 'Start'}
          </button>
          {isRecording && (
            <div style={styles.liveIndicator}>
              <span style={styles.liveDot} />
              <span style={styles.timer}>{elapsed}</span>
              {pendingChunk !== null && <span style={styles.processingBadge}>Processing...</span>}
              <button onClick={clearAll} style={styles.resetBtn} title="Clear transcript and reset">Reset</button>
            </div>
          )}
        </div>
      </div>

      {error && <div style={styles.errorBanner}>{error}</div>}

      <div style={styles.mainContent}>
        <div style={styles.transcriptPanel}>
          <div style={styles.panelHeader}>
            <span>Transcript</span>
            {chunks.length > 0 && (
              <span style={styles.chunkCount}>{chunks.length} chunks · {chunks.reduce((sum, c) => sum + c.text.split(' ').length, 0)} words</span>
            )}
          </div>
          <div style={styles.transcriptScroll}>
            {chunks.length === 0 ? (
              <div style={styles.emptyState}>
                <div style={styles.emptyIcon}></div>
                <p>Press Start to begin recording</p>
                <p style={styles.emptyHint}>Transcription appears here in real-time</p>
              </div>
            ) : (
              chunks.map((chunk, idx) => (
                <div key={idx} style={styles.chunkBlock}>
                  <div style={styles.chunkMeta}>#{chunk.chunk + 1} · {chunk.timestamp}</div>
                  <div style={styles.chunkText}>{chunk.text}</div>
                </div>
              ))
            )}
          </div>
          {chunks.length > 0 && !isRecording && (
            <div style={styles.actions}>
              <button onClick={copyText} style={styles.actionBtn}>Copy</button>
              <button onClick={exportJSON} style={styles.actionBtn}>Export</button>
              <button onClick={() => summarizeTranscript(chunks)} style={styles.actionBtn} disabled={isSummarizing}>
                {isSummarizing ? 'Summarizing...' : 'Summarize'}
              </button>
              <button onClick={clearAll} style={styles.actionBtn}>Clear</button>
            </div>
          )}
        </div>

        <div style={styles.sidePanel}>
          <div style={styles.sidePanelSection}>
            <div style={styles.panelHeader}>Topics</div>
            <div style={styles.topicsList}>
              {topics.length === 0 ? (
                <div style={styles.emptySmall}>Topics appear after a few chunks</div>
              ) : (
                topics.map((topic, idx) => (
                  <div key={idx} style={styles.topicItem}>
                    <span>{topic.keyword}</span>
                    <span style={styles.confidence}>{Math.round(topic.confidence * 100)}%</span>
                  </div>
                ))
              )}
            </div>
          </div>
          <div style={styles.sidePanelSection}>
            <div style={styles.panelHeader}>Summary</div>
            <div style={styles.summaryBox}>
              {summary ? summary : <span style={styles.emptySmall}>Click Summarize or enable auto-summary</span>}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: { width: '100%', height: '100vh', backgroundColor: '#0f172a', color: '#e5e7eb', display: 'flex', flexDirection: 'column', overflow: 'hidden' },
  header: { display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '16px 24px', borderBottom: '1px solid #1e293b', backgroundColor: '#111827' },
  headerLeft: { display: 'flex', flexDirection: 'column', gap: '4px' },
  headerRight: { display: 'flex', alignItems: 'center', gap: '16px' },
  title: { margin: 0, fontSize: '1.5em', fontWeight: 700 },
  statusInfo: { display: 'flex', gap: '16px', fontSize: '0.85em', color: '#9ca3af' },
  recordButton: { padding: '12px 24px', fontSize: '1em', fontWeight: 600, color: 'white', border: 'none', borderRadius: '8px', cursor: 'pointer' },
  liveIndicator: { display: 'flex', alignItems: 'center', gap: '8px' },
  liveDot: { width: '10px', height: '10px', borderRadius: '50%', backgroundColor: '#ef4444', animation: 'pulse 1.5s infinite' },
  timer: { fontSize: '1.25em', fontWeight: 600, color: '#ef4444', fontFamily: 'monospace' },
  errorBanner: { padding: '12px 24px', backgroundColor: 'rgba(220, 38, 38, 0.2)', color: '#fca5a5', fontSize: '0.9em' },
  mainContent: { display: 'flex', flex: 1, overflow: 'hidden' },
  transcriptPanel: { flex: 1, display: 'flex', flexDirection: 'column', borderRight: '1px solid #1e293b' },
  panelHeader: { padding: '12px 24px', fontWeight: 600, fontSize: '0.95em', borderBottom: '1px solid #1e293b', backgroundColor: '#111827', display: 'flex', justifyContent: 'space-between', alignItems: 'center' },
  chunkCount: { fontWeight: 400, fontSize: '0.85em', color: '#9ca3af' },
  transcriptScroll: { flex: 1, padding: '16px 24px', overflowY: 'auto', display: 'flex', flexDirection: 'column', gap: '12px' },
  emptyState: { flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', color: '#6b7280' },
  emptyIcon: { display: 'none' },
  emptyHint: { fontSize: '0.85em', color: '#4b5563' },
  emptySmall: { color: '#6b7280', fontSize: '0.9em', fontStyle: 'italic' },
  chunkBlock: { padding: '12px 16px', backgroundColor: '#1e293b', borderRadius: '8px', borderLeft: '3px solid #6366f1' },
  chunkMeta: { fontSize: '0.75em', color: '#6b7280', marginBottom: '6px', fontWeight: 600 },
  chunkText: { fontSize: '0.95em', lineHeight: 1.6 },
  actions: { display: 'flex', gap: '8px', padding: '12px 24px', borderTop: '1px solid #1e293b', backgroundColor: '#111827' },
  actionBtn: { padding: '8px 16px', fontSize: '0.85em', fontWeight: 500, color: '#e5e7eb', backgroundColor: '#1f2937', border: '1px solid #374151', borderRadius: '6px', cursor: 'pointer' },
  sidePanel: { width: '320px', display: 'flex', flexDirection: 'column', backgroundColor: '#111827' },
  sidePanelSection: { display: 'flex', flexDirection: 'column', borderBottom: '1px solid #1e293b' },
  topicsList: { padding: '12px 16px', display: 'flex', flexDirection: 'column', gap: '6px', maxHeight: '200px', overflowY: 'auto' },
  processingBadge: { fontSize: '0.75em', color: '#fbbf24', backgroundColor: 'rgba(251, 191, 36, 0.15)', padding: '4px 8px', borderRadius: '4px', marginLeft: '8px' },
  resetBtn: { marginLeft: '8px', padding: '6px 12px', fontSize: '0.8em', fontWeight: 500, color: '#fbbf24', backgroundColor: 'transparent', border: '1px solid #fbbf24', borderRadius: '6px', cursor: 'pointer' },
  topicItem: { display: 'flex', justifyContent: 'space-between', padding: '8px 12px', backgroundColor: '#1e293b', borderRadius: '6px', fontSize: '0.9em' },
  confidence: { color: '#9ca3af', fontFamily: 'monospace', fontSize: '0.85em' },
  summaryBox: { padding: '12px 16px', fontSize: '0.9em', lineHeight: 1.6, flex: 1, overflowY: 'auto' },
};
