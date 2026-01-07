import "./App.css";
import { LiveTranscriberV2 } from "./components/LiveTranscriberV2";
import { SettingsProvider, useSettings } from "./components/SettingsContext";
import React from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [showSettings, setShowSettings] = React.useState(false);

  return (
    <SettingsProvider>
      <main className="container">
        <LiveTranscriberV2 />
        
        {/* Floating Settings Button */}
        <button
          onClick={() => setShowSettings(!showSettings)}
          style={styles.settingsButton}
          title="Settings"
        >
          Settings
        </button>

        {/* Settings Modal */}
        {showSettings && (
          <div style={styles.modalOverlay} onClick={() => setShowSettings(false)}>
            <div style={styles.modalContent} onClick={(e: React.MouseEvent) => e.stopPropagation()}>
              <div style={styles.modalHeader}>
                <h2 style={styles.modalTitle}>Settings</h2>
                <button onClick={() => setShowSettings(false)} style={styles.closeButton}>
                  ✕
                </button>
              </div>
              <SettingsModalContent />
            </div>
          </div>
        )}
      </main>
    </SettingsProvider>
  );
}

function SettingsModalContent() {
  const {
    recorderPreference,
    setRecorderPreference,
    segmentSeconds,
    setSegmentSeconds,
      minChunksPerTopic,
      setMinChunksPerTopic,
      minPhraseWords,
      setMinPhraseWords,
      maxPhraseWords,
      setMaxPhraseWords,
      contentDensity,
      setContentDensity,
      maxTopics,
      setMaxTopics,
      useTauriExport,
      setUseTauriExport,
    enableLLMSummary,
    setEnableLLMSummary,
    ollamaUrl,
    setOllamaUrl,
    ollamaModel,
    setOllamaModel,
    llmMaxTokens,
    setLlmMaxTokens,
    llmTemperature,
    setLlmTemperature,
  } = useSettings();

  const [testStatus, setTestStatus] = React.useState<string>('');
  const runMicTest = async () => {
    try {
      setTestStatus('Running mic test…');
      await invoke('start_live_recording', { preferred_recorder: recorderPreference, segment_seconds: 2 });
      await new Promise(r => setTimeout(r, 2500));
      await invoke('stop_live_recording');
      setTestStatus('Mic test complete.');
    } catch (e) {
      setTestStatus(`Mic test failed: ${e}`);
    }
  };

  return (
    <div style={styles.settingsGrid}>
        <div style={styles.sectionTitle}>Recording</div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Recorder</label>
        <select
          value={recorderPreference}
          onChange={(e) => setRecorderPreference(e.target.value as any)}
          style={styles.select}
        >
          <option value="auto">Auto (arecord)</option>
          <option value="arecord">arecord</option>
          <option value="ffmpeg">ffmpeg (experimental)</option>
        </select>
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Segment Length</label>
        <select
          value={segmentSeconds}
          onChange={(e) => setSegmentSeconds(Number(e.target.value))}
          style={styles.select}
        >
          {[5, 10, 15, 20, 30].map((s) => (
            <option key={s} value={s}>{s} seconds</option>
          ))}
        </select>
      </div>

      <div style={styles.sectionTitle}>Topic Extraction</div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Min chunks per topic</label>
        <input
          type="number"
          min={1}
          value={minChunksPerTopic}
          onChange={(e) => setMinChunksPerTopic(Number(e.target.value))}
          style={styles.input}
        />
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Min phrase words</label>
        <input
          type="number"
          min={1}
          value={minPhraseWords}
          onChange={(e) => setMinPhraseWords(Number(e.target.value))}
          style={styles.input}
        />
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Max phrase words</label>
        <input
          type="number"
          min={minPhraseWords}
          value={maxPhraseWords}
          onChange={(e) => setMaxPhraseWords(Number(e.target.value))}
          style={styles.input}
        />
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Content density ({contentDensity.toFixed(2)})</label>
        <input
          type="range"
          min={0.1}
          max={0.95}
          step={0.05}
          value={contentDensity}
          onChange={(e) => setContentDensity(Number(e.target.value))}
          style={styles.slider}
        />
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Max topics</label>
        <input
          type="number"
          min={5}
          value={maxTopics}
          onChange={(e) => setMaxTopics(Number(e.target.value))}
          style={styles.input}
        />
      </div>

      <div style={styles.sectionTitle}>Export</div>
      <div style={styles.settingRow}>
        <label style={styles.label}>
          <input
            type="checkbox"
            checked={useTauriExport}
            onChange={(e) => setUseTauriExport(e.target.checked)}
            style={{ marginRight: '8px' }}
          />
          Use Tauri save dialog
        </label>
      </div>

      <div style={styles.sectionTitle}>Ollama Summary</div>
      <div style={styles.settingRow}>
        <label style={styles.label}>
          <input
            type="checkbox"
            checked={enableLLMSummary}
            onChange={(e) => setEnableLLMSummary(e.target.checked)}
            style={{ marginRight: '8px' }}
          />
          Enable Ollama auto-summary
        </label>
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Ollama URL</label>
        <input
          type="text"
          value={ollamaUrl}
          onChange={(e) => setOllamaUrl(e.target.value)}
          placeholder="http://localhost:11434"
          style={styles.input}
        />
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Model name</label>
        <input
          type="text"
          value={ollamaModel}
          onChange={(e) => setOllamaModel(e.target.value)}
          placeholder="llama3.2:1b, phi3:mini, etc."
          style={styles.input}
        />
        <div style={{ fontSize: '0.8em', color: '#9ca3af', marginTop: '4px' }}>Run: ollama pull {ollamaModel || 'model-name'}</div>
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Max tokens</label>
        <input
          type="number"
          min={64}
          value={llmMaxTokens}
          onChange={(e) => setLlmMaxTokens(Number(e.target.value))}
          style={styles.input}
        />
      </div>
      <div style={styles.settingRow}>
        <label style={styles.label}>Temperature ({llmTemperature.toFixed(2)})</label>
        <input
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={llmTemperature}
          onChange={(e) => setLlmTemperature(Number(e.target.value))}
          style={styles.slider}
        />
      </div>
      <div style={styles.settingRow}>
        <button 
          style={styles.button} 
          onClick={async () => {
            setTestStatus('Testing Ollama connection...');
            try {
              const res = await fetch(`${ollamaUrl}/api/tags`);
              if (!res.ok) throw new Error(`HTTP ${res.status}`);
              const data = await res.json();
              const models = data.models?.map((m: any) => m.name).join(', ') || 'none';
              setTestStatus(`Connected! Models: ${models}`);
            } catch (e) {
              setTestStatus(`Error: Cannot reach Ollama at ${ollamaUrl}. Is it running?`);
            }
          }}
        >
          Test Ollama Connection
        </button>
      </div>

      <div style={styles.sectionTitle}>Tests</div>
      <div style={{ display: 'flex', gap: '8px' }}>
        <button style={styles.button} onClick={runMicTest}>Run 2s Mic Test</button>
      </div>
      {testStatus && <div style={{ fontSize: '0.9em', color: '#9ca3af' }}>{testStatus}</div>}
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  settingsButton: {
    position: 'fixed',
    bottom: '24px',
    right: '24px',
    padding: '12px 20px',
    borderRadius: '8px',
    backgroundColor: '#1f2937',
    border: '1px solid #374151',
    color: '#e5e7eb',
    fontSize: '0.9em',
    fontWeight: 500,
    cursor: 'pointer',
    boxShadow: '0 4px 12px rgba(0, 0, 0, 0.3)',
    transition: 'all 0.2s',
  },
  modalOverlay: {
    position: 'fixed',
    inset: 0,
    backgroundColor: 'rgba(0, 0, 0, 0.75)',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    zIndex: 1000,
  },
  modalContent: {
    backgroundColor: '#111827',
    borderRadius: '16px',
    border: '1px solid #1f2937',
    maxWidth: '500px',
    width: '90%',
    maxHeight: '80vh',
    overflow: 'auto',
  },
  modalHeader: {
    display: 'flex',
    justifyContent: 'space-between',
    alignItems: 'center',
    padding: '24px',
    borderBottom: '1px solid #1f2937',
  },
  modalTitle: {
    margin: 0,
    fontSize: '1.5em',
    fontWeight: 600,
    color: '#e5e7eb',
  },
  closeButton: {
    background: 'none',
    border: 'none',
    color: '#9ca3af',
    fontSize: '1.5em',
    cursor: 'pointer',
    padding: '4px 8px',
  },
  settingsGrid: {
    padding: '24px',
    display: 'flex',
    flexDirection: 'column',
    gap: '20px',
  },
  settingRow: {
    display: 'flex',
    flexDirection: 'column',
    gap: '8px',
  },
  sectionTitle: {
    fontSize: '1em',
    fontWeight: 700,
    color: '#e5e7eb',
    marginTop: '16px',
    paddingBottom: '8px',
    borderBottom: '1px solid #374151',
  },
  label: {
    fontSize: '0.9em',
    fontWeight: 600,
    color: '#9ca3af',
    textTransform: 'uppercase',
    letterSpacing: '0.5px',
  },
  select: {
    padding: '10px 12px',
    backgroundColor: '#1f2937',
    border: '1px solid #374151',
    borderRadius: '8px',
    color: '#e5e7eb',
    fontSize: '1em',
    cursor: 'pointer',
  },
  input: {
    padding: '10px 12px',
    backgroundColor: '#1f2937',
    border: '1px solid #374151',
    borderRadius: '8px',
    color: '#e5e7eb',
    fontSize: '1em',
  },
  slider: {
    width: '100%',
  },
};

export default App;
