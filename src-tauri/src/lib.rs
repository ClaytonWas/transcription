use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::fs;
use std::path::PathBuf;
use sysinfo::System;
use tauri::Emitter;
use tokio::io::AsyncWriteExt;
use std::process::Command as StdCommand;
use std::process::Child as StdChild;
use std::sync::Mutex;

#[derive(Serialize, Deserialize)]
struct GpuStatus {
    gpu_name: String,
    is_iris_xe: bool,
    status: String,
}

#[derive(Serialize, Deserialize)]
struct PowerStatus {
    power_now_mw: f64,
    power_now_w: f64,
}

#[derive(Serialize, Deserialize)]
struct MicPortalStatus {
    portal_running: bool,
    portal_gtk_running: bool,
    pipewire_running: bool,
    message: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub percent: f32,
    pub status: String,
}

#[derive(Serialize, Deserialize)]
pub struct BinaryStatus {
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub version: Option<String>,
}

// Shared recorder state for long-running system recordings
struct RecorderProcess {
    child: StdChild,
    path: PathBuf,
}

struct RecorderState {
    current: Mutex<Option<RecorderProcess>>,
}

// Live chunked recording state (30s segments)
use std::sync::Arc;

struct ChunkedRecorderState {
    active: Arc<Mutex<bool>>,
    chunk_index: Arc<Mutex<usize>>,
    base_dir: Arc<Mutex<Option<PathBuf>>>,
    transcripts: Arc<Mutex<Vec<String>>>,
    ffmpeg_pid: Arc<Mutex<Option<u32>>>,
}

/// Get the app data directory for storing binaries
fn get_binaries_dir() -> Result<PathBuf, String> {
    let data_dir = dirs::data_local_dir()
        .ok_or("Could not find local data directory")?
        .join("last-gen-notes")
        .join("binaries");
    
    fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create binaries directory: {}", e))?;
    
    Ok(data_dir)
}

/// Check if a binary is installed and valid
#[tauri::command]
async fn check_binary_status(binary_name: String) -> Result<BinaryStatus, String> {
    let binaries_dir = get_binaries_dir()?;
    
    let binary_path = if cfg!(target_os = "windows") {
        binaries_dir.join(format!("{}.exe", binary_name))
    } else {
        binaries_dir.join(&binary_name)
    };
    
    let installed = binary_path.exists();
    
    Ok(BinaryStatus {
        name: binary_name,
        installed,
        path: if installed { Some(binary_path.to_string_lossy().to_string()) } else { None },
        version: None, // Could run --version to get this
    })
}

/// Download whisper.cpp binary from GitHub releases
#[tauri::command]
async fn download_whisper(window: tauri::Window) -> Result<String, String> {
    let binaries_dir = get_binaries_dir()?;
    
    // Determine platform-specific download
    let (download_url, expected_sha256, archive_name) = if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            (
                "https://github.com/ggerganov/whisper.cpp/releases/download/v1.8.2/whisper-bin-x64.zip",
                "b1514ebc099765e39fa37eb780b92a140a94c86bb0b3b3d98226b38825979732",
                "whisper-bin-x64.zip"
            )
        } else {
            (
                "https://github.com/ggerganov/whisper.cpp/releases/download/v1.8.2/whisper-bin-Win32.zip",
                "49244b4d13cc95f2f27a0098809a8514a835929fa0d24d1a8db6b9073650ba96",
                "whisper-bin-Win32.zip"
            )
        }
    } else if cfg!(target_os = "linux") {
        // Linux - we'll need to build from source or use a custom release
        // For now, point to a hypothetical Linux release
        return Err("Linux binary not available from official releases. Please build from source or use whisper.cpp AppImage.".to_string());
    } else if cfg!(target_os = "macos") {
        (
            "https://github.com/ggerganov/whisper.cpp/releases/download/v1.8.2/whisper-v1.8.2-xcframework.zip",
            "3ffeec1df254d908f01ee3d87bf0aedb8fbc8f29cbf50dc8702741bb85381385",
            "whisper-xcframework.zip"
        )
    } else {
        return Err("Unsupported platform".to_string());
    };
    
    let archive_path = binaries_dir.join(archive_name);
    
    // Download with progress
    emit_progress(&window, 0, None, "Starting download...");
    
    let client = reqwest::Client::new();
    let response = client.get(download_url)
        .send()
        .await
        .map_err(|e| format!("Download request failed: {}", e))?;
    
    let total_size = response.content_length();
    let mut downloaded: u64 = 0;
    
    let mut file = tokio::fs::File::create(&archive_path)
        .await
        .map_err(|e| format!("Failed to create file: {}", e))?;
    
    let mut stream = response.bytes_stream();
    use futures_util::StreamExt;
    
    let mut hasher = Sha256::new();
    
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;
        
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Failed to write chunk: {}", e))?;
        
        hasher.update(&chunk);
        downloaded += chunk.len() as u64;
        
        let percent = total_size.map(|t| (downloaded as f32 / t as f32) * 100.0).unwrap_or(0.0);
        emit_progress(&window, downloaded, total_size, &format!("Downloading... {:.1}%", percent));
    }
    
    file.flush().await.map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);
    
    // Verify SHA256
    emit_progress(&window, downloaded, total_size, "Verifying checksum...");
    let hash = hex::encode(hasher.finalize());
    
    if hash != expected_sha256 {
        fs::remove_file(&archive_path).ok();
        return Err(format!("Checksum mismatch! Expected: {}, Got: {}", expected_sha256, hash));
    }
    
    // Extract archive
    emit_progress(&window, downloaded, total_size, "Extracting...");
    extract_zip(&archive_path, &binaries_dir)?;
    
    // Clean up archive
    fs::remove_file(&archive_path).ok();
    
    // Make binary executable on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let whisper_path = binaries_dir.join("main");
        if whisper_path.exists() {
            let mut perms = fs::metadata(&whisper_path)
                .map_err(|e| format!("Failed to get permissions: {}", e))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&whisper_path, perms)
                .map_err(|e| format!("Failed to set permissions: {}", e))?;
        }
    }
    
    emit_progress(&window, downloaded, total_size, "Complete!");
    
    Ok(binaries_dir.to_string_lossy().to_string())
}

fn extract_zip(archive_path: &PathBuf, dest_dir: &PathBuf) -> Result<(), String> {
    let file = fs::File::open(archive_path)
        .map_err(|e| format!("Failed to open archive: {}", e))?;
    
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("Failed to read zip archive: {}", e))?;
    
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)
            .map_err(|e| format!("Failed to read archive entry: {}", e))?;
        
        let outpath = match file.enclosed_name() {
            Some(path) => dest_dir.join(path),
            None => continue,
        };
        
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).ok();
        } else {
            if let Some(p) = outpath.parent() {
                fs::create_dir_all(p).ok();
            }
            let mut outfile = fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create extracted file: {}", e))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file: {}", e))?;
        }
    }
    
    Ok(())
}

fn emit_progress(window: &tauri::Window, downloaded: u64, total: Option<u64>, status: &str) {
    let percent = total.map(|t| (downloaded as f32 / t as f32) * 100.0).unwrap_or(0.0);
    let _ = window.emit("download-progress", DownloadProgress {
        downloaded,
        total,
        percent,
        status: status.to_string(),
    });
}

/// Get the path to a downloaded binary
#[tauri::command]
async fn get_binary_path(binary_name: String) -> Result<String, String> {
    let binaries_dir = get_binaries_dir()?;
    
    let binary_path = if cfg!(target_os = "windows") {
        binaries_dir.join(format!("{}.exe", binary_name))
    } else {
        binaries_dir.join(&binary_name)
    };
    
    if binary_path.exists() {
        Ok(binary_path.to_string_lossy().to_string())
    } else {
        Err(format!("Binary '{}' not found", binary_name))
    }
}

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

/// Detects Intel GPU and checks if it's Iris Xe
#[tauri::command]
async fn detect_gpu() -> Result<GpuStatus, String> {
    let mut sys = System::new_all();
    sys.refresh_all();

    // Try to read GPU info from /sys/class/drm on Linux
    let gpu_name = if cfg!(target_os = "linux") {
        read_linux_gpu_name().unwrap_or_else(|_| "Unknown GPU".to_string())
    } else {
        "Unknown GPU (non-Linux)".to_string()
    };

    let is_iris_xe = gpu_name.to_lowercase().contains("iris xe");
    let status = if is_iris_xe {
        "Ready".to_string()
    } else if gpu_name.to_lowercase().contains("intel") {
        "Intel GPU detected, but not Iris Xe".to_string()
    } else {
        "Not Ready - Intel Iris Xe not detected".to_string()
    };

    Ok(GpuStatus {
        gpu_name,
        is_iris_xe,
        status,
    })
}

/// Read GPU name from Linux using lspci
fn read_linux_gpu_name() -> Result<String, std::io::Error> {
    // Use lspci to get GPU info directly
    use std::process::Command;
    
    let output = Command::new("lspci")
        .arg("-v")
        .output();
    
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Look for VGA controller line
        for line in stdout.lines() {
            if line.contains("VGA") && line.contains("Intel") {
                // Extract the GPU name after the colon
                if let Some(name_part) = line.split(':').nth(2) {
                    let gpu_name = name_part.trim().to_string();
                    
                    // Check if it's Iris Xe (Alder Lake-P, Tiger Lake, etc.)
                    if gpu_name.contains("Iris Xe") || 
                       gpu_name.contains("Alder Lake") || 
                       gpu_name.contains("Tiger Lake") ||
                       gpu_name.contains("DG1") {
                        return Ok(gpu_name);
                    }
                    
                    return Ok(gpu_name);
                }
            }
        }
    }
    
    // Fallback to sysinfo
    Ok("Intel Integrated Graphics".to_string())
}

/// Monitor battery power consumption on Linux
#[tauri::command]
async fn get_power_status() -> Result<PowerStatus, String> {
    if cfg!(target_os = "linux") {
        let power_now_path = "/sys/class/power_supply/BAT0/power_now";
        
        match fs::read_to_string(power_now_path) {
            Ok(content) => {
                let power_uw: f64 = content
                    .trim()
                    .parse()
                    .map_err(|e| format!("Failed to parse power value: {}", e))?;
                
                // Convert from microwatts to milliwatts and watts
                let power_now_mw = power_uw / 1000.0;
                let power_now_w = power_uw / 1_000_000.0;

                Ok(PowerStatus {
                    power_now_mw,
                    power_now_w,
                })
            }
            Err(e) => Err(format!("Failed to read power status: {}", e)),
        }
    } else {
        Err("Power monitoring only supported on Linux".to_string())
    }
}

/// Get the full path to a recording file in app cache
#[tauri::command]
async fn get_recording_path(filename: String) -> Result<String, String> {
    let cache_dir = dirs::cache_dir()
        .ok_or("Could not find cache directory")?
        .join("last-gen-notes");
    
    fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create cache directory: {}", e))?;
    
    let path = cache_dir.join(&filename);
    Ok(path.to_string_lossy().to_string())
}

/// Cleanup helper: kill recorder processes and clear cached wav chunks
#[tauri::command]
async fn cleanup_recorders_and_cache() -> Result<String, String> {
    // Kill ffmpeg/arecord best-effort
    let _ = StdCommand::new("pkill").arg("ffmpeg").output();
    let _ = StdCommand::new("pkill").arg("arecord").output();

    let cache_base = dirs::cache_dir()
        .ok_or("Could not find cache directory")?
        .join("last-gen-notes");

    // Remove live-session wavs and sys-recording wavs
    let live_dir = cache_base.join("live-session");
    if live_dir.exists() {
        let _ = std::fs::remove_dir_all(&live_dir);
        let _ = std::fs::create_dir_all(&live_dir);
    }
    if cache_base.exists() {
        for entry in std::fs::read_dir(&cache_base).unwrap_or_else(|_| std::fs::read_dir("/dev/null").unwrap()) {
            if let Ok(ent) = entry {
                let path = ent.path();
                if let Some(ext) = path.extension() {
                    if ext == "wav" {
                        let _ = std::fs::remove_file(&path);
                    }
                }
            }
        }
    }

    Ok("Cache cleared and recorder processes signaled".to_string())
}

/// Check if the Linux portals/pipewire needed for mic prompts are running
#[tauri::command]
async fn check_mic_portal() -> Result<MicPortalStatus, String> {
    fn is_running(name: &str) -> bool {
        StdCommand::new("pgrep").arg(name).output().map(|o| o.status.success()).unwrap_or(false)
    }

    let portal = is_running("xdg-desktop-portal");
    let portal_gtk = is_running("xdg-desktop-portal-gtk");
    let pipewire = is_running("pipewire");

    let message = if portal && pipewire {
        "Portal and pipewire are running".to_string()
    } else {
        let mut missing = vec![];
        if !portal { missing.push("xdg-desktop-portal"); }
        if !portal_gtk { missing.push("xdg-desktop-portal-gtk"); }
        if !pipewire { missing.push("pipewire"); }
        format!("Missing: {}", missing.join(", "))
    };

    Ok(MicPortalStatus {
        portal_running: portal,
        portal_gtk_running: portal_gtk,
        pipewire_running: pipewire,
        message,
    })
}

/// Record audio via system arecord for 10 seconds and return the file path
#[tauri::command]
async fn record_system_audio() -> Result<String, String> {
    // Ensure cache dir exists
    let cache_dir = dirs::cache_dir()
        .ok_or("Could not find cache directory")?
        .join("last-gen-notes");
    fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create cache directory: {}", e))?;

    // Simple unique filename using system time
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("time error: {}", e))?
        .as_secs();
    let outfile = cache_dir.join(format!("sys-recording-{}.wav", ts));

    // arecord command: 16-bit PCM, mono, 16kHz, duration 10s
    let status = StdCommand::new("arecord")
        .arg("-f").arg("S16_LE")
        .arg("-r").arg("16000")
        .arg("-c").arg("1")
        .arg("-d").arg("10")
        .arg(outfile.to_string_lossy().to_string())
        .status()
        .map_err(|e| format!("Failed to start arecord: {}", e))?;

    if !status.success() {
        return Err("arecord did not complete successfully".to_string());
    }

    Ok(outfile.to_string_lossy().to_string())
}

/// Start long system recording (until stopped). Returns output path.
#[tauri::command]
async fn start_system_recording(state: tauri::State<'_, RecorderState>) -> Result<String, String> {
    if state.current.lock().unwrap().is_some() {
        return Err("Recording already in progress".into());
    }

    let cache_dir = dirs::cache_dir()
        .ok_or("Could not find cache directory")?
        .join("last-gen-notes");
    fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create cache directory: {}", e))?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| format!("time error: {}", e))?
        .as_secs();
    let outfile = cache_dir.join(format!("sys-recording-{}.wav", ts));

    let child = StdCommand::new("arecord")
        .arg("-f").arg("S16_LE")
        .arg("-r").arg("16000")
        .arg("-c").arg("1")
        .arg(outfile.to_string_lossy().to_string())
        .spawn()
        .map_err(|e| format!("Failed to start arecord: {}", e))?;

    *state.current.lock().unwrap() = Some(RecorderProcess { child, path: outfile.clone() });
    Ok(outfile.to_string_lossy().to_string())
}

/// Stop long system recording. Returns path to recorded file.
#[tauri::command]
async fn stop_system_recording(state: tauri::State<'_, RecorderState>) -> Result<String, String> {
    let proc = {
        let mut guard = state.current.lock().unwrap();
        guard.take()
    };
    
    if let Some(mut proc) = proc {
        // Try to send SIGTERM via pkill (more reliable than child.kill on some systems)
        let pid = proc.child.id();
        let _ = StdCommand::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output();
        
        // Wait for process to exit
        let _ = proc.child.wait();
        
        // Give OS time to flush buffers and finalize the file
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        
        // Verify file exists and has content
        match std::fs::metadata(&proc.path) {
            Ok(meta) if meta.len() > 44 => {
                // WAV header is 44 bytes; file is valid if larger
                return Ok(proc.path.to_string_lossy().to_string());
            }
            _ => {
                return Err(format!(
                    "Recording file invalid or empty at: {}",
                    proc.path.display()
                ));
            }
        }
    }
    Err("No recording in progress".into())
}

/// Transcribe audio file using whisper-cli
#[tauri::command]
async fn transcribe_audio(window: tauri::Window, audio_path: String) -> Result<String, String> {
    // Emit start debug with file size if possible
    let size = std::fs::metadata(&audio_path).map(|m| m.len()).unwrap_or(0);
    let _ = window.emit("transcribe-start", serde_json::json!({
        "path": audio_path.clone(),
        "size": size,
    }));

    match transcribe_audio_internal(&audio_path).await {
        Ok(text) => {
            let _ = window.emit("transcribe-complete", serde_json::json!({
                "path": audio_path,
                "ok": true,
            }));
            Ok(text)
        }
        Err(e) => {
            let _ = window.emit("transcribe-complete", serde_json::json!({
                "path": audio_path,
                "ok": false,
                "error": e,
            }));
            Err(e)
        }
    }
}

/// Start live chunked recording (default 30s segments with auto-transcription)
#[tauri::command]
fn start_live_recording(
    state: tauri::State<'_, ChunkedRecorderState>,
    app: tauri::AppHandle,
    preferred_recorder: Option<String>,
    segment_seconds: Option<u64>,
) -> Result<String, String> {
    let _ = preferred_recorder; // Mark parameter as intentionally used
    let mut active = state.active.lock().unwrap();
    if *active {
        return Err("Live recording already in progress".into());
    }
    
    let cache_dir = dirs::cache_dir()
        .ok_or("Could not find cache directory")?
        .join("last-gen-notes")
        .join("live-session");
    if cache_dir.exists() {
        let _ = fs::remove_dir_all(&cache_dir);
    }
    fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create cache directory: {}", e))?;
    
    *active = true;
    *state.chunk_index.lock().unwrap() = 0;
    *state.base_dir.lock().unwrap() = Some(cache_dir.clone());
    state.transcripts.lock().unwrap().clear();
    drop(active);
    
    // Clone Arc references for the background task
    let active_clone = state.active.clone();
    let chunk_index_clone = state.chunk_index.clone();
    let base_dir_clone = state.base_dir.clone();
    let transcripts_clone = state.transcripts.clone();
    
    // Clamp segment length to a safe range to avoid overly short or long files
    let segment_len = segment_seconds.unwrap_or(10).max(5).min(60);

    // Decide method: prefer arecord for reliability; use ffmpeg only if explicitly requested
    let prefer = preferred_recorder.unwrap_or_else(|| "auto".to_string());
    let has_ff = has_ffmpeg();
    let use_ffmpeg = match prefer.as_str() {
        "ffmpeg" => has_ff,
        "arecord" => false,
        _ => false,  // "auto" defaults to arecord (more reliable); ffmpeg has timing issues
    };

    if use_ffmpeg {
        let base_dir_for_ff = cache_dir.clone();
        let pid_holder = state.ffmpeg_pid.clone();
        // spawn ffmpeg process once to segment into files
        // Use -segment_time to create exact-length segments
        // Note: ffmpeg may create partial first segment before filling up to segment_time
        let child = StdCommand::new("ffmpeg")
            .arg("-hide_banner")
            .arg("-loglevel").arg("error")
            .arg("-f").arg("alsa")
            .arg("-i").arg("default")
            .arg("-ac").arg("1")
            .arg("-ar").arg("16000")
            .arg("-f").arg("segment")
            .arg("-segment_time").arg(segment_len.to_string())
            .arg("-reset_timestamps").arg("1")
            .arg("-segment_start_number").arg("0")
            .arg(base_dir_for_ff.join("chunk-%04d.wav").to_string_lossy().to_string())
            .spawn()
            .map_err(|e| format!("Failed to start ffmpeg: {}", e))?;
        *pid_holder.lock().unwrap() = Some(child.id());

        // Emit recorder mode to frontend
        let _ = app.emit("live-recorder-mode", "ffmpeg");

        // Spawn background task to watch and transcribe segments
        tauri::async_runtime::spawn(async move {
            let _ = chunked_recording_loop_ffmpeg(
                active_clone,
                chunk_index_clone,
                base_dir_clone,
                transcripts_clone,
                app,
                segment_len
            ).await;
        });
    } else {
        // Fallback to arecord per-chunk
        let _ = app.emit("live-recorder-mode", "arecord");
        tauri::async_runtime::spawn(async move {
            let _ = chunked_recording_loop(
                active_clone,
                chunk_index_clone,
                base_dir_clone,
                transcripts_clone,
                app,
                segment_len
            ).await;
        });
    }
    
    Ok(cache_dir.to_string_lossy().to_string())
}
/// Get current recorder mode (ffmpeg/arecord/inactive)
#[tauri::command]
async fn get_recorder_mode(state: tauri::State<'_, ChunkedRecorderState>) -> Result<String, String> {
    if *state.active.lock().unwrap() {
        if state.ffmpeg_pid.lock().unwrap().is_some() {
            Ok("ffmpeg".to_string())
        } else {
            Ok("arecord".to_string())
        }
    } else {
        Ok("inactive".to_string())
    }
}

/// Stop live chunked recording
#[tauri::command]
fn stop_live_recording(state: tauri::State<'_, ChunkedRecorderState>) -> Result<String, String> {
    let mut active = state.active.lock().unwrap();
    if !*active {
        return Err("No live recording in progress".into());
    }
    *active = false;
    drop(active);

    // If ffmpeg is running, terminate it (non-blocking to avoid deadlock)
    if let Some(pid) = *state.ffmpeg_pid.lock().unwrap() {
        let _ = StdCommand::new("kill").arg("-TERM").arg(pid.to_string()).output();
        *state.ffmpeg_pid.lock().unwrap() = None;
        // Background loop will check active flag and exit cleanly
    }
    
    let transcripts = state.transcripts.lock().unwrap().clone();
    Ok(transcripts.join(" "))
}

/// Get accumulated live transcripts
#[tauri::command]
async fn get_live_transcripts(state: tauri::State<'_, ChunkedRecorderState>) -> Result<Vec<String>, String> {
    Ok(state.transcripts.lock().unwrap().clone())
}

/// Chunked recording loop - records 30s segments and transcribes each
async fn chunked_recording_loop(
    active: Arc<Mutex<bool>>,
    chunk_index: Arc<Mutex<usize>>,
    base_dir: Arc<Mutex<Option<PathBuf>>>,
    transcripts: Arc<Mutex<Vec<String>>>,
    app: tauri::AppHandle,
    segment_len: u64,
) -> Result<(), String> {
    loop {
        let is_active = *active.lock().unwrap();
        if !is_active {
            break;
        }
        
        let chunk_idx = {
            let mut idx = chunk_index.lock().unwrap();
            let current = *idx;
            *idx += 1;
            current
        };
        
        let base_dir_path = base_dir.lock().unwrap().clone()
            .ok_or("Base dir not set")?;
        let chunk_file = base_dir_path.join(format!("chunk-{:04}.wav", chunk_idx));
        
        // Record chunk: add 3 seconds to capture leading context from previous chunk
        // This ensures we don't lose content at chunk boundaries
        let record_duration = segment_len + 3;
        let output = StdCommand::new("arecord")
            .arg("-f").arg("S16_LE")
            .arg("-r").arg("16000")
            .arg("-c").arg("1")
            .arg("-d").arg(record_duration.to_string())
            .arg(chunk_file.to_string_lossy().to_string())
            .output()
            .map_err(|e| format!("Failed to record chunk: {}", e))?;
        
        if !output.status.success() {
            let _ = app.emit("live-recording-error", "Chunk recording failed");
            break;
        }

        // Note: We record segment_len + 3 seconds to capture startup delay and previous context.
        // Do NOT trim - all audio is needed to avoid gaps in transcription.
        
        // Verify chunk file and spawn transcription in background to avoid blocking
        if chunk_file.exists() {
            let size = std::fs::metadata(&chunk_file).map(|m| m.len()).unwrap_or(0);
            let chunk_path = chunk_file.to_string_lossy().to_string();
            let transcripts_clone = transcripts.clone();
            let app_clone = app.clone();
            
            // Spawn transcription in background so we can immediately start next recording
            tauri::async_runtime::spawn(async move {
                match transcribe_audio_internal(&chunk_path).await {
                    Ok(text) => {
                        transcripts_clone.lock().unwrap().push(text.clone());
                        let _ = app_clone.emit("live-transcript-chunk", serde_json::json!({
                            "chunk": chunk_idx,
                            "text": text,
                            "path": chunk_path,
                            "size": size
                        }));
                    }
                    Err(e) => {
                        let _ = app_clone.emit("live-recording-error", format!("Transcription error: {}", e));
                    }
                }
            });
        }
        
        // Check if still active after processing
        if !*active.lock().unwrap() {
            break;
        }
    }
    
    Ok(())
}

/// Gapless recording watcher using ffmpeg's segment muxer
async fn chunked_recording_loop_ffmpeg(
    active: Arc<Mutex<bool>>,
    chunk_index: Arc<Mutex<usize>>,
    base_dir: Arc<Mutex<Option<PathBuf>>>,
    transcripts: Arc<Mutex<Vec<String>>>,
    app: tauri::AppHandle,
    segment_len: u64,
) -> Result<(), String> {
    loop {
        if !*active.lock().unwrap() { break; }

        let next_idx = {
            let idx = chunk_index.lock().unwrap();
            let current = *idx;
            current
        };

        let base_dir_path = base_dir.lock().unwrap().clone().ok_or("Base dir not set")?;
        let chunk_file = base_dir_path.join(format!("chunk-{next_idx:04}.wav"));

        // Wait until the segment file appears and has reasonable data
        // WAV header is 44 bytes; skip obviously incomplete segments
        let mut waited_ms = 0u64;
        loop {
            if !*active.lock().unwrap() { return Ok(()); }
            
            let file_size = std::fs::metadata(&chunk_file).map(|m| m.len()).unwrap_or(0);
            // Accept file if it exists and is larger than WAV header + minimal audio
            if chunk_file.exists() && file_size > 1000 {
                break;
            }
            
            tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            waited_ms += 200;
            if waited_ms > (segment_len + 10) * 1000 {
                // Timeout; move to next chunk
                let mut idx = chunk_index.lock().unwrap();
                *idx += 1;
                break;
            }
        }

        // Transcribe if file exists and has content
        if !chunk_file.exists() {
            continue;
        }

        // Transcribe
        let chunk_path = chunk_file.to_string_lossy().to_string();
        let size = std::fs::metadata(&chunk_file).map(|m| m.len()).unwrap_or(0);
        match transcribe_audio_internal(&chunk_path).await {
            Ok(text) => {
                transcripts.lock().unwrap().push(text.clone());
                let _ = app.emit("live-transcript-chunk", serde_json::json!({
                    "chunk": next_idx,
                    "text": text,
                    "path": chunk_path,
                    "size": size
                }));
                // advance index after processing
                let mut idx = chunk_index.lock().unwrap();
                *idx += 1;
            }
            Err(e) => {
                let _ = app.emit("live-recording-error", format!("Transcription error: {}", e));
            }
        }
    }
    Ok(())
}

fn has_ffmpeg() -> bool {
    StdCommand::new("which").arg("ffmpeg").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Internal transcription helper (shared logic)
async fn transcribe_audio_internal(audio_path: &str) -> Result<String, String> {
    use std::process::Command;
    
    // Verify file exists and has minimum size
    let file_path = std::path::PathBuf::from(audio_path);
    if !file_path.exists() {
        return Err(format!("Audio file not found: {}", audio_path));
    }
    
    let file_size = std::fs::metadata(&file_path)
        .map_err(|e| format!("Failed to stat file: {}", e))?
        .len();
    
    if file_size < 100 {
        return Err(format!("Audio file too small ({} bytes). Recording may have failed.", file_size));
    }
    
    // Try multiple possible whisper binary locations
    let whisper_candidates = [
        "/home/cwas/Desktop/last-gen-notes/src-tauri/binaries/whisper-cli-x86_64-unknown-linux-gnu",
        "/home/cwas/Desktop/last-gen-notes/src-tauri/target/debug/whisper-cli",
    ];
    
    let whisper_path = whisper_candidates.iter()
        .find(|p| std::path::Path::new(p).exists())
        .ok_or("Whisper binary not found in known locations")?;
    
    let exe_dir = std::env::current_exe()
        .map_err(|e| format!("Failed to get exe path: {}", e))?
        .parent()
        .ok_or("Failed to get parent directory")?
        .to_path_buf();
    
    // Prefer tiny model for speed, fall back to base
    let model_candidates = vec![
        exe_dir.join("../../../models/ggml-tiny.en.bin"),
        exe_dir.join("models/ggml-tiny.en.bin"),
        PathBuf::from("/home/cwas/Desktop/last-gen-notes/models/ggml-tiny.en.bin"),
        exe_dir.join("../../../models/ggml-base.en.bin"),
        exe_dir.join("models/ggml-base.en.bin"),
        PathBuf::from("/home/cwas/Desktop/last-gen-notes/models/ggml-base.en.bin"),
    ];
    
    let model_path = model_candidates.iter()
        .find(|p| p.exists())
        .ok_or("Model not found")?;
    
    // Use 4 threads for faster transcription on multicore CPUs
    let num_threads = std::thread::available_parallelism()
        .map(|p| p.get().min(4))
        .unwrap_or(2);
    
    let output = Command::new(whisper_path)
        .arg("-m")
        .arg(model_path)
        .arg("-f")
        .arg(audio_path)
        .arg("-t")
        .arg(num_threads.to_string())
        .arg("--no-timestamps")
        .output()
        .map_err(|e| format!("Failed to run whisper-cli: {}", e))?;
    
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.is_empty() { 
            stderr.to_string() 
        } else if !stdout.is_empty() { 
            stdout.to_string() 
        } else { 
            format!("Unknown error (exit code: {:?})", output.status.code())
        };
        eprintln!("Whisper error: {}", msg);
        return Err(format!("Whisper failed: {}", msg));
    }
    
    let result = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(result.trim().to_string())
}

/// Summarize text using a local llama.cpp CLI binary and a provided or default model path
#[tauri::command]
fn summarize_text_llama(
    text: String,
    model_path: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
) -> Result<String, String> {
    let llama_candidates = [
        "/home/cwas/Desktop/last-gen-notes/src-tauri/binaries/llama-cli",
        "/home/cwas/Desktop/last-gen-notes/src-tauri/target/debug/llama-cli",
        "/usr/bin/llama-cli",
    ];

    let llama_path = llama_candidates
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .ok_or("llama-cli binary not found in known locations")?;

    // Determine model path: prefer provided, else look in models directory
    let model = if let Some(mp) = model_path {
        std::path::PathBuf::from(mp)
    } else {
        // Try common locations
        let exe_dir = std::env::current_exe()
            .map_err(|e| format!("Failed to get exe path: {}", e))?
            .parent()
            .ok_or("Failed to get parent directory")?
            .to_path_buf();
        let candidates = vec![
            exe_dir.join("../../../models/llm.gguf"),
            exe_dir.join("models/llm.gguf"),
            PathBuf::from("/home/cwas/Desktop/last-gen-notes/models/llm.gguf"),
        ];
        candidates
            .into_iter()
            .find(|p| p.exists())
            .ok_or("Model gguf not found; provide model_path in Settings")?
    };

    let ntok = max_tokens.unwrap_or(256);
    let temp = temperature.unwrap_or(0.7);
    // Use logical CPUs if available via env or fallback to 4
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .to_string();

    let prompt = format!(
        "You are a concise note-taking assistant. Summarize the following transcript into clear bullet points with timestamps if present, avoiding speculation.\n\nTranscript:\n{}\n\nSummary:",
        text
    );

    let output = StdCommand::new(llama_path)
        .arg("-m").arg(&model)
        .arg("-p").arg(&prompt)
        .arg("-n").arg(ntok.to_string())
        .arg("--temp").arg(format!("{:.2}", temp))
        .arg("-t").arg(&threads)
        .output()
        .map_err(|e| format!("Failed to run llama-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let msg = if !stderr.is_empty() { stderr.to_string() } else { stdout.to_string() };
        return Err(format!("llama-cli failed: {}", msg));
    }

    let mut result = String::from_utf8_lossy(&output.stdout).to_string();
    // Best-effort trimming
    result = result.trim().to_string();
    Ok(result)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(RecorderState { current: Mutex::new(None) })
        .manage(ChunkedRecorderState {
            active: Arc::new(Mutex::new(false)),
            chunk_index: Arc::new(Mutex::new(0)),
            base_dir: Arc::new(Mutex::new(None)),
            transcripts: Arc::new(Mutex::new(Vec::new())),
            ffmpeg_pid: Arc::new(Mutex::new(None)),
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            detect_gpu,
            get_power_status,
            check_binary_status,
            download_whisper,
            get_binary_path,
            check_mic_portal,
            record_system_audio,
            start_system_recording,
            stop_system_recording,
            start_live_recording,
            stop_live_recording,
            get_live_transcripts,
            get_recording_path,
            transcribe_audio,
            summarize_text_llama,
            get_recorder_mode,
            cleanup_recorders_and_cache
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
