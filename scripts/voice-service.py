"""Local MLX voice service for Avatar3D Live."""

import asyncio
from concurrent.futures import ThreadPoolExecutor
from contextlib import asynccontextmanager
import io
import os
from pathlib import Path

from fastapi import FastAPI, HTTPException, Response
import mlx.core as mx
from mlx_audio.audio_io import write
from mlx_audio.tts import load
from model_fetch import fetch, report
import numpy as np
from pydantic import BaseModel, Field
import uvicorn


MING = "jorgepb96/Ming-3.0-tts-0.5B-4bit"
ZONOS = "mlx-community/ZONOS3-Mini"
VOICES = {MING: "灵小柔", ZONOS: "willow"}
ACTIVE_LANG = "en" if os.environ.get("AVATAR3D_VOICE_LANG", "zh").strip().lower() == "en" else "zh"
ACTIVE_MODEL = ZONOS if ACTIVE_LANG == "en" else MING
MING_FILES = (
    "config.json", "model.safetensors", "tokenizer.json", "tokenizer_config.json",
    "vocab.json", "merges.txt", "special_tokens_map.json", "added_tokens.json",
)
ZONOS_FILES = (
    "config.json", "model.safetensors", "tokenizer.json", "voices.json",
    "codec/config.json", "codec/model.safetensors", "voices/Willow_en-asrm_F_1.embedding.npz",
)


class SpeechRequest(BaseModel):
    model: str
    input: str = Field(min_length=1, max_length=500)
    voice: str | None = None
    response_format: str = "wav"
    ref_audio: str | None = None
    ref_text: str | None = None


def checkpoint(repo, revision, files, model_type):
    path = Path(os.environ["HF_HOME"]) / "tts" / repo.split("/")[-1]
    if not all((path / name).is_file() for name in files):
        fetch(repo, revision=revision, allow_patterns=list(files), local_dir=path)
        report("load")
    return load(str(path), model_type=model_type)


def compile_ming_flow(model):
    flow = model.flowloss.cfm
    original = flow.sample
    compiled = {}

    def sample(noise, c, latent_history, **options):
        sde_noise = options.pop("sde_noise", None)
        if sde_noise is None:
            return original(noise, c, latent_history, **options)
        key = tuple(sorted(options.items()))
        if key not in compiled:
            @mx.compile
            def integrate(noise, condition, history, sde_noise):
                return original(
                    noise, condition, history, sde_noise=sde_noise, **options
                )[0]

            compiled[key] = integrate
        return compiled[key](noise, c, latent_history, sde_noise), []

    flow.sample = sample


def render(models, model_id, text):
    model = models[model_id]
    if model_id == MING:
        mx.random.seed(73)
        results = model.generate(
            text=text, voice=VOICES[MING], lang_code="z",
            ddpm_steps=5, max_length=14, max_tokens=265,
        )
    else:
        results = model.generate(text=text, voice=VOICES[ZONOS], lang_code="en", seed=29)

    parts = [np.asarray(result.audio, dtype=np.float32).reshape(-1) for result in results]
    if not parts:
        raise RuntimeError("The model generated no audio")
    audio = np.concatenate(parts)
    if not audio.size or not np.isfinite(audio).all():
        raise RuntimeError("The model generated invalid audio")
    if model_id == ZONOS:
        # Willow is a quiet ASMR preset. Add gain with peak headroom for live playback.
        peak = np.max(np.abs(audio))
        if peak > 0:
            audio *= min(8.0, 0.85 / peak)
    result = io.BytesIO()
    write(result, audio, model.sample_rate, format="wav")
    return result.getvalue()


def load_voices():
    if ACTIVE_LANG == "en":
        zonos = checkpoint(ZONOS, "c3b1320037be83fdbe57586bd6678573de1789f4", ZONOS_FILES, "zonos3")
        models = {ZONOS: zonos}
        render(models, ZONOS, "Hi, welcome to the stream.")
        return models
    ming = checkpoint(MING, "d06229cac0b9059337ffdc3d5440300bb304e181", MING_FILES, "bailingmm")
    compile_ming_flow(ming)
    models = {MING: ming}
    render(models, MING, "你好，欢迎来到直播间！")
    return models


@asynccontextmanager
async def lifespan(app):
    # Serialize synthesis and keep MLX inference on the same worker thread.
    with ThreadPoolExecutor(max_workers=1) as worker:
        app.state.worker = worker
        app.state.models = await asyncio.get_running_loop().run_in_executor(worker, load_voices)
        yield


app = FastAPI(title="Avatar3D Live Voice", lifespan=lifespan)


@app.get("/v1/models")
async def models():
    return {
        "object": "list",
        "language": ACTIVE_LANG,
        "data": [{"id": ACTIVE_MODEL, "voice": VOICES[ACTIVE_MODEL]}],
    }


@app.post("/v1/audio/speech")
async def speech(request: SpeechRequest):
    if request.model not in app.state.models:
        raise HTTPException(
            400,
            f"Local voice service is active for {ACTIVE_LANG} ({ACTIVE_MODEL}); restart voice service to switch language",
        )
    if request.voice is not None and request.voice != VOICES[request.model]:
        raise HTTPException(400, "This voice is not configured for the selected model")
    if request.ref_audio or request.ref_text or request.response_format != "wav":
        raise HTTPException(400, "Local speech uses the configured voice and WAV output")
    text = request.input.strip()
    if not text:
        raise HTTPException(400, "Speech input is empty")
    audio = await asyncio.get_running_loop().run_in_executor(
        app.state.worker, render, app.state.models, request.model, text
    )
    return Response(audio, media_type="audio/wav")


if __name__ == "__main__":
    uvicorn.run(app, host="127.0.0.1", port=19849, access_log=False)
