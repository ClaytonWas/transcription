# Last Gen Notes

Local live transcription app for 12th Gen Intel Framework laptops with Iris Xe graphics.

## Requirements

- 12th Gen Intel Core with Iris Xe
- Ubuntu/Debian Linux
- `alsa-utils` and `ffmpeg`
- Ollama (optional, for AI summaries)

## Setup

```bash
# Install dependencies
sudo apt install alsa-utils ffmpeg

# Clone and install
git clone <repo-url>
cd last-gen-notes
npm install

# Download whisper model
mkdir -p models
wget -P models/ https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin

# Build whisper-cli
git clone https://github.com/ggerganov/whisper.cpp
cd whisper.cpp && make -j && cd ..
mkdir -p src-tauri/binaries
cp whisper.cpp/build/bin/whisper-cli src-tauri/binaries/whisper-cli-x86_64-unknown-linux-gnu

# Run
npm run tauri dev
```

## Ollama (optional)

```bash
curl -fsSL https://ollama.com/install.sh | sh
ollama pull phi3:mini
```

## License

MIT
