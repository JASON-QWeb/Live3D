#!/bin/bash
set -euo pipefail
export PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
: "${AVATAR3D_MODEL_DIR:?missing model service directory}"
mkdir -p "$AVATAR3D_MODEL_DIR"
export AVATAR3D_PROGRESS="$AVATAR3D_MODEL_DIR/progress.json"
progress() { printf '{"stage":"%s"}\n' "$1" > "$AVATAR3D_PROGRESS.tmp" && mv -f "$AVATAR3D_PROGRESS.tmp" "$AVATAR3D_PROGRESS"; }
if ! command -v uv >/dev/null; then echo "MLX 需要 uv。可用 brew install uv 安装。" >&2; exit 1; fi
case "$1" in
  voice) version=voice-0.6.7-0.9.4; packages=('mlx-audio[server]==0.6.7' 'misaki[zh]==0.9.4');;
  chat) version=chat-0.32.2; packages=('mlx-lm==0.32.2');;
  *) exit 2;;
esac
if [[ ! -x "$AVATAR3D_MODEL_DIR/venv/bin/python" ]]; then
  progress env
  uv venv --python 3.12 "$AVATAR3D_MODEL_DIR/venv"
fi
if [[ ! -f "$AVATAR3D_MODEL_DIR/$version.ready" ]]; then
  progress packages
  uv pip install --python "$AVATAR3D_MODEL_DIR/venv/bin/python" "${packages[@]}"
  touch "$AVATAR3D_MODEL_DIR/$version.ready"
fi
export HF_HOME="${AVATAR3D_MODEL_CACHE:-$AVATAR3D_MODEL_DIR/models}"
export PYTHONUNBUFFERED=1
progress load
if [[ "$1" == voice ]]; then
  exec "$AVATAR3D_MODEL_DIR/venv/bin/python" "$AVATAR3D_MODEL_DIR/voice-service.py"
fi
model=mlx-community/Qwen2.5-1.5B-Instruct-4bit
"$AVATAR3D_MODEL_DIR/venv/bin/python" "$AVATAR3D_MODEL_DIR/model_fetch.py" "$model"
progress load
exec "$AVATAR3D_MODEL_DIR/venv/bin/python" -m mlx_lm.server --host 127.0.0.1 --port 19847 --model "$model"
